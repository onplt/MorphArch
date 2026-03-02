// =============================================================================
// db.rs — MorphArch SQLite database layer
// =============================================================================
//
// Responsibilities:
//   1. Open database connection (file or in-memory)
//   2. Create table schema (migration)
//   3. Commit CRUD: insert, list, count
//   4. Graph snapshot CRUD: insert, list_recent, count
//   5. Drift CRUD: insert_with_drift, list_drift_trend (Sprint 3)
//   6. Graph snapshot lookup: get_graph_snapshot (Sprint 3)
//   7. Bulk snapshot loading: get_recent_snapshots (Sprint 4 TUI)
//   8. Commit message fetch: get_commit_messages_for_snapshots (Sprint 4)
//
// Tables:
//   commits          → Commit metadata (Sprint 1)
//   graph_snapshots  → Dependency graph JSON + drift JSON (Sprint 2-3)
//
// Compiled with SQLite "bundled" feature — no system SQLite dependency.
// WAL mode is enabled for concurrent read performance.
// =============================================================================

use anyhow::{Context, Result};
use rusqlite::Connection;
use std::path::Path;
use tracing::{debug, info};

use crate::models::{CommitInfo, DriftScore, GraphSnapshot};

/// MorphArch database wrapper.
///
/// Holds the SQLite connection and provides all database operations.
/// Migration runs automatically on creation.
pub struct Database {
    /// Active SQLite connection
    conn: Connection,
}

impl Database {
    /// Opens (or creates) the database at the specified file path.
    ///
    /// After opening the connection:
    ///   - WAL mode is enabled (for concurrent reads)
    ///   - Required tables are created (CREATE IF NOT EXISTS)
    ///   - Sprint 3 migration (drift_json column) is applied
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

