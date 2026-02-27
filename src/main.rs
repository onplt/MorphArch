// =============================================================================
// main.rs — MorphArch giriş noktası
// =============================================================================
//
// Program akışı:
//   1. Logging başlat (tracing-subscriber)
//   2. CLI argümanlarını parse et (clap)
//   3. Yapılandırmayı yükle (config — ~/.morpharch/ dizini + DB yolu)
//   4. Veritabanını aç (SQLite — migration otomatik çalışır)
//   5. Komuta göre işlem yap:
//      - scan        → Depoyu tara (commit + graph + drift), sonuç yazdır
//      - watch       → Depoyu tara, izleme mesajı göster (TUI Sprint 4'te)
//      - list-graphs → Son N graph snapshot'ı tablo formatında listele
//      - analyze     → Belirtilen commit'in detaylı drift raporu
//      - list-drift  → Son 20 commit'in drift trend tablosu
//   6. Hata oluşursa kullanıcı dostu mesaj göster
//
// Çıkış kodları:
//   0 → Başarılı
//   1 → Hata (detay stderr'de)
// =============================================================================

mod cli;
mod commands;
mod config;
mod db;
mod git_scanner;
mod graph_builder;
mod models;
mod parser;
mod scoring;
mod utils;

use std::process;
use std::time::Instant;

use anyhow::Result;
use clap::Parser;
use tracing::info;

use cli::{Cli, Commands};
use config::MorphArchConfig;
use db::Database;

fn main() {
    // Logging altyapısını başlat — diğer her şeyden önce
    utils::init_logging();

    // Asıl iş mantığını çalıştır, hata varsa güzel yazdır ve çık
    if let Err(err) = run() {
        utils::print_error(&err);
        process::exit(1);
    }
}

/// Ana iş mantığı — anyhow::Result döner, main() hataları yakalar.
///
/// Bu ayrım sayesinde tüm hatalar tek noktada (main) yakalanır
/// ve kullanıcı dostu formatta gösterilir.
fn run() -> Result<()> {
    // CLI argümanlarını parse et
    let cli = Cli::parse();

    // Yapılandırmayı yükle (~/.morpharch/ dizini otomatik oluşturulur)
    let config = MorphArchConfig::load()?;
    info!(db_path = %config.db_path.display(), "Yapılandırma hazır");

    // SQLite veritabanını aç (tablo migration'ı otomatik)
    let db = Database::open(&config.db_path)?;

    // Alt komuta göre dallan
    match cli.command {
        Commands::Scan { path } => {
            execute_scan(&path, &db, config.max_commits)?;
        }
        Commands::Watch { path } => {
            // Watch = Scan + izleme mesajı (TUI sonraki sprint)
            execute_scan(&path, &db, config.max_commits)?;
            println!();
            println!("👀 Watch mode active. TUI sonraki sprintte gelecek.");
            println!("   Değişiklik izleme ve canlı graf Sprint 4'te eklenecek.");
        }
        Commands::ListGraphs => {
            execute_list_graphs(&db)?;
        }
        Commands::Analyze { commit, path } => {
            commands::analyze::run_analyze(&path, commit.as_deref(), &db)?;
        }
        Commands::ListDrift => {
            execute_list_drift(&db)?;
        }
    }

    Ok(())
}

/// Scan işlemini çalıştırır ve sonuç özetini yazdırır.
///
/// Sprint 3'ten itibaren commit tarama + dependency graph + drift skoru
/// tek komutta çalışır. `commands::scan::run_scan` üç adımı orkestre eder.
fn execute_scan(path: &std::path::Path, db: &Database, max_commits: usize) -> Result<()> {
    println!("🔍 Depo taranıyor: {}", path.display());
    println!();

    // Zamanlama başlat
    let start = Instant::now();

    // Sprint 3: commit tarama + dependency graph + drift skoru
    let result = commands::scan::run_scan(path, db, max_commits)?;

    // Geçen süreyi hesapla
    let elapsed = start.elapsed();

    // Veritabanındaki toplam kayıt sayıları
    let total_commits = db.commit_count()?;
    let total_graphs = db.graph_snapshot_count()?;

    // Sonuç özeti
    println!(
        "✅ {} commit tarandı, {} graph + {} drift skoru hesaplandı, {:.1} sn",
        result.commits_scanned,
        result.graphs_created,
        result.drifts_calculated,
        elapsed.as_secs_f64()
    );

    if total_commits > result.commits_scanned {
        println!(
            "📊 Veritabanında toplam {} commit, {} graph snapshot kayıtlı",
            total_commits, total_graphs
        );
    }

    Ok(())
}

