// =============================================================================
// commands/scan.rs — Scan command: commit scanning + dependency graph + drift score
// =============================================================================
//
// Orchestration module with performance-optimized scanning pipeline:
//
//   1. Incremental scan: detect existing snapshots, only process new commits
//   2. Open repo with gix
//   3. Get valid commit list (new commits only, or full if first scan)
//   4. Reverse the list so commits are processed oldest → newest (chronological)
//   5. For each commit:
//      a. Skip if same tree_id was already processed (deduplicate)
//      b. Recursively walk the commit's tree (subtree-cached)
//      c. Phase 1: Classify entries — cache hits vs misses (single-threaded)
//      d. Phase 2: Parse cache misses in parallel (rayon + tree-sitter)
//      e. Phase 3: Merge results, build edges, aggregate weights
//      f. Build dependency graph with petgraph
//      g. Calculate drift score (compare with previous graph)
//      h. Save GraphSnapshot with drift to DB
//
// Performance:
//   - Incremental scan: 5-20× faster (only new commits)
//   - LRU blob cache: bounded memory (50K entries max)
//   - Parallel parsing: 2-4× on multi-core (rayon)
//   - Tree deduplicate: same tree_id is not re-parsed
//   - Subtree cache: unchanged directories never re-walked
// =============================================================================

use std::collections::{HashMap, HashSet};
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use lru::LruCache;
use petgraph::graph::DiGraph;
use rayon::prelude::*;
use tracing::{debug, info};

use crate::db::Database;
use crate::git_scanner;
use crate::graph_builder;
use crate::models::{CommitInfo, DependencyEdge, GraphSnapshot};
use crate::parser;
use crate::scoring;

// =============================================================================
// Path & import filters — keep only meaningful architecture nodes
// =============================================================================

/// Returns `true` if the file path belongs to a test / fixture / example
/// directory that should be excluded from the dependency graph.
///
/// Test files create thousands of "noise" nodes (e.g. `001_hello`, `002_hello`)
/// that drown out the real architecture.
fn is_test_path(path: &Path) -> bool {
    let s = path.to_string_lossy();
    // Normalize to forward slashes for cross-platform matching
    let lower = s.to_ascii_lowercase().replace('\\', "/");

    // Directory-based patterns (any component match)
    const TEST_DIRS: &[&str] = &[
        "/test/",
        "/tests/",
        "/testdata/",
        "/test_data/",
        "/__tests__/",
        "/spec/",
        "/fixtures/",
        "/fixture/",
        "/examples/",
        "/example/",
        "/benchmarks/",
        "/bench/",
        "/testutil/",
        "/testing/",
        "/mock/",
        "/mocks/",
        "/snapshots/",
        "/e2e/",
    ];
    for pat in TEST_DIRS {
        if lower.contains(pat) {
            return true;
        }
    }

    // Also match when path starts with these directories (no leading slash)
    const TEST_DIR_PREFIXES: &[&str] = &[
        "test/",
        "tests/",
        "testdata/",
        "test_data/",
        "__tests__/",
        "spec/",
        "fixtures/",
        "fixture/",
        "examples/",
        "example/",
        "benchmarks/",
        "bench/",
        ".github/",
    ];
    for pat in TEST_DIR_PREFIXES {
        if lower.starts_with(pat) {
            return true;
        }
    }

    // File-name suffix patterns
    let file_name = path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_ascii_lowercase();
    if file_name.ends_with("_test.ts")
        || file_name.ends_with("_test.tsx")
        || file_name.ends_with("_test.rs")
        || file_name.ends_with("_test.go")
        || file_name.ends_with("_test.py")
        || file_name.ends_with(".test.ts")
        || file_name.ends_with(".test.tsx")
        || file_name.ends_with(".test.js")
        || file_name.ends_with(".spec.ts")
        || file_name.ends_with(".spec.tsx")
        || file_name.starts_with("test_")
    {
        return true;
    }

    false
}

