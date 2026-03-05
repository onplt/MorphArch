//! Git repository scanner using gitoxide (gix).

use dashmap::DashMap;
use anyhow::{Result};
use std::path::PathBuf;

pub fn get_tree_for_commit(repo: &gix::Repository, commit_hash: &str) -> Result<gix::ObjectId> {
    let commit_oid = gix::ObjectId::from_hex(commit_hash.as_bytes())?;
    let obj = repo.find_object(commit_oid)?;
    let commit = obj.into_commit();
    let tree_id = commit.tree_id()?;
    Ok(tree_id.detach())
}

pub fn get_commits_in_order(repo: &gix::Repository, max: usize) -> Result<Vec<gix::Commit<'_>>> {
    let head = repo.head_commit()?;
    let mut commits = Vec::new();
    let ancestors = head.ancestors().all()?;
    for ancestor_result in ancestors {
        if commits.len() >= max { break; }
        let ancestor_info = ancestor_result?;
        commits.push(repo.find_object(ancestor_info.id)?.into_commit());
    }
    Ok(commits)
}

pub fn get_commits_since<'a>(repo: &'a gix::Repository, stop_at_hash: &str, max: usize) -> Result<Vec<gix::Commit<'a>>> {
    let head = repo.head_commit()?;
    let mut commits = Vec::new();
    let ancestors = head.ancestors().all()?;
    for ancestor_result in ancestors {
        if commits.len() >= max { break; }
        let ancestor_info = ancestor_result?;
        let hash = ancestor_info.id.to_string();
        if hash == stop_at_hash { break; }
        commits.push(repo.find_object(ancestor_info.id)?.into_commit());
    }
    Ok(commits)
}

#[derive(Default)]
pub struct SubtreeCache {
    pub entries: DashMap<gix::ObjectId, Vec<(String, gix::ObjectId)>>,
}

impl SubtreeCache {
    pub fn new() -> Self { Self::default() }
}

pub fn walk_tree_entries_cached(
    repo: &gix::Repository,
    tree_oid: gix::ObjectId,
    cache: &SubtreeCache,
) -> Result<Vec<(PathBuf, gix::ObjectId)>> {
    let raw = walk_tree_collect(repo, tree_oid, 0, cache)?;
    Ok(raw.into_iter().map(|(p, oid)| (PathBuf::from(p), oid)).collect())
}

fn walk_tree_collect(
    repo: &gix::Repository,
    tree_oid: gix::ObjectId,
    depth: usize,
    cache: &SubtreeCache,
) -> Result<Vec<(String, gix::ObjectId)>> {
    if depth > 30 { return Ok(Vec::new()); }
    if let Some(cached) = cache.entries.get(&tree_oid) {
        let entries: &Vec<(String, gix::ObjectId)> = cached.value();
        return Ok(entries.clone());
    }
    let tree = repo.find_tree(tree_oid)?;
    let decoded = tree.decode()?;
    let mut result = Vec::new();
    for entry in &decoded.entries {
        let name = entry.filename.to_string();
        if entry.mode.is_tree() {
            let sub = walk_tree_collect(repo, entry.oid.to_owned(), depth + 1, cache)?;
            for (sub_path, blob_oid) in sub { result.push((format!("{}/{}", name, sub_path), blob_oid)); }
        } else if entry.mode.is_blob() || entry.mode.is_executable() {
            if let Some(ext) = name.rsplit('.').next() {
                if ["rs", "ts", "tsx", "py", "go"].contains(&ext) { result.push((name, entry.oid.to_owned())); }
            }
        }
    }
    cache.entries.insert(tree_oid, result.clone());
    Ok(result)
}
