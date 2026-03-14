//! SQLite persistence layer for MorphArch.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension};
use tracing::{debug, info, warn};

use crate::models::{
    CommitInfo, DependencyEdge, DriftScore, EdgeOrigin, GraphCheckpoint, GraphDelta, GraphSnapshot,
    HeavySnapshotArtifacts, NodeKind, NodeMetadata, RepoScanState, ScanMetadata, SnapshotFrame,
    SnapshotMetadata,
};

pub struct Database {
    conn: Connection,
}

#[derive(Debug, Default, Clone)]
pub struct SnapshotLoadResult {
    pub snapshots: Vec<GraphSnapshot>,
    pub skipped_corrupt: usize,
}

fn serialize_blob<T: serde::Serialize>(value: &T, label: &str) -> Result<Vec<u8>> {
    bincode::serde::encode_to_vec(value, bincode::config::standard())
        .with_context(|| format!("Failed to serialize {label}"))
}

fn deserialize_blob<T: serde::de::DeserializeOwned>(bytes: &[u8], label: &str) -> Result<T> {
    let (value, _): (T, usize) =
        bincode::serde::decode_from_slice(bytes, bincode::config::standard())
            .with_context(|| format!("Failed to deserialize {label}"))?;
    Ok(value)
}

fn map_anyhow(err: anyhow::Error) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(
        0,
        rusqlite::types::Type::Blob,
        Box::new(std::io::Error::other(err.to_string())),
    )
}

