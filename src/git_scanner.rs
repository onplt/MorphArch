//! Git repository scanner using gitoxide (gix).

use anyhow::{Context, Result};
use dashmap::DashMap;
use globset::GlobSet;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tracing::warn;

const SOURCE_EXTENSIONS: &[&str] = &["rs", "ts", "tsx", "js", "jsx", "mjs", "cjs", "py", "go"];
const SUBTREE_CACHE_VERSION: u32 = 1;
const TREE_WALK_SEMANTICS_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathChange {
    Delete(String),
    Upsert(String),
}

#[derive(Debug, Clone)]
pub struct IncrementalCommitPlan {
    pub commits: Vec<String>,
    pub baseline_hash: Option<String>,
    pub reached_stop: bool,
    pub truncated: bool,
    pub stop_commit_on_first_parent: bool,
}

pub fn resolve_repo_root(path: &Path) -> Result<PathBuf> {
    let repo =
        gix::discover(path).with_context(|| format!("Failed to open repo: {}", path.display()))?;
    let root = repo.workdir().map(Path::to_path_buf).unwrap_or_else(|| {
        repo.path()
            .parent()
            .unwrap_or_else(|| repo.path())
            .to_path_buf()
    });
    std::fs::canonicalize(&root)
        .with_context(|| format!("Failed to canonicalize repo root: {}", root.display()))
}

pub fn repo_id_for_path(path: &Path) -> Result<String> {
    let root = resolve_repo_root(path)?;
    Ok(root.to_string_lossy().replace('\\', "/"))
}

pub fn get_tree_for_commit(repo: &gix::Repository, commit_hash: &str) -> Result<gix::ObjectId> {
    let commit_oid = gix::ObjectId::from_hex(commit_hash.as_bytes())?;
    let obj = repo.find_object(commit_oid)?;
    let commit = obj.into_commit();
    let tree_id = commit.tree_id()?;
    Ok(tree_id.detach())
}

pub fn get_commits_in_order(repo: &gix::Repository, max: usize) -> Result<Vec<gix::Commit<'_>>> {
    first_parent_hashes(repo, max)?
        .into_iter()
        .map(|hash| {
            let oid = gix::ObjectId::from_hex(hash.as_bytes())?;
            repo.find_object(oid)
                .map(|obj| obj.into_commit())
                .with_context(|| format!("Failed to load commit {hash}"))
        })
        .collect()
}

pub fn first_parent_commit_count(repo: &gix::Repository) -> Result<usize> {
    Ok(first_parent_hashes(repo, usize::MAX)?.len())
}

pub fn get_commits_since<'a>(
    repo: &'a gix::Repository,
    stop_at_hash: &str,
    max: usize,
) -> Result<Vec<gix::Commit<'a>>> {
    let plan = plan_incremental_commits(repo, stop_at_hash, max)?;
    plan.commits
        .into_iter()
        .rev()
        .map(|hash| {
            let oid = gix::ObjectId::from_hex(hash.as_bytes())?;
            repo.find_object(oid)
                .map(|obj| obj.into_commit())
                .with_context(|| format!("Failed to load commit {hash}"))
        })
        .collect()
}

pub fn plan_incremental_commits(
    repo: &gix::Repository,
    stop_at_hash: &str,
    max: usize,
) -> Result<IncrementalCommitPlan> {
    let head_hash = repo.head_commit()?.id().to_string();
    Ok(plan_incremental_commits_from_lineage(
        first_parent_lineage_from(repo, &head_hash)?,
        stop_at_hash,
        max,
    ))
}

fn first_parent_hashes(repo: &gix::Repository, max: usize) -> Result<Vec<String>> {
    let head_hash = repo.head_commit()?.id().to_string();
    let mut hashes = first_parent_lineage_from(repo, &head_hash)?;
    if max != usize::MAX && hashes.len() > max {
        hashes.truncate(max);
    }
    Ok(hashes)
}

fn first_parent_lineage_from(repo: &gix::Repository, start_hash: &str) -> Result<Vec<String>> {
    let mut hashes = Vec::new();
    let mut current_hash = Some(start_hash.to_string());

    while let Some(hash) = current_hash {
        hashes.push(hash.clone());
        current_hash = first_parent_hash(repo, &hash)?;
    }

    Ok(hashes)
}

fn first_parent_hash(repo: &gix::Repository, commit_hash: &str) -> Result<Option<String>> {
    let commit_oid = gix::ObjectId::from_hex(commit_hash.as_bytes())?;
    let commit = repo.find_object(commit_oid)?.into_commit();
    Ok(commit.parent_ids().next().map(|id| id.detach().to_string()))
}

