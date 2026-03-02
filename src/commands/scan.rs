// =============================================================================
// commands/scan.rs — Scan command: commit scanning + dependency graph + drift score
// =============================================================================
//
// Sprint 2-3-4 main orchestration module. Workflow:
//
//   1. Clear old snapshot data from the database
//   2. Open repo with gix
//   3. Get valid commit list (in order) from git_scanner
//   4. Reverse the list so commits are processed oldest → newest (chronological)
//   5. For each commit:
//      a. Skip if same tree_id was already processed (deduplicate)
//      b. Recursively walk the commit's tree
//      c. Find supported files (*.rs, *.ts, *.tsx, *.py, *.go)
//      d. Extract imports with tree-sitter
//      e. Build dependency graph with petgraph
//      f. Calculate drift score (compare with previous graph)
//      g. Save GraphSnapshot with drift to DB
//
// Performance notes:
//   - Tree deduplicate: same tree_id is not re-parsed
//   - Blob reading: directly from gix object DB, no disk checkout
//   - Large files (>512KB) are skipped by the parser
// =============================================================================

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use petgraph::graph::DiGraph;
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
        "/test/", "/tests/", "/testdata/", "/test_data/",
        "/__tests__/", "/spec/", "/fixtures/", "/fixture/",
        "/examples/", "/example/", "/benchmarks/", "/bench/",
        "/testutil/", "/testing/", "/mock/", "/mocks/",
        "/snapshots/", "/e2e/",
    ];
    for pat in TEST_DIRS {
        if lower.contains(pat) {
            return true;
        }
    }

    // Also match when path starts with these directories (no leading slash)
    const TEST_DIR_PREFIXES: &[&str] = &[
        "test/", "tests/", "testdata/", "test_data/",
        "__tests__/", "spec/", "fixtures/", "fixture/",
        "examples/", "example/", "benchmarks/", "bench/",
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
        let without_version = if rest.starts_with('@') {
            // Scoped: @scope/pkg@version — find the second '@' for version
            match rest[1..].find('@') {
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

/// Scan result — used by main.rs for printing the summary
pub struct ScanResult {
    pub commits_scanned: usize,
    pub graphs_created: usize,
    /// Sprint 3: Number of drift scores calculated
    pub drifts_calculated: usize,
}

/// Scans the repository: creates commit metadata + dependency graphs + drift scores.
///
/// # Workflow
/// 1. DB cleanup
/// 2. Get valid commit list
/// 3. Reverse to chronological order (oldest → newest)
/// 4. For each commit: tree_walk + graph/drift processing
pub fn run_scan(path: &Path, db: &Database, max_commits: usize) -> Result<ScanResult> {
    // ── Step 1: Clear all existing data ──
    db.clear_all_graph_snapshots()?;

    info!("Building dependency graphs...");

    let repo = gix::discover(path)
        .with_context(|| format!("Failed to open repo for graph building: {}", path.display()))?;

    // ── Step 2: Get valid commit list ──
    let mut commits = git_scanner::get_commits_in_order(&repo, max_commits)?;

    // ── Step 3: Reverse to chronological order (oldest → newest) ──
    commits.reverse();

    // ── Begin batch transaction (one fsync for ALL inserts) ──
    db.begin_transaction()?;

    let mut graphs_created: usize = 0;
    let mut drifts_calculated: usize = 0;

    // Avoid re-processing the same tree
    let mut seen_trees: HashSet<String> = HashSet::new();
    let mut prev_graph: Option<DiGraph<String, ()>> = None;

    // ── Two-level cache for incremental scanning ──────────────────
    //
    // Level 1 — Subtree cache: skip re-walking unchanged directories.
    //   Key = tree OID, Value = list of (relative_path, blob_oid).
    //   Between adjacent commits, ~95 % of subtrees share the same OID.
    //
    // Level 2 — Blob import cache: skip re-reading unchanged file blobs.
    //   Key = blob OID hex, Value = parsed import list.
    //   Even when the tree walk finds the file, if the blob OID matches
    //   a previous parse result we skip the expensive read + parse.
    let mut subtree_cache = git_scanner::SubtreeCache::new();
    // Blob import cache keyed by raw ObjectId bytes (avoids 3700× hex encode per commit)
    let mut blob_import_cache: HashMap<[u8; 20], Vec<String>> = HashMap::new();
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

        let commit_info = CommitInfo {
            hash: commit_hash.clone(),
            author_name: decoded.author.name.to_string(),
            author_email: decoded.author.email.to_string(),
            message: decoded.message.to_string(),
            timestamp: decoded.author.time.seconds,
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
        let entries = match git_scanner::walk_tree_entries_cached(&repo, &tree, &mut subtree_cache) {
            Ok(e) => e,
            Err(e) => {
                debug!(hash = %commit_hash, error = %e, "Tree walk failed");
                continue;
            }
        };

        if entries.is_empty() {
            continue;
        }

        let mut all_nodes: HashSet<String> = HashSet::new();
        let mut all_edges: Vec<DependencyEdge> = Vec::new();
        let mut cache_hits: usize = 0;

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
            let imports = if let Some(cached) = blob_import_cache.get(&oid_key) {
                cache_hits += 1;
                cached
            } else {
                // Cache miss → read blob content and parse imports
                let blob = match repo.find_object(*blob_oid) {
                    Ok(b) => b,
                    Err(_) => {
                        blob_import_cache.insert(oid_key, Vec::new());
                        continue;
                    }
                };
                let content = match std::str::from_utf8(&blob.data) {
                    Ok(s) => s,
                    Err(_) => {
                        blob_import_cache.insert(oid_key, Vec::new());
                        continue;
                    }
                };
                let file_path_str = file_path.to_string_lossy();
                let lang = match parser::detect_language(file_path_str.as_ref()) {
                    Some(l) => l,
                    None => {
                        blob_import_cache.insert(oid_key, Vec::new());
                        continue;
                    }
                };
                let parsed = parser::parse_imports(content, lang, file_path.as_path());
                blob_import_cache.insert(oid_key, parsed);
                blob_import_cache.get(&oid_key).unwrap()
            };

            if !imports.is_empty() {
                // Build file_path_str only when there are actual edges to create
                let file_path_str = file_path.to_string_lossy().replace('\\', "/");
                for imp in imports {
                    // Filter noise imports (URLs, .css, versions, etc.)
                    if is_noise_import(imp) {
                        continue;
                    }
                    let imp = normalize_import(imp);
                    if imp.is_empty() {
                        continue;
                    }
                    all_nodes.insert(imp.clone());
                    all_edges.push(DependencyEdge {
                        from_module: source_pkg.clone(),
                        to_module: imp,
                        file_path: file_path_str.clone(),
                        line: 0,
                        weight: 1,
                    });
                }
            }
        }

        if all_nodes.is_empty() {
            continue;
        }

        // Aggregate duplicate edges: same (from, to) pair → merge with weight count
        let mut edge_weight_map: HashMap<(String, String), DependencyEdge> = HashMap::new();
        for edge in all_edges {
            let key = (edge.from_module.clone(), edge.to_module.clone());
            edge_weight_map
                .entry(key)
                .and_modify(|existing| existing.weight += 1)
                .or_insert(edge);
        }
        let all_edges: Vec<DependencyEdge> = edge_weight_map.into_values().collect();

        let graph = graph_builder::build_graph(&all_nodes, &all_edges);

        let nodes_vec: Vec<String> = all_nodes.iter().cloned().collect();
        let edges_pairs = scoring::edges_to_pairs(&all_edges);
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
            edges: all_edges,
            node_count: graph.node_count(),
            edge_count: graph.edge_count(),
            timestamp: commit_info.timestamp,
            drift: Some(drift),
        };

        db.insert_graph_snapshot(&snapshot)?;
        graphs_created += 1;

        prev_graph = Some(graph);

        // ── Progress indicator (every 25 commits or at the end) ──
        if (ci + 1).is_multiple_of(25) || ci + 1 == total_commits {
            let elapsed = scan_start.elapsed().as_secs_f64();
            let pct = ((ci + 1) as f64 / total_commits as f64 * 100.0) as u32;
            info!(
                "[{}/{}] {}% — {} graphs, {} cached blobs ({} hits), {:.1}s",
                ci + 1, total_commits, pct, graphs_created,
                blob_import_cache.len(), cache_hits, elapsed,
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
// Git tree walk — reads blobs in-memory via gix
// =============================================================================

#[allow(dead_code)]
fn walk_and_parse_tree<'repo>(
    repo: &'repo gix::Repository,
    tree: &gix::Tree<'repo>,
) -> Result<Vec<(PathBuf, Vec<u8>)>> {
    git_scanner::walk_tree_files(repo, tree)
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
#[allow(dead_code)]
pub fn path_to_module(path: &str) -> String {
    let path = path.replace("\\", "/");
    let without_ext = path.rsplit_once('.').map_or(path.as_str(), |(base, _)| base);
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
