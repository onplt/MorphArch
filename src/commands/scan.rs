// =============================================================================
// commands/scan.rs - Scan command: commit scanning + dependency graph + drift score
// =============================================================================

use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use dashmap::DashMap;
use petgraph::graph::{DiGraph, NodeIndex};
use rayon::prelude::*;
use tracing::{debug, info};

use crate::analysis;
use crate::analysis::SnapshotAnalysisDetail;
use crate::config::ProjectConfig;
use crate::db::Database;
use crate::git_scanner;
use crate::models::{
    CURRENT_ANALYSIS_VERSION, CommitInfo, FileDependencyState, FileImportTarget,
    FilteredExternalSample, GraphCheckpoint, GraphDelta, GraphSnapshot, HeavySnapshotArtifacts,
    RepoScanState, ScanMetadata, SnapshotFrame,
};
use crate::parser;

const SCAN_BATCH_SIZE: usize = 50;
const PARALLEL_PARSE_THRESHOLD: usize = 16;
const CHECKPOINT_INTERVAL: i64 = 20;

fn is_test_path(path: &Path, test_path_patterns: &[String]) -> bool {
    let s = path.to_string_lossy();
    let lower = s.to_ascii_lowercase().replace('\\', "/");
    let bounded = format!("/{}/", lower.trim_matches('/'));
    for pat in test_path_patterns {
        if bounded.contains(pat) {
            return true;
        }
    }
    false
}

fn is_noise_import(name: &str) -> bool {
    let name = name.trim();
    if name.len() <= 1 {
        return true;
    }
    if name.starts_with("http://")
        || name.starts_with("https://")
        || name.starts_with("npm:")
        || name.starts_with("node:")
    {
        return true;
    }
    let lower = name.to_ascii_lowercase();
    if lower.ends_with(".css")
        || lower.ends_with(".scss")
        || lower.ends_with(".json")
        || lower.ends_with(".svg")
        || lower.ends_with(".png")
        || lower.ends_with(".jpg")
        || lower.ends_with(".wasm")
        || lower.ends_with(".html")
        || lower.ends_with(".md")
        || lower.ends_with(".txt")
    {
        return true;
    }
    if name.starts_with(|c: char| c.is_ascii_digit()) && name.contains('.') {
        return true;
    }
    name.chars().all(|c| c.is_ascii_digit() || c == '.')
}

fn normalize_import(name: &str) -> String {
    let name = name.trim();
    if let Some(rest) = name.strip_prefix("npm:") {
        let without_version = if let Some(stripped) = rest.strip_prefix('@') {
            match stripped.find('@') {
                Some(pos) => &rest[..pos + 1],
                None => rest,
            }
        } else {
            rest.split('@').next().unwrap_or(rest)
        };
        return without_version.to_string();
    }
    if let Some(rest) = name.strip_prefix("node:") {
        return rest.to_string();
    }
    name.to_string()
}

pub struct ScanResult {
    pub commits_scanned: usize,
    pub graphs_created: usize,
    pub drifts_calculated: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IncrementalHistoryAction {
    Skip,
    Append,
    RebuildWindow,
    RebuildFull,
}

#[derive(Default, Clone)]
struct ScanPhaseTimings {
    tree_walk: Duration,
    git_diff: Duration,
    git_lookup: Duration,
    parse: Duration,
    state_apply: Duration,
    analysis: Duration,
    analysis_graph_build: Duration,
    analysis_drift: Duration,
    drift_cycle: Duration,
    drift_layering: Duration,
    drift_boundary_rules: Duration,
    drift_hub: Duration,
    drift_coupling: Duration,
    drift_cognitive: Duration,
    drift_instability: Duration,
    drift_fan_deltas: Duration,
    analysis_blast_radius: Duration,
    analysis_instability: Duration,
    analysis_diagnostics: Duration,
    analysis_graph_clone: Duration,
    db_write: Duration,
}

impl ScanPhaseTimings {
    fn total_accounted(&self) -> Duration {
        self.tree_walk
            + self.git_diff
            + self.git_lookup
            + self.parse
            + self.state_apply
            + self.analysis
            + self.db_write
    }
}

struct ScanContext {
    repo: gix::ThreadSafeRepository,
    subtree_cache: Arc<git_scanner::SubtreeCache>,
    blob_import_cache: Arc<DashMap<BlobParseCacheKey, Vec<String>>>,
    package_name_cache: Arc<DashMap<String, String>>,
    ignore_globs: Option<globset::GlobSet>,
    package_depth: usize,
    test_path_patterns: Vec<String>,
    external_min_importers: usize,
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct BlobParseCacheKey {
    language: parser::Language,
    oid: Vec<u8>,
}

impl Hash for BlobParseCacheKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.language.hash(state);
        self.oid.hash(state);
    }
}

#[derive(Clone)]
struct IncrementalGraphState {
    files: HashMap<String, FileDependencyState>,
    internal_file_counts: HashMap<String, usize>,
    edge_weights: HashMap<(String, String), u32>,
    edge_samples: HashMap<(String, String), HashSet<String>>,
    target_importers: HashMap<String, HashMap<String, usize>>,
    graph: DiGraph<String, u32>,
    node_indices: HashMap<String, NodeIndex>,
    external_min_importers: usize,
}

impl Default for IncrementalGraphState {
    fn default() -> Self {
        Self::new(3)
    }
}

