// =============================================================================
// config.rs — MorphArch yapılandırma yönetimi
// =============================================================================
//
// Varsayılan yapılandırma değerlerini ve veritabanı yolunu yönetir.
//
// Veritabanı konumu: ~/.morpharch/morpharch.db
//   - Klasör yoksa otomatik oluşturulur
//   - Platform bağımsız: dirs crate ile home dizini tespit edilir
//
// max_commits: Tek seferde taranacak maksimum commit sayısı (varsayılan: 500)
// =============================================================================

use anyhow::{Context, Result};
use std::path::PathBuf;
use tracing::info;

/// Taranacak maksimum commit sayısı (Sprint 1 için sabit)
const DEFAULT_MAX_COMMITS: usize = 500;

/// MorphArch uygulamasının çalışma zamanı yapılandırması
///
/// Veritabanı yolu ve tarama parametrelerini tutar.
/// İleride TOML/YAML dosyasından okunabilir hale getirilecek.
#[derive(Debug)]
pub struct MorphArchConfig {
    /// SQLite veritabanı dosyasının tam yolu
    pub db_path: PathBuf,

    /// Tek taramada okunacak maksimum commit sayısı
    pub max_commits: usize,
}

impl MorphArchConfig {
    /// Varsayılan yapılandırmayı yükler.
    ///
    /// ~/.morpharch/ dizinini oluşturur (yoksa) ve veritabanı yolunu ayarlar.
    /// Home dizini bulunamazsa anlamlı bir hata mesajı döner.
    pub fn load() -> Result<Self> {
        // Platform-bağımsız home dizini tespiti
        let home = dirs::home_dir().context(
            "Home dizini bulunamadı. \
             HOME (Linux/macOS) veya USERPROFILE (Windows) ortam değişkenini kontrol edin.",
        )?;

        // ~/.morpharch/ veri dizinini oluştur
        let morpharch_dir = home.join(".morpharch");
        std::fs::create_dir_all(&morpharch_dir).with_context(|| {
            format!(
                "MorphArch veri dizini oluşturulamadı: {}",
                morpharch_dir.display()
            )
        })?;

        let db_path = morpharch_dir.join("morpharch.db");
        info!(path = %db_path.display(), "Yapılandırma yüklendi");

        Ok(Self {
            db_path,
            max_commits: DEFAULT_MAX_COMMITS,
        })
    }
}
