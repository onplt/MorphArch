// =============================================================================
// db.rs — MorphArch SQLite veritabanı katmanı
// =============================================================================
//
// Sorumluluklar:
//   1. Veritabanı bağlantısı açma (dosya veya in-memory)
//   2. Tablo şemasını oluşturma (migration)
//   3. Commit CRUD: insert, list, count
//   4. Graph snapshot CRUD: insert, list_recent, count
//   5. Drift CRUD: insert_with_drift, list_drift_trend (Sprint 3)
//   6. Graph snapshot lookup: get_graph_snapshot (Sprint 3)
//
// Tablolar:
//   commits          → Commit metadata (Sprint 1)
//   graph_snapshots  → Dependency graph JSON + drift JSON (Sprint 2-3)
//
// SQLite "bundled" özelliği ile derlendiğinden sistem SQLite'a bağımlılık yoktur.
// WAL modu etkinleştirilerek eşzamanlı okuma performansı artırılır.
// =============================================================================

use anyhow::{Context, Result};
use rusqlite::Connection;
use std::path::Path;
use tracing::{debug, info};

use crate::models::{CommitInfo, DriftScore, GraphSnapshot};

/// MorphArch veritabanı sarmalayıcısı
///
/// SQLite bağlantısını tutar ve tüm veritabanı işlemlerini sağlar.
/// Oluşturulduğunda otomatik olarak migration çalıştırır.
pub struct Database {
    /// Aktif SQLite bağlantısı
    conn: Connection,
}