impl IncrementalGraphState {
    fn new(external_min_importers: usize) -> Self {
        Self {
            files: HashMap::new(),
            internal_file_counts: HashMap::new(),
            edge_weights: HashMap::new(),
            edge_samples: HashMap::new(),
            target_importers: HashMap::new(),
            graph: DiGraph::new(),
            node_indices: HashMap::new(),
            external_min_importers,
        }
    }

    fn from_repo_state(repo_state: RepoScanState, external_min_importers: usize) -> Self {
        let mut state = Self::new(external_min_importers);
        for (path, file_state) in repo_state.files {
            state.upsert_file(path, file_state);
        }
        state
    }

    fn to_repo_state(&self) -> RepoScanState {
        RepoScanState {
            files: self.files.clone(),
        }
    }

    fn visible_external(&self, module: &str) -> bool {
        !self.internal_file_counts.contains_key(module)
            && self
                .target_importers
                .get(module)
                .is_some_and(|importers| importers.len() >= self.external_min_importers)
    }

    fn should_include_node(&self, module: &str) -> bool {
        self.internal_file_counts.contains_key(module) || self.visible_external(module)
    }

    fn ensure_node(&mut self, module: &str) -> NodeIndex {
        if let Some(&idx) = self.node_indices.get(module) {
            return idx;
        }
        let idx = self.graph.add_node(module.to_string());
        self.node_indices.insert(module.to_string(), idx);
        idx
    }

    fn remove_node_by_name(&mut self, module: &str) {
        let Some(idx) = self.node_indices.remove(module) else {
            return;
        };
        let last_index = self.graph.node_count().saturating_sub(1);
        let moved_label = if idx.index() != last_index {
            self.graph.node_weight(NodeIndex::new(last_index)).cloned()
        } else {
            None
        };
        self.graph.remove_node(idx);
        if let Some(label) = moved_label {
            self.node_indices.insert(label, idx);
        }
    }

    fn sync_node(&mut self, module: &str) {
        if self.should_include_node(module) {
            self.ensure_node(module);
        } else {
            self.remove_node_by_name(module);
        }
    }

    fn sync_graph_edge(&mut self, from_module: &str, to_module: &str) {
        let weight = self
            .edge_weights
            .get(&(from_module.to_string(), to_module.to_string()))
            .copied()
            .unwrap_or(0);
        let include = weight > 0
            && self.should_include_node(from_module)
            && self.should_include_node(to_module);

        let existing_edge = self
            .node_indices
            .get(from_module)
            .copied()
            .zip(self.node_indices.get(to_module).copied())
            .and_then(|(from_idx, to_idx)| self.graph.find_edge(from_idx, to_idx));

        if !include {
            if let Some(edge_idx) = existing_edge {
                self.graph.remove_edge(edge_idx);
            }
            return;
        }

        let from_idx = self.ensure_node(from_module);
        let to_idx = self.ensure_node(to_module);
        if let Some(edge_idx) = self.graph.find_edge(from_idx, to_idx) {
            self.graph[edge_idx] = weight;
        } else {
            self.graph.add_edge(from_idx, to_idx, weight);
        }
    }

    fn sync_external_target(&mut self, module: &str) {
        let sources: Vec<String> = self
            .target_importers
            .get(module)
            .map(|importers| importers.keys().cloned().collect())
            .unwrap_or_default();
        for source in &sources {
            self.sync_graph_edge(source, module);
        }
        self.sync_node(module);
    }

    fn upsert_file(&mut self, path: String, file_state: FileDependencyState) {
        self.remove_file(&path);

        let package_name = file_state.package_name.clone();
        *self
            .internal_file_counts
            .entry(package_name.clone())
            .or_insert(0) += 1;
        self.sync_node(&package_name);

        let mut seen_targets = HashSet::new();
        let mut targets = Vec::new();
        for import in &file_state.imports {
            let target = import.module_name.clone();
            *self
                .edge_weights
                .entry((package_name.clone(), target.clone()))
                .or_insert(0) += import.weight;
            self.edge_samples
                .entry((package_name.clone(), target.clone()))
                .or_default()
                .insert(path.clone());

            if seen_targets.insert(target.clone()) {
                *self
                    .target_importers
                    .entry(target.clone())
                    .or_default()
                    .entry(package_name.clone())
                    .or_insert(0) += 1;
                targets.push(target);
            }
        }

        self.files.insert(path, file_state);
        for target in &targets {
            if self.internal_file_counts.contains_key(target) {
                self.sync_node(target);
                self.sync_graph_edge(&package_name, target);
            } else {
                self.sync_external_target(target);
            }
        }
        self.sync_node(&package_name);
    }