impl Database {
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)
            .with_context(|| format!("Failed to open database: {}", path.display()))?;
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA synchronous=NORMAL;
             PRAGMA cache_size=-64000;
             PRAGMA temp_store=MEMORY;",
        )?;

        let db = Self { conn };
        db.migrate()?;
        Ok(db)
    }

    #[cfg(test)]
    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory().context("Failed to create in-memory database")?;
        let db = Self { conn };
        db.migrate()?;
        Ok(db)
    }

    fn migrate(&self) -> Result<()> {
        self.invalidate_legacy_table_if_needed(
            "commits",
            &[
                "repo_id",
                "hash",
                "author_name",
                "author_email",
                "message",
                "timestamp",
                "tree_id",
            ],
        )?;
        self.invalidate_legacy_table_if_needed(
            "snapshot_frames",
            &[
                "repo_id",
                "commit_hash",
                "scan_order",
                "timestamp",
                "node_count",
                "edge_count",
                "drift_bin",
                "scan_metadata_bin",
                "delta_bin",
                "analysis_version",
                "config_fingerprint",
                "has_full_artifacts",
            ],
        )?;
        self.invalidate_legacy_table_if_needed(
            "graph_checkpoints",
            &[
                "repo_id",
                "commit_hash",
                "scan_order",
                "state_bin",
                "full_artifacts_bin",
            ],
        )?;
        self.invalidate_legacy_table_if_needed(
            "repo_scan_state",
            &["repo_id", "commit_hash", "state_bin"],
        )?;
        if self.table_exists("graph_snapshots")? {
            self.conn
                .execute_batch("DROP TABLE IF EXISTS graph_snapshots;")
                .context("Failed to drop legacy graph_snapshots table")?;
        }

        self.conn
            .execute_batch(
                "
                CREATE TABLE IF NOT EXISTS repositories (
                    repo_id   TEXT PRIMARY KEY,
                    repo_root TEXT NOT NULL UNIQUE
                );

                CREATE TABLE IF NOT EXISTS commits (
                    repo_id      TEXT NOT NULL,
                    hash         TEXT NOT NULL,
                    author_name  TEXT NOT NULL,
                    author_email TEXT NOT NULL,
                    message      TEXT NOT NULL,
                    timestamp    INTEGER NOT NULL,
                    tree_id      TEXT NOT NULL,
                    PRIMARY KEY (repo_id, hash)
                );
                CREATE INDEX IF NOT EXISTS idx_commits_repo_timestamp
                    ON commits(repo_id, timestamp);

                CREATE TABLE IF NOT EXISTS snapshot_frames (
                    repo_id             TEXT NOT NULL,
                    commit_hash         TEXT NOT NULL,
                    scan_order          INTEGER NOT NULL,
                    timestamp           INTEGER NOT NULL,
                    node_count          INTEGER NOT NULL DEFAULT 0,
                    edge_count          INTEGER NOT NULL DEFAULT 0,
                    drift_bin           BLOB DEFAULT NULL,
                    scan_metadata_bin   BLOB NOT NULL,
                    delta_bin           BLOB NOT NULL,
                    analysis_version    INTEGER NOT NULL,
                    config_fingerprint  TEXT NOT NULL,
                    has_full_artifacts  INTEGER NOT NULL DEFAULT 0,
                    PRIMARY KEY (repo_id, commit_hash),
                    UNIQUE (repo_id, scan_order)
                );
                CREATE INDEX IF NOT EXISTS idx_snapshot_frames_repo_scan_order
                    ON snapshot_frames(repo_id, scan_order DESC);

                CREATE TABLE IF NOT EXISTS graph_checkpoints (
                    repo_id             TEXT NOT NULL,
                    commit_hash         TEXT NOT NULL,
                    scan_order          INTEGER NOT NULL,
                    state_bin           BLOB NOT NULL,
                    full_artifacts_bin  BLOB DEFAULT NULL,
                    PRIMARY KEY (repo_id, scan_order),
                    UNIQUE (repo_id, commit_hash)
                );
                CREATE INDEX IF NOT EXISTS idx_graph_checkpoints_repo_scan_order
                    ON graph_checkpoints(repo_id, scan_order DESC);

                CREATE TABLE IF NOT EXISTS repo_scan_state (
                    repo_id      TEXT PRIMARY KEY,
                    commit_hash  TEXT NOT NULL,
                    state_bin    BLOB NOT NULL
                );
                ",
            )
            .context("Database migration failed")?;

        info!("Database migration complete");
        Ok(())
    }

    fn invalidate_legacy_table_if_needed(
        &self,
        table: &str,
        required_columns: &[&str],
    ) -> Result<()> {
        if !self.table_exists(table)? {
            return Ok(());
        }

        let columns = self.table_columns(table)?;
        let is_compatible = required_columns
            .iter()
            .all(|column| columns.iter().any(|existing| existing == column));
        if is_compatible {
            return Ok(());
        }

        self.conn
            .execute_batch(&format!("DROP TABLE IF EXISTS {table};"))
            .with_context(|| format!("Failed to invalidate legacy table: {table}"))?;
        debug!(table, "Legacy cache table invalidated");
        Ok(())
    }

    fn table_exists(&self, table: &str) -> Result<bool> {
        self.conn
            .query_row(
                "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1 LIMIT 1",
                [table],
                |_| Ok(()),
            )
            .optional()
            .map(|row| row.is_some())
            .context("Failed to check table existence")
    }

    fn table_columns(&self, table: &str) -> Result<Vec<String>> {
        let mut stmt = self
            .conn
            .prepare(&format!("PRAGMA table_info({table})"))
            .with_context(|| format!("Failed to inspect table: {table}"))?;

        stmt.query_map([], |row| row.get::<_, String>(1))
            .with_context(|| format!("Failed to query table info: {table}"))?
            .collect::<Result<Vec<_>, _>>()
            .with_context(|| format!("Failed to collect table columns: {table}"))
    }

    pub fn begin_transaction(&self) -> Result<()> {
        self.conn
            .execute_batch("BEGIN TRANSACTION")
            .context("Failed to begin transaction")
    }

    pub fn commit_transaction(&self) -> Result<()> {
        self.conn
            .execute_batch("COMMIT")
            .context("Failed to commit transaction")
    }

    pub fn ensure_repository(&self, repo_id: &str) -> Result<()> {
        self.conn
            .execute(
                "INSERT OR IGNORE INTO repositories (repo_id, repo_root) VALUES (?1, ?2)",
                rusqlite::params![repo_id, repo_id],
            )
            .with_context(|| format!("Failed to register repository: {repo_id}"))?;
        Ok(())
    }

    pub fn clear_repo_graph_snapshots(&self, repo_id: &str) -> Result<()> {
        self.ensure_repository(repo_id)?;
        self.conn
            .execute("DELETE FROM snapshot_frames WHERE repo_id = ?1", [repo_id])
            .with_context(|| format!("Failed to clear snapshot frames for repo: {repo_id}"))?;
        self.conn
            .execute(
                "DELETE FROM graph_checkpoints WHERE repo_id = ?1",
                [repo_id],
            )
            .with_context(|| format!("Failed to clear checkpoints for repo: {repo_id}"))?;
        self.conn
            .execute("DELETE FROM commits WHERE repo_id = ?1", [repo_id])
            .with_context(|| format!("Failed to clear commits for repo: {repo_id}"))?;
        self.conn
            .execute("DELETE FROM repo_scan_state WHERE repo_id = ?1", [repo_id])
            .with_context(|| format!("Failed to clear scan state for repo: {repo_id}"))?;
        info!(repo_id, "Repository snapshot data cleared from DB");
        Ok(())
    }

    pub fn load_repo_scan_state(&self, repo_id: &str) -> Result<Option<(String, RepoScanState)>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT commit_hash, state_bin
                 FROM repo_scan_state
                 WHERE repo_id = ?1",
            )
            .context("Failed to prepare repo scan state query")?;

        let row = stmt
            .query_row([repo_id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, Vec<u8>>(1)?))
            })
            .optional()
            .context("Failed to load repo scan state")?;

        row.map(|(commit_hash, bytes)| {
            deserialize_blob::<RepoScanState>(&bytes, &format!("repo scan state for {repo_id}"))
                .map(|state| (commit_hash, state))
        })
        .transpose()
    }

    pub fn save_repo_scan_state(
        &self,
        repo_id: &str,
        commit_hash: &str,
        state: &RepoScanState,
    ) -> Result<()> {
        self.ensure_repository(repo_id)?;
        let state_bin = serialize_blob(state, &format!("repo scan state for {repo_id}"))?;
        self.conn
            .execute(
                "INSERT OR REPLACE INTO repo_scan_state (repo_id, commit_hash, state_bin)
                 VALUES (?1, ?2, ?3)",
                rusqlite::params![repo_id, commit_hash, state_bin],
            )
            .with_context(|| format!("Failed to save repo scan state: {repo_id}"))?;
        Ok(())
    }

    pub fn insert_commit(&self, repo_id: &str, commit: &CommitInfo) -> Result<()> {
        self.ensure_repository(repo_id)?;
        self.conn
            .execute(
                "INSERT OR REPLACE INTO commits
                    (repo_id, hash, author_name, author_email, message, timestamp, tree_id)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                rusqlite::params![
                    repo_id,
                    commit.hash,
                    commit.author_name,
                    commit.author_email,
                    commit.message,
                    commit.timestamp,
                    commit.tree_id,
                ],
            )
            .with_context(|| format!("Failed to write commit to database: {}", &commit.hash))?;
        Ok(())
    }

    #[cfg(test)]
    pub fn list_commits(&self, repo_id: &str) -> Result<Vec<CommitInfo>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT hash, author_name, author_email, message, timestamp, tree_id
                 FROM commits
                 WHERE repo_id = ?1
                 ORDER BY timestamp DESC",
            )
            .context("Failed to prepare commit list query")?;

        let commits = stmt
            .query_map([repo_id], |row| {
                Ok(CommitInfo {
                    hash: row.get(0)?,
                    author_name: row.get(1)?,
                    author_email: row.get(2)?,
                    message: row.get(3)?,
                    timestamp: row.get(4)?,
                    tree_id: row.get(5)?,
                })
            })
            .context("Failed to execute commit list query")?
            .collect::<Result<Vec<_>, _>>()
            .context("Failed to read commit data")?;

        Ok(commits)
    }

    pub fn commit_count(&self, repo_id: &str) -> Result<usize> {
        let count: i64 = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM commits WHERE repo_id = ?1",
                [repo_id],
                |row| row.get(0),
            )
            .context("Failed to query commit count")?;
        Ok(count as usize)
    }

    pub fn insert_snapshot_frame(&self, repo_id: &str, frame: &SnapshotFrame) -> Result<()> {
        self.ensure_repository(repo_id)?;
        let drift_bin = frame
            .drift
            .as_ref()
            .map(|drift| serialize_blob(drift, &format!("drift {}", frame.commit_hash)))
            .transpose()?;
        let scan_metadata_bin = serialize_blob(
            &frame.scan_metadata,
            &format!("scan metadata {}", frame.commit_hash),
        )?;
        let delta_bin =
            serialize_blob(&frame.delta, &format!("graph delta {}", frame.commit_hash))?;

        self.conn
            .execute(
                "INSERT OR REPLACE INTO snapshot_frames
                    (repo_id, commit_hash, scan_order, timestamp, node_count, edge_count,
                     drift_bin, scan_metadata_bin, delta_bin, analysis_version,
                     config_fingerprint, has_full_artifacts)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                rusqlite::params![
                    repo_id,
                    frame.commit_hash,
                    frame.scan_order,
                    frame.timestamp,
                    frame.node_count as i64,
                    frame.edge_count as i64,
                    drift_bin,
                    scan_metadata_bin,
                    delta_bin,
                    frame.analysis_version as i64,
                    frame.config_fingerprint,
                    frame.has_full_artifacts as i64,
                ],
            )
            .with_context(|| format!("Failed to insert snapshot frame {}", frame.commit_hash))?;
        Ok(())
    }

    pub fn insert_graph_checkpoint(
        &self,
        repo_id: &str,
        checkpoint: &GraphCheckpoint,
    ) -> Result<()> {
        self.ensure_repository(repo_id)?;
        let state_bin = serialize_blob(
            &checkpoint.state,
            &format!("graph checkpoint {}", checkpoint.commit_hash),
        )?;
        let full_artifacts_bin = checkpoint
            .full_artifacts
            .as_ref()
            .map(|value| {
                serialize_blob(value, &format!("full artifacts {}", checkpoint.commit_hash))
            })
            .transpose()?;

        self.conn
            .execute(
                "INSERT OR REPLACE INTO graph_checkpoints
                    (repo_id, commit_hash, scan_order, state_bin, full_artifacts_bin)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![
                    repo_id,
                    checkpoint.commit_hash,
                    checkpoint.scan_order,
                    state_bin,
                    full_artifacts_bin,
                ],
            )
            .with_context(|| {
                format!(
                    "Failed to insert graph checkpoint {}",
                    checkpoint.commit_hash
                )
            })?;
        Ok(())
    }

    pub fn graph_snapshot_count(&self, repo_id: &str) -> Result<usize> {
        let count: i64 = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM snapshot_frames WHERE repo_id = ?1",
                [repo_id],
                |row| row.get(0),
            )
            .context("Failed to query snapshot frame count")?;
        Ok(count as usize)
    }

    #[allow(clippy::type_complexity)]
    pub fn list_recent_graphs(
        &self,
        repo_id: &str,
        limit: usize,
    ) -> Result<Vec<(String, String, i64, usize, usize)>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT f.commit_hash, c.message, f.timestamp, f.node_count, f.edge_count
                   FROM snapshot_frames f
                   JOIN commits c ON f.repo_id = c.repo_id AND f.commit_hash = c.hash
                  WHERE f.repo_id = ?1
                  ORDER BY f.scan_order DESC
                  LIMIT ?2",
            )
            .context("Failed to prepare graph list query")?;

        let rows = stmt
            .query_map(rusqlite::params![repo_id, limit as i64], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)? as usize,
                    row.get::<_, i64>(4)? as usize,
                ))
            })
            .context("Failed to execute graph list query")?
            .collect::<Result<Vec<_>, _>>()
            .context("Failed to read graph list")?;

        Ok(rows)
    }

    #[allow(clippy::type_complexity)]
    pub fn list_drift_trend(
        &self,
        repo_id: &str,
        limit: usize,
    ) -> Result<Vec<(String, String, usize, usize, Option<u8>, i64)>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT f.commit_hash, c.message, f.timestamp, f.node_count, f.edge_count, f.drift_bin
                   FROM snapshot_frames f
                   JOIN commits c ON f.repo_id = c.repo_id AND f.commit_hash = c.hash
                  WHERE f.repo_id = ?1
                  ORDER BY f.scan_order DESC
                  LIMIT ?2",
            )
            .context("Failed to prepare drift trend query")?;

        let rows = stmt
            .query_map(rusqlite::params![repo_id, limit as i64], |row| {
                let drift_bytes: Option<Vec<u8>> = row.get(5)?;
                let drift_total = drift_bytes
                    .as_deref()
                    .map(|bytes| deserialize_blob::<DriftScore>(bytes, "drift trend entry"))
                    .transpose()
                    .map_err(map_anyhow)?
                    .map(|drift| drift.total);
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(3)? as usize,
                    row.get::<_, i64>(4)? as usize,
                    drift_total,
                    row.get::<_, i64>(2)?,
                ))
            })
            .context("Failed to execute drift trend query")?
            .collect::<Result<Vec<_>, _>>()
            .context("Failed to collect drift trend")?;

        Ok(rows)
    }

    pub fn get_latest_scanned_commit(&self, repo_id: &str) -> Result<Option<(String, i64)>> {
        self.conn
            .query_row(
                "SELECT commit_hash, scan_order
                   FROM snapshot_frames
                  WHERE repo_id = ?1
                  ORDER BY scan_order DESC
                  LIMIT 1",
                [repo_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
            )
            .optional()
            .context("Failed to fetch latest scanned commit")
    }

    pub fn get_scan_order(&self, repo_id: &str, commit_hash: &str) -> Result<Option<i64>> {
        self.conn
            .query_row(
                "SELECT scan_order
                   FROM snapshot_frames
                  WHERE repo_id = ?1 AND commit_hash = ?2",
                rusqlite::params![repo_id, commit_hash],
                |row| row.get::<_, i64>(0),
            )
            .optional()
            .with_context(|| format!("Failed to fetch scan order for commit: {commit_hash}"))
    }

    pub fn get_snapshot_metadata(
        &self,
        repo_id: &str,
        commit_hash: &str,
    ) -> Result<Option<SnapshotMetadata>> {
        self.conn
            .query_row(
                "SELECT commit_hash, scan_order, timestamp, drift_bin
                   FROM snapshot_frames
                  WHERE repo_id = ?1 AND commit_hash = ?2",
                rusqlite::params![repo_id, commit_hash],
                |row| {
                    let drift_bytes: Option<Vec<u8>> = row.get(3)?;
                    let drift = drift_bytes
                        .as_deref()
                        .map(|bytes| {
                            deserialize_blob::<DriftScore>(bytes, "snapshot metadata drift")
                        })
                        .transpose()
                        .map_err(map_anyhow)?;
                    Ok(SnapshotMetadata {
                        commit_hash: row.get(0)?,
                        scan_order: row.get(1)?,
                        timestamp: row.get(2)?,
                        drift,
                    })
                },
            )
            .optional()
            .context("Failed to fetch snapshot metadata")
    }

    pub fn get_graph_snapshot(
        &self,
        repo_id: &str,
        commit_hash: &str,
    ) -> Result<Option<GraphSnapshot>> {
        let Some(frame) = self.fetch_snapshot_frame(repo_id, commit_hash)? else {
            return Ok(None);
        };
        self.reconstruct_snapshot_from_frame(repo_id, &frame)
            .map(Some)
    }

    pub fn get_previous_snapshot(
        &self,
        repo_id: &str,
        scan_order: i64,
    ) -> Result<Option<GraphSnapshot>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT commit_hash, scan_order, timestamp, node_count, edge_count, drift_bin,
                        scan_metadata_bin, delta_bin, analysis_version, config_fingerprint,
                        has_full_artifacts
                   FROM snapshot_frames
                  WHERE repo_id = ?1 AND scan_order < ?2
                  ORDER BY scan_order DESC
                  LIMIT 1",
            )
            .context("Failed to prepare previous snapshot query")?;
        let frame = stmt
            .query_row(rusqlite::params![repo_id, scan_order], |row| {
                self.read_frame(row)
            })
            .optional()
            .context("Failed to load previous snapshot frame")?;
        frame
            .map(|value| self.reconstruct_snapshot_from_frame(repo_id, &value))
            .transpose()
    }

    #[allow(clippy::type_complexity)]
    pub fn list_previous_drift_entries(
        &self,
        repo_id: &str,
        scan_order: i64,
        limit: usize,
    ) -> Result<Vec<(String, String, usize, usize, Option<u8>, i64)>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT f.commit_hash, c.message, f.timestamp, f.node_count, f.edge_count, f.drift_bin
                   FROM snapshot_frames f
                   JOIN commits c ON f.repo_id = c.repo_id AND f.commit_hash = c.hash
                  WHERE f.repo_id = ?1 AND f.scan_order < ?2
                  ORDER BY f.scan_order DESC
                  LIMIT ?3",
            )
            .context("Failed to prepare previous drift query")?;

        let rows = stmt
            .query_map(
                rusqlite::params![repo_id, scan_order, limit as i64],
                |row| {
                    let drift_bytes: Option<Vec<u8>> = row.get(5)?;
                    let drift_total = drift_bytes
                        .as_deref()
                        .map(|bytes| deserialize_blob::<DriftScore>(bytes, "previous drift entry"))
                        .transpose()
                        .map_err(map_anyhow)?
                        .map(|drift| drift.total);
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(3)? as usize,
                        row.get::<_, i64>(4)? as usize,
                        drift_total,
                        row.get::<_, i64>(2)?,
                    ))
                },
            )
            .context("Failed to execute previous drift query")?
            .collect::<Result<Vec<_>, _>>()
            .context("Failed to collect previous drift entries")?;

        Ok(rows)
    }

    pub fn get_recent_snapshots(&self, repo_id: &str, limit: usize) -> Result<SnapshotLoadResult> {
        let metadata = self.get_recent_snapshot_metadata(repo_id, limit)?;
        self.load_snapshots_from_metadata(repo_id, &metadata)
    }

    pub fn get_sampled_snapshots(
        &self,
        repo_id: &str,
        target_count: usize,
    ) -> Result<SnapshotLoadResult> {
        let metadata = self.get_sampled_snapshot_metadata(repo_id, target_count)?;
        self.load_snapshots_from_metadata(repo_id, &metadata)
    }

    pub fn get_recent_snapshot_metadata(
        &self,
        repo_id: &str,
        limit: usize,
    ) -> Result<Vec<SnapshotMetadata>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT commit_hash, scan_order, timestamp, drift_bin
                   FROM snapshot_frames
                  WHERE repo_id = ?1
                  ORDER BY scan_order DESC
                  LIMIT ?2",
            )
            .context("Failed to prepare recent metadata query")?;

        let rows = stmt
            .query_map(rusqlite::params![repo_id, limit as i64], |row| {
                let drift_bytes: Option<Vec<u8>> = row.get(3)?;
                let drift = drift_bytes
                    .as_deref()
                    .map(|bytes| deserialize_blob::<DriftScore>(bytes, "recent snapshot metadata"))
                    .transpose()
                    .map_err(map_anyhow)?;
                Ok(SnapshotMetadata {
                    commit_hash: row.get(0)?,
                    scan_order: row.get(1)?,
                    timestamp: row.get(2)?,
                    drift,
                })
            })
            .context("Failed to execute recent metadata query")?
            .collect::<Result<Vec<_>, _>>()
            .context("Failed to collect recent metadata")?;

        Ok(rows)
    }

    pub fn get_sampled_snapshot_metadata(
        &self,
        repo_id: &str,
        target_count: usize,
    ) -> Result<Vec<SnapshotMetadata>> {
        let total = self.graph_snapshot_count(repo_id)?;
        if total == 0 {
            return Ok(Vec::new());
        }
        if target_count == 0 || target_count >= total {
            return self.get_recent_snapshot_metadata(repo_id, total);
        }
        if target_count == 1 {
            return self.get_recent_snapshot_metadata(repo_id, 1);
        }

        let all = self.get_recent_snapshot_metadata(repo_id, total)?;
        let n = all.len();
        let mut picked_indices: Vec<usize> = (0..target_count)
            .map(|i| i * (n - 1) / (target_count - 1))
            .collect();
        picked_indices.dedup();

        Ok(picked_indices
            .into_iter()
            .filter_map(|index| all.get(index).cloned())
            .collect())
    }

    pub fn get_commit_messages_for_metadata(
        &self,
        repo_id: &str,
        metadata: &[SnapshotMetadata],
    ) -> Result<Vec<(String, String, i64)>> {
        let mut result = Vec::with_capacity(metadata.len());
        let mut stmt = self
            .conn
            .prepare(
                "SELECT message, timestamp
                   FROM commits
                  WHERE repo_id = ?1 AND hash = ?2",
            )
            .context("Failed to prepare commit message query")?;

        for meta in metadata {
            let row = stmt
                .query_row(rusqlite::params![repo_id, meta.commit_hash], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
                })
                .optional()
                .context("Failed to fetch commit message")?;

            if let Some((message, timestamp)) = row {
                result.push((meta.commit_hash.clone(), message, timestamp));
            }
        }

        Ok(result)
    }

    fn load_snapshots_from_metadata(
        &self,
        repo_id: &str,
        metadata: &[SnapshotMetadata],
    ) -> Result<SnapshotLoadResult> {
        let mut result = SnapshotLoadResult {
            snapshots: Vec::with_capacity(metadata.len()),
            skipped_corrupt: 0,
        };

        for meta in metadata {
            match self.get_graph_snapshot(repo_id, &meta.commit_hash) {
                Ok(Some(snapshot)) => result.snapshots.push(snapshot),
                Ok(None) => result.skipped_corrupt += 1,
                Err(err) => {
                    result.skipped_corrupt += 1;
                    warn!(hash = %meta.commit_hash, error = %err, "Failed to reconstruct snapshot");
                }
            }
        }

        Ok(result)
    }

    fn fetch_snapshot_frame(
        &self,
        repo_id: &str,
        commit_hash: &str,
    ) -> Result<Option<SnapshotFrame>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT commit_hash, scan_order, timestamp, node_count, edge_count, drift_bin,
                        scan_metadata_bin, delta_bin, analysis_version, config_fingerprint,
                        has_full_artifacts
                   FROM snapshot_frames
                  WHERE repo_id = ?1 AND commit_hash = ?2",
            )
            .context("Failed to prepare snapshot frame query")?;

        stmt.query_row(rusqlite::params![repo_id, commit_hash], |row| {
            self.read_frame(row)
        })
        .optional()
        .context("Failed to execute snapshot frame query")
    }

    fn read_frame(&self, row: &rusqlite::Row<'_>) -> rusqlite::Result<SnapshotFrame> {
        let drift_bytes: Option<Vec<u8>> = row.get(5)?;
        let drift = drift_bytes
            .as_deref()
            .map(|bytes| deserialize_blob::<DriftScore>(bytes, "snapshot frame drift"))
            .transpose()
            .map_err(map_anyhow)?;
        let scan_metadata_bytes: Vec<u8> = row.get(6)?;
        let scan_metadata =
            deserialize_blob::<ScanMetadata>(&scan_metadata_bytes, "snapshot frame metadata")
                .map_err(map_anyhow)?;
        let delta_bytes: Vec<u8> = row.get(7)?;
        let delta = deserialize_blob::<GraphDelta>(&delta_bytes, "snapshot frame delta")
            .map_err(map_anyhow)?;

        Ok(SnapshotFrame {
            commit_hash: row.get(0)?,
            scan_order: row.get(1)?,
            timestamp: row.get(2)?,
            node_count: row.get::<_, i64>(3)? as usize,
            edge_count: row.get::<_, i64>(4)? as usize,
            drift,
            scan_metadata,
            delta,
            analysis_version: row.get::<_, i64>(8)? as u32,
            config_fingerprint: row.get(9)?,
            has_full_artifacts: row.get::<_, i64>(10)? != 0,
        })
    }

    fn load_checkpoint_before_or_at(
        &self,
        repo_id: &str,
        scan_order: i64,
    ) -> Result<Option<GraphCheckpoint>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT commit_hash, scan_order, state_bin, full_artifacts_bin
                   FROM graph_checkpoints
                  WHERE repo_id = ?1 AND scan_order <= ?2
                  ORDER BY scan_order DESC
                  LIMIT 1",
            )
            .context("Failed to prepare checkpoint lookup")?;

        let row = stmt
            .query_row(rusqlite::params![repo_id, scan_order], |row| {
                let state_bin: Vec<u8> = row.get(2)?;
                let state = deserialize_blob::<RepoScanState>(&state_bin, "checkpoint state")
                    .map_err(map_anyhow)?;
                let artifacts_bin: Option<Vec<u8>> = row.get(3)?;
                let full_artifacts = artifacts_bin
                    .as_deref()
                    .map(|bytes| {
                        deserialize_blob::<HeavySnapshotArtifacts>(bytes, "checkpoint artifacts")
                    })
                    .transpose()
                    .map_err(map_anyhow)?;
                Ok(GraphCheckpoint {
                    commit_hash: row.get(0)?,
                    scan_order: row.get(1)?,
                    state,
                    full_artifacts,
                })
            })
            .optional()
            .context("Failed to load checkpoint")?;

        Ok(row)
    }

    fn load_frames_between(
        &self,
        repo_id: &str,
        start_scan_order: i64,
        end_scan_order: i64,
    ) -> Result<Vec<SnapshotFrame>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT commit_hash, scan_order, timestamp, node_count, edge_count, drift_bin,
                        scan_metadata_bin, delta_bin, analysis_version, config_fingerprint,
                        has_full_artifacts
                   FROM snapshot_frames
                  WHERE repo_id = ?1 AND scan_order > ?2 AND scan_order <= ?3
                  ORDER BY scan_order ASC",
            )
            .context("Failed to prepare frame range query")?;

        let rows = stmt
            .query_map(
                rusqlite::params![repo_id, start_scan_order, end_scan_order],
                |row| self.read_frame(row),
            )
            .context("Failed to execute frame range query")?
            .collect::<Result<Vec<_>, _>>()
            .context("Failed to collect frame range")?;

        Ok(rows)
    }

    fn load_checkpoint_artifacts(
        &self,
        repo_id: &str,
        scan_order: i64,
    ) -> Result<Option<HeavySnapshotArtifacts>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT full_artifacts_bin
                   FROM graph_checkpoints
                  WHERE repo_id = ?1 AND scan_order = ?2",
            )
            .context("Failed to prepare checkpoint artifact query")?;
        let bytes = stmt
            .query_row(rusqlite::params![repo_id, scan_order], |row| {
                row.get::<_, Option<Vec<u8>>>(0)
            })
            .optional()
            .context("Failed to load checkpoint artifact row")?
            .flatten();

        bytes
            .map(|blob| deserialize_blob::<HeavySnapshotArtifacts>(&blob, "checkpoint artifacts"))
            .transpose()
    }

    fn reconstruct_snapshot_from_frame(
        &self,
        repo_id: &str,
        frame: &SnapshotFrame,
    ) -> Result<GraphSnapshot> {
        let Some(mut checkpoint) = self.load_checkpoint_before_or_at(repo_id, frame.scan_order)?
        else {
            anyhow::bail!("No checkpoint available for {}", frame.commit_hash);
        };
        let deltas = self.load_frames_between(repo_id, checkpoint.scan_order, frame.scan_order)?;
        for delta_frame in &deltas {
            apply_delta_to_repo_state(&mut checkpoint.state, &delta_frame.delta);
        }

        let full_artifacts = if frame.has_full_artifacts {
            self.load_checkpoint_artifacts(repo_id, frame.scan_order)?
                .or(checkpoint.full_artifacts.take())
        } else {
            None
        };

        Ok(materialize_snapshot_from_repo_state(
            &checkpoint.state,
            frame,
            full_artifacts,
        ))
    }
}