impl Database {
    /// Belirtilen dosya yolunda veritabanını açar (veya oluşturur).
    ///
    /// Bağlantı açıldıktan sonra:
    ///   - WAL modu etkinleştirilir (eşzamanlı okuma için)
    ///   - Gerekli tablolar oluşturulur (CREATE IF NOT EXISTS)
    ///   - Sprint 3 migration (drift_json sütunu) uygulanır
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)
            .with_context(|| format!("Veritabanı açılamadı: {}", path.display()))?;

        conn.execute_batch("PRAGMA journal_mode=WAL;")?;

        let db = Self { conn };
        db.migrate()?;
        Ok(db)
    }

    /// In-memory veritabanı oluşturur (testler için).
    #[cfg(test)]
    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory().context("In-memory veritabanı oluşturulamadı")?;
        let db = Self { conn };
        db.migrate()?;
        Ok(db)
    }

    /// Veritabanı şemasını oluşturur veya günceller (idempotent).
    ///
    /// Sprint 3 eklentisi: graph_snapshots tablosuna drift_json sütunu eklenir.
    /// ALTER TABLE IF NOT EXISTS desteklenmiyor, bu yüzden sütun varlığı kontrol edilir.
    fn migrate(&self) -> Result<()> {
        self.conn
            .execute_batch(
                "
            -- Sprint 1: Commit metadata tablosu
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

            -- Sprint 2: Dependency graph snapshot tablosu
            -- snapshot_json: GraphSnapshot struct'ının tam JSON serileştirmesi
            -- node_count/edge_count: Hızlı sorgu için denormalize alanlar
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
            .context("Veritabanı migration'ı başarısız oldu")?;

        // Sprint 3: drift_json sütunu ekle (idempotent)
        // SQLite'da "ADD COLUMN IF NOT EXISTS" yok, PRAGMA table_info ile kontrol et
        self.migrate_drift_column()?;

        info!("Veritabanı migration tamamlandı");
        Ok(())
    }

    /// Sprint 3 migration: graph_snapshots tablosuna drift_json sütunu ekler.
    ///
    /// Sütun zaten varsa sessizce atlar (idempotent).
    fn migrate_drift_column(&self) -> Result<()> {
        // PRAGMA table_info ile mevcut sütunları kontrol et
        let has_drift = {
            let mut stmt = self
                .conn
                .prepare("PRAGMA table_info(graph_snapshots)")
                .context("PRAGMA table_info sorgusu hazırlanamadı")?;

            let columns: Vec<String> = stmt
                .query_map([], |row| row.get::<_, String>(1))
                .context("PRAGMA sorgusu çalıştırılamadı")?
                .filter_map(|r| r.ok())
                .collect();

            columns.contains(&"drift_json".to_string())
        };

        if !has_drift {
            self.conn
                .execute_batch(
                    "ALTER TABLE graph_snapshots ADD COLUMN drift_json TEXT DEFAULT NULL;",
                )
                .context("drift_json sütunu eklenemedi")?;
            debug!("Sprint 3 migration: drift_json sütunu eklendi");
        }

        Ok(())
    }

    // =========================================================================
    // Commit işlemleri (Sprint 1)
    // =========================================================================

    /// Tek bir commit'i veritabanına ekler (INSERT OR REPLACE — idempotent).
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
            .with_context(|| format!("Commit veritabanına yazılamadı: {}", &commit.hash))?;

        debug!(hash = %commit.hash, "Commit kaydedildi");
        Ok(())
    }

    /// Tüm commit'leri zaman damgasına göre azalan sırada listeler.
    pub fn list_commits(&self) -> Result<Vec<CommitInfo>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT hash, author_name, author_email, message, timestamp, tree_id
                 FROM commits
                 ORDER BY timestamp DESC",
            )
            .context("Commit listesi sorgusu hazırlanamadı")?;

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
            .context("Commit listesi sorgusu çalıştırılamadı")?
            .collect::<Result<Vec<_>, _>>()
            .context("Commit verileri okunamadı")?;

        Ok(commits)
    }

    /// Veritabanındaki toplam commit sayısını döner.
    pub fn commit_count(&self) -> Result<usize> {
        let count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM commits", [], |row| row.get(0))
            .context("Commit sayısı sorgulanamadı")?;
        Ok(count as usize)
    }

    // =========================================================================
    // Graph snapshot işlemleri (Sprint 2)
    // =========================================================================

    /// Bir graph snapshot'ı JSON olarak veritabanına kaydeder.
    ///
    /// GraphSnapshot struct'ı serde_json ile serileştirilir, snapshot_json
    /// sütununa yazılır. node_count ve edge_count denormalize olarak da
    /// ayrı sütunlarda tutulur (hızlı sorgu için).
    pub fn insert_graph_snapshot(&self, snapshot: &GraphSnapshot) -> Result<()> {
        let json = serde_json::to_string(snapshot).with_context(|| {
            format!(
                "GraphSnapshot JSON'a dönüştürülemedi: {}",
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
                    "Graph snapshot veritabanına yazılamadı: {}",
                    &snapshot.commit_hash
                )
            })?;

        debug!(
            hash = %snapshot.commit_hash,
            nodes = snapshot.node_count,
            edges = snapshot.edge_count,
            drift = ?snapshot.drift.as_ref().map(|d| d.total),
            "Graph snapshot kaydedildi"
        );
        Ok(())
    }

    /// Son N graph snapshot'ı commit bilgileriyle birlikte listeler.
    ///
    /// Commit tablosu ile JOIN yapılarak commit mesajı ve zaman damgası
    /// da döndürülür. Sonuçlar zaman damgasına göre azalan sırada.
    ///
    /// # Dönüş
    /// `Vec<(commit_hash, message_ilk_satır, timestamp, node_count, edge_count)>`
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
            .context("Graph listesi sorgusu hazırlanamadı")?;

        let rows = stmt
            .query_map([limit as i64], |row| {
                let hash: String = row.get(0)?;
                let message: String = row.get(1)?;
                let timestamp: i64 = row.get(2)?;
                let nodes: i64 = row.get(3)?;
                let edges: i64 = row.get(4)?;
                Ok((hash, message, timestamp, nodes as usize, edges as usize))
            })
            .context("Graph listesi sorgusu çalıştırılamadı")?
            .collect::<Result<Vec<_>, _>>()
            .context("Graph verileri okunamadı")?;

        Ok(rows)
    }

    /// Veritabanındaki toplam graph snapshot sayısını döner.
    pub fn graph_snapshot_count(&self) -> Result<usize> {
        let count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM graph_snapshots", [], |row| row.get(0))
            .context("Graph snapshot sayısı sorgulanamadı")?;
        Ok(count as usize)
    }

    // =========================================================================
    // Sprint 3: Drift-aware graph snapshot işlemleri
    // =========================================================================

    /// Belirli bir commit hash'i için graph snapshot'ı getirir.
    ///
    /// JSON'dan deserialize ederek tam `GraphSnapshot` döndürür.
    /// Drift bilgisi varsa `drift` alanı dolu olur.
    ///
    /// # Dönüş
    /// - `Ok(Some(snapshot))`: Snapshot bulundu
    /// - `Ok(None)`: Bu commit için snapshot yok
    pub fn get_graph_snapshot(&self, commit_hash: &str) -> Result<Option<GraphSnapshot>> {
        let mut stmt = self
            .conn
            .prepare("SELECT snapshot_json FROM graph_snapshots WHERE commit_hash = ?1")
            .context("Graph snapshot sorgusu hazırlanamadı")?;

        let result = stmt
            .query_row([commit_hash], |row| {
                let json: String = row.get(0)?;
                Ok(json)
            })
            .optional()
            .context("Graph snapshot sorgusu çalıştırılamadı")?;

        match result {
            Some(json) => {
                let snapshot: GraphSnapshot = serde_json::from_str(&json).with_context(|| {
                    format!("Graph snapshot JSON parse edilemedi: {commit_hash}")
                })?;
                Ok(Some(snapshot))
            }
            None => Ok(None),
        }
    }

    /// Son N commit için drift trend verilerini listeler.
    ///
    /// Drift skoru olan snapshot'ları zaman sırasına göre getirir.
    /// Her satır: commit_hash, mesaj, düğüm sayısı, kenar sayısı,
    /// drift skoru, önceki commit'e göre delta.
    ///
    /// # Dönüş
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
            .context("Drift trend sorgusu hazırlanamadı")?;

        let rows = stmt
            .query_map([limit as i64], |row| {
                let hash: String = row.get(0)?;
                let message: String = row.get(1)?;
                let nodes: i64 = row.get(2)?;
                let edges: i64 = row.get(3)?;
                let drift_json: Option<String> = row.get(4)?;
                let timestamp: i64 = row.get(5)?;

                // drift_json varsa parse et, yoksa None
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
            .context("Drift trend sorgusu çalıştırılamadı")?
            .collect::<Result<Vec<_>, _>>()
            .context("Drift verileri okunamadı")?;

        Ok(rows)
    }

    /// Belirli bir commit'in drift skorunu günceller.
    ///
    /// Mevcut graph snapshot'ın drift_json sütununu günceller.
    /// Snapshot zaten varsa sadece drift bilgisi eklenir.
    #[allow(dead_code)] // Gelecek sprintlerde mevcut snapshot'lara drift ekleme için kullanılacak
    pub fn update_drift_score(&self, commit_hash: &str, drift: &DriftScore) -> Result<()> {
        let drift_json = serde_json::to_string(drift)
            .with_context(|| format!("DriftScore JSON'a dönüştürülemedi: {commit_hash}"))?;

        self.conn
            .execute(
                "UPDATE graph_snapshots SET drift_json = ?1 WHERE commit_hash = ?2",
                rusqlite::params![drift_json, commit_hash],
            )
            .with_context(|| format!("Drift skoru güncellenemedi: {commit_hash}"))?;

        debug!(
            hash = %commit_hash,
            total = drift.total,
            "Drift skoru güncellendi"
        );
        Ok(())
    }
}