    fn remove_file(&mut self, path: &str) {
        let Some(existing) = self.files.remove(path) else {
            return;
        };

        let package_name = existing.package_name.clone();
        if let Some(count) = self.internal_file_counts.get_mut(&package_name) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                self.internal_file_counts.remove(&package_name);
            }
        }

        let mut seen_targets = HashSet::new();
        let mut targets = Vec::new();
        for import in &existing.imports {
            let target = import.module_name.clone();
            let key = (package_name.clone(), target.clone());
            if let Some(weight) = self.edge_weights.get_mut(&key) {
                *weight = weight.saturating_sub(import.weight);
                if *weight == 0 {
                    self.edge_weights.remove(&key);
                }
            }

            if let Some(paths) = self.edge_samples.get_mut(&key) {
                paths.remove(path);
                if paths.is_empty() {
                    self.edge_samples.remove(&key);
                }
            }

            if seen_targets.insert(target.clone()) {
                if let Some(importers) = self.target_importers.get_mut(&target) {
                    if let Some(count) = importers.get_mut(&package_name) {
                        *count = count.saturating_sub(1);
                        if *count == 0 {
                            importers.remove(&package_name);
                        }
                    }
                    if importers.is_empty() {
                        self.target_importers.remove(&target);
                    }
                }
                targets.push(target);
            }
        }

        for target in &targets {
            if self.internal_file_counts.contains_key(target) {
                self.sync_graph_edge(&package_name, target);
                self.sync_node(target);
            } else {
                self.sync_external_target(target);
            }
        }
        self.sync_node(&package_name);
    }

    fn scan_metadata(&self) -> ScanMetadata {
        let internal_nodes: HashSet<String> = self.internal_file_counts.keys().cloned().collect();
        let mut filtered_external_samples = Vec::new();
        let mut filtered_external_count = 0usize;
        let mut included_external_count = 0usize;

        for (module_name, importers) in &self.target_importers {
            if internal_nodes.contains(module_name) {
                continue;
            }
            if importers.len() >= self.external_min_importers {
                included_external_count += 1;
            } else {
                filtered_external_count += 1;
                filtered_external_samples.push(FilteredExternalSample {
                    module_name: module_name.clone(),
                    importer_count: importers.len() as u32,
                });
            }
        }

        filtered_external_samples.sort_by(|a, b| {
            b.importer_count
                .cmp(&a.importer_count)
                .then(a.module_name.cmp(&b.module_name))
        });
        filtered_external_samples.truncate(10);

        ScanMetadata {
            external_min_importers: self.external_min_importers as u32,
            included_external_count,
            filtered_external_count,
            filtered_external_samples,
        }
    }
}

fn should_track_file_with_patterns(
    path: &str,
    ignore_globs: Option<&globset::GlobSet>,
    test_path_patterns: &[String],
) -> bool {
    if ignore_globs.is_some_and(|globs| globs.is_match(path)) {
        return false;
    }
    if parser::detect_language(path).is_none() {
        return false;
    }
    !is_test_path(Path::new(path), test_path_patterns)
}

fn extract_file_target_counts(
    lang: parser::Language,
    source_pkg: &str,
    imports: &[String],
    file_path: &str,
    package_depth: usize,
) -> HashMap<String, u32> {
    let mut targets = HashMap::with_capacity(imports.len());
    let source_dir = source_dir_prefix(file_path);

    for imp in imports {
        if is_noise_import(imp) {
            continue;
        }
        let imp = normalize_import(imp);
        if imp.is_empty() {
            continue;
        }
        let target = if matches!(lang, parser::Language::Python) && imp.starts_with('.') {
            let Some(resolved) = resolve_python_relative_module(file_path, &imp) else {
                continue;
            };
            parser::extract_package_name_str_with_depth(&resolved, package_depth)
        } else if imp.starts_with("./") || imp.starts_with("../") {
            let Some(resolved) = resolve_relative_module(source_dir, &imp) else {
                continue;
            };
            parser::extract_package_name_str_with_depth(&resolved, package_depth)
        } else if imp.contains('/') {
            parser::extract_package_name_str_with_depth(&imp, package_depth)
        } else {
            imp
        };

        if target.is_empty() || target == source_pkg {
            continue;
        }

        *targets.entry(target).or_insert(0) += 1;
    }

    targets
}

fn source_dir_prefix(path: &str) -> &str {
    path.rfind(['/', '\\'])
        .map(|idx| &path[..idx])
        .unwrap_or("")
}

fn resolve_relative_module(source_dir: &str, import_path: &str) -> Option<String> {
    let mut parts: Vec<&str> = source_dir
        .split(['/', '\\'])
        .filter(|part| !part.is_empty())
        .collect();

    for part in import_path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                if parts.is_empty() {
                    return None;
                }
                parts.pop();
            }
            _ => parts.push(part),
        }
    }

    if parts.is_empty() {
        None
    } else {
        Some(parts.join("/"))
    }
}

fn resolve_python_relative_module(file_path: &str, import_path: &str) -> Option<String> {
    let normalized = file_path.replace('\\', "/");
    let mut parts: Vec<&str> = normalized
        .split('/')
        .filter(|part| !part.is_empty())
        .collect();
    if parts.is_empty() {
        return None;
    }
    let file_name = parts.pop()?;
    let mut module_parts = parts;
    if file_name == "__init__.py" {
        // Keep the package directory as-is for package-level relative imports.
    }

    let level = import_path.chars().take_while(|c| *c == '.').count();
    let remainder = import_path[level..]
        .split('.')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    let ascent = level.saturating_sub(1);
    if ascent > module_parts.len() {
        return None;
    }
    module_parts.truncate(module_parts.len() - ascent);
    module_parts.extend(remainder);

    if module_parts.is_empty() {
        None
    } else {
        Some(module_parts.join("/"))
    }
}