/// Returns `true` if the import target is noise and should be excluded
/// from the dependency graph.
///
/// Filters out:
/// - URL imports (`https://...`) common in Deno
/// - npm/node specifiers (`npm:chalk`, `node:fs`)
/// - Non-code file imports (`.css`, `.json`, `.svg`, etc.)
/// - Version-like strings (`0.1.0`, `1.2.3`)
/// - Very short or empty names
fn is_noise_import(name: &str) -> bool {
    let name = name.trim();

    // Empty or too short
    if name.len() <= 1 {
        return true;
    }

    // URL imports (Deno-style)
    if name.starts_with("http://") || name.starts_with("https://") {
        return true;
    }

    // npm/node specifiers — we normalize these separately
    if name.starts_with("npm:") || name.starts_with("node:") {
        return true;
    }

    // Non-code file imports
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

    // Version-like strings (e.g. "0.1.0", "1.2.3")
    if name.starts_with(|c: char| c.is_ascii_digit()) && name.contains('.') {
        return true;
    }

    // Pure numbers
    if name.chars().all(|c| c.is_ascii_digit() || c == '.') {
        return true;
    }

    false
}

/// Normalizes an import name to a clean module identifier.
///
/// - `npm:chalk@5` → `chalk`
/// - `node:fs` → `fs`
/// - `@scope/package` → `@scope/package`
/// - Strips leading `./` or `../`
fn normalize_import(name: &str) -> String {
    let name = name.trim();

    // npm: specifier → extract package name
    // Handle scoped packages: npm:@scope/pkg@version → @scope/pkg
    if let Some(rest) = name.strip_prefix("npm:") {
        let without_version = if let Some(stripped) = rest.strip_prefix('@') {
            // Scoped: @scope/pkg@version — find the second '@' for version
            match stripped.find('@') {
                Some(pos) => &rest[..pos + 1],
                None => rest,
            }
        } else {
            rest.split('@').next().unwrap_or(rest)
        };
        return without_version.to_string();
    }

    // node: specifier → extract builtin name
    if let Some(rest) = name.strip_prefix("node:") {
        return rest.to_string();
    }

    name.to_string()
}

/// Collects filtered edges from an import list into the node/edge accumulators.
///
/// For path-like imports (containing `/`), the import is converted to a
/// directory-level package name via `extract_package_name` — this makes the
/// target name match source packages so internal edges are preserved.
///
/// For relative imports (`./` or `../`), we resolve them against the source
/// file's directory first, then extract the package name.
///
/// Bare specifiers like crate names (`tokio`, `deno_core`) are left as-is;
/// they'll be filtered out later by the source_pkgs check.
fn collect_edges(
    source_pkg: &str,
    imports: &[String],
    file_path_str: &str,
    all_nodes: &mut HashSet<String>,
    all_edges: &mut Vec<DependencyEdge>,
) {
    let source_dir = Path::new(file_path_str)
        .parent()
        .unwrap_or_else(|| Path::new(""));

    for imp in imports {
        if is_noise_import(imp) {
            continue;
        }
        let imp = normalize_import(imp);
        if imp.is_empty() {
            continue;
        }

        // Convert import to a directory-level package name that matches source_pkgs
        let target = if imp.starts_with("./") || imp.starts_with("../") {
            // Relative import → resolve against source file's directory
            let resolved = source_dir.join(&imp);
            // Normalize: collapse `foo/../bar` segments
            let resolved_str = resolved.to_string_lossy().replace('\\', "/");
            let mut parts: Vec<&str> = Vec::new();
            for part in resolved_str.split('/') {
                match part {
                    ".." if !parts.is_empty() => {
                        parts.pop();
                    }
                    "." | "" => {}
                    _ => parts.push(part),
                }
            }
            if parts.is_empty() {
                continue;
            }
            // Re-join and extract package name
            let joined = parts.join("/");
            parser::extract_package_name(Path::new(&joined))
        } else if imp.contains('/') {
            // Absolute path-like import (e.g., "ext/node/polyfills/path.ts")
            parser::extract_package_name(Path::new(&imp))
        } else {
            // Bare specifier (crate name, npm package, etc.)
            imp
        };

        if target.is_empty() {
            continue;
        }

        // Skip self-edges
        if target == source_pkg {
            continue;
        }

        all_nodes.insert(target.clone());
        all_edges.push(DependencyEdge {
            from_module: source_pkg.to_string(),
            to_module: target,
            file_path: file_path_str.to_string(),
            line: 0,
            weight: 1,
        });
    }
}

/// Scan result — used by main.rs for printing the summary
pub struct ScanResult {
    pub commits_scanned: usize,
    pub graphs_created: usize,
    /// Number of drift scores calculated
    pub drifts_calculated: usize,
}