/// rusqlite optional trait yardımcısı — query_row sonucu yoksa None döner.
///
/// rusqlite'ın kendi `query_row` fonksiyonu satır bulamazsa hata verir.
/// Bu trait `optional()` metodunu ekleyerek None döndürmeyi sağlar.
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
// Testler
// =============================================================================
#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::DependencyEdge;

    #[test]
    fn test_insert_and_list_commits() {
        let db = Database::open_in_memory().expect("DB açılmalı");

        let commit1 = CommitInfo {
            hash: "aaa111".to_string(),
            author_name: "Ali".to_string(),
            author_email: "ali@test.com".to_string(),
            message: "İlk commit".to_string(),
            timestamp: 1_000_000,
            tree_id: "tree_aaa".to_string(),
        };
        let commit2 = CommitInfo {
            hash: "bbb222".to_string(),
            author_name: "Veli".to_string(),
            author_email: "veli@test.com".to_string(),
            message: "İkinci commit".to_string(),
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
        let db = Database::open_in_memory().expect("DB açılmalı");

        let original = CommitInfo {
            hash: "abc123".to_string(),
            author_name: "Eski İsim".to_string(),
            author_email: "eski@test.com".to_string(),
            message: "Orijinal mesaj".to_string(),
            timestamp: 1_000_000,
            tree_id: "tree_1".to_string(),
        };
        let updated = CommitInfo {
            hash: "abc123".to_string(),
            author_name: "Yeni İsim".to_string(),
            author_email: "yeni@test.com".to_string(),
            message: "Güncellenmiş mesaj".to_string(),
            timestamp: 1_000_000,
            tree_id: "tree_1".to_string(),
        };

        db.insert_commit(&original).unwrap();
        db.insert_commit(&updated).unwrap();

        assert_eq!(db.commit_count().unwrap(), 1);
        let commits = db.list_commits().unwrap();
        assert_eq!(commits[0].author_name, "Yeni İsim");
    }

    #[test]
    fn test_insert_and_list_graph_snapshots() {
        let db = Database::open_in_memory().expect("DB açılmalı");

        // Önce bir commit ekle (JOIN için gerekli)
        let commit = CommitInfo {
            hash: "abc123".to_string(),
            author_name: "Test".to_string(),
            author_email: "test@test.com".to_string(),
            message: "Test commit\nBody here".to_string(),
            timestamp: 1_000_000,
            tree_id: "tree_abc".to_string(),
        };
        db.insert_commit(&commit).unwrap();

        // Graph snapshot ekle
        let snapshot = GraphSnapshot {
            commit_hash: "abc123".to_string(),
            nodes: vec!["main".to_string(), "serde".to_string(), "std".to_string()],
            edges: vec![
                DependencyEdge {
                    from_module: "main".to_string(),
                    to_module: "serde".to_string(),
                    file_path: "src/main.rs".to_string(),
                    line: 1,
                },
                DependencyEdge {
                    from_module: "main".to_string(),
                    to_module: "std".to_string(),
                    file_path: "src/main.rs".to_string(),
                    line: 2,
                },
            ],
            node_count: 3,
            edge_count: 2,
            timestamp: 1_000_000,
            drift: None,
        };
        db.insert_graph_snapshot(&snapshot).unwrap();

        // Sayım kontrolü
        assert_eq!(db.graph_snapshot_count().unwrap(), 1);

        // Liste kontrolü
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
        let db = Database::open_in_memory().expect("DB açılmalı");

        // Commit ekle
        let commit = CommitInfo {
            hash: "drift01".to_string(),
            author_name: "Test".to_string(),
            author_email: "test@test.com".to_string(),
            message: "Drift test commit".to_string(),
            timestamp: 3_000_000,
            tree_id: "tree_drift".to_string(),
        };
        db.insert_commit(&commit).unwrap();

        // Drift'li snapshot ekle
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
            }],
            node_count: 2,
            edge_count: 1,
            timestamp: 3_000_000,
            drift: Some(drift),
        };
        db.insert_graph_snapshot(&snapshot).unwrap();

        // Snapshot geri oku
        let retrieved = db
            .get_graph_snapshot("drift01")
            .unwrap()
            .expect("Snapshot bulunmalı");
        assert_eq!(retrieved.commit_hash, "drift01");
        let d = retrieved.drift.expect("Drift olmalı");
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
        let db = Database::open_in_memory().expect("DB açılmalı");

        let commit = CommitInfo {
            hash: "upd01".to_string(),
            author_name: "Test".to_string(),
            author_email: "t@t.com".to_string(),
            message: "Update test".to_string(),
            timestamp: 4_000_000,
            tree_id: "tree_upd".to_string(),
        };
        db.insert_commit(&commit).unwrap();

        // Drift'siz snapshot ekle
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

        // Sonradan drift güncelle
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

        // Trend'den kontrol et
        let trend = db.list_drift_trend(10).unwrap();
        assert_eq!(trend[0].4, Some(42), "Güncellenen drift skoru 42 olmalı");
    }

    #[test]
    fn test_get_graph_snapshot_not_found() {
        let db = Database::open_in_memory().expect("DB açılmalı");

        let result = db.get_graph_snapshot("nonexistent").unwrap();
        assert!(result.is_none(), "Olmayan commit için None dönmeli");
    }
}