fn scan_blob_to_file_state(
    repo: &gix::Repository,
    path: &str,
    blob_oid: gix::ObjectId,
    ctx: &ScanContext,
) -> Result<Option<FileDependencyState>> {
    if !should_track_file_with_patterns(path, ctx.ignore_globs.as_ref(), &ctx.test_path_patterns) {
        return Ok(None);
    }

    let package_name = if let Some(cached) = ctx.package_name_cache.get(path) {
        cached.value().clone()
    } else {
        let pkg = parser::extract_package_name_str_with_depth(path, ctx.package_depth);
        ctx.package_name_cache.insert(path.to_string(), pkg.clone());
        pkg
    };

    let Some(lang) = parser::detect_language(path) else {
        return Ok(None);
    };
    let cache_key = BlobParseCacheKey {
        language: lang,
        oid: blob_oid.as_bytes().to_vec(),
    };
    let imports = if let Some(cached) = ctx.blob_import_cache.get(&cache_key) {
        cached.value().clone()
    } else {
        let blob = repo.find_object(blob_oid)?;
        let Ok(content) = std::str::from_utf8(&blob.data) else {
            return Ok(None);
        };
        let parsed = parser::parse_imports(content, lang);
        ctx.blob_import_cache.insert(cache_key, parsed.clone());
        parsed
    };

    let mut file_imports: Vec<FileImportTarget> =
        extract_file_target_counts(lang, &package_name, &imports, path, ctx.package_depth)
            .into_iter()
            .map(|(module_name, weight)| FileImportTarget {
                module_name,
                weight,
            })
            .collect();
    file_imports.sort_unstable_by(|a, b| a.module_name.cmp(&b.module_name));

    Ok(Some(FileDependencyState {
        package_name,
        imports: file_imports,
    }))
}

fn parse_file_states(
    entries: &[(String, gix::ObjectId)],
    ctx: &ScanContext,
) -> Result<Vec<(String, FileDependencyState)>> {
    if entries.is_empty() {
        return Ok(Vec::new());
    }

    if entries.len() < PARALLEL_PARSE_THRESHOLD {
        let repo = ctx.repo.to_thread_local();
        let mut parsed = Vec::with_capacity(entries.len());
        for (path, blob_oid) in entries {
            if let Some(file_state) = scan_blob_to_file_state(&repo, path, *blob_oid, ctx)? {
                parsed.push((path.clone(), file_state));
            }
        }
        return Ok(parsed);
    }

    let results: Vec<Result<Option<(String, FileDependencyState)>>> = entries
        .par_iter()
        .map(|(path, blob_oid)| {
            let repo = ctx.repo.to_thread_local();
            scan_blob_to_file_state(&repo, path, *blob_oid, ctx)
                .map(|state| state.map(|file_state| (path.clone(), file_state)))
        })
        .collect();

    let mut parsed = Vec::with_capacity(entries.len());
    for result in results {
        if let Some(entry) = result? {
            parsed.push(entry);
        }
    }
    Ok(parsed)
}

fn build_state_from_commit(
    commit_hash: &str,
    ctx: &ScanContext,
    timings: &mut ScanPhaseTimings,
) -> Result<IncrementalGraphState> {
    let repo = ctx.repo.to_thread_local();
    let tree_walk_start = Instant::now();
    let tree_oid = git_scanner::get_tree_for_commit(&repo, commit_hash)?;
    let entries = git_scanner::walk_tree_entries_cached(
        &repo,
        tree_oid,
        &ctx.subtree_cache,
        ctx.ignore_globs.as_ref(),
    )?;
    timings.tree_walk += tree_walk_start.elapsed();

    let mut state = IncrementalGraphState::new(ctx.external_min_importers);
    let parse_start = Instant::now();
    let parsed = parse_file_states(&entries, ctx)?;
    timings.parse += parse_start.elapsed();

    let apply_start = Instant::now();
    for (path, file_state) in parsed {
        state.upsert_file(path, file_state);
    }
    timings.state_apply += apply_start.elapsed();
    Ok(state)
}

fn apply_commit_diff(
    repo: &gix::Repository,
    previous_hash: &str,
    current_hash: &str,
    ctx: &ScanContext,
    state: &mut IncrementalGraphState,
    timings: &mut ScanPhaseTimings,
) -> Result<GraphDelta> {
    let diff_start = Instant::now();
    let changes = git_scanner::diff_paths_between(repo, previous_hash, current_hash)?;
    timings.git_diff += diff_start.elapsed();
    if changes.is_empty() {
        return Ok(GraphDelta::default());
    }

    let mut deletes = Vec::new();
    let mut upsert_paths = Vec::new();
    let apply_start = Instant::now();
    for change in changes {
        match change {
            git_scanner::PathChange::Delete(path) => {
                state.remove_file(&path);
                deletes.push(path);
            }
            git_scanner::PathChange::Upsert(path) => {
                state.remove_file(&path);
                upsert_paths.push(path);
            }
        }
    }
    timings.state_apply += apply_start.elapsed();

    let mut upserts = Vec::new();
    if !upsert_paths.is_empty() {
        let lookup_start = Instant::now();
        let entries = git_scanner::list_blob_oids_for_paths(repo, current_hash, &upsert_paths)?;
        timings.git_lookup += lookup_start.elapsed();

        let parse_start = Instant::now();
        let parsed = parse_file_states(&entries, ctx)?;
        timings.parse += parse_start.elapsed();

        let apply_start = Instant::now();
        for (path, file_state) in parsed {
            state.upsert_file(path.clone(), file_state.clone());
            upserts.push((path, file_state));
        }
        timings.state_apply += apply_start.elapsed();
    }

    Ok(GraphDelta { upserts, deletes })
}

