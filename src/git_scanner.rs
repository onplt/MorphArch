// =============================================================================
// git_scanner.rs — Git repository scanner
// =============================================================================
//
// Uses the gitoxide (gix) library to scan Git repositories:
//   1. Discovers and opens the repo with gix::discover()
//   2. Walks backwards from HEAD commit (ancestor walk)
//   3. Extracts metadata for each commit (author, message, time, tree)
//   4. Saves extracted data to SQLite database
//
// Performance notes:
//   - gix is faster than libgit2 and pure-Rust
//   - Ancestor walk uses lazy iterator (memory-friendly)
//   - Progress is logged via debug every 100 commits
// =============================================================================

use std::collections::HashMap;

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use tracing::{debug, info, warn};

use crate::db::Database;
use crate::models::CommitInfo;

/// Discovers and opens a Git repository.
///
/// Searches upward from the given directory for `.git`.
/// Can be used by `commands::scan` and other modules.
///
/// # Errors
/// - If the path is not a valid Git repository
#[allow(dead_code)]
pub fn open_repository(path: &Path) -> Result<gix::Repository> {
    gix::discover(path).with_context(|| {
        format!(
            "'{}' is not a valid Git repository. \
             No .git directory found.",
            path.display()
        )
    })
}

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

/// Recursively walks a Git tree object and returns files with their contents.
pub fn walk_tree_files<'repo>(
    repo: &'repo gix::Repository,
    tree: &gix::Tree<'repo>,
) -> Result<Vec<(PathBuf, Vec<u8>)>> {
    let mut files = Vec::new();
    walk_tree_recursive_new(repo, tree.id, "", &mut files, 0)?;
    Ok(files)
}

/// Recursive tree walking function. Supports Windows paths, returns file contents.
fn walk_tree_recursive_new(
    repo: &gix::Repository,
    tree_oid: gix::ObjectId,
    prefix: &str,
    files: &mut Vec<(PathBuf, Vec<u8>)>,
    depth: usize,
) -> Result<()> {
    // Depth guard
    if depth > 30 {
        return Ok(());
    }

    let tree = repo
        .find_tree(tree_oid)
        .with_context(|| format!("Tree object not found: {tree_oid}"))?;
    let decoded = tree.decode().context("Failed to decode tree")?;

    for entry in &decoded.entries {
        let name = entry.filename.to_string();

        let path = if prefix.is_empty() {
            name
        } else {
            format!("{}/{}", prefix, name)
        };

        if entry.mode.is_tree() {
            walk_tree_recursive_new(repo, entry.oid.to_owned(), &path, files, depth + 1)?;
        } else if entry.mode.is_blob() || entry.mode.is_executable() {
            if let Some(ext) = path.rsplit('.').next() {
                if ["rs", "ts", "tsx", "py", "go"].contains(&ext) {
                    match repo.find_object(entry.oid.to_owned()) {
                        Ok(blob) => {
                            files.push((PathBuf::from(&path), blob.data.to_vec()));
                        }
                        Err(e) => {
                            debug!(path = %path, error = %e, "Failed to read blob, skipping");
                        }
                    }
                }
            }
        }
    }

    Ok(())
}

/// Recursively walks a Git tree and returns `(path, blob_oid)` pairs WITHOUT
/// reading blob content. This is the fast path for incremental scanning:
/// the caller uses blob OID caching to avoid re-reading unchanged files.
///
/// Performance: Only reads tree objects (~100 bytes each), never blobs
/// (~KB–MB each). Combined with OID caching this reduces per-commit I/O
/// from O(all_files) to O(changed_files).
#[allow(dead_code)]
pub fn walk_tree_entries(
    repo: &gix::Repository,
    tree: &gix::Tree<'_>,
) -> Result<Vec<(PathBuf, gix::ObjectId)>> {
    let mut entries = Vec::new();
    walk_tree_entries_recursive(repo, tree.id, "", &mut entries, 0)?;
    Ok(entries)
}