fn apply_delta_to_repo_state(state: &mut RepoScanState, delta: &GraphDelta) {
    for path in &delta.deletes {
        state.files.remove(path);
    }
    for (path, file_state) in &delta.upserts {
        state.files.insert(path.clone(), file_state.clone());
    }
}

fn materialize_snapshot_from_repo_state(
    repo_state: &RepoScanState,
    frame: &SnapshotFrame,
    full_artifacts: Option<HeavySnapshotArtifacts>,
) -> GraphSnapshot {
    let projection = project_repo_state(
        repo_state,
        frame.scan_metadata.external_min_importers as usize,
    );
    GraphSnapshot {
        commit_hash: frame.commit_hash.clone(),
        nodes: projection.nodes,
        edges: projection.edges,
        node_count: frame.node_count,
        edge_count: frame.edge_count,
        timestamp: frame.timestamp,
        analysis_version: frame.analysis_version,
        config_fingerprint: frame.config_fingerprint.clone(),
        node_metadata: projection.node_metadata,
        scan_metadata: frame.scan_metadata.clone(),
        drift: frame.drift.clone(),
        blast_radius: full_artifacts
            .as_ref()
            .map(|value| value.blast_radius.clone()),
        instability_metrics: full_artifacts
            .as_ref()
            .map(|value| value.instability_metrics.clone())
            .unwrap_or_default(),
        diagnostics: full_artifacts
            .as_ref()
            .map(|value| value.diagnostics.clone())
            .unwrap_or_default(),
    }
}