fn analysis_detail_for_commit(index: usize, total_commits: usize) -> SnapshotAnalysisDetail {
    if index + 1 == total_commits {
        SnapshotAnalysisDetail::Full
    } else {
        SnapshotAnalysisDetail::Core
    }
}

fn should_checkpoint(scan_order: i64, index: usize, total_commits: usize) -> bool {
    scan_order == 1 || scan_order % CHECKPOINT_INTERVAL == 0 || index + 1 == total_commits
}

fn snapshot_scan_scope_fingerprint(snapshot: &GraphSnapshot) -> Option<String> {
    let parsed: serde_json::Value = serde_json::from_str(&snapshot.config_fingerprint).ok()?;
    serde_json::to_string(&serde_json::json!({
        "ignore": parsed.get("ignore")?,
        "scan": parsed.get("scan")?,
    }))
    .ok()
}

fn commit_hashes_from_commits(commits: &[gix::Commit]) -> Vec<String> {
    commits.iter().map(|c| c.id().to_string()).collect()
}

fn effective_requested_history_depth(
    max_commits: usize,
    latest_snapshot_matches_head: bool,
    full_history_count: Option<usize>,
) -> Option<usize> {
    match max_commits {
        usize::MAX if latest_snapshot_matches_head => full_history_count,
        usize::MAX => None,
        value => Some(value),
    }
}

fn determine_incremental_history_action(
    max_commits: usize,
    plan: &git_scanner::IncrementalCommitPlan,
) -> IncrementalHistoryAction {
    if !plan.stop_commit_on_first_parent {
        if max_commits == usize::MAX {
            IncrementalHistoryAction::RebuildFull
        } else {
            IncrementalHistoryAction::RebuildWindow
        }
    } else if plan.commits.is_empty() {
        IncrementalHistoryAction::Skip
    } else if plan.truncated {
        IncrementalHistoryAction::RebuildWindow
    } else {
        IncrementalHistoryAction::Append
    }
}

