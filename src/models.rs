// =============================================================================
// models.rs — MorphArch veri modelleri
// =============================================================================
//
// Uygulama genelinde kullanılan temel veri yapıları:
//
//   CommitInfo       → Git commit metadata'sı (hash, yazar, mesaj, zaman, tree)
//   DependencyEdge   → İki modül arasındaki bağımlılık kenarı
//   GraphSnapshot    → Belirli bir commit'teki tam bağımlılık grafi
//   DriftScore       → Mimari drift skoru (0-100) ve alt metrikler (Sprint 3)
//   TemporalDelta    → İki commit arası drift karşılaştırması (Sprint 3)
//
// Tüm struct'lar Serialize/Deserialize destekler (JSON depolama + gelecek API).
// =============================================================================

use serde::{Deserialize, Serialize};

/// Tek bir Git commit'inin metadata bilgisi
///
/// Git deposundan okunan her commit için bu struct doldurulur ve
/// SQLite veritabanına kaydedilir.
///
/// # Alanlar
/// - `hash`: Commit'in tam SHA-1 hash'i (40 karakter hex)
/// - `author_name`: Commit'i yapan kişinin adı
/// - `author_email`: Commit'i yapan kişinin e-posta adresi
/// - `message`: Commit mesajı (ilk satır + gövde)
/// - `timestamp`: Unix epoch'tan bu yana saniye cinsinden zaman damgası
/// - `tree_id`: Bu commit'in işaret ettiği tree nesnesinin hash'i
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommitInfo {
    pub hash: String,
    pub author_name: String,
    pub author_email: String,
    pub message: String,
    pub timestamp: i64,
    pub tree_id: String,
}

/// İki modül/paket arasındaki bağımlılık kenarı
///
/// Kaynak dosyadaki bir import/use/require ifadesinden çıkarılır.
///
/// # Alanlar
/// - `from_module`: Bağımlılığı içeren kaynak modül (ör. "src/main")
/// - `to_module`: Bağımlı olunan hedef modül/paket (ör. "serde")
/// - `file_path`: Import'un bulunduğu dosya yolu (ör. "src/main.rs")
/// - `line`: Import ifadesinin satır numarası (0 = bilinmiyor)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DependencyEdge {
    pub from_module: String,
    pub to_module: String,
    pub file_path: String,
    pub line: usize,
}

/// Belirli bir commit anındaki tam bağımlılık grafi
///
/// Bir commit'in tree'si taranarak elde edilen tüm modüller (düğümler)
/// ve aralarındaki import ilişkileri (kenarlar) bu struct'ta saklanır.
/// JSON olarak serileştirilip SQLite'a kaydedilir.
///
/// # Alanlar
/// - `commit_hash`: İlişkili commit'in SHA hash'i
/// - `nodes`: Graftaki benzersiz modül/paket adları
/// - `edges`: Modüller arası bağımlılık kenarları
/// - `node_count`: Toplam düğüm sayısı (hızlı erişim için denormalize)
/// - `edge_count`: Toplam kenar sayısı (hızlı erişim için denormalize)
/// - `timestamp`: Commit'in zaman damgası (sıralama kolaylığı için)
/// - `drift`: Bu commit için hesaplanmış drift skoru (Sprint 3, None = henüz hesaplanmadı)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphSnapshot {
    pub commit_hash: String,
    pub nodes: Vec<String>,
    pub edges: Vec<DependencyEdge>,
    pub node_count: usize,
    pub edge_count: usize,
    pub timestamp: i64,
    /// Sprint 3: Mimari drift skoru. None ise henüz hesaplanmadı.
    #[serde(default)]
    pub drift: Option<DriftScore>,
}

/// Mimari drift skoru — bir commit'teki grafın "sağlık" ölçümü (0-100)
///
/// Skor 0 = mükemmel mimari, 100 = tamamen kaotik.
/// Baseline (ilk commit veya önceki graf yoksa) = 50.
///
/// # Alt Metrikler
/// - `fan_in_delta`: Ortalama fan-in değişimi (önceki commit'e göre)
/// - `fan_out_delta`: Ortalama fan-out değişimi (önceki commit'e göre)
/// - `new_cycles`: Bu commit'te tespit edilen yeni döngüsel bağımlılık sayısı
/// - `boundary_violations`: Paket sınırı ihlali sayısı (ör. apps/ → packages/ arası)
/// - `cognitive_complexity`: Bilişsel karmaşıklık proxy'si
/// - `timestamp`: Skor hesaplama anının zaman damgası
///
/// # Formül
/// ```text
/// raw = fan_delta_component + cycle_penalty + boundary_penalty + complexity_component
/// total = clamp(normalize(raw), 0, 100)
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DriftScore {
    /// Toplam drift skoru (0 = temiz, 100 = kaotik)
    pub total: u8,
    /// Ortalama fan-in değişimi (pozitif = artan bağımlılık)
    pub fan_in_delta: i32,
    /// Ortalama fan-out değişimi (pozitif = artan dış bağımlılık)
    pub fan_out_delta: i32,
    /// Yeni döngüsel bağımlılık sayısı
    pub new_cycles: usize,
    /// Paket sınırı ihlali sayısı
    pub boundary_violations: usize,
    /// Bilişsel karmaşıklık proxy'si: (edges/nodes)*10 + cycles*5
    pub cognitive_complexity: f64,
    /// Skor hesaplama zaman damgası
    pub timestamp: i64,
}

/// İki ardışık commit arasındaki drift karşılaştırması
///
/// `compare_graphs` fonksiyonu tarafından üretilir. Temporal analiz
/// yaparak bir commit'in mimariyi ne yönde değiştirdiğini gösterir.
///
/// # Alanlar
/// - `prev_commit_hash`: Karşılaştırılan önceki commit'in hash'i
/// - `current_commit_hash`: Mevcut commit'in hash'i
/// - `score_delta`: Drift skoru değişimi (pozitif = kötüleşme)
/// - `nodes_added`: Yeni eklenen modül sayısı
/// - `nodes_removed`: Kaldırılan modül sayısı
/// - `edges_added`: Yeni eklenen bağımlılık sayısı
/// - `edges_removed`: Kaldırılan bağımlılık sayısı
/// - `new_cycles`: Yeni ortaya çıkan döngü sayısı
/// - `resolved_cycles`: Çözülen döngü sayısı
#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(dead_code)] // Kullanım: scoring::compare_graphs — gelecek sprintlerde CLI'dan çağrılacak
pub struct TemporalDelta {
    pub prev_commit_hash: String,
    pub current_commit_hash: String,
    pub score_delta: i32,
    pub nodes_added: usize,
    pub nodes_removed: usize,
    pub edges_added: usize,
    pub edges_removed: usize,
    pub new_cycles: usize,
    pub resolved_cycles: usize,
}

/// Belirli bir commit anındaki paket/modül durumu
///
/// Sprint 3+ için ayrılmış. Her commit için monorepo'daki paketlerin
/// versiyonlarını ve bağımlılıklarını tutar.
#[allow(dead_code)] // Sonraki sprintlerde aktif olacak
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageSnapshot {
    pub commit_hash: String,
    pub package_name: String,
    pub version: String,
    pub dependencies: Vec<String>,
}