/// Son graph snapshot'ları tablo formatında listeler.
///
/// Veritabanından son 10 graph snapshot'ı çeker ve her biri için:
/// - Commit hash'inin ilk 7 karakteri
/// - Commit mesajının ilk satırı (max 50 karakter)
/// - Düğüm (node) sayısı
/// - Kenar (edge) sayısı
/// - Tarih (Unix timestamp → okunabilir format)
fn execute_list_graphs(db: &Database) -> Result<()> {
    let total = db.graph_snapshot_count()?;

    if total == 0 {
        println!("📭 Henüz graph snapshot yok. Önce 'morpharch scan <path>' çalıştırın.");
        return Ok(());
    }

    let graphs = db.list_recent_graphs(10)?;

    println!("📊 Son graph snapshot'lar ({total} kayıttan):");
    println!();
    let header = format!(
        "{:<9} {:<50} {:>6} {:>6}   {}",
        "HASH", "MESSAGE", "NODES", "EDGES", "DATE"
    );
    println!("{header}");
    let separator = "─".repeat(95);
    println!("{separator}");

    for (hash, message, timestamp, nodes, edges) in &graphs {
        // Hash: ilk 7 karakter
        let short_hash = if hash.len() >= 7 { &hash[..7] } else { hash };

        // Message: ilk satır, max 50 karakter
        let first_line = message.lines().next().unwrap_or("");
        let truncated = if first_line.len() > 50 {
            format!("{}…", &first_line[..49])
        } else {
            first_line.to_string()
        };

        // Timestamp → okunabilir tarih
        let date = chrono::DateTime::from_timestamp(*timestamp, 0)
            .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
            .unwrap_or_else(|| "?".to_string());

        println!(
            "{:<9} {:<50} {:>6} {:>6}   {}",
            short_hash, truncated, nodes, edges, date
        );
    }

    println!();
    println!("Toplam: {total} graph snapshot");

    Ok(())
}

/// Son 20 commit'in drift skor trendini tablo formatında gösterir.
///
/// Her satır: commit hash, mesaj, düğüm sayısı, kenar sayısı,
/// drift skoru ve önceki commit'e göre delta.
fn execute_list_drift(db: &Database) -> Result<()> {
    let trend = db.list_drift_trend(20)?;

    if trend.is_empty() {
        println!("📭 Henüz drift verisi yok. Önce 'morpharch scan <path>' çalıştırın.");
        return Ok(());
    }

    println!("📈 Drift Skor Trendi (son {} commit):", trend.len());
    println!();
    let header = format!(
        "{:<9} {:<35} {:>6} {:>6} {:>7} {:>7}   {}",
        "HASH", "MESSAGE", "NODES", "EDGES", "DRIFT", "DELTA", "DATE"
    );
    println!("{header}");
    let separator = "─".repeat(100);
    println!("{separator}");

    let mut prev_drift: Option<u8> = None;

    // Trend zaman damgasına göre azalan sırada — tersten itereyerek
    // kronolojik sırada delta hesaplıyoruz
    let reversed: Vec<_> = trend.iter().rev().collect();

    for (hash, message, nodes, edges, drift_total, timestamp) in &reversed {
        let short_hash = if hash.len() >= 7 { &hash[..7] } else { hash };

        let first_line = message.lines().next().unwrap_or("");
        let truncated = if first_line.len() > 35 {
            format!("{}…", &first_line[..34])
        } else {
            first_line.to_string()
        };

        let drift_str = drift_total
            .map(|d| format!("{d}"))
            .unwrap_or_else(|| "—".to_string());

        let delta_str = match (*drift_total, prev_drift) {
            (Some(curr), Some(prev)) => {
                let d = curr as i32 - prev as i32;
                if d > 0 {
                    format!("+{d}")
                } else if d < 0 {
                    format!("{d}")
                } else {
                    "0".to_string()
                }
            }
            _ => "—".to_string(),
        };

        let date = chrono::DateTime::from_timestamp(*timestamp, 0)
            .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
            .unwrap_or_else(|| "?".to_string());

        println!(
            "{:<9} {:<35} {:>6} {:>6} {:>7} {:>7}   {}",
            short_hash, truncated, nodes, edges, drift_str, delta_str, date
        );

        prev_drift = *drift_total;
    }

    println!();
    println!("Toplam: {} commit analiz edildi", trend.len());

    Ok(())
}
