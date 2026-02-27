// =============================================================================
// utils.rs — MorphArch yardımcı araçlar
// =============================================================================
//
// Logging altyapısı ve hata formatlama:
//
//   init_logging()  → tracing-subscriber ile yapılandırılmış log başlatır
//     - RUST_LOG ortam değişkeni ile seviye ayarlanabilir
//     - Varsayılan seviye: INFO
//     - Hedef bilgisi gizlenir (daha temiz çıktı)
//     - Zaman damgası gösterilmez (CLI aracı için gereksiz)
//
//   print_error()   → anyhow hata zincirini kullanıcı dostu formatta yazdırır
//     - Her bağlam (context) katmanı ayrı satırda gösterilir
//     - Kök neden (root cause) vurgulanır
// =============================================================================

use anyhow::Error;
use tracing_subscriber::{EnvFilter, fmt};

/// Structured logging altyapısını başlatır.
///
/// tracing-subscriber ile:
///   - `RUST_LOG` ortam değişkeninden filtre okunur (ör. RUST_LOG=debug)
///   - Ortam değişkeni yoksa varsayılan "info" seviyesi kullanılır
///   - Hedef modül bilgisi gizlenir (kompakt çıktı)
///   - Çıktı stderr'e yönlendirilir (stdout'u kirletmemek için)
///
/// Bu fonksiyon programın başında bir kez çağrılmalıdır.
/// İkinci çağrıda set_global_default hatası oluşur ama sessizce yutulur.
pub fn init_logging() {
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("morpharch=info"));

    fmt()
        .with_env_filter(filter)
        .with_target(false) // "morpharch::db" gibi hedef bilgisini gizle
        .without_time() // CLI aracı için zaman damgası gereksiz
        .init();
}

/// Anyhow hata zincirini kullanıcı dostu formatta stderr'e yazdırır.
///
/// Hata mesajını ve tüm bağlam katmanlarını (context chain) numaralı
/// liste halinde gösterir. Bu sayede kullanıcı hatanın kaynağını
/// kolayca takip edebilir.
///
/// # Çıktı Formatı
/// ```text
/// ❌ Hata: Commit veritabanına yazılamadı
///    1: SQLite UNIQUE constraint hatası
///    2: disk dolu
/// ```
pub fn print_error(err: &Error) {
    eprintln!("\n❌ Hata: {err}");
    for (i, cause) in err.chain().skip(1).enumerate() {
        eprintln!("   {}: {cause}", i + 1);
    }
    eprintln!();
}