fn plan_incremental_commits_from_lineage(
    lineage: Vec<String>,
    stop_at_hash: &str,
    max: usize,
) -> IncrementalCommitPlan {
    let mut newest_hashes = Vec::new();
    let mut baseline_hash = None;
    let mut reached_stop = false;
    let mut truncated = false;

    for hash in lineage {
        if hash == stop_at_hash {
            reached_stop = true;
            break;
        }

        if max == usize::MAX || newest_hashes.len() < max {
            newest_hashes.push(hash);
        } else {
            truncated = true;
            if baseline_hash.is_none() {
                baseline_hash = Some(hash);
            }
        }
    }

    newest_hashes.reverse();
    IncrementalCommitPlan {
        commits: newest_hashes,
        baseline_hash,
        reached_stop,
        truncated,
        stop_commit_on_first_parent: reached_stop,
    }
}

#[derive(Default)]
pub struct SubtreeCache {
    pub entries: DashMap<gix::ObjectId, Arc<Vec<(String, gix::ObjectId)>>>,
}

#[derive(Debug, Serialize, Deserialize)]
struct PersistentSubtreeCache {
    version: u32,
    tree_walk_semantics_version: u32,
    source_extensions: Vec<String>,
    entries: Vec<PersistentSubtreeCacheEntry>,
}

#[derive(Debug, Serialize, Deserialize)]
struct PersistentSubtreeCacheEntry {
    tree_id: String,
    entries: Vec<(String, String)>,
}

impl SubtreeCache {
    pub fn new() -> Self {
        Self::default()
    }
}

pub fn load_persistent_subtree_cache(cache_dir: &Path, repo_id: &str) -> Result<SubtreeCache> {
    let cache_path = persistent_cache_path(cache_dir, repo_id);
    let Ok(bytes) = std::fs::read(&cache_path) else {
        return Ok(SubtreeCache::new());
    };

    let stored: PersistentSubtreeCache =
        match bincode::serde::decode_from_slice(&bytes, bincode::config::standard()) {
            Ok((stored, _)) => stored,
            Err(err) => {
                warn!(
                    path = %cache_path.display(),
                    error = %err,
                    "Ignoring corrupt subtree cache file"
                );
                return Ok(SubtreeCache::new());
            }
        };

    let expected_extensions: Vec<String> = SOURCE_EXTENSIONS
        .iter()
        .map(|ext| (*ext).to_string())
        .collect();
    if stored.version != SUBTREE_CACHE_VERSION
        || stored.tree_walk_semantics_version != TREE_WALK_SEMANTICS_VERSION
        || stored.source_extensions != expected_extensions
    {
        warn!(
            path = %cache_path.display(),
            "Ignoring stale subtree cache with incompatible schema or tree-walk semantics"
        );
        return Ok(SubtreeCache::new());
    }

    let cache = SubtreeCache::new();
    for entry in stored.entries {
        let tree_id = match gix::ObjectId::from_hex(entry.tree_id.as_bytes()) {
            Ok(tree_id) => tree_id,
            Err(err) => {
                warn!(
                    path = %cache_path.display(),
                    tree_id = %entry.tree_id,
                    error = %err,
                    "Ignoring subtree cache with invalid tree object id"
                );
                return Ok(SubtreeCache::new());
            }
        };
        let mapped_entries = match entry
            .entries
            .into_iter()
            .map(|(path, oid_hex): (String, String)| {
                gix::ObjectId::from_hex(oid_hex.as_bytes())
                    .map(|oid| (path, oid))
                    .with_context(|| format!("Invalid blob id in subtree cache: {oid_hex}"))
            })
            .collect::<Result<Vec<_>>>()
        {
            Ok(entries) => entries,
            Err(err) => {
                warn!(
                    path = %cache_path.display(),
                    error = %err,
                    "Ignoring subtree cache with invalid blob object ids"
                );
                return Ok(SubtreeCache::new());
            }
        };
        cache.entries.insert(tree_id, Arc::new(mapped_entries));
    }

    Ok(cache)
}