pub fn run_scan(
    path: &Path,
    repo_id: &str,
    cache_dir: &Path,
    db: &Database,
    max_commits: usize,
    project_config: &ProjectConfig,
) -> Result<ScanResult> {
    let repo_handle =
        gix::discover(path).with_context(|| format!("Failed to open repo: {}", path.display()))?;

    let head_commit = repo_handle.head_commit().context("Failed to get HEAD")?;
    let head_hash = head_commit.id().to_string();
    let config_fingerprint = project_config.config_fingerprint()?;
    let ignore_fingerprint = project_config.ignore_fingerprint()?;

    let mut latest_scanned = db.get_latest_scanned_commit(repo_id)?;
    let latest_snapshot = if let Some((ref last_hash, _)) = latest_scanned {
        db.get_graph_snapshot(repo_id, last_hash)?
    } else {
        None
    };
    let existing_count = db.graph_snapshot_count(repo_id)?;
    let latest_snapshot_matches_head = latest_snapshot
        .as_ref()
        .is_some_and(|snapshot| snapshot.commit_hash == head_hash);

    let requested_history_depth = effective_requested_history_depth(
        max_commits,
        latest_snapshot_matches_head,
        if latest_snapshot_matches_head {
            Some(git_scanner::first_parent_commit_count(&repo_handle)?)
        } else {
            None
        },
    );

    let mut needs_full_rescan = latest_snapshot.as_ref().is_some_and(|snapshot| {
        snapshot.analysis_version < CURRENT_ANALYSIS_VERSION
            || snapshot.config_fingerprint != config_fingerprint
            || snapshot_scan_scope_fingerprint(snapshot).as_deref()
                != Some(ignore_fingerprint.as_str())
    });
    let needs_history_backfill = requested_history_depth.is_some_and(|requested_depth| {
        latest_snapshot.as_ref().is_some_and(|snapshot| {
            snapshot.commit_hash == head_hash && existing_count < requested_depth
        })
    });
    if needs_history_backfill {
        needs_full_rescan = true;
        info!(
            existing_count,
            requested = requested_history_depth.unwrap_or(usize::MAX),
            "Scan mode: history backfill rebuild"
        );
    }
    if needs_full_rescan {
        db.clear_repo_graph_snapshots(repo_id)?;
        info!("Scan mode: fresh full scan after cache invalidation");
        latest_scanned = None;
    }

    if !needs_history_backfill
        && let Ok(Some(snapshot)) = db.get_graph_snapshot(repo_id, &head_hash)
        && snapshot.analysis_version == CURRENT_ANALYSIS_VERSION
        && snapshot.config_fingerprint == config_fingerprint
        && !snapshot.needs_full_analysis()
    {
        debug!(hash = %head_hash, "HEAD already scanned for this repo, skipping incremental check.");
        return Ok(ScanResult {
            commits_scanned: 0,
            graphs_created: 0,
            drifts_calculated: 0,
        });
    }

    let last_commit = latest_scanned;

    let (commit_hashes, effective_last_commit) = if let Some((ref last_hash, _)) = last_commit {
        let plan = git_scanner::plan_incremental_commits(&repo_handle, last_hash, max_commits)?;
        match determine_incremental_history_action(max_commits, &plan) {
            IncrementalHistoryAction::Skip => {
                return Ok(ScanResult {
                    commits_scanned: 0,
                    graphs_created: 0,
                    drifts_calculated: 0,
                });
            }
            IncrementalHistoryAction::Append => {
                info!(
                    existing_count,
                    new_commits = plan.commits.len(),
                    "Scan mode: incremental update"
                );
                (plan.commits, last_commit.clone())
            }
            IncrementalHistoryAction::RebuildWindow => {
                info!(
                    existing_count,
                    target_window = max_commits,
                    "Scan mode: history window rebuild"
                );
                db.clear_repo_graph_snapshots(repo_id)?;
                let commits = git_scanner::get_commits_in_order(&repo_handle, max_commits)?;
                let mut hashes = commit_hashes_from_commits(&commits);
                hashes.reverse();
                (hashes, None)
            }
            IncrementalHistoryAction::RebuildFull => {
                info!(
                    last_hash,
                    "Latest scanned commit is no longer on HEAD first-parent ancestry; rebuilding repository cache from scratch"
                );
                db.clear_repo_graph_snapshots(repo_id)?;
                let commits = git_scanner::get_commits_in_order(&repo_handle, max_commits)?;
                let mut hashes = commit_hashes_from_commits(&commits);
                hashes.reverse();
                (hashes, None)
            }
        }
    } else {
        info!("Scan mode: fresh full scan");
        let commits = git_scanner::get_commits_in_order(&repo_handle, max_commits)?;
        let mut hashes = commit_hashes_from_commits(&commits);
        hashes.reverse();
        (hashes, None)
    };
    if commit_hashes.is_empty() {
        return Ok(ScanResult {
            commits_scanned: 0,
            graphs_created: 0,
            drifts_calculated: 0,
        });
    }

    let ctx = ScanContext {
        repo: gix::ThreadSafeRepository::open(repo_handle.path().to_owned())?,
        subtree_cache: Arc::new(git_scanner::load_persistent_subtree_cache(
            cache_dir, repo_id,
        )?),
        blob_import_cache: Arc::new(DashMap::with_capacity(50_000)),
        package_name_cache: Arc::new(DashMap::with_capacity(10_000)),
        ignore_globs: project_config.ignore_globs().cloned(),
        package_depth: project_config.scan.package_depth,
        test_path_patterns: project_config.scan.normalized_test_path_patterns(),
        external_min_importers: project_config.scan.external_min_importers,
    };
    let mut timings = ScanPhaseTimings::default();

    let mut scan_state = if let Some((ref last_hash, _)) = effective_last_commit {
        match db.load_repo_scan_state(repo_id)? {
            Some((stored_hash, repo_state)) if stored_hash == *last_hash => {
                info!(hash = %stored_hash, "Loaded incremental scan baseline from DB cache");
                IncrementalGraphState::from_repo_state(repo_state, ctx.external_min_importers)
            }
            _ => {
                info!(hash = %last_hash, "Bootstrapping incremental scan baseline from last scanned commit");
                build_state_from_commit(last_hash, &ctx, &mut timings)?
            }
        }
    } else {
        IncrementalGraphState::default()
    };

    let mut prev_graph = if effective_last_commit.is_some() {
        Some(scan_state.graph.clone())
    } else {
        None
    };
    let mut previous_commit_hash = effective_last_commit.as_ref().map(|(hash, _)| hash.clone());
    let mut next_scan_order = effective_last_commit
        .as_ref()
        .map(|(_, scan_order)| scan_order + 1)
        .unwrap_or(1);
    let total_commits = commit_hashes.len();
    let mut transaction_open = false;
    let mut graphs_created = 0usize;
    let mut drifts_calculated = 0usize;
    let scan_start = Instant::now();

    info!(
        total_commits,
        batch_size = SCAN_BATCH_SIZE.min(total_commits.max(1)),
        "Scanning commit history"
    );

    for (idx, hash) in commit_hashes.iter().enumerate() {
        if idx % SCAN_BATCH_SIZE == 0 {
            if transaction_open {
                db.commit_transaction()?;
                let elapsed = scan_start.elapsed().as_secs_f64();
                info!(
                    "[{}/{}] {:.1}s - Batch complete, cache: {}",
                    graphs_created,
                    total_commits,
                    elapsed,
                    ctx.blob_import_cache.len()
                );
            }
            let batch_start = idx + 1;
            let batch_end = (idx + SCAN_BATCH_SIZE).min(total_commits);
            info!("[{batch_start}-{batch_end}/{total_commits}] Scanning batch...");
            db.begin_transaction()?;
            transaction_open = true;
        }

        let commit_oid = gix::ObjectId::from_hex(hash.as_bytes())?;
        let commit_obj = repo_handle.find_object(commit_oid)?.into_commit();
        let decoded = commit_obj.decode()?;
        let (author_name, author_email, timestamp) = match decoded.author() {
            Ok(sig) => (sig.name.to_string(), sig.email.to_string(), sig.seconds()),
            Err(_) => ("unknown".to_string(), "unknown".to_string(), 0),
        };
        let commit_info = CommitInfo {
            hash: hash.clone(),
            author_name,
            author_email,
            message: decoded.message.to_string(),
            timestamp,
            tree_id: decoded.tree().to_string(),
        };

        let delta = if let Some(previous_hash) = previous_commit_hash.as_deref() {
            apply_commit_diff(
                &repo_handle,
                previous_hash,
                &commit_info.hash,
                &ctx,
                &mut scan_state,
                &mut timings,
            )?
        } else {
            scan_state = build_state_from_commit(&commit_info.hash, &ctx, &mut timings)?;
            GraphDelta::default()
        };

        let db_write_start = Instant::now();
        db.insert_commit(repo_id, &commit_info)?;
        timings.db_write += db_write_start.elapsed();
        let detail = analysis_detail_for_commit(idx, total_commits);
        let scan_metadata = scan_state.scan_metadata();
        let analysis_start = Instant::now();
        let analysis_artifacts = analysis::analyze_graph(
            &scan_state.graph,
            prev_graph.as_ref(),
            commit_info.timestamp,
            &project_config.scoring,
            detail,
        );
        timings.analysis += analysis_start.elapsed();
        timings.analysis_graph_build += analysis_artifacts.timings.graph_build;
        timings.analysis_drift += analysis_artifacts.timings.drift;
        timings.drift_cycle += analysis_artifacts.timings.drift_cycle;
        timings.drift_layering += analysis_artifacts.timings.drift_layering;
        timings.drift_boundary_rules += analysis_artifacts.timings.drift_boundary_rules;
        timings.drift_hub += analysis_artifacts.timings.drift_hub;
        timings.drift_coupling += analysis_artifacts.timings.drift_coupling;
        timings.drift_cognitive += analysis_artifacts.timings.drift_cognitive;
        timings.drift_instability += analysis_artifacts.timings.drift_instability;
        timings.drift_fan_deltas += analysis_artifacts.timings.drift_fan_deltas;
        timings.analysis_blast_radius += analysis_artifacts.timings.blast_radius;
        timings.analysis_instability += analysis_artifacts.timings.instability;
        timings.analysis_diagnostics += analysis_artifacts.timings.diagnostics;
        timings.analysis_graph_clone += analysis_artifacts.timings.graph_clone;
        let frame = SnapshotFrame {
            commit_hash: commit_info.hash.clone(),
            scan_order: next_scan_order,
            timestamp: commit_info.timestamp,
            node_count: scan_state.graph.node_count(),
            edge_count: scan_state.graph.edge_count(),
            analysis_version: CURRENT_ANALYSIS_VERSION,
            config_fingerprint: config_fingerprint.clone(),
            drift: Some(analysis_artifacts.drift.clone()),
            scan_metadata,
            delta,
            has_full_artifacts: matches!(detail, SnapshotAnalysisDetail::Full),
        };
        let db_write_start = Instant::now();
        db.insert_snapshot_frame(repo_id, &frame)?;

        if should_checkpoint(next_scan_order, idx, total_commits) {
            db.insert_graph_checkpoint(
                repo_id,
                &GraphCheckpoint {
                    commit_hash: commit_info.hash.clone(),
                    scan_order: next_scan_order,
                    state: scan_state.to_repo_state(),
                    full_artifacts: if matches!(detail, SnapshotAnalysisDetail::Full) {
                        Some(HeavySnapshotArtifacts {
                            blast_radius: analysis_artifacts
                                .blast_radius
                                .clone()
                                .expect("full analysis should contain blast radius"),
                            instability_metrics: analysis_artifacts.instability_metrics.clone(),
                            diagnostics: analysis_artifacts.diagnostics.clone(),
                        })
                    } else {
                        None
                    },
                },
            )?;
        }
        timings.db_write += db_write_start.elapsed();

        prev_graph = Some(analysis_artifacts.graph);
        previous_commit_hash = Some(commit_info.hash);
        next_scan_order += 1;
        graphs_created += 1;
        drifts_calculated += 1;
    }

    if transaction_open {
        db.commit_transaction()?;
        let elapsed = scan_start.elapsed().as_secs_f64();
        info!(
            "[{}/{}] {:.1}s - Batch complete, cache: {}",
            graphs_created,
            total_commits,
            elapsed,
            ctx.blob_import_cache.len()
        );
    }

    if let Some(commit_hash) = previous_commit_hash.as_deref() {
        let db_write_start = Instant::now();
        db.save_repo_scan_state(repo_id, commit_hash, &scan_state.to_repo_state())?;
        timings.db_write += db_write_start.elapsed();
    }
    git_scanner::save_persistent_subtree_cache(cache_dir, repo_id, &ctx.subtree_cache)?;

    let total_wall = scan_start.elapsed();
    let accounted = timings.total_accounted();
    let unaccounted = total_wall.saturating_sub(accounted);
    info!(
        total_secs = format!("{:.3}", total_wall.as_secs_f64()),
        accounted_secs = format!("{:.3}", accounted.as_secs_f64()),
        unaccounted_secs = format!("{:.3}", unaccounted.as_secs_f64()),
        "Scan profiling summary"
    );
    for (phase, duration) in [
        ("tree-walk", timings.tree_walk),
        ("git-diff", timings.git_diff),
        ("git-lookup", timings.git_lookup),
        ("parse", timings.parse),
        ("state-apply", timings.state_apply),
        ("analysis", timings.analysis),
        ("analysis.drift", timings.analysis_drift),
        ("drift.cycle", timings.drift_cycle),
        ("drift.layering", timings.drift_layering),
        ("drift.boundary-rules", timings.drift_boundary_rules),
        ("drift.hub", timings.drift_hub),
        ("drift.coupling", timings.drift_coupling),
        ("drift.cognitive", timings.drift_cognitive),
        ("drift.instability", timings.drift_instability),
        ("drift.fan-deltas", timings.drift_fan_deltas),
        ("analysis.blast-radius", timings.analysis_blast_radius),
        ("analysis.instability", timings.analysis_instability),
        ("analysis.diagnostics", timings.analysis_diagnostics),
        ("analysis.graph-clone", timings.analysis_graph_clone),
        ("db-write", timings.db_write),
        ("unaccounted", unaccounted),
    ] {
        info!(
            phase,
            secs = format!("{:.3}", duration.as_secs_f64()),
            pct = format!(
                "{:.1}",
                duration.as_secs_f64() / total_wall.as_secs_f64().max(f64::EPSILON) * 100.0
            ),
            "Scan profiling phase"
        );
    }

    Ok(ScanResult {
        commits_scanned: total_commits,
        graphs_created,
        drifts_calculated,
    })
}

