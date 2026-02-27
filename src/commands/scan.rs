// =============================================================================
// commands/scan.rs — Scan komutu: commit tarama + dependency graph + drift skoru
// =============================================================================
//
// Sprint 2-3 ana orkestrasyon modülü. İş akışı:
//
//   1. git_scanner ile commit metadata'larını DB'ye kaydet (Sprint 1)
//   2. gix ile depoyu aç
//   3. DB'deki her commit için:
//      a. Aynı tree_id daha önce işlendiyse atla (deduplicate)
//      b. Commit'in tree'sini recursive yürü
//      c. Desteklenen dosyaları (*.rs, *.ts, *.py, *.go) bul
//      d. tree-sitter ile import'ları çıkar
//      e. petgraph ile bağımlılık grafi oluştur
//      f. Sprint 3: drift skoru hesapla (önceki graf ile karşılaştır)
//      g. GraphSnapshot'ı drift ile birlikte DB'ye kaydet
//
// Performans notları:
//   - Tree deduplicate: aynı tree_id için tekrar parse yapılmaz
//   - Blob okuma: gix object DB'den direkt, disk checkout yok
//   - Büyük dosyalar (>512KB) parser tarafından atlanır
//   - Drift hesaplaması O(V+E) — 500 commit için <3 sn ek yük
// =============================================================================

use std::collections::HashSet;
use std::path::Path;

use anyhow::{Context, Result};
use petgraph::graph::DiGraph;
use tracing::{debug, info, warn};

use crate::db::Database;
use crate::git_scanner;
use crate::graph_builder;
use crate::models::{DependencyEdge, GraphSnapshot};
use crate::parser;
use crate::scoring;

/// Desteklenen dosya uzantıları — sadece bu uzantıdaki dosyalar parse edilir
const SUPPORTED_EXTENSIONS: &[&str] = &["rs", "ts", "tsx", "py", "go"];

/// Scan sonucu — main.rs'te özet yazdırmak için
pub struct ScanResult {
    pub commits_scanned: usize,
    pub graphs_created: usize,
    /// Sprint 3: Hesaplanan drift skoru sayısı
    pub drifts_calculated: usize,
}

/// Depoyu tarar: commit metadata + dependency graph + drift skoru oluşturur.
///
/// # İş Akışı
/// 1. Sprint 1 scan: commit'leri DB'ye kaydet
/// 2. Sprint 2 scan: her commit için dependency graph oluştur
/// 3. Sprint 3 scan: her graph için drift skoru hesapla
///
/// # Parametreler
/// - `path`: Git deposunun yolu
/// - `db`: SQLite veritabanı referansı
/// - `max_commits`: Taranacak maksimum commit sayısı
///
/// # Dönüş
/// `ScanResult` — taranan commit, oluşturulan graph ve drift sayıları
pub fn run_scan(path: &Path, db: &Database, max_commits: usize) -> Result<ScanResult> {
    // ── Adım 1: Commit metadata'larını tara ve DB'ye kaydet ──
    let commits_scanned = git_scanner::scan_repository(path, db, max_commits)?;

    // ── Adım 2: Her commit için dependency graph + drift skoru oluştur ──
    info!("Dependency graph'ları oluşturuluyor...");

    let repo = gix::discover(path)
        .with_context(|| format!("Graph oluşturma için depo açılamadı: {}", path.display()))?;

    // DB'den tüm commit'leri al (zaman damgasına göre azalan — en yeni ilk)
    let commits = db.list_commits()?;
    let mut graphs_created: usize = 0;
    let mut drifts_calculated: usize = 0;

    // Aynı tree'yi tekrar tekrar işlemekten kaçın
    let mut seen_trees: HashSet<String> = HashSet::new();

    // Sprint 3: Önceki commit'in graf'ını tut (temporal analiz için)
    // Commit'ler zaman damgasına göre azalan sırada, bu yüzden
    // "prev_graph" aslında bir sonraki kronolojik commit'in grafıdır
    let mut prev_graph: Option<DiGraph<String, ()>> = None;

    for commit in &commits {
        // Deduplicate: aynı tree_id zaten işlendiyse atla
        if !seen_trees.insert(commit.tree_id.clone()) {
            debug!(
                hash = %commit.hash,
                tree = %commit.tree_id,
                "Aynı tree zaten işlendi, atlanıyor"
            );
            continue;
        }

        // Tree OID'yi hex string'den parse et
        let tree_oid = match gix::ObjectId::from_hex(commit.tree_id.as_bytes()) {
            Ok(oid) => oid,
            Err(e) => {
                warn!(
                    hash = %commit.hash,
                    tree = %commit.tree_id,
                    error = %e,
                    "Tree ID parse edilemedi, atlanıyor"
                );
                continue;
            }
        };

        // Tree'yi recursive yürü ve kaynak dosyaları topla
        let files = match walk_tree_collect(&repo, tree_oid) {
            Ok(f) => f,
            Err(e) => {
                warn!(
                    hash = %commit.hash,
                    error = %e,
                    "Tree yürüyüşü başarısız, atlanıyor"
                );
                continue;
            }
        };

        if files.is_empty() {
            continue;
        }

        // Her dosyayı parse et, import'ları çıkar, kenar ve düğüm listesi oluştur
        let mut all_nodes: HashSet<String> = HashSet::new();
        let mut all_edges: Vec<DependencyEdge> = Vec::new();

        for (file_path, content_bytes) in &files {
            // UTF-8 değilse atla (binary dosya olabilir)
            let content = match std::str::from_utf8(content_bytes) {
                Ok(s) => s,
                Err(_) => continue,
            };

            // Dili tespit et
            let lang = match parser::detect_language(file_path) {
                Some(l) => l,
                None => continue,
            };

            // Import'ları çıkar
            let imports = parser::parse_imports(content, lang);

            if imports.is_empty() {
                continue;
            }

            // Dosya yolundan modül adı çıkar
            let source_module = path_to_module(file_path);
            all_nodes.insert(source_module.clone());

            for imp in imports {
                all_nodes.insert(imp.clone());
                all_edges.push(DependencyEdge {
                    from_module: source_module.clone(),
                    to_module: imp,
                    file_path: file_path.clone(),
                    line: 0,
                });
            }
        }

        // Boş graf kaydetmeye gerek yok
        if all_nodes.is_empty() {
            continue;
        }

        // petgraph ile graf oluştur
        let graph = graph_builder::build_graph(&all_nodes, &all_edges);

        // Sprint 3: Drift skoru hesapla
        let nodes_vec: Vec<String> = all_nodes.iter().cloned().collect();
        let edges_pairs = scoring::edges_to_pairs(&all_edges);
        let drift = scoring::calculate_drift(
            &graph,
            prev_graph.as_ref(),
            &nodes_vec,
            &edges_pairs,
            commit.timestamp,
        );
        drifts_calculated += 1;

        // GraphSnapshot oluştur ve DB'ye kaydet
        let snapshot = GraphSnapshot {
            commit_hash: commit.hash.clone(),
            nodes: nodes_vec,
            edges: all_edges,
            node_count: graph.node_count(),
            edge_count: graph.edge_count(),
            timestamp: commit.timestamp,
            drift: Some(drift),
        };

        db.insert_graph_snapshot(&snapshot)?;
        graphs_created += 1;

        // Önceki grafı güncelle (temporal analiz için)
        prev_graph = Some(graph);

        if graphs_created.is_multiple_of(50) {
            debug!(count = graphs_created, "Graph oluşturma ilerlemesi");
        }
    }

    info!(
        total = graphs_created,
        drifts = drifts_calculated,
        "Dependency graph + drift oluşturma tamamlandı"
    );

    Ok(ScanResult {
        commits_scanned,
        graphs_created,
        drifts_calculated,
    })
}