#[allow(dead_code)]
fn walk_tree_entries_recursive(
    repo: &gix::Repository,
    tree_oid: gix::ObjectId,
    prefix: &str,
    entries: &mut Vec<(PathBuf, gix::ObjectId)>,
    depth: usize,
) -> Result<()> {
    if depth > 30 {
        return Ok(());
    }

    let tree = repo
        .find_tree(tree_oid)
        .with_context(|| format!("Tree object not found: {tree_oid}"))?;
    let decoded = tree.decode().context("Failed to decode tree")?;

    for entry in &decoded.entries {
        let name = entry.filename.to_string();
        let path = if prefix.is_empty() {
            name
        } else {
            format!("{}/{}", prefix, name)
        };

        if entry.mode.is_tree() {
            walk_tree_entries_recursive(repo, entry.oid.to_owned(), &path, entries, depth + 1)?;
        } else if entry.mode.is_blob() || entry.mode.is_executable() {
            if let Some(ext) = path.rsplit('.').next() {
                if ["rs", "ts", "tsx", "py", "go"].contains(&ext) {
                    entries.push((PathBuf::from(&path), entry.oid.to_owned()));
                }
            }
        }
    }

    Ok(())
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

    #[allow(dead_code)]
    pub fn len(&self) -> usize {
        self.entries.len()
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

/// Scans the Git repository at the specified directory and saves commit metadata to the DB.
///
/// # Parameters
/// - `path`: Root directory of the Git repository (or any directory inside it)
/// - `db`: Database reference where commits will be written
/// - `max_commits`: Maximum number of commits to read (default 500)
///
/// # Returns
/// Number of commits successfully scanned.
///
/// # Errors
/// - If path is not a valid Git repository
/// - If repository is empty (no HEAD commit)
/// - If commit objects cannot be read or decoded
/// - If database write errors occur
#[allow(dead_code)]
pub fn scan_repository(path: &Path, db: &Database, max_commits: usize) -> Result<usize> {
    info!(path = %path.display(), "Opening Git repository");

    // Discover Git repository — use open_repository helper
    let repo = open_repository(path)?;

    // Get HEAD commit
    let head = repo.head_commit().context(
        "HEAD commit not found. \
         The repository may be empty — at least one commit is required.",
    )?;

    info!(head = %head.id, "HEAD commit found");

    let mut count: usize = 0;

    // Walk all ancestor commits from HEAD backwards
    let ancestors = head.ancestors().all().context(
        "Failed to start commit history walk. \
         Check repository integrity with 'git fsck'.",
    )?;

    for ancestor_result in ancestors {
        // Stop if maximum limit reached
        if count >= max_commits {
            info!(
                limit = max_commits,
                "Maximum commit limit reached, stopping scan"
            );
            break;
        }

        // Get ancestor info (ID + parent IDs)
        let ancestor_info = match ancestor_result {
            Ok(info) => info,
            Err(e) => {
                warn!(error = %e, "Failed to read a commit, skipping");
                continue;
            }
        };

        // Load and decode commit object
        let commit_object = repo
            .find_object(ancestor_info.id)
            .with_context(|| format!("Failed to load commit object: {}", ancestor_info.id))?;

        let commit = commit_object.into_commit();
        let decoded = commit
            .decode()
            .with_context(|| format!("Failed to decode commit: {}", ancestor_info.id))?;

        // Extract commit metadata
        let commit_info = CommitInfo {
            hash: ancestor_info.id.to_string(),
            author_name: decoded.author.name.to_string(),
            author_email: decoded.author.email.to_string(),
            message: decoded.message.to_string(),
            timestamp: decoded.author.time.seconds,
            tree_id: decoded.tree().to_string(),
        };

        // Save to database
        db.insert_commit(&commit_info)?;
        count += 1;

        // Progress report every 100 commits
        if count.is_multiple_of(100) {
            debug!(count, "Scan progress");
        }
    }

    info!(total = count, "Repository scan complete");
    Ok(count)
}

// =============================================================================
// Tests
// =============================================================================
#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;
    use std::path::PathBuf;

    /// Tests that git_scanner works correctly by scanning the current repo
    /// (MorphArch itself).
    ///
    /// - The repo should have at least 1 commit (Initial commit)
    /// - Limited to max_commits=10
    /// - Scanned count should be > 0 and <= 10
    /// - DB record count should match scanned count
    #[test]
    fn test_scan_current_repo() {
        let db = Database::open_in_memory().expect("In-memory DB should open");

        // Current project directory is a Git repository
        let path = PathBuf::from(".");
        let count = scan_repository(&path, &db, 10).expect("Scan should succeed");

        assert!(count > 0, "Should find at least 1 commit");
        assert!(count <= 10, "Should respect max_commits=10 limit");

        // DB record count should match
        let db_count = db.commit_count().expect("Count should succeed");
        assert_eq!(count, db_count, "Scanned and saved counts should match");
    }

    /// Tests that an invalid directory returns a meaningful error.
    #[test]
    fn test_scan_invalid_path_returns_error() {
        let db = Database::open_in_memory().expect("In-memory DB should open");

        let result = scan_repository(Path::new("/nonexistent/fake/repo"), &db, 10);
        assert!(result.is_err(), "Invalid path should return error");

        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("not a valid Git repository"),
            "Error message should be descriptive, but got: {err_msg}"
        );
    }
}
