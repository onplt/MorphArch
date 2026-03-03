//! Git repository scanner using gitoxide (gix).
//!
//! Provides commit walking, tree traversal, and blob reading for the scan pipeline.
//!
//! # Performance
//!
//! - **Pure Rust**: `gix` avoids FFI overhead and is faster than `libgit2`.
//! - **Subtree caching**: [`SubtreeCache`] makes tree walks `O(changed_dirs)`
//!   instead of `O(all_dirs)` across consecutive commits.
//! - **Lazy iteration**: Ancestor walks use iterators to stay memory-friendly.

use std::collections::HashMap;

use anyhow::{Context, Result};
use std::path::PathBuf;
use tracing::{debug, warn};

/// Returns the tree ObjectId for a given commit hash.
///
/// Loads the commit object directly from git and uses `Commit::tree_id()`
/// to get the tree OID. This is more reliable than the DB's tree_id column.
///
/// # Parameters
/// - `repo`: Open gix Repository reference
/// - `commit_hash`: 40-character hex commit hash
///
/// # Errors
/// - If commit hash cannot be parsed
/// - If commit object cannot be found
/// - If commit cannot be decoded
pub fn get_tree_for_commit(repo: &gix::Repository, commit_hash: &str) -> Result<gix::ObjectId> {
    let commit_oid = gix::ObjectId::from_hex(commit_hash.as_bytes())
        .with_context(|| format!("Failed to parse commit hash as hex: {commit_hash}"))?;

    let obj = repo
        .find_object(commit_oid)
        .with_context(|| format!("Commit object not found: {commit_hash}"))?;

    let commit = obj.into_commit();
    let tree_id = commit
        .tree_id()
        .with_context(|| format!("Failed to get tree ID for commit: {commit_hash}"))?;

    debug!(
        commit = %commit_hash,
        tree = %tree_id,
        "Resolved commit → tree OID"
    );

    Ok(tree_id.detach())
}

/// Returns the list of valid commits in order.
pub fn get_commits_in_order<'repo>(
    repo: &'repo gix::Repository,
    max: usize,
) -> Result<Vec<gix::Commit<'repo>>> {
    let head = repo
        .head_commit()
        .context("HEAD commit not found. The repository may be empty.")?;

    let mut commits = Vec::new();
    let mut count = 0;

    let ancestors = head
        .ancestors()
        .all()
        .context("Failed to start commit history walk.")?;

    for ancestor_result in ancestors {
        if count >= max {
            break;
        }

        let ancestor_info = match ancestor_result {
            Ok(info) => info,
            Err(e) => {
                warn!(error = %e, "Failed to read a commit, skipping");
                continue;
            }
        };

        let commit = repo
            .find_object(ancestor_info.id)
            .with_context(|| format!("Failed to load commit object: {}", ancestor_info.id))?
            .into_commit();

        commits.push(commit);
        count += 1;
    }

    Ok(commits)
}

/// Returns commits from HEAD to a known commit (exclusive).
///
/// Used for incremental scanning — only returns new commits since the
/// last scanned commit. Walks backwards from HEAD and stops when
/// `stop_at_hash` is encountered.
pub fn get_commits_since<'repo>(
    repo: &'repo gix::Repository,
    stop_at_hash: &str,
    max: usize,
) -> Result<Vec<gix::Commit<'repo>>> {
    let head = repo
        .head_commit()
        .context("HEAD commit not found. The repository may be empty.")?;

    let mut commits = Vec::new();

    let ancestors = head
        .ancestors()
        .all()
        .context("Failed to start commit history walk.")?;

    for ancestor_result in ancestors {
        if commits.len() >= max {
            break;
        }

        let ancestor_info = match ancestor_result {
            Ok(info) => info,
            Err(e) => {
                warn!(error = %e, "Failed to read a commit, skipping");
                continue;
            }
        };

        let hash = ancestor_info.id.to_string();
        if hash == stop_at_hash {
            break; // Reached the last known commit — stop here
        }

        let commit = repo
            .find_object(ancestor_info.id)
            .with_context(|| format!("Failed to load commit object: {}", ancestor_info.id))?
            .into_commit();

        commits.push(commit);
    }

    Ok(commits)
}

// =============================================================================
// Subtree-cached tree walk — O(changed_dirs) instead of O(all_dirs)
// =============================================================================

/// Cache for subtree walk results. Persists across commits so unchanged
/// directory subtrees (same tree OID) are never re-traversed.
///
/// A typical commit changes 1-3 files → 1-5 subtrees out of ~200.
/// With caching, commit N+1 reads only the changed subtrees.
pub struct SubtreeCache {
    /// tree_oid → Vec<(relative_path, blob_oid)> for supported files
    entries: HashMap<gix::ObjectId, Vec<(String, gix::ObjectId)>>,
}

impl SubtreeCache {
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }
}

/// Walks a Git tree with **subtree-level caching**.
///
/// Unchanged subtrees (same tree OID across commits) return cached
/// file entries without any git object reads. Only new/modified
/// subtrees trigger actual I/O.
pub fn walk_tree_entries_cached(
    repo: &gix::Repository,
    tree: &gix::Tree<'_>,
    cache: &mut SubtreeCache,
) -> Result<Vec<(PathBuf, gix::ObjectId)>> {
    let raw = walk_tree_collect(repo, tree.id, 0, cache)?;
    Ok(raw
        .into_iter()
        .map(|(p, oid)| (PathBuf::from(p), oid))
        .collect())
}

/// Recursive helper — returns entries with paths relative to the given tree.
/// Results are cached per tree OID so identical subtrees across commits
/// are traversed only once.
fn walk_tree_collect(
    repo: &gix::Repository,
    tree_oid: gix::ObjectId,
    depth: usize,
    cache: &mut SubtreeCache,
) -> Result<Vec<(String, gix::ObjectId)>> {
    if depth > 30 {
        return Ok(Vec::new());
    }

    // ── Subtree cache hit → zero git I/O ──
    if let Some(cached) = cache.entries.get(&tree_oid) {
        return Ok(cached.clone());
    }

    // ── Cache miss → read tree object and recurse ──
    let tree = repo
        .find_tree(tree_oid)
        .with_context(|| format!("Tree object not found: {tree_oid}"))?;
    let decoded = tree.decode().context("Failed to decode tree")?;
    let mut result: Vec<(String, gix::ObjectId)> = Vec::new();

    for entry in &decoded.entries {
        let name = entry.filename.to_string();

        if entry.mode.is_tree() {
            let sub = walk_tree_collect(repo, entry.oid.to_owned(), depth + 1, cache)?;
            for (sub_path, blob_oid) in sub {
                result.push((format!("{}/{}", name, sub_path), blob_oid));
            }
        } else if entry.mode.is_blob() || entry.mode.is_executable() {
            if let Some(ext) = name.rsplit('.').next() {
                if ["rs", "ts", "tsx", "py", "go"].contains(&ext) {
                    result.push((name, entry.oid.to_owned()));
                }
            }
        }
    }

    // Store in cache for future commits
    cache.entries.insert(tree_oid, result.clone());
    Ok(result)
}