pub fn save_persistent_subtree_cache(
    cache_dir: &Path,
    repo_id: &str,
    cache: &SubtreeCache,
) -> Result<()> {
    std::fs::create_dir_all(cache_dir).with_context(|| {
        format!(
            "Failed to create subtree cache directory: {}",
            cache_dir.display()
        )
    })?;
    let cache_path = persistent_cache_path(cache_dir, repo_id);

    let entries = cache
        .entries
        .iter()
        .map(|item| PersistentSubtreeCacheEntry {
            tree_id: item.key().to_string(),
            entries: item
                .value()
                .iter()
                .map(|(path, oid)| (path.clone(), oid.to_string()))
                .collect(),
        })
        .collect();

    let bytes = bincode::serde::encode_to_vec(
        &PersistentSubtreeCache {
            version: SUBTREE_CACHE_VERSION,
            tree_walk_semantics_version: TREE_WALK_SEMANTICS_VERSION,
            source_extensions: SOURCE_EXTENSIONS
                .iter()
                .map(|ext| (*ext).to_string())
                .collect(),
            entries,
        },
        bincode::config::standard(),
    )
    .with_context(|| format!("Failed to encode subtree cache: {}", cache_path.display()))?;

    std::fs::write(&cache_path, bytes)
        .with_context(|| format!("Failed to write subtree cache: {}", cache_path.display()))
}

pub fn diff_paths_between(
    repo: &gix::Repository,
    old_commit: &str,
    new_commit: &str,
) -> Result<Vec<PathChange>> {
    let old_tree = repo
        .find_tree(get_tree_for_commit(repo, old_commit)?)
        .with_context(|| format!("Failed to load old tree for commit {old_commit}"))?;
    let new_tree = repo
        .find_tree(get_tree_for_commit(repo, new_commit)?)
        .with_context(|| format!("Failed to load new tree for commit {new_commit}"))?;

    let mut options = gix::diff::Options::default();
    options.track_path();
    options.track_rewrites(Some(gix::diff::Rewrites::default()));

    let changes = repo
        .diff_tree_to_tree(Some(&old_tree), Some(&new_tree), Some(options))
        .with_context(|| format!("Failed to diff commits {old_commit}..{new_commit}"))?;

    Ok(changes
        .into_iter()
        .flat_map(|change| match change {
            gix::object::tree::diff::ChangeDetached::Addition {
                entry_mode,
                location,
                ..
            } => {
                if entry_mode.is_blob() {
                    vec![PathChange::Upsert(path_to_string(location.as_ref()))]
                } else {
                    Vec::new()
                }
            }
            gix::object::tree::diff::ChangeDetached::Deletion {
                entry_mode,
                location,
                ..
            } => {
                if entry_mode.is_blob() {
                    vec![PathChange::Delete(path_to_string(location.as_ref()))]
                } else {
                    Vec::new()
                }
            }
            gix::object::tree::diff::ChangeDetached::Modification {
                entry_mode,
                location,
                ..
            } => {
                if entry_mode.is_blob() {
                    vec![PathChange::Upsert(path_to_string(location.as_ref()))]
                } else {
                    Vec::new()
                }
            }
            gix::object::tree::diff::ChangeDetached::Rewrite {
                source_entry_mode,
                entry_mode,
                source_location,
                location,
                copy,
                ..
            } => {
                let mut out = Vec::new();
                if source_entry_mode.is_blob() && !copy {
                    out.push(PathChange::Delete(path_to_string(source_location.as_ref())));
                }
                if entry_mode.is_blob() {
                    out.push(PathChange::Upsert(path_to_string(location.as_ref())));
                }
                out
            }
        })
        .collect())
}

pub fn list_blob_oids_for_paths(
    repo: &gix::Repository,
    commit_hash: &str,
    paths: &[String],
) -> Result<Vec<(String, gix::ObjectId)>> {
    if paths.is_empty() {
        return Ok(Vec::new());
    }

    let tree = repo
        .find_tree(get_tree_for_commit(repo, commit_hash)?)
        .with_context(|| format!("Failed to load tree for commit {commit_hash}"))?;
    let mut result = Vec::with_capacity(paths.len());
    for path in paths {
        let Some(entry) = tree
            .lookup_entry_by_path(Path::new(path))
            .with_context(|| format!("Failed to look up path {path} in commit {commit_hash}"))?
        else {
            continue;
        };
        let mode = entry.mode();
        if mode.is_blob() || mode.is_executable() {
            result.push((path.clone(), entry.object_id()));
        }
    }
    Ok(result)
}

