// =============================================================================
// commands/scan.rs — Scan command: commit scanning + dependency graph + drift score
// =============================================================================

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use dashmap::DashMap;
use petgraph::graph::DiGraph;
use rayon::prelude::*;
use tracing::{debug, info};

use crate::db::Database;
use crate::git_scanner;
use crate::graph_builder;
use crate::models::{CommitInfo, DependencyEdge, GraphSnapshot};
use crate::parser;
use crate::scoring;

type CommitScanResult = Result<(
    CommitInfo,
    Vec<DependencyEdge>,
    HashSet<String>,
    HashSet<String>,
)>;

fn is_test_path(path: &Path) -> bool {
    let s = path.to_string_lossy();
    let lower = s.to_ascii_lowercase().replace('\\', "/");
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
    let file_name = path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_ascii_lowercase();
    file_name.ends_with("_test.ts")
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
}

fn is_noise_import(name: &str) -> bool {
    let name = name.trim();
    if name.len() <= 1 {
        return true;
    }
    if name.starts_with("http://") || name.starts_with("https://") {
        return true;
    }
    if name.starts_with("npm:") || name.starts_with("node:") {
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
        let target = if imp.starts_with("./") || imp.starts_with("../") {
            let resolved = source_dir.join(&imp);
            let resolved_str = resolved.to_string_lossy().replace('\\', "/");
            let mut parts = Vec::new();
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
            parser::extract_package_name(Path::new(&parts.join("/")))
        } else if imp.contains('/') {
            parser::extract_package_name(Path::new(&imp))
        } else {
            imp
        };
        if target.is_empty() || target == source_pkg {
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

pub struct ScanResult {
    pub commits_scanned: usize,
    pub graphs_created: usize,
    pub drifts_calculated: usize,
}

struct ScanContext {
    repo: gix::ThreadSafeRepository,
    subtree_cache: Arc<git_scanner::SubtreeCache>,
    blob_import_cache: Arc<DashMap<[u8; 20], Vec<String>>>,
}

pub fn run_scan(path: &Path, db: &Database, max_commits: usize) -> Result<ScanResult> {
    let repo_handle =
        gix::discover(path).with_context(|| format!("Failed to open repo: {}", path.display()))?;

    let head_commit = repo_handle.head_commit().context("Failed to get HEAD")?;
    let head_hash = head_commit.id().to_string();

    if let Ok(Some(_)) = db.get_graph_snapshot(&head_hash) {
        debug!(hash = %head_hash, "HEAD already scanned, skipping incremental check.");
        return Ok(ScanResult {
            commits_scanned: 0,
            graphs_created: 0,
            drifts_calculated: 0,
        });
    }

    let last_commit = db.get_latest_scanned_commit()?;
    let existing_count = db.graph_snapshot_count()?;

    let mut commits_raw = if let Some(ref last_hash) = last_commit {
        let new_commits = git_scanner::get_commits_since(&repo_handle, last_hash, max_commits)?;
        if new_commits.is_empty() {
            return Ok(ScanResult {
                commits_scanned: 0,
                graphs_created: 0,
                drifts_calculated: 0,
            });
        }
        info!(
            "Incremental scan: {} existing snapshots, {} new commits",
            existing_count,
            new_commits.len()
        );
        new_commits
    } else {
        db.clear_all_graph_snapshots()?;
        info!("Building dependency graphs from scratch...");
        git_scanner::get_commits_in_order(&repo_handle, max_commits)?
    };
    commits_raw.reverse();

    let commit_hashes: Vec<String> = commit_hashes_from_commits(&commits_raw);

    let mut prev_graph: Option<DiGraph<String, ()>> = None;
    if let Some(ref last_hash) = last_commit {
        if let Some(snapshot) = db.get_graph_snapshot(last_hash)? {
            let nodes: HashSet<String> = snapshot.nodes.into_iter().collect();
            prev_graph = Some(graph_builder::build_graph(&nodes, &snapshot.edges));
        }
    }

    let ctx = Arc::new(ScanContext {
        repo: gix::ThreadSafeRepository::open(repo_handle.path().to_owned())?,
        subtree_cache: Arc::new(git_scanner::SubtreeCache::new()),
        blob_import_cache: Arc::new(DashMap::with_capacity(50_000)),
    });

    let mut graphs_created = 0;
    let mut drifts_calculated = 0;
    let total_commits = commit_hashes.len();
    let scan_start = std::time::Instant::now();

    for chunk in commit_hashes.chunks(200) {
        db.begin_transaction()?;

        let chunk_results: Vec<CommitScanResult> = chunk
            .par_iter()
            .map(|hash: &String| {
                let repo = ctx.repo.to_thread_local();
                let commit_oid = gix::ObjectId::from_hex(hash.as_bytes())?;
                let commit_obj = repo.find_object(commit_oid)?.into_commit();

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

                let tree_oid = commit_obj.tree_id()?.detach();
                let entries =
                    git_scanner::walk_tree_entries_cached(&repo, tree_oid, &ctx.subtree_cache)?;

                let mut all_nodes = HashSet::new();
                let mut all_edges = Vec::new();
                let mut source_pkgs = HashSet::new();
                let mut parse_jobs = Vec::new();

                for (file_path, blob_oid) in entries {
                    if is_test_path(&file_path) {
                        continue;
                    }
                    let source_pkg = parser::extract_package_name(&file_path);
                    source_pkgs.insert(source_pkg.clone());
                    all_nodes.insert(source_pkg.clone());

                    let oid_key: [u8; 20] = blob_oid.as_bytes().try_into().unwrap_or([0u8; 20]);

                    if let Some(cached) = ctx.blob_import_cache.get(&oid_key) {
                        let imports: &Vec<String> = cached.value();
                        if !imports.is_empty() {
                            collect_edges(
                                &source_pkg,
                                imports,
                                &file_path.to_string_lossy().replace('\\', "/"),
                                &mut all_nodes,
                                &mut all_edges,
                            );
                        }
                    } else {
                        let blob = repo.find_object(blob_oid)?;
                        if let Ok(content) = std::str::from_utf8(&blob.data) {
                            if let Some(lang) =
                                parser::detect_language(&file_path.to_string_lossy())
                            {
                                parse_jobs.push((
                                    source_pkg,
                                    oid_key,
                                    content.to_string(),
                                    file_path.clone(),
                                    lang,
                                ));
                            }
                        }
                    }
                }

                for (source_pkg, oid_key, content, file_path, lang) in parse_jobs {
                    let imports = parser::parse_imports(&content, lang, &file_path);
                    ctx.blob_import_cache.insert(oid_key, imports.clone());
                    collect_edges(
                        &source_pkg,
                        &imports,
                        &file_path.to_string_lossy().replace('\\', "/"),
                        &mut all_nodes,
                        &mut all_edges,
                    );
                }

                Ok((commit_info, all_edges, source_pkgs, all_nodes))
            })
            .collect();

        for res in chunk_results {
            let (commit_info, all_edges, source_pkgs, all_nodes) = res?;
            db.insert_commit(&commit_info)?;

            let mut edge_weight_map: HashMap<(String, String), DependencyEdge> =
                HashMap::with_capacity(all_edges.len());
            for edge in all_edges {
                edge_weight_map
                    .entry((edge.from_module.clone(), edge.to_module.clone()))
                    .and_modify(|e| e.weight += 1)
                    .or_insert(edge);
            }
            let merged_edges: Vec<DependencyEdge> = edge_weight_map.into_values().collect();

            let mut ext_importers: HashMap<String, HashSet<String>> = HashMap::new();
            for edge in &merged_edges {
                if !source_pkgs.contains(&edge.to_module) {
                    ext_importers
                        .entry(edge.to_module.clone())
                        .or_default()
                        .insert(edge.from_module.clone());
                }
            }

            let kept_nodes: HashSet<String> = all_nodes
                .iter()
                .filter(|n| {
                    source_pkgs.contains(*n)
                        || ext_importers
                            .get(*n)
                            .is_some_and(|importers| importers.len() >= 3)
                })
                .cloned()
                .collect();

            let final_edges: Vec<DependencyEdge> = merged_edges
                .into_iter()
                .filter(|e| {
                    kept_nodes.contains(&e.from_module) && kept_nodes.contains(&e.to_module)
                })
                .collect();

            let graph = graph_builder::build_graph(&kept_nodes, &final_edges);
            let nodes_vec: Vec<String> = kept_nodes.into_iter().collect();
            let drift = scoring::calculate_drift(
                &graph,
                prev_graph.as_ref(),
                &nodes_vec,
                &scoring::edges_to_pairs(&final_edges),
                commit_info.timestamp,
            );

            db.insert_graph_snapshot(&GraphSnapshot {
                commit_hash: commit_info.hash,
                nodes: nodes_vec,
                edges: final_edges,
                node_count: graph.node_count(),
                edge_count: graph.edge_count(),
                timestamp: commit_info.timestamp,
                drift: Some(drift),
            })?;

            prev_graph = Some(graph);
            graphs_created += 1;
            drifts_calculated += 1;
        }

        db.commit_transaction()?;
        let elapsed = scan_start.elapsed().as_secs_f64();
        info!(
            "[{}/{}] {:.1}s — Batch complete, cache: {}",
            graphs_created,
            total_commits,
            elapsed,
            ctx.blob_import_cache.len()
        );
    }

    Ok(ScanResult {
        commits_scanned: total_commits,
        graphs_created,
        drifts_calculated,
    })
}

fn commit_hashes_from_commits(commits: &[gix::Commit]) -> Vec<String> {
    commits.iter().map(|c| c.id().to_string()).collect()
}
