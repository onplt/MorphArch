// =============================================================================
// git_scanner.rs — Git deposu tarayıcı
// =============================================================================
//
// gitoxide (gix) kütüphanesi ile Git deposunu tarar:
//   1. gix::discover() ile depoyu bulur ve açar
//   2. HEAD commit'inden geriye doğru yürür (ancestor walk)
//   3. Her commit için metadata çıkarır (yazar, mesaj, zaman, tree)
//   4. Çıkarılan veriyi SQLite veritabanına kaydeder
//
// Performans notları:
//   - gix, libgit2'ye göre daha hızlı ve pure-Rust
//   - Ancestor walk lazy iterator ile çalışır (bellek dostu)
//   - Her 100 commit'te debug log ile ilerleme raporlanır
// =============================================================================

use anyhow::{Context, Result};
use std::path::Path;
use tracing::{debug, info, warn};

use crate::db::Database;
use crate::models::CommitInfo;

/// Git deposunu keşfeder ve açar.
///
/// Verilen dizinden yukarı doğru `.git` arayarak depoyu bulur.
/// `commands::scan` ve diğer modüller tarafından kullanılabilir.
///
/// # Hatalar
/// - Yol geçerli bir Git deposu değilse
pub fn open_repository(path: &Path) -> Result<gix::Repository> {
    gix::discover(path).with_context(|| {
        format!(
            "'{}' geçerli bir Git deposu değil. \
             .git dizini bulunamadı.",
            path.display()
        )
    })
}

/// Belirtilen dizindeki Git deposunu tarar ve commit metadata'sını DB'ye kaydeder.
///
/// # Parametreler
/// - `path`: Git deposunun kök dizini (veya içindeki herhangi bir dizin)
/// - `db`: Commit'lerin yazılacağı veritabanı referansı
/// - `max_commits`: Okunacak maksimum commit sayısı (varsayılan 500)
///
/// # Dönüş Değeri
/// Başarıyla taranan commit sayısı.
///
/// # Hatalar
/// - Yol geçerli bir Git deposu değilse
/// - Depo boşsa (HEAD commit yoksa)
/// - Commit nesneleri okunamazsa veya çözümlenemezse
/// - Veritabanı yazma hatası oluşursa
pub fn scan_repository(path: &Path, db: &Database, max_commits: usize) -> Result<usize> {
    info!(path = %path.display(), "Git deposu açılıyor");

    // Git deposunu bul — open_repository yardımcı fonksiyonunu kullan
    let repo = open_repository(path)?;

    // HEAD commit'ini al
    let head = repo.head_commit().context(
        "HEAD commit bulunamadı. \
         Depo boş olabilir — en az bir commit gerekli.",
    )?;

    info!(head = %head.id, "HEAD commit bulundu");

    let mut count: usize = 0;

    // HEAD'den geriye doğru tüm ancestor commit'leri yürü
    let ancestors = head.ancestors().all().context(
        "Commit geçmişi yürüyüşü başlatılamadı. \
         Depo bütünlüğünü 'git fsck' ile kontrol edin.",
    )?;

    for ancestor_result in ancestors {
        // Maksimum limite ulaşıldıysa dur
        if count >= max_commits {
            info!(
                limit = max_commits,
                "Maksimum commit limitine ulaşıldı, tarama durduruluyor"
            );
            break;
        }

        // Ancestor bilgisini al (ID + parent ID'leri)
        let ancestor_info = match ancestor_result {
            Ok(info) => info,
            Err(e) => {
                warn!(error = %e, "Bir commit okunamadı, atlanıyor");
                continue;
            }
        };

        // Commit nesnesini yükle ve decode et
        let commit_object = repo
            .find_object(ancestor_info.id)
            .with_context(|| format!("Commit nesnesi yüklenemedi: {}", ancestor_info.id))?;

        let commit = commit_object.into_commit();
        let decoded = commit
            .decode()
            .with_context(|| format!("Commit çözümlenemedi: {}", ancestor_info.id))?;

        // Commit metadata'sını çıkar
        let commit_info = CommitInfo {
            hash: ancestor_info.id.to_string(),
            author_name: decoded.author.name.to_string(),
            author_email: decoded.author.email.to_string(),
            message: decoded.message.to_string(),
            timestamp: decoded.author.time.seconds,
            tree_id: decoded.tree().to_string(),
        };

        // Veritabanına kaydet
        db.insert_commit(&commit_info)?;
        count += 1;

        // Her 100 commit'te ilerleme raporu
        if count.is_multiple_of(100) {
            debug!(count, "Tarama ilerlemesi");
        }
    }

    info!(total = count, "Depo taraması tamamlandı");
    Ok(count)
}

// =============================================================================
// Testler
// =============================================================================
#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;
    use std::path::PathBuf;

    /// Mevcut repo'yu (MorphArch kendisi) tarayarak git_scanner'ın
    /// doğru çalıştığını test eder.
    ///
    /// - Depo en az 1 commit içermeli (Initial commit)
    /// - max_commits=10 ile sınırlandırıyoruz
    /// - Taranan commit sayısı > 0 ve <= 10 olmalı
    /// - DB'deki kayıt sayısı taranan ile eşleşmeli
    #[test]
    fn test_scan_current_repo() {
        let db = Database::open_in_memory().expect("In-memory DB açılmalı");

        // Mevcut proje dizini bir Git deposu
        let path = PathBuf::from(".");
        let count = scan_repository(&path, &db, 10).expect("Tarama başarılı olmalı");

        assert!(count > 0, "En az 1 commit bulunmalı");
        assert!(count <= 10, "max_commits=10 sınırına uyulmalı");

        // DB'deki kayıt sayısı eşleşmeli
        let db_count = db.commit_count().expect("Sayım başarılı olmalı");
        assert_eq!(count, db_count, "Taranan ve kaydedilen sayı eşleşmeli");
    }

    /// Geçersiz bir dizin verildiğinde anlamlı hata döndüğünü test eder.
    #[test]
    fn test_scan_invalid_path_returns_error() {
        let db = Database::open_in_memory().expect("In-memory DB açılmalı");

        let result = scan_repository(Path::new("/nonexistent/fake/repo"), &db, 10);
        assert!(result.is_err(), "Geçersiz yol hata döndürmeli");

        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("Git deposu değil"),
            "Hata mesajı açıklayıcı olmalı, ama şu geldi: {err_msg}"
        );
    }
}