pub fn walk_tree_entries_cached(
    repo: &gix::Repository,
    tree_oid: gix::ObjectId,
    cache: &SubtreeCache,
    ignore: Option<&GlobSet>,
) -> Result<Vec<(String, gix::ObjectId)>> {
    let raw = walk_tree_collect(repo, tree_oid, 0, cache)?;
    Ok(raw
        .iter()
        .filter(|(path, _)| ignore.is_none_or(|globs| !globs.is_match(path)))
        .map(|(path, oid)| (path.clone(), oid.to_owned()))
        .collect())
}

fn walk_tree_collect(
    repo: &gix::Repository,
    tree_oid: gix::ObjectId,
    depth: usize,
    cache: &SubtreeCache,
) -> Result<Arc<Vec<(String, gix::ObjectId)>>> {
    if depth > 30 {
        return Ok(Arc::new(Vec::new()));
    }

    if let Some(cached) = cache.entries.get(&tree_oid) {
        return Ok(Arc::clone(cached.value()));
    }

    let tree = repo.find_tree(tree_oid)?;
    let decoded = tree.decode()?;
    let mut result = Vec::new();
    for entry in &decoded.entries {
        let name = entry.filename.to_string();
        if entry.mode.is_tree() {
            let sub = walk_tree_collect(repo, entry.oid.to_owned(), depth + 1, cache)?;
            for (sub_path, blob_oid) in sub.iter() {
                result.push((format!("{name}/{sub_path}"), blob_oid.to_owned()));
            }
        } else if (entry.mode.is_blob() || entry.mode.is_executable())
            && let Some(ext) = name.rsplit('.').next()
            && SOURCE_EXTENSIONS.contains(&ext)
        {
            result.push((name, entry.oid.to_owned()));
        }
    }

    let cached = Arc::new(result);
    cache.entries.insert(tree_oid, Arc::clone(&cached));
    Ok(cached)
}

fn path_to_string(path: &[u8]) -> String {
    String::from_utf8_lossy(path).replace('\\', "/")
}

fn persistent_cache_path(cache_dir: &Path, repo_id: &str) -> PathBuf {
    cache_dir.join(format!("{}.bin", stable_repo_cache_key(repo_id)))
}

fn stable_repo_cache_key(repo_id: &str) -> String {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in repo_id.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

#[cfg(test)]
mod tests {
    use super::{
        load_persistent_subtree_cache, path_to_string, plan_incremental_commits_from_lineage,
        stable_repo_cache_key,
    };
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn path_to_string_normalizes_separators() {
        assert_eq!(path_to_string(b"src\\main.ts"), "src/main.ts");
    }

    #[test]
    fn repo_cache_key_is_stable() {
        assert_eq!(
            stable_repo_cache_key("C:/repo"),
            stable_repo_cache_key("C:/repo")
        );
    }

    #[test]
    fn corrupt_subtree_cache_is_ignored() {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let cache_dir = std::env::temp_dir().join(format!("morpharch-cache-test-{suffix}"));
        std::fs::create_dir_all(&cache_dir).unwrap();
        let cache_file = cache_dir.join(format!("{}.bin", stable_repo_cache_key("repo/test")));
        std::fs::write(&cache_file, b"not-bincode").unwrap();

        let cache = load_persistent_subtree_cache(&cache_dir, "repo/test").unwrap();
        assert_eq!(cache.entries.len(), 0);

        let _ = std::fs::remove_file(cache_file);
        let _ = std::fs::remove_dir_all(cache_dir);
    }

    #[test]
    fn incremental_plan_detects_stop_beyond_truncated_window_on_first_parent() {
        let plan = plan_incremental_commits_from_lineage(
            vec![
                "head".to_string(),
                "c4".to_string(),
                "c3".to_string(),
                "c2".to_string(),
                "c1".to_string(),
                "base".to_string(),
            ],
            "base",
            3,
        );

        assert!(plan.truncated);
        assert!(plan.reached_stop);
        assert!(plan.stop_commit_on_first_parent);
        assert_eq!(
            plan.commits,
            vec!["c3".to_string(), "c4".to_string(), "head".to_string()]
        );
        assert_eq!(plan.baseline_hash.as_deref(), Some("c2"));
    }

    #[test]
    fn incremental_plan_marks_diverged_history_even_when_truncated() {
        let plan = plan_incremental_commits_from_lineage(
            vec![
                "head".to_string(),
                "c4".to_string(),
                "c3".to_string(),
                "c2".to_string(),
                "c1".to_string(),
            ],
            "base",
            2,
        );

        assert!(plan.truncated);
        assert!(!plan.reached_stop);
        assert!(!plan.stop_commit_on_first_parent);
    }
}
