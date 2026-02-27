// =============================================================================
// cli.rs — MorphArch komut satırı arayüzü tanımları
// =============================================================================
//
// Clap derive makroları ile ergonomik CLI yapısı:
//   morpharch scan <path>        → Depoyu tara: commit metadata + graph + drift
//   morpharch watch <path>       → Tara + izleme modu (TUI sonraki sprint)
//   morpharch list-graphs        → Son dependency graph snapshot'ları listele
//   morpharch analyze [commit]   → Belirtilen commit'in drift raporunu göster
//   morpharch list-drift         → Son 20 commit'in drift trend tablosu
//   morpharch --help             → Yardım mesajı
//
// Her subcommand isteğe bağlı bir path alır; verilmezse "." (mevcut dizin) kullanılır.
// list-graphs, list-drift ve analyze path almaz — doğrudan DB'den okur.
// =============================================================================

use clap::{Parser, Subcommand};
use std::path::PathBuf;

/// MorphArch — Monorepo architecture drift visualizer
///
/// Büyük monorepo'ların Git geçmişini tarar, paket/modül bağımlılık grafini
/// oluşturur, architecture drift skorunu hesaplar ve animasyonlu TUI ile gösterir.
#[derive(Parser, Debug)]
#[command(
    name = "morpharch",
    version,
    about = "Monorepo architecture drift visualizer with animated TUI",
    long_about = "MorphArch scans monorepo Git history, builds per-commit dependency graphs,\n\
                  calculates architecture drift scores, and visualizes them with an\n\
                  animated force-graph + timeline using ratatui.\n\n\
                  Sprint 1: Git history scanning & SQLite storage.\n\
                  Sprint 2: Dependency graph building with tree-sitter.\n\
                  Sprint 3: Architecture drift scoring & temporal analysis.",
    after_help = "Examples:\n  morpharch scan .          Scan repo: commits + graphs + drift scores\n  morpharch scan ../myrepo  Scan a specific repository\n  morpharch watch .         Scan + activate watch mode\n  morpharch list-graphs     Show last 10 graph snapshots\n  morpharch list-drift      Show drift score trend (last 20 commits)\n  morpharch analyze         Analyze HEAD commit drift\n  morpharch analyze main~5  Analyze specific commit drift"
)]
pub struct Cli {
    /// Çalıştırılacak alt komut
    #[command(subcommand)]
    pub command: Commands,
}

/// Kullanılabilir alt komutlar
#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Git deposunu tara: commit metadata + dependency graph + drift skoru
    ///
    /// Belirtilen dizindeki Git deposunun son N commit'ini (varsayılan: 500)
    /// okur, her commit için bağımlılık grafini oluşturur, drift skoru
    /// hesaplar ve SQLite veritabanına kaydeder.
    Scan {
        /// Taranacak Git deposunun yolu
        #[arg(default_value = ".")]
        path: PathBuf,
    },

    /// Depoyu tara ve izleme modunu etkinleştir
    ///
    /// Önce bir scan işlemi yapar, ardından dosya değişikliklerini izlemeye
    /// başlar. (TUI görselleştirme sonraki sprintte eklenecek.)
    Watch {
        /// İzlenecek Git deposunun yolu
        #[arg(default_value = ".")]
        path: PathBuf,
    },

    /// Son dependency graph snapshot'ları listele
    ///
    /// Veritabanındaki son 10 graph snapshot'ı commit bilgileriyle birlikte
    /// tablo formatında gösterir.
    ListGraphs,

    /// Belirtilen commit'in detaylı drift raporunu göster
    ///
    /// Drift skoru, alt metrikler, boundary ihlalleri, döngüsel bağımlılıklar
    /// ve iyileştirme önerileri içerir. Commit belirtilmezse HEAD kullanılır.
    Analyze {
        /// Analiz edilecek commit referansı (ör. HEAD, main~5, abc1234)
        #[arg(default_value = None)]
        commit: Option<String>,

        /// Git deposunun yolu (rev-parse için gerekli)
        #[arg(short, long, default_value = ".")]
        path: PathBuf,
    },

    /// Son commit'lerin drift skor trendini tablo olarak göster
    ///
    /// Veritabanındaki son 20 commit'in drift skorlarını, düğüm/kenar
    /// sayılarını ve önceki commit'e göre delta değişimini gösterir.
    ListDrift,
}
