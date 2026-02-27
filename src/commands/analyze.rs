// =============================================================================
// commands/analyze.rs — Analyze komutu: detaylı drift raporu
// =============================================================================
//
// Sprint 3 analiz modülü. Belirtilen commit (veya HEAD) için:
//   1. Graph snapshot'ı DB'den çeker
//   2. Drift skorunu ve alt metrikleri gösterir
//   3. Önceki 3 commit ile temporal delta hesaplar
//   4. Top boundary violator'ları listeler
//   5. Döngü bilgisini raporlar
//   6. İyileştirme önerileri sunar
//
// Kullanım:
//   morpharch analyze           → HEAD commit analizi
//   morpharch analyze main~5    → Belirtilen commit analizi
// =============================================================================

use std::collections::HashSet;
use std::path::Path;

use anyhow::{Context, Result};
use tracing::info;

use crate::db::Database;
use crate::graph_builder;
use crate::models::DriftScore;
use crate::scoring;

/// Analyze komutunu çalıştırır: detaylı drift raporu üretir.
///
/// # Parametreler
/// - `repo_path`: Git deposunun yolu (commit-ish resolve için)
/// - `commit_ish`: Analiz edilecek commit referansı (None = HEAD)
/// - `db`: SQLite veritabanı referansı
///
/// # İş Akışı
/// 1. commit-ish'i resolve et (gix ile)
/// 2. DB'den graph snapshot'ı çek
/// 3. Drift raporu yazdır
/// 4. Önceki commit'lerle karşılaştır
/// 5. Öneriler sun
pub fn run_analyze(repo_path: &Path, commit_ish: Option<&str>, db: &Database) -> Result<()> {
    // ── Commit hash'i resolve et ──
    let commit_hash = resolve_commit(repo_path, commit_ish)?;
    let short_hash = if commit_hash.len() >= 7 {
        &commit_hash[..7]
    } else {
        &commit_hash
    };

    info!(hash = %commit_hash, "Commit analiz ediliyor");

    // ── Graph snapshot'ı getir ──
    let snapshot = db
        .get_graph_snapshot(&commit_hash)?
        .with_context(|| format!("Bu commit için graph snapshot bulunamadı: {short_hash}"))?;

    println!("🔬 Commit Analizi: {short_hash}");
    println!();

    // ── Drift raporu ──
    if let Some(ref drift) = snapshot.drift {
        print_drift_report(drift, snapshot.node_count, snapshot.edge_count);
    } else {
        println!("⚠️  Bu commit için drift skoru hesaplanmamış.");
        println!("   Önce 'morpharch scan <path>' ile yeniden tarama yapın.");
        return Ok(());
    }

    // ── Temporal analiz: önceki 3 commit ile karşılaştır ──
    println!();
    println!("📈 Temporal Analiz (önceki commit'lerle karşılaştırma):");
    println!();

    let trend = db.list_drift_trend(20)?;

    // Mevcut commit'in pozisyonunu bul
    let current_pos = trend.iter().position(|(h, ..)| h == &commit_hash);

    if let Some(pos) = current_pos {
        // Sonraki 3 kayıt (kronolojik olarak önceki commit'ler)
        let prev_commits: Vec<_> = trend.iter().skip(pos + 1).take(3).collect();

        if prev_commits.is_empty() {
            println!("  İlk commit — karşılaştırılacak önceki commit yok.");
        } else {
            let header = format!(
                "  {:<9} {:>6} {:>6} {:>7} {:>8}",
                "HASH", "NODES", "EDGES", "DRIFT", "DELTA"
            );
            println!("{header}");
            let separator = format!("  {}", "─".repeat(45));
            println!("{separator}");

            // Mevcut commit'in drift skoru
            let current_drift = snapshot.drift.as_ref().map(|d| d.total).unwrap_or(0);

            for (prev_hash, _msg, prev_nodes, prev_edges, prev_drift, _ts) in &prev_commits {
                let prev_short = if prev_hash.len() >= 7 {
                    &prev_hash[..7]
                } else {
                    prev_hash
                };
                let drift_str = prev_drift
                    .map(|d| format!("{d}"))
                    .unwrap_or_else(|| "?".to_string());
                let delta = prev_drift
                    .map(|d| current_drift as i32 - d as i32)
                    .map(|d| {
                        if d > 0 {
                            format!("+{d}")
                        } else {
                            format!("{d}")
                        }
                    })
                    .unwrap_or_else(|| "?".to_string());

                println!(
                    "  {:<9} {:>6} {:>6} {:>7} {:>8}",
                    prev_short, prev_nodes, prev_edges, drift_str, delta
                );
            }
        }
    } else {
        println!("  Bu commit trend verisinde bulunamadı.");
    }

    // ── Boundary violation detayları ──
    println!();
    print_boundary_details(&snapshot.edges);

    // ── Döngü bilgisi ──
    println!();
    print_cycle_info(&snapshot.nodes, &snapshot.edges);

    // ── Öneriler ──
    println!();
    print_recommendations(&snapshot.drift);

    Ok(())
}