    /// Creates an in-memory database (for tests).
    #[cfg(test)]
    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory().context("Failed to create in-memory database")?;
        let db = Self { conn };
        db.migrate()?;
        Ok(db)
    }

    /// Creates or updates the database schema (idempotent).
    ///
    /// Sprint 3 addition: drift_json column added to graph_snapshots table.
    /// ALTER TABLE IF NOT EXISTS is not supported, so column existence is checked.
    fn migrate(&self) -> Result<()> {
        self.conn
            .execute_batch(
                "
            -- Sprint 1: Commit metadata table
            CREATE TABLE IF NOT EXISTS commits (
                hash         TEXT PRIMARY KEY,
                author_name  TEXT NOT NULL,
                author_email TEXT NOT NULL,
                message      TEXT NOT NULL,
                timestamp    INTEGER NOT NULL,
                tree_id      TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_commits_timestamp
                ON commits(timestamp);
            CREATE INDEX IF NOT EXISTS idx_commits_author
                ON commits(author_email);

            -- Sprint 2: Dependency graph snapshot table
            -- snapshot_json: Full JSON serialization of GraphSnapshot struct
            -- node_count/edge_count: Denormalized fields for fast queries
            CREATE TABLE IF NOT EXISTS graph_snapshots (
                commit_hash   TEXT PRIMARY KEY,
                snapshot_json TEXT NOT NULL,
                node_count    INTEGER NOT NULL DEFAULT 0,
                edge_count    INTEGER NOT NULL DEFAULT 0
            );
            CREATE INDEX IF NOT EXISTS idx_graph_snapshots_counts
                ON graph_snapshots(node_count, edge_count);
            ",
            )
            .context("Database migration failed")?;

        // Sprint 3: Add drift_json column (idempotent)
        // SQLite doesn't have "ADD COLUMN IF NOT EXISTS" — check with PRAGMA table_info
        self.migrate_drift_column()?;

        info!("Database migration complete");
        Ok(())
    }

    /// Sprint 3 migration: adds drift_json column to graph_snapshots table.
    ///
    /// Silently skips if column already exists (idempotent).
    fn migrate_drift_column(&self) -> Result<()> {
        // Check existing columns with PRAGMA table_info
        let has_drift = {
            let mut stmt = self
                .conn
                .prepare("PRAGMA table_info(graph_snapshots)")
                .context("Failed to prepare PRAGMA table_info query")?;

            let columns: Vec<String> = stmt
                .query_map([], |row| row.get::<_, String>(1))
                .context("Failed to execute PRAGMA query")?
                .filter_map(|r| r.ok())
                .collect();

            columns.contains(&"drift_json".to_string())
        };

        if !has_drift {
            self.conn
                .execute_batch(
                    "ALTER TABLE graph_snapshots ADD COLUMN drift_json TEXT DEFAULT NULL;",
                )
                .context("Failed to add drift_json column")?;
            debug!("Sprint 3 migration: drift_json column added");
        }

        Ok(())
    }

    /// Begins an explicit transaction. Call `commit_transaction()` when done.
    /// All inserts between begin/commit are batched into a single fsync.
    pub fn begin_transaction(&self) -> Result<()> {
        self.conn
            .execute_batch("BEGIN TRANSACTION")
            .context("Failed to begin transaction")
    }

    /// Commits the current transaction (flushes all batched writes).
    pub fn commit_transaction(&self) -> Result<()> {
        self.conn
            .execute_batch("COMMIT")
            .context("Failed to commit transaction")
    }

    /// Clears all graph data before a scan operation.
    pub fn clear_all_graph_snapshots(&self) -> Result<()> {
        self.conn
            .execute_batch(
                "DELETE FROM commits;
                 DELETE FROM graph_snapshots;",
            )
            .context("Failed to clear all snapshots")?;
        info!("All snapshot data cleared from DB");
        Ok(())
    }

    // =========================================================================
    // Commit operations (Sprint 1)
    // =========================================================================

    /// Inserts a single commit into the database (INSERT OR REPLACE — idempotent).
    pub fn insert_commit(&self, commit: &CommitInfo) -> Result<()> {
        self.conn
            .execute(
                "INSERT OR REPLACE INTO commits
                    (hash, author_name, author_email, message, timestamp, tree_id)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                rusqlite::params![
                    commit.hash,
                    commit.author_name,
                    commit.author_email,
                    commit.message,
                    commit.timestamp,
                    commit.tree_id,
                ],
            )
            .with_context(|| format!("Failed to write commit to database: {}", &commit.hash))?;

        debug!(hash = %commit.hash, "Commit saved");
        Ok(())
    }

    /// Lists all commits in descending timestamp order.
    #[allow(dead_code)]
    pub fn list_commits(&self) -> Result<Vec<CommitInfo>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT hash, author_name, author_email, message, timestamp, tree_id
                 FROM commits
                 ORDER BY timestamp DESC",
            )
            .context("Failed to prepare commit list query")?;

        let commits = stmt
            .query_map([], |row| {
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

    /// Returns the total number of commits in the database.
    pub fn commit_count(&self) -> Result<usize> {
        let count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM commits", [], |row| row.get(0))
            .context("Failed to query commit count")?;
        Ok(count as usize)
    }

    // =========================================================================
    // Graph snapshot operations (Sprint 2)
    // =========================================================================

    /// Saves a graph snapshot as JSON to the database.
    ///
    /// The GraphSnapshot struct is serialized with serde_json and written to the
    /// snapshot_json column. node_count and edge_count are also stored as
    /// denormalized columns for fast queries.
    pub fn insert_graph_snapshot(&self, snapshot: &GraphSnapshot) -> Result<()> {
        let json = serde_json::to_string(snapshot).with_context(|| {
            format!(
                "Failed to serialize GraphSnapshot to JSON: {}",
                &snapshot.commit_hash
            )
        })?;

        let drift_json = snapshot
            .drift
            .as_ref()
            .map(|d| serde_json::to_string(d).unwrap_or_default());

        self.conn
            .execute(
                "INSERT OR REPLACE INTO graph_snapshots
                    (commit_hash, snapshot_json, node_count, edge_count, drift_json)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![
                    snapshot.commit_hash,
                    json,
                    snapshot.node_count as i64,
                    snapshot.edge_count as i64,
                    drift_json,
                ],
            )
            .with_context(|| {
                format!(
                    "Failed to write graph snapshot to database: {}",
                    &snapshot.commit_hash
                )
            })?;

        debug!(
            hash = %snapshot.commit_hash,
            nodes = snapshot.node_count,
            edges = snapshot.edge_count,
            drift = ?snapshot.drift.as_ref().map(|d| d.total),
            "Graph snapshot saved"
        );
        Ok(())
    }

    /// Lists the last N graph snapshots with commit information.
    ///
    /// JOINs with the commits table to include commit message and timestamp.
    /// Results are in descending timestamp order.
    ///
    /// # Returns
    /// `Vec<(commit_hash, message_first_line, timestamp, node_count, edge_count)>`
    #[allow(clippy::type_complexity)]
    pub fn list_recent_graphs(
        &self,
        limit: usize,
    ) -> Result<Vec<(String, String, i64, usize, usize)>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT g.commit_hash, c.message, c.timestamp, g.node_count, g.edge_count
                 FROM graph_snapshots g
                 JOIN commits c ON g.commit_hash = c.hash
                 ORDER BY c.timestamp DESC
                 LIMIT ?1",
            )
            .context("Failed to prepare graph list query")?;

        let rows = stmt
            .query_map([limit as i64], |row| {
                let hash: String = row.get(0)?;
                let message: String = row.get(1)?;
                let timestamp: i64 = row.get(2)?;
                let nodes: i64 = row.get(3)?;
                let edges: i64 = row.get(4)?;
                Ok((hash, message, timestamp, nodes as usize, edges as usize))
            })
            .context("Failed to execute graph list query")?
            .collect::<Result<Vec<_>, _>>()
            .context("Failed to read graph data")?;

        Ok(rows)
    }

    /// Returns the total number of graph snapshots in the database.
    pub fn graph_snapshot_count(&self) -> Result<usize> {
        let count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM graph_snapshots", [], |row| row.get(0))
            .context("Failed to query graph snapshot count")?;
        Ok(count as usize)
    }

    // =========================================================================
    // Sprint 3: Drift-aware graph snapshot operations
    // =========================================================================

    /// Retrieves a graph snapshot for a specific commit hash.
    ///
    /// Deserializes from JSON and returns the full `GraphSnapshot`.
    /// If drift info exists, the `drift` field will be populated.
    ///
    /// # Returns
    /// - `Ok(Some(snapshot))`: Snapshot found
    /// - `Ok(None)`: No snapshot for this commit
    pub fn get_graph_snapshot(&self, commit_hash: &str) -> Result<Option<GraphSnapshot>> {
        let mut stmt = self
            .conn
            .prepare("SELECT snapshot_json FROM graph_snapshots WHERE commit_hash = ?1")
            .context("Failed to prepare graph snapshot query")?;

        let result = stmt
            .query_row([commit_hash], |row| {
                let json: String = row.get(0)?;
                Ok(json)
            })
            .optional()
            .context("Failed to execute graph snapshot query")?;

        match result {
            Some(json) => {
                let snapshot: GraphSnapshot = serde_json::from_str(&json).with_context(|| {
                    format!("Failed to parse graph snapshot JSON: {commit_hash}")
                })?;
                Ok(Some(snapshot))
            }
            None => Ok(None),
        }
    }

    /// Lists drift trend data for the last N commits.
    ///
    /// Returns snapshots with drift scores in timestamp order.
    /// Each row: commit_hash, message, node count, edge count,
    /// drift score, delta from previous commit.
    ///
    /// # Returns
    /// `Vec<(commit_hash, message, nodes, edges, drift_total, timestamp)>`
    #[allow(clippy::type_complexity)]
    pub fn list_drift_trend(
        &self,
        limit: usize,
    ) -> Result<Vec<(String, String, usize, usize, Option<u8>, i64)>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT g.commit_hash, c.message, g.node_count, g.edge_count,
                        g.drift_json, c.timestamp
                 FROM graph_snapshots g
                 JOIN commits c ON g.commit_hash = c.hash
                 ORDER BY c.timestamp DESC
                 LIMIT ?1",
            )
            .context("Failed to prepare drift trend query")?;

        let rows = stmt
            .query_map([limit as i64], |row| {
                let hash: String = row.get(0)?;
                let message: String = row.get(1)?;
                let nodes: i64 = row.get(2)?;
                let edges: i64 = row.get(3)?;
                let drift_json: Option<String> = row.get(4)?;
                let timestamp: i64 = row.get(5)?;

                // Parse drift_json if present, otherwise None
                let drift_total: Option<u8> = drift_json.and_then(|json| {
                    serde_json::from_str::<DriftScore>(&json)
                        .ok()
                        .map(|d| d.total)
                });

                Ok((
                    hash,
                    message,
                    nodes as usize,
                    edges as usize,
                    drift_total,
                    timestamp,
                ))
            })
            .context("Failed to execute drift trend query")?
            .collect::<Result<Vec<_>, _>>()
            .context("Failed to read drift data")?;

        Ok(rows)
    }

    /// Updates the drift score for a specific commit.
    ///
    /// Updates the drift_json column of an existing graph snapshot.
    /// Only drift info is added if the snapshot already exists.
    #[allow(dead_code)]
    pub fn update_drift_score(&self, commit_hash: &str, drift: &DriftScore) -> Result<()> {
        let drift_json = serde_json::to_string(drift)
            .with_context(|| format!("Failed to serialize DriftScore to JSON: {commit_hash}"))?;

        self.conn
            .execute(
                "UPDATE graph_snapshots SET drift_json = ?1 WHERE commit_hash = ?2",
                rusqlite::params![drift_json, commit_hash],
            )
            .with_context(|| format!("Failed to update drift score: {commit_hash}"))?;

        debug!(
            hash = %commit_hash,
            total = drift.total,
            "Drift score updated"
        );
        Ok(())
    }

    // =========================================================================
    // Sprint 4: Bulk snapshot loading for TUI
    // =========================================================================

    /// Loads the last N graph snapshots with full JSON deserialization.
    ///
    /// Returns snapshots in newest → oldest order for use in the TUI timeline.
    /// Each snapshot contains the full `GraphSnapshot` struct (nodes, edges, drift).
    pub fn get_recent_snapshots(&self, limit: usize) -> Result<Vec<GraphSnapshot>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT g.snapshot_json
                 FROM graph_snapshots g
                 JOIN commits c ON g.commit_hash = c.hash
                 ORDER BY c.timestamp DESC
                 LIMIT ?1",
            )
            .context("Failed to prepare recent snapshots query")?;

        let snapshots = stmt
            .query_map([limit as i64], |row| {
                let json: String = row.get(0)?;
                Ok(json)
            })
            .context("Failed to execute recent snapshots query")?
            .filter_map(|r| r.ok())
            .filter_map(|json| serde_json::from_str::<GraphSnapshot>(&json).ok())
            .collect();

        Ok(snapshots)
    }

    /// Retrieves commit message info for the given snapshot list.
    ///
    /// Returns (hash, first_line_of_message, timestamp) tuples for display
    /// in the timeline widget. Preserves snapshot order.
    #[allow(clippy::type_complexity)]
    pub fn get_commit_messages_for_snapshots(
        &self,
        snapshots: &[GraphSnapshot],
    ) -> Result<Vec<(String, String, i64)>> {
        let mut result = Vec::with_capacity(snapshots.len());

        for snapshot in snapshots {
            let commit_info = self
                .conn
                .query_row(
                    "SELECT message, timestamp FROM commits WHERE hash = ?1",
                    [&snapshot.commit_hash],
                    |row| {
                        let message: String = row.get(0)?;
                        let timestamp: i64 = row.get(1)?;
                        Ok((message, timestamp))
                    },
                )
                .optional()
                .with_context(|| format!("Failed to fetch commit info: {}", &snapshot.commit_hash))?;

            match commit_info {
                Some((message, timestamp)) => {
                    let first_line = message.lines().next().unwrap_or("").to_string();
                    result.push((snapshot.commit_hash.clone(), first_line, timestamp));
                }
                None => {
                    // If not in commits table, use snapshot info
                    result.push((
                        snapshot.commit_hash.clone(),
                        String::new(),
                        snapshot.timestamp,
                    ));
                }
            }
        }

        Ok(result)
    }
}