struct StateProjection {
    nodes: Vec<String>,
    edges: Vec<DependencyEdge>,
    node_metadata: HashMap<String, NodeMetadata>,
}

fn project_repo_state(
    repo_state: &RepoScanState,
    external_min_importers: usize,
) -> StateProjection {
    let mut internal_file_counts: HashMap<String, usize> = HashMap::new();
    let mut edge_weights: HashMap<(String, String), u32> = HashMap::new();
    let mut edge_samples: HashMap<(String, String), HashSet<String>> = HashMap::new();
    let mut target_importers: HashMap<String, HashMap<String, usize>> = HashMap::new();

    for (path, file_state) in &repo_state.files {
        *internal_file_counts
            .entry(file_state.package_name.clone())
            .or_insert(0) += 1;

        let mut seen_targets = HashSet::new();
        for import in &file_state.imports {
            *edge_weights
                .entry((file_state.package_name.clone(), import.module_name.clone()))
                .or_insert(0) += import.weight;
            edge_samples
                .entry((file_state.package_name.clone(), import.module_name.clone()))
                .or_default()
                .insert(path.clone());

            if seen_targets.insert(import.module_name.clone()) {
                *target_importers
                    .entry(import.module_name.clone())
                    .or_default()
                    .entry(file_state.package_name.clone())
                    .or_insert(0) += 1;
            }
        }
    }

    let internal_nodes: HashSet<String> = internal_file_counts.keys().cloned().collect();
    let kept_external: HashSet<String> = target_importers
        .iter()
        .filter(|(module_name, importers)| {
            !internal_nodes.contains(*module_name) && importers.len() >= external_min_importers
        })
        .map(|(module_name, _)| module_name.clone())
        .collect();
    let kept_nodes: HashSet<String> = internal_nodes
        .iter()
        .cloned()
        .chain(kept_external.iter().cloned())
        .collect();

    let mut edges = Vec::new();
    for ((from_module, to_module), weight) in &edge_weights {
        if !kept_nodes.contains(from_module) || !kept_nodes.contains(to_module) || *weight == 0 {
            continue;
        }

        let mut sample_paths: Vec<String> = edge_samples
            .get(&(from_module.clone(), to_module.clone()))
            .map(|paths| paths.iter().cloned().collect())
            .unwrap_or_default();
        sample_paths.sort();
        sample_paths.truncate(5);
        let file_path = sample_paths.first().cloned().unwrap_or_default();
        let sample_origins = sample_paths
            .into_iter()
            .map(|file_path| EdgeOrigin {
                file_path,
                line: None,
            })
            .collect();

        edges.push(DependencyEdge {
            from_module: from_module.clone(),
            to_module: to_module.clone(),
            file_path,
            line: None,
            weight: *weight,
            sample_origins,
        });
    }
    edges.sort_by(|a, b| {
        a.from_module
            .cmp(&b.from_module)
            .then(a.to_module.cmp(&b.to_module))
    });

    let mut nodes: Vec<String> = kept_nodes.iter().cloned().collect();
    nodes.sort();

    let mut node_metadata = HashMap::with_capacity(kept_nodes.len());
    for node in &kept_nodes {
        let metadata = if internal_nodes.contains(node) {
            NodeMetadata {
                kind: NodeKind::Internal,
                importer_count: None,
            }
        } else {
            NodeMetadata {
                kind: NodeKind::External,
                importer_count: target_importers
                    .get(node)
                    .map(|importers| importers.len() as u32),
            }
        };
        node_metadata.insert(node.clone(), metadata);
    }

    StateProjection {
        nodes,
        edges,
        node_metadata,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{
        BlastRadiusReport, BlastRadiusSummary, CascadePath, FileDependencyState,
        FilteredExternalSample, InstabilityMetric, ModuleImpact,
    };

    const TEST_REPO_ID: &str = "repo/test";

    fn test_config_fingerprint() -> String {
        "{\"ignore\":{\"paths\":[]},\"scoring\":{}}".to_string()
    }

    fn make_commit(hash: &str, timestamp: i64) -> CommitInfo {
        CommitInfo {
            hash: hash.to_string(),
            author_name: "Test".to_string(),
            author_email: "test@test.com".to_string(),
            message: format!("Commit {hash}"),
            timestamp,
            tree_id: format!("tree_{hash}"),
        }
    }

    fn make_frame(
        hash: &str,
        scan_order: i64,
        delta: GraphDelta,
        has_full_artifacts: bool,
    ) -> SnapshotFrame {
        SnapshotFrame {
            commit_hash: hash.to_string(),
            scan_order,
            timestamp: scan_order,
            node_count: 1,
            edge_count: 0,
            analysis_version: crate::models::CURRENT_ANALYSIS_VERSION,
            config_fingerprint: test_config_fingerprint(),
            drift: Some(DriftScore {
                total: scan_order as u8,
                fan_in_delta: 0,
                fan_out_delta: 0,
                new_cycles: 0,
                boundary_violations: 0,
                layering_violations: 0,
                cognitive_complexity: 0.0,
                timestamp: scan_order,
                cycle_debt: 0.0,
                layering_debt: 0.0,
                hub_debt: 0.0,
                coupling_debt: 0.0,
                cognitive_debt: 0.0,
                instability_debt: 0.0,
            }),
            scan_metadata: ScanMetadata {
                external_min_importers: 3,
                included_external_count: 0,
                filtered_external_count: 0,
                filtered_external_samples: Vec::<FilteredExternalSample>::new(),
            },
            delta,
            has_full_artifacts,
        }
    }

    fn make_full_artifacts() -> HeavySnapshotArtifacts {
        HeavySnapshotArtifacts {
            blast_radius: BlastRadiusReport {
                impacts: vec![ModuleImpact {
                    module_name: "core".to_string(),
                    blast_score: 0.8,
                    downstream_count: 2,
                    weighted_reach: 1.0,
                    is_articulation_point: false,
                }],
                articulation_points: Vec::new(),
                critical_paths: vec![CascadePath {
                    chain: vec!["core".to_string(), "web".to_string()],
                    total_weight: 1,
                    depth: 2,
                }],
                summary: BlastRadiusSummary {
                    articulation_point_count: 0,
                    max_blast_score: 0.8,
                    most_impactful_module: "core".to_string(),
                    mean_blast_score: 0.8,
                    longest_chain_depth: 2,
                },
            },
            instability_metrics: vec![InstabilityMetric {
                module_name: "core".to_string(),
                instability: 0.2,
                fan_in: 1,
                fan_out: 1,
            }],
            diagnostics: vec!["stable".to_string()],
        }
    }

    #[test]
    fn reconstructs_snapshot_from_checkpoint_and_delta() {
        let db = Database::open_in_memory().unwrap();
        let commit_a = make_commit("a", 1);
        let commit_b = make_commit("b", 2);
        db.insert_commit(TEST_REPO_ID, &commit_a).unwrap();
        db.insert_commit(TEST_REPO_ID, &commit_b).unwrap();

        let baseline_state = RepoScanState {
            files: HashMap::from([(
                "src/core.ts".to_string(),
                FileDependencyState {
                    package_name: "core".to_string(),
                    imports: Vec::new(),
                },
            )]),
        };
        db.insert_graph_checkpoint(
            TEST_REPO_ID,
            &GraphCheckpoint {
                commit_hash: "a".to_string(),
                scan_order: 1,
                state: baseline_state,
                full_artifacts: None,
            },
        )
        .unwrap();
        db.insert_snapshot_frame(
            TEST_REPO_ID,
            &make_frame("a", 1, GraphDelta::default(), false),
        )
        .unwrap();
        db.insert_snapshot_frame(
            TEST_REPO_ID,
            &make_frame(
                "b",
                2,
                GraphDelta {
                    upserts: vec![(
                        "src/web.ts".to_string(),
                        FileDependencyState {
                            package_name: "web".to_string(),
                            imports: vec![crate::models::FileImportTarget {
                                module_name: "core".to_string(),
                                weight: 1,
                            }],
                        },
                    )],
                    deletes: Vec::new(),
                },
                false,
            ),
        )
        .unwrap();

        let snapshot = db.get_graph_snapshot(TEST_REPO_ID, "b").unwrap().unwrap();
        assert_eq!(snapshot.nodes, vec!["core".to_string(), "web".to_string()]);
        assert_eq!(snapshot.edges.len(), 1);
        assert_eq!(snapshot.edges[0].from_module, "web");
        assert_eq!(snapshot.edges[0].to_module, "core");
    }

    #[test]
    fn repo_scoped_frames_do_not_collide() {
        let db = Database::open_in_memory().unwrap();
        let repo_b = "repo/other";
        let commit = make_commit("samehash", 1);
        db.insert_commit(TEST_REPO_ID, &commit).unwrap();
        db.insert_commit(repo_b, &commit).unwrap();

        let state_a = RepoScanState {
            files: HashMap::from([(
                "src/core.ts".to_string(),
                FileDependencyState {
                    package_name: "core".to_string(),
                    imports: Vec::new(),
                },
            )]),
        };
        let state_b = RepoScanState {
            files: HashMap::from([(
                "src/web.ts".to_string(),
                FileDependencyState {
                    package_name: "web".to_string(),
                    imports: Vec::new(),
                },
            )]),
        };
        db.insert_graph_checkpoint(
            TEST_REPO_ID,
            &GraphCheckpoint {
                commit_hash: "samehash".to_string(),
                scan_order: 1,
                state: state_a,
                full_artifacts: Some(make_full_artifacts()),
            },
        )
        .unwrap();
        db.insert_graph_checkpoint(
            repo_b,
            &GraphCheckpoint {
                commit_hash: "samehash".to_string(),
                scan_order: 1,
                state: state_b,
                full_artifacts: Some(make_full_artifacts()),
            },
        )
        .unwrap();
        db.insert_snapshot_frame(
            TEST_REPO_ID,
            &make_frame("samehash", 1, GraphDelta::default(), true),
        )
        .unwrap();
        db.insert_snapshot_frame(
            repo_b,
            &make_frame("samehash", 1, GraphDelta::default(), true),
        )
        .unwrap();

        let repo_a = db
            .get_graph_snapshot(TEST_REPO_ID, "samehash")
            .unwrap()
            .unwrap();
        let repo_b_loaded = db.get_graph_snapshot(repo_b, "samehash").unwrap().unwrap();
        assert_eq!(repo_a.nodes, vec!["core".to_string()]);
        assert_eq!(repo_b_loaded.nodes, vec!["web".to_string()]);
        assert_eq!(
            repo_a
                .blast_radius
                .as_ref()
                .unwrap()
                .summary
                .max_blast_score,
            0.8
        );
    }

    #[test]
    fn previous_drift_entries_return_older_commits_first() {
        let db = Database::open_in_memory().unwrap();
        for idx in 1..=4 {
            let hash = format!("c{idx}");
            db.insert_commit(TEST_REPO_ID, &make_commit(&hash, idx as i64))
                .unwrap();
            db.insert_snapshot_frame(
                TEST_REPO_ID,
                &make_frame(&hash, idx as i64, GraphDelta::default(), false),
            )
            .unwrap();
        }

        let previous = db.list_previous_drift_entries(TEST_REPO_ID, 4, 3).unwrap();
        let hashes: Vec<String> = previous.into_iter().map(|entry| entry.0).collect();
        assert_eq!(
            hashes,
            vec!["c3".to_string(), "c2".to_string(), "c1".to_string()]
        );
    }
}