/// Scans the repository: creates commit metadata + dependency graphs + drift scores.
///
/// Supports **incremental scanning**: if existing snapshots are found in the DB,
/// only new commits since the last scan are processed. This gives 5-20× speedup
/// for large repositories on subsequent scans.
///
/// # Workflow
/// 1. Check for existing snapshots (incremental scan detection)
/// 2. Get valid commit list (new commits only, or full)
/// 3. Reverse to chronological order (oldest → newest)
/// 4. For each commit: tree_walk + parallel parse + graph/drift processing
pub fn run_scan(path: &Path, db: &Database, max_commits: usize) -> Result<ScanResult> {
    let repo = gix::discover(path)
        .with_context(|| format!("Failed to open repo for graph building: {}", path.display()))?;

    // ── Incremental scan: detect existing snapshots ──
    let last_commit = db.get_latest_scanned_commit()?;
    let existing_count = db.graph_snapshot_count()?;

    let mut commits = if let Some(ref last_hash) = last_commit {
        // Incremental: only get commits since last scan
        let new_commits = git_scanner::get_commits_since(&repo, last_hash, max_commits)?;
        if new_commits.is_empty() {
            info!("No new commits since last scan ({})", &last_hash[..7]);
            return Ok(ScanResult {
                commits_scanned: 0,
                graphs_created: 0,
                drifts_calculated: 0,
            });
        }
        info!(
            "Incremental scan: {} existing snapshots, {} new commits since {}",
            existing_count,
            new_commits.len(),
            &last_hash[..7]
        );
        new_commits
    } else {
        // Full scan — clear existing data
        db.clear_all_graph_snapshots()?;
        info!("Building dependency graphs...");
        git_scanner::get_commits_in_order(&repo, max_commits)?
    };

    // Reverse to chronological order (oldest → newest)
    commits.reverse();

    // ── Load prev_graph for drift continuity (incremental) ──
    let mut prev_graph: Option<DiGraph<String, ()>> = None;
    if let Some(ref last_hash) = last_commit {
        if let Some(snapshot) = db.get_graph_snapshot(last_hash)? {
            let nodes: HashSet<String> = snapshot.nodes.into_iter().collect();
            prev_graph = Some(graph_builder::build_graph(&nodes, &snapshot.edges));
            debug!(
                "Loaded previous graph for drift continuity ({} nodes)",
                nodes.len()
            );
        }
    }

    // ── Begin batch transaction (one fsync for ALL inserts) ──
    db.begin_transaction()?;

    let mut graphs_created: usize = 0;
    let mut drifts_calculated: usize = 0;

    // Avoid re-processing the same tree
    let mut seen_trees: HashSet<String> = HashSet::new();

    // ── Two-level cache for incremental scanning ──────────────────
    //
    // Level 1 — Subtree cache: skip re-walking unchanged directories.
    //   Key = tree OID, Value = list of (relative_path, blob_oid).
    //   Between adjacent commits, ~95 % of subtrees share the same OID.
    //
    // Level 2 — Blob import cache: LRU-bounded to prevent unbounded growth.
    //   Key = blob OID (20 bytes), Value = parsed import list.
    //   Even when the tree walk finds the file, if the blob OID matches
    //   a previous parse result we skip the expensive read + parse.
    //   Capacity: 50K entries ≈ 50K × ~200 bytes = ~10 MB max.
    let mut subtree_cache = git_scanner::SubtreeCache::new();
    let mut blob_import_cache: LruCache<[u8; 20], Vec<String>> =
        LruCache::new(NonZeroUsize::new(50_000).unwrap());
    let total_commits = commits.len();
    let scan_start = std::time::Instant::now();

    for (ci, commit) in commits.iter().enumerate() {
        let commit_hash = commit.id().to_string();

        let decoded = match commit.decode() {
            Ok(d) => d,
            Err(e) => {
                debug!(hash = %commit_hash, error = %e, "Failed to decode commit, skipping");
                continue;
            }
        };

        let (author_name, author_email, timestamp) = match decoded.author() {
            Ok(sig) => (sig.name.to_string(), sig.email.to_string(), sig.seconds()),
            Err(_) => ("unknown".to_string(), "unknown".to_string(), 0),
        };
        let commit_info = CommitInfo {
            hash: commit_hash.clone(),
            author_name,
            author_email,
            message: decoded.message.to_string(),
            timestamp,
            tree_id: decoded.tree().to_string(),
        };

        // Save commit to DB
        db.insert_commit(&commit_info)?;

        let tree_oid = match git_scanner::get_tree_for_commit(&repo, &commit_hash) {
            Ok(oid) => oid,
            Err(e) => {
                debug!(hash = %commit_hash, error = %e, "Failed to get commit tree, skipping");
                continue;
            }
        };

        let tree_hex = tree_oid.to_string();
        if !seen_trees.insert(tree_hex) {
            debug!(hash = %commit_hash, "Same tree already processed, skipping");
            continue;
        }

        let tree = match repo.find_tree(tree_oid) {
            Ok(t) => t,
            Err(e) => {
                debug!(hash = %commit_hash, error = %e, "Tree not found");
                continue;
            }
        };

        // ── Fast tree walk: subtree-cached + blob-oid-only ──
        let entries = match git_scanner::walk_tree_entries_cached(&repo, &tree, &mut subtree_cache)
        {
            Ok(e) => e,
            Err(e) => {
                debug!(hash = %commit_hash, error = %e, "Tree walk failed");
                continue;
            }
        };

        if entries.is_empty() {
            continue;
        }

        let mut all_nodes: HashSet<String> = HashSet::with_capacity(entries.len() / 4);
        let mut all_edges: Vec<DependencyEdge> = Vec::with_capacity(entries.len());
        let mut cache_hits: usize = 0;

        // ────────────────────────────────────────────────────────────────
        // Phase 1: Classify entries — cache hits vs cache misses
        //   Single-threaded: needs &repo for blob reads and &mut cache
        // ────────────────────────────────────────────────────────────────
        struct ParseJob {
            source_pkg: String,
            oid_key: [u8; 20],
            content: String,
            file_path: PathBuf,
        }

        let mut cached_imports: Vec<(String, Vec<String>, String)> = Vec::new();
        let mut parse_jobs: Vec<ParseJob> = Vec::new();

        for (file_path, blob_oid) in &entries {
            // Skip test / fixture / example files — they create noise nodes
            if is_test_path(file_path.as_path()) {
                continue;
            }

            let source_pkg = parser::extract_package_name(file_path.as_path());
            all_nodes.insert(source_pkg.clone());

            // Use raw 20-byte OID as cache key (no hex string allocation)
            let oid_key: [u8; 20] = blob_oid.as_bytes().try_into().unwrap_or([0u8; 20]);

            // ── Cache lookup: skip blob read + parse if OID unchanged ──
            if let Some(cached) = blob_import_cache.get(&oid_key) {
                cache_hits += 1;
                if !cached.is_empty() {
                    let file_path_str = file_path.to_string_lossy().replace('\\', "/");
                    cached_imports.push((source_pkg, cached.clone(), file_path_str));
                }
            } else {
                // Cache miss → read blob content (needs &repo, single-threaded)
                let blob = match repo.find_object(*blob_oid) {
                    Ok(b) => b,
                    Err(_) => {
                        blob_import_cache.put(oid_key, Vec::new());
                        continue;
                    }
                };
                let content = match std::str::from_utf8(&blob.data) {
                    Ok(s) => s.to_string(),
                    Err(_) => {
                        blob_import_cache.put(oid_key, Vec::new());
                        continue;
                    }
                };
                let file_path_str = file_path.to_string_lossy();
                if parser::detect_language(file_path_str.as_ref()).is_none() {
                    blob_import_cache.put(oid_key, Vec::new());
                    continue;
                }
                parse_jobs.push(ParseJob {
                    source_pkg,
                    oid_key,
                    content,
                    file_path: file_path.clone(),
                });
            }
        }

        // ── Snapshot source packages BEFORE collect_edges adds imports ──
        // At this point all_nodes contains ONLY source packages (from extract_package_name).
        // After collect_edges runs, external import targets will be added too.
        // We capture source_pkgs now so we can filter externals out later.
        let source_pkgs: HashSet<String> = all_nodes.clone();

        // ────────────────────────────────────────────────────────────────
        // Phase 2: Parse cache misses in parallel (rayon)
        //   Each thread creates its own tree-sitter Parser (Parser is !Sync
        //   but Send, and parse_imports creates a fresh one per call).
        //   No repo access needed — only CPU-bound tree-sitter parsing.
        // ────────────────────────────────────────────────────────────────
        let parsed_results: Vec<(String, [u8; 20], Vec<String>, String)> = parse_jobs
            .into_par_iter()
            .filter_map(|job| {
                let path_str = job.file_path.to_string_lossy();
                let lang = parser::detect_language(path_str.as_ref())?;
                let imports = parser::parse_imports(&job.content, lang, job.file_path.as_path());
                let file_path_str = path_str.replace('\\', "/");
                Some((job.source_pkg, job.oid_key, imports, file_path_str))
            })
            .collect();

        // ────────────────────────────────────────────────────────────────
        // Phase 3: Merge results — update cache + build edges
        //   Single-threaded: needs &mut cache and &mut all_nodes/all_edges
        // ────────────────────────────────────────────────────────────────
        for (source_pkg, oid_key, imports, file_path_str) in parsed_results {
            blob_import_cache.put(oid_key, imports.clone());
            if !imports.is_empty() {
                collect_edges(
                    &source_pkg,
                    &imports,
                    &file_path_str,
                    &mut all_nodes,
                    &mut all_edges,
                );
            }
        }

        // Process cached imports (already parsed, just need edge creation)
        for (source_pkg, imports, file_path_str) in cached_imports {
            collect_edges(
                &source_pkg,
                &imports,
                &file_path_str,
                &mut all_nodes,
                &mut all_edges,
            );
        }

        if all_nodes.is_empty() {
            continue;
        }

        // Aggregate duplicate edges: same (from, to) pair → merge with weight count
        let mut edge_weight_map: HashMap<(String, String), DependencyEdge> =
            HashMap::with_capacity(all_edges.len() / 2);
        for edge in all_edges {
            let key = (edge.from_module.clone(), edge.to_module.clone());
            edge_weight_map
                .entry(key)
                .and_modify(|existing| existing.weight += 1)
                .or_insert(edge);
        }
        let all_edges: Vec<DependencyEdge> = edge_weight_map.into_values().collect();

        // ── Prune noise: keep source packages + high-connectivity external deps ──
        //
        // Strategy:
        //   - Source packages (from extract_package_name) are ALWAYS kept
        //   - External import targets are kept only if they're imported by
        //     ≥ MIN_EXT_IMPORTERS different source packages (shared deps are
        //     architecturally significant; single-use imports are noise)
        //   - Edges are kept only if both endpoints survive the filter
        //
        // This reduces e.g. 622 → ~80-120 nodes while preserving meaningful edges.
        const MIN_EXT_IMPORTERS: usize = 3;

        // Count how many DISTINCT source packages import each external target
        let mut ext_importer_count: HashMap<String, HashSet<String>> = HashMap::new();
        for edge in &all_edges {
            if !source_pkgs.contains(&edge.to_module) {
                ext_importer_count
                    .entry(edge.to_module.clone())
                    .or_default()
                    .insert(edge.from_module.clone());
            }
        }

        let kept_nodes: HashSet<String> = all_nodes
            .iter()
            .filter(|n| {
                source_pkgs.contains(*n)
                    || ext_importer_count
                        .get(*n)
                        .is_some_and(|importers| importers.len() >= MIN_EXT_IMPORTERS)
            })
            .cloned()
            .collect();

        let filtered_edges: Vec<DependencyEdge> = all_edges
            .into_iter()
            .filter(|e| kept_nodes.contains(&e.from_module) && kept_nodes.contains(&e.to_module))
            .collect();

        let graph = graph_builder::build_graph(&kept_nodes, &filtered_edges);

        let nodes_vec: Vec<String> = kept_nodes.iter().cloned().collect();
        let edges_pairs = scoring::edges_to_pairs(&filtered_edges);
        let drift = scoring::calculate_drift(
            &graph,
            prev_graph.as_ref(),
            &nodes_vec,
            &edges_pairs,
            commit_info.timestamp,
        );
        drifts_calculated += 1;

        let snapshot = GraphSnapshot {
            commit_hash: commit_hash.clone(),
            nodes: nodes_vec,
            edges: filtered_edges,
            node_count: graph.node_count(),
            edge_count: graph.edge_count(),
            timestamp: commit_info.timestamp,
            drift: Some(drift),
        };

        db.insert_graph_snapshot(&snapshot)?;
        graphs_created += 1;

        prev_graph = Some(graph);

        // ── Progress indicator (every 25 commits or at the end) ──
        if (ci + 1) % 25 == 0 || ci + 1 == total_commits {
            let elapsed = scan_start.elapsed().as_secs_f64();
            let pct = ((ci + 1) as f64 / total_commits as f64 * 100.0) as u32;
            info!(
                "[{}/{}] {}% — {} graphs, {} cached blobs ({} hits), {:.1}s",
                ci + 1,
                total_commits,
                pct,
                graphs_created,
                blob_import_cache.len(),
                cache_hits,
                elapsed,
            );
        }
    }

    // ── Commit all batched writes in one fsync ──
    db.commit_transaction()?;

    info!(
        total = graphs_created,
        drifts = drifts_calculated,
        "Dependency graph + drift creation complete"
    );

    Ok(ScanResult {
        commits_scanned: commits.len(),
        graphs_created,
        drifts_calculated,
    })
}