#[cfg(test)]
mod tests {
    use super::{
        BlobParseCacheKey, IncrementalGraphState, IncrementalHistoryAction,
        determine_incremental_history_action, effective_requested_history_depth,
        extract_file_target_counts, is_test_path, resolve_python_relative_module,
    };
    use crate::git_scanner::IncrementalCommitPlan;
    use crate::models::{FileDependencyState, FileImportTarget};
    use crate::parser::Language;
    use std::path::Path;

    #[test]
    fn unlimited_history_backfills_when_head_cache_is_shallow() {
        assert_eq!(
            effective_requested_history_depth(usize::MAX, true, Some(5000)),
            Some(5000)
        );
    }

    #[test]
    fn unlimited_history_does_not_force_backfill_without_head_snapshot() {
        assert_eq!(
            effective_requested_history_depth(usize::MAX, false, Some(5000)),
            None
        );
    }

    #[test]
    fn finite_history_depth_is_preserved() {
        assert_eq!(
            effective_requested_history_depth(150, true, Some(5000)),
            Some(150)
        );
    }

    #[test]
    fn truncated_incremental_plan_rebuilds_latest_window() {
        let plan = IncrementalCommitPlan {
            commits: vec!["c3".to_string(), "c4".to_string(), "head".to_string()],
            baseline_hash: Some("c2".to_string()),
            reached_stop: true,
            truncated: true,
            stop_commit_on_first_parent: true,
        };

        assert_eq!(
            determine_incremental_history_action(3, &plan),
            IncrementalHistoryAction::RebuildWindow
        );
    }