// =============================================================================
// Git tree yürüyüşü — gix ile blob'ları bellek üzerinde okur
// =============================================================================

/// Bir Git tree nesnesini recursive yürüyerek desteklenen kaynak dosyaları toplar.
///
/// # Dönüş
/// `Vec<(path, content_bytes)>` — dosya yolu ve içerik byte'ları
fn walk_tree_collect(
    repo: &gix::Repository,
    tree_oid: gix::ObjectId,
) -> Result<Vec<(String, Vec<u8>)>> {
    let mut files = Vec::new();
    walk_tree_recursive(repo, tree_oid, "", &mut files, 0)?;
    Ok(files)
}

/// Maksimum dizin derinliği — çok derin iç içe yapılar için stack koruması
const MAX_TREE_DEPTH: usize = 30;

/// Tree nesnesini recursive yürür, desteklenen uzantıdaki blob'ları toplar.
///
/// # Parametreler
/// - `repo`: gix Repository referansı
/// - `tree_oid`: Yürünecek tree nesnesinin ObjectId'si
/// - `prefix`: Mevcut dizin yolu (ör. "src/commands")
/// - `files`: Toplanan dosyaların yazılacağı vektör
/// - `depth`: Mevcut derinlik (stack overflow koruması)
fn walk_tree_recursive(
    repo: &gix::Repository,
    tree_oid: gix::ObjectId,
    prefix: &str,
    files: &mut Vec<(String, Vec<u8>)>,
    depth: usize,
) -> Result<()> {
    // Derinlik koruması
    if depth > MAX_TREE_DEPTH {
        return Ok(());
    }

    // Tree nesnesini bul ve decode et
    let tree_obj = repo
        .find_object(tree_oid)
        .context("Tree nesnesi bulunamadı")?;
    let tree = tree_obj.into_tree();
    let decoded = tree.decode().context("Tree decode edilemedi")?;

    for entry in &decoded.entries {
        let name = entry.filename.to_string();

        // Tam dosya yolu oluştur
        let path = if prefix.is_empty() {
            name
        } else {
            format!("{prefix}/{name}")
        };

        if entry.mode.is_tree() {
            // Alt dizine recursive dallan
            walk_tree_recursive(repo, entry.oid.to_owned(), &path, files, depth + 1)?;
        } else if entry.mode.is_blob() || entry.mode.is_executable() {
            // Dosya uzantısını kontrol et
            if let Some(ext) = path.rsplit('.').next()
                && SUPPORTED_EXTENSIONS.contains(&ext)
            {
                // Blob içeriğini oku
                match repo.find_object(entry.oid.to_owned()) {
                    Ok(blob) => {
                        files.push((path, blob.data.to_vec()));
                    }
                    Err(e) => {
                        debug!(path = %path, error = %e, "Blob okunamadı, atlanıyor");
                    }
                }
            }
        }
    }

    Ok(())
}

// =============================================================================
// Yardımcı fonksiyonlar
// =============================================================================

/// Dosya yolundan modül adı çıkarır.
///
/// Uzantıyı kaldırır ve dizin ayraçlarını `::` ile değiştirir.
///
/// # Örnekler
/// - "src/main.rs" → "src::main"
/// - "packages/ui/index.ts" → "packages::ui::index"
/// - "cmd/server/main.go" → "cmd::server::main"
pub fn path_to_module(path: &str) -> String {
    let without_ext = path.rsplit_once('.').map_or(path, |(base, _)| base);
    without_ext.replace('/', "::")
}

// =============================================================================
// Testler
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
    }
}