/// commit-ish referansını tam SHA hash'e çevirir.
///
/// `gix` ile depoyu açar ve rev-parse yapar.
/// None verilmişse HEAD kullanılır.
fn resolve_commit(repo_path: &Path, commit_ish: Option<&str>) -> Result<String> {
    let repo = gix::discover(repo_path)
        .with_context(|| format!("Git deposu bulunamadı: {}", repo_path.display()))?;

    let reference = commit_ish.unwrap_or("HEAD");

    // gix ile rev-parse — detach() ile ObjectId'ye çevir
    let object = repo
        .rev_parse_single(reference)
        .with_context(|| format!("Commit referansı çözümlenemedi: '{reference}'"))?;

    Ok(object.detach().to_string())
}

/// Drift skoru detay raporu yazdırır.
///
/// Skor seviyesine göre emoji ve renk kodu:
/// - 0-30: 🟢 Sağlıklı
/// - 31-60: 🟡 Dikkat
/// - 61-80: 🟠 Uyarı
/// - 81-100: 🔴 Kritik
fn print_drift_report(drift: &DriftScore, node_count: usize, edge_count: usize) {
    let (emoji, level) = match drift.total {
        0..=30 => ("🟢", "Sağlıklı"),
        31..=60 => ("🟡", "Dikkat"),
        61..=80 => ("🟠", "Uyarı"),
        _ => ("🔴", "Kritik"),
    };

    println!("{emoji} Drift Skoru: {}/100 ({level})", drift.total);
    println!();
    println!("  📊 Graf İstatistikleri:");
    println!("     Düğüm (modül) sayısı:    {node_count}");
    println!("     Kenar (bağımlılık) sayısı: {edge_count}");
    println!();
    println!("  📐 Alt Metrikler:");
    println!("     Fan-in değişimi:          {:+}", drift.fan_in_delta);
    println!("     Fan-out değişimi:         {:+}", drift.fan_out_delta);
    println!("     Yeni döngüsel bağımlılık: {}", drift.new_cycles);
    println!(
        "     Sınır ihlali:             {}",
        drift.boundary_violations
    );
    println!(
        "     Bilişsel karmaşıklık:     {:.2}",
        drift.cognitive_complexity
    );
}

/// Boundary violation detaylarını yazdırır.
///
/// Ham kenar listesinden ihlal eden kenarları bulur ve listeler.
fn print_boundary_details(edges: &[crate::models::DependencyEdge]) {
    let pairs = scoring::edges_to_pairs(edges);
    let violations: Vec<_> = pairs
        .iter()
        .filter(|(from, to)| {
            scoring::BOUNDARY_RULES
                .iter()
                .any(|(fp, tp)| from.starts_with(fp) && to.starts_with(tp))
        })
        .collect();

    if violations.is_empty() {
        println!("✅ Boundary İhlali: Yok — paket sınırları temiz.");
    } else {
        println!("⚠️  Boundary İhlalleri ({} adet):", violations.len());
        for (i, (from, to)) in violations.iter().enumerate().take(10) {
            println!("     {}. {} → {}", i + 1, from, to);
        }
        if violations.len() > 10 {
            println!("     ... ve {} tane daha", violations.len() - 10);
        }
    }
}

/// Döngüsel bağımlılık bilgisini yazdırır.
fn print_cycle_info(nodes: &[String], edges: &[crate::models::DependencyEdge]) {
    let node_set: HashSet<String> = nodes.iter().cloned().collect();
    let graph = graph_builder::build_graph(&node_set, edges);
    let cycle_count = scoring::count_cycles_public(&graph);

    if cycle_count == 0 {
        println!("✅ Döngüsel Bağımlılık: Yok — DAG yapısı korunuyor.");
    } else {
        println!("⚠️  Döngüsel Bağımlılık: {cycle_count} adet döngü tespit edildi.");
        println!("     Döngüler mimari karmaşıklığı artırır ve refactoring'i zorlaştırır.");
    }
}

/// Drift skoruna göre iyileştirme önerileri sunar.
fn print_recommendations(drift: &Option<DriftScore>) {
    println!("💡 Öneriler:");

    let Some(d) = drift else {
        println!("   Drift skoru hesaplanmamış — 'morpharch scan' çalıştırın.");
        return;
    };

    let mut suggestions = Vec::new();

    if d.new_cycles > 0 {
        suggestions.push(format!(
            "🔄 {} yeni döngüsel bağımlılık var. Dependency Inversion prensibi \
             uygulayarak interface/trait ile kırın.",
            d.new_cycles
        ));
    }

    if d.boundary_violations > 0 {
        suggestions.push(format!(
            "🚧 {} sınır ihlali var. Kütüphane katmanı (packages/lib) \
             uygulama katmanına (apps/cmd) bağımlı olmamalı.",
            d.boundary_violations
        ));
    }

    if d.fan_out_delta > 5 {
        suggestions.push(
            "📤 Fan-out artışı yüksek. Modüller çok fazla dış bağımlılık ekliyor. \
             Facade pattern veya modül birleştirme düşünün."
                .to_string(),
        );
    }

    if d.cognitive_complexity > 20.0 {
        suggestions.push(
            "🧠 Bilişsel karmaşıklık yüksek. Graf çok yoğun — modülleri daha \
             küçük, odaklı parçalara bölmeyi düşünün."
                .to_string(),
        );
    }

    if d.total <= 30 {
        suggestions.push("🎉 Mimari sağlıklı görünüyor! İyi gidiyorsunuz.".to_string());
    }

    if suggestions.is_empty() {
        suggestions.push("👍 Genel durum kabul edilebilir düzeyde.".to_string());
    }

    for suggestion in &suggestions {
        println!("   {suggestion}");
    }
}