    #[test]
    fn diverged_incremental_plan_rebuilds_fully_for_unlimited_history() {
        let plan = IncrementalCommitPlan {
            commits: vec!["head".to_string()],
            baseline_hash: None,
            reached_stop: false,
            truncated: false,
            stop_commit_on_first_parent: false,
        };

        assert_eq!(
            determine_incremental_history_action(usize::MAX, &plan),
            IncrementalHistoryAction::RebuildFull
        );
    }

    #[test]
    fn contiguous_incremental_plan_appends_when_window_is_safe() {
        let plan = IncrementalCommitPlan {
            commits: vec!["c1".to_string(), "head".to_string()],
            baseline_hash: None,
            reached_stop: true,
            truncated: false,
            stop_commit_on_first_parent: true,
        };

        assert_eq!(
            determine_incremental_history_action(5, &plan),
            IncrementalHistoryAction::Append
        );
    }

    #[test]
    fn blob_parse_cache_key_is_language_aware() {
        let ts_key = BlobParseCacheKey {
            language: Language::TypeScript,
            oid: vec![1, 2, 3, 4],
        };
        let rust_key = BlobParseCacheKey {
            language: Language::Rust,
            oid: vec![1, 2, 3, 4],
        };

        assert_ne!(ts_key, rust_key);
    }

    #[test]
    fn python_relative_imports_resolve_from_source_path() {
        assert_eq!(
            resolve_python_relative_module("pkg/sub/module.py", ".config").as_deref(),
            Some("pkg/sub/config")
        );
        assert_eq!(
            resolve_python_relative_module("pkg/sub/module.py", "..shared").as_deref(),
            Some("pkg/shared")
        );
    }

    #[test]
    fn python_relative_targets_use_configured_package_depth() {
        let targets = extract_file_target_counts(
            Language::Python,
            "auth/strategies",
            &[".config".to_string(), "..shared".to_string()],
            "src/auth/strategies/jwt.py",
            2,
        );

        assert!(!targets.contains_key("auth/strategies"));
        assert_eq!(targets.get("auth"), Some(&1));
    }

    #[test]
    fn examples_are_not_test_paths_by_default() {
        let patterns = vec!["/tests/".to_string(), "/e2e/".to_string()];
        assert!(!is_test_path(
            Path::new("src/examples/tutorial.rs"),
            &patterns
        ));
        assert!(is_test_path(Path::new("tests/unit/tutorial.rs"), &patterns));
    }

    #[test]
    fn external_visibility_threshold_is_configurable() {
        let mut state = IncrementalGraphState::new(0);
        state.upsert_file(
            "src/auth.rs".to_string(),
            FileDependencyState {
                package_name: "auth".to_string(),
                imports: vec![FileImportTarget {
                    module_name: "jsonwebtoken".to_string(),
                    weight: 1,
                }],
            },
        );

        assert!(state.visible_external("jsonwebtoken"));
        assert_eq!(state.scan_metadata().external_min_importers, 0);
    }
}