// =============================================================================
// Helper functions
// =============================================================================

/// Extracts a module name from a file path.
///
/// Removes the extension and replaces directory separators with `::`.
///
/// # Examples
/// - "src/main.rs" → "src::main"
/// - "packages/ui/index.ts" → "packages::ui::index"
/// - "cmd/server/main.go" → "cmd::server::main"
#[cfg(test)]
fn path_to_module(path: &str) -> String {
    let path = path.replace("\\", "/");
    let without_ext = path
        .rsplit_once('.')
        .map_or(path.as_str(), |(base, _)| base);
    without_ext.replace('/', "::")
}

// =============================================================================
// Tests
// =============================================================================
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_path_to_module() {
        assert_eq!(path_to_module("src/main.rs"), "src::main");
        assert_eq!(
            path_to_module("packages/ui/index.ts"),
            "packages::ui::index"
        );
        assert_eq!(path_to_module("cmd/server/main.go"), "cmd::server::main");
        assert_eq!(path_to_module("lib.rs"), "lib");
        assert_eq!(path_to_module("src\\win\\main.rs"), "src::win::main");
    }

    #[test]
    fn test_is_test_path() {
        // Should be filtered
        assert!(is_test_path(Path::new("cli/tests/testdata/001_hello.ts")));
        assert!(is_test_path(Path::new("src/__tests__/app.test.tsx")));
        assert!(is_test_path(Path::new("tests/integration/run.rs")));
        assert!(is_test_path(Path::new("examples/hello/main.rs")));
        assert!(is_test_path(Path::new("benchmarks/perf.go")));
        assert!(is_test_path(Path::new("src/utils_test.go")));
        assert!(is_test_path(Path::new("lib/parser.test.ts")));
        assert!(is_test_path(Path::new("test_helper.py")));
        assert!(is_test_path(Path::new("fixtures/data.ts")));

        // Should NOT be filtered
        assert!(!is_test_path(Path::new("src/main.rs")));
        assert!(!is_test_path(Path::new("cli/tools/run.ts")));
        assert!(!is_test_path(Path::new("packages/core/index.ts")));
        assert!(!is_test_path(Path::new("runtime/ops/fs.rs")));
    }

    #[test]
    fn test_is_noise_import() {
        // Should be filtered
        assert!(is_noise_import("https://deno.land/std/testing/asserts.ts"));
        assert!(is_noise_import("http://example.com/mod.ts"));
        assert!(is_noise_import("npm:chalk@5"));
        assert!(is_noise_import("node:fs"));
        assert!(is_noise_import("./styles.css"));
        assert!(is_noise_import("../data.json"));
        assert!(is_noise_import("logo.svg"));
        assert!(is_noise_import("0.1.0"));
        assert!(is_noise_import("1.2.3"));
        assert!(is_noise_import("x")); // single char

        // Should NOT be filtered
        assert!(!is_noise_import("react"));
        assert!(!is_noise_import("serde"));
        assert!(!is_noise_import("std"));
        assert!(!is_noise_import("@scope/package"));
        assert!(!is_noise_import("tokio"));
    }

    #[test]
    fn test_normalize_import() {
        assert_eq!(normalize_import("npm:chalk@5"), "chalk");
        assert_eq!(normalize_import("npm:@types/node"), "@types/node");
        assert_eq!(normalize_import("node:fs"), "fs");
        assert_eq!(normalize_import("node:path"), "path");
        assert_eq!(normalize_import("react"), "react");
    }
}