/// rusqlite optional trait helper — returns None when query_row finds no rows.
///
/// rusqlite's `query_row` returns an error if no row is found.
/// This trait adds an `optional()` method that returns None instead.
trait OptionalRow<T> {
    fn optional(self) -> Result<Option<T>, rusqlite::Error>;
}

impl<T> OptionalRow<T> for Result<T, rusqlite::Error> {
    fn optional(self) -> Result<Option<T>, rusqlite::Error> {
        match self {
            Ok(val) => Ok(Some(val)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e),
        }
    }
}

// =============================================================================
// Tests
// =============================================================================
#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::DependencyEdge;

    #[test]
    fn test_insert_and_list_commits() {
        let db = Database::open_in_memory().expect("DB should open");

        let commit1 = CommitInfo {
            hash: "aaa111".to_string(),
            author_name: "Alice".to_string(),
            author_email: "alice@test.com".to_string(),
            message: "First commit".to_string(),
            timestamp: 1_000_000,
            tree_id: "tree_aaa".to_string(),
        };
        let commit2 = CommitInfo {
            hash: "bbb222".to_string(),
            author_name: "Bob".to_string(),
            author_email: "bob@test.com".to_string(),
            message: "Second commit".to_string(),
            timestamp: 2_000_000,
            tree_id: "tree_bbb".to_string(),
        };

        db.insert_commit(&commit1).unwrap();
        db.insert_commit(&commit2).unwrap();

        assert_eq!(db.commit_count().unwrap(), 2);

        let commits = db.list_commits().unwrap();
        assert_eq!(commits.len(), 2);
        assert_eq!(commits[0].hash, "bbb222");
        assert_eq!(commits[1].hash, "aaa111");
    }

    #[test]
    fn test_upsert_commit() {
        let db = Database::open_in_memory().expect("DB should open");

        let original = CommitInfo {
            hash: "abc123".to_string(),
            author_name: "Old Name".to_string(),
            author_email: "old@test.com".to_string(),
            message: "Original message".to_string(),
            timestamp: 1_000_000,
            tree_id: "tree_1".to_string(),
        };
        let updated = CommitInfo {
            hash: "abc123".to_string(),
            author_name: "New Name".to_string(),
            author_email: "new@test.com".to_string(),
            message: "Updated message".to_string(),
            timestamp: 1_000_000,
            tree_id: "tree_1".to_string(),
        };

        db.insert_commit(&original).unwrap();
        db.insert_commit(&updated).unwrap();

        assert_eq!(db.commit_count().unwrap(), 1);
        let commits = db.list_commits().unwrap();
        assert_eq!(commits[0].author_name, "New Name");
    }

    #[test]
    fn test_insert_and_list_graph_snapshots() {
        let db = Database::open_in_memory().expect("DB should open");

        // First insert a commit (required for JOIN)
        let commit = CommitInfo {
            hash: "abc123".to_string(),
            author_name: "Test".to_string(),
            author_email: "test@test.com".to_string(),
            message: "Test commit\nBody here".to_string(),
            timestamp: 1_000_000,
            tree_id: "tree_abc".to_string(),
        };
        db.insert_commit(&commit).unwrap();

        // Insert graph snapshot
        let snapshot = GraphSnapshot {
            commit_hash: "abc123".to_string(),
            nodes: vec!["main".to_string(), "serde".to_string(), "std".to_string()],
            edges: vec![
                DependencyEdge {
                    from_module: "main".to_string(),
                    to_module: "serde".to_string(),
                    file_path: "src/main.rs".to_string(),
                    line: 1,
                    weight: 1,
                },
                DependencyEdge {
                    from_module: "main".to_string(),
                    to_module: "std".to_string(),
                    file_path: "src/main.rs".to_string(),
                    line: 2,
                    weight: 1,
                },
            ],
            node_count: 3,
            edge_count: 2,
            timestamp: 1_000_000,
            drift: None,
        };
        db.insert_graph_snapshot(&snapshot).unwrap();

        // Count check
        assert_eq!(db.graph_snapshot_count().unwrap(), 1);

        // List check
        let graphs = db.list_recent_graphs(10).unwrap();
        assert_eq!(graphs.len(), 1);
        let (hash, message, _ts, nodes, edges) = &graphs[0];
        assert_eq!(hash, "abc123");
        assert!(message.starts_with("Test commit"));
        assert_eq!(*nodes, 3);
        assert_eq!(*edges, 2);
    }

    #[test]
    fn test_drift_score_storage_and_retrieval() {
        let db = Database::open_in_memory().expect("DB should open");

        // Insert commit
        let commit = CommitInfo {
            hash: "drift01".to_string(),
            author_name: "Test".to_string(),
            author_email: "test@test.com".to_string(),
            message: "Drift test commit".to_string(),
            timestamp: 3_000_000,
            tree_id: "tree_drift".to_string(),
        };
        db.insert_commit(&commit).unwrap();

        // Insert snapshot with drift
        let drift = DriftScore {
            total: 65,
            fan_in_delta: 3,
            fan_out_delta: -2,
            new_cycles: 1,
            boundary_violations: 2,
            cognitive_complexity: 12.5,
            timestamp: 3_000_000,
        };

        let snapshot = GraphSnapshot {
            commit_hash: "drift01".to_string(),
            nodes: vec!["A".to_string(), "B".to_string()],
            edges: vec![DependencyEdge {
                from_module: "A".to_string(),
                to_module: "B".to_string(),
                file_path: "src/a.rs".to_string(),
                line: 1,
                weight: 1,
            }],
            node_count: 2,
            edge_count: 1,
            timestamp: 3_000_000,
            drift: Some(drift),
        };
        db.insert_graph_snapshot(&snapshot).unwrap();

        // Read back snapshot
        let retrieved = db
            .get_graph_snapshot("drift01")
            .unwrap()
            .expect("Snapshot should be found");
        assert_eq!(retrieved.commit_hash, "drift01");
        let d = retrieved.drift.expect("Drift should exist");
        assert_eq!(d.total, 65);
        assert_eq!(d.fan_in_delta, 3);
        assert_eq!(d.new_cycles, 1);

        // Drift trend
        let trend = db.list_drift_trend(10).unwrap();
        assert_eq!(trend.len(), 1);
        let (hash, _msg, nodes, edges, drift_total, _ts) = &trend[0];
        assert_eq!(hash, "drift01");
        assert_eq!(*nodes, 2);
        assert_eq!(*edges, 1);
        assert_eq!(*drift_total, Some(65));
    }

    #[test]
    fn test_update_drift_score() {
        let db = Database::open_in_memory().expect("DB should open");

        let commit = CommitInfo {
            hash: "upd01".to_string(),
            author_name: "Test".to_string(),
            author_email: "t@t.com".to_string(),
            message: "Update test".to_string(),
            timestamp: 4_000_000,
            tree_id: "tree_upd".to_string(),
        };
        db.insert_commit(&commit).unwrap();

        // Insert snapshot without drift
        let snapshot = GraphSnapshot {
            commit_hash: "upd01".to_string(),
            nodes: vec!["X".to_string()],
            edges: vec![],
            node_count: 1,
            edge_count: 0,
            timestamp: 4_000_000,
            drift: None,
        };
        db.insert_graph_snapshot(&snapshot).unwrap();

        // Update drift afterwards
        let drift = DriftScore {
            total: 42,
            fan_in_delta: 0,
            fan_out_delta: 1,
            new_cycles: 0,
            boundary_violations: 0,
            cognitive_complexity: 5.0,
            timestamp: 4_000_000,
        };
        db.update_drift_score("upd01", &drift).unwrap();

        // Verify from trend
        let trend = db.list_drift_trend(10).unwrap();
        assert_eq!(trend[0].4, Some(42), "Updated drift score should be 42");
    }

    #[test]
    fn test_get_graph_snapshot_not_found() {
        let db = Database::open_in_memory().expect("DB should open");

        let result = db.get_graph_snapshot("nonexistent").unwrap();
        assert!(result.is_none(), "Should return None for missing commit");
    }
}
