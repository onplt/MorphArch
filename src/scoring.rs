// =============================================================================
// scoring.rs — Architecture Drift Score hesaplama motoru
// =============================================================================
//
// Sprint 3'ün temel modülü. Her commit için dependency graph'ın mimari
// "sağlığını" ölçer ve 0-100 arasında bir skor üretir.
//
// Metrikler:
//   1. Fan-in / Fan-out değişimi (her node'un gelen/giden kenar sayısı)
//   2. Döngüsel bağımlılık sayısı (petgraph SCC analizi)
//   3. Boundary violation (paket sınır ihlalleri: apps/ ↔ packages/)
//   4. Cognitive complexity proxy = (edges/nodes)*10 + cycles*5
//   5. Toplam skor = normalize(0-100)
//
// Temporal analiz:
//   compare_graphs() iki ardışık commit'in graph'ını karşılaştırır,
//   TemporalDelta üretir.
//
// Determinizm:
//   Tüm hesaplamalar aynı girdi ile aynı çıktıyı verir.
//   Floating-point yuvarlama tutarlı olması için round() kullanılır.
// =============================================================================

use petgraph::algo::kosaraju_scc;
use petgraph::graph::DiGraph;
use std::collections::HashSet;
use tracing::debug;

use crate::models::{DriftScore, TemporalDelta};

/// Baseline drift skoru — önceki graf yoksa bu değer kullanılır.
const BASELINE_SCORE: u8 = 50;

/// Boundary violation kuralları: bu prefix çiftleri arası bağımlılık ihlal sayılır.
///
/// Genel monorepo konvansiyonu:
///   - `apps/` → uygulamalar (son kullanıcıya açık)
///   - `packages/` → paylaşılan kütüphaneler
///
/// İhlal: `packages/` → `apps/` yönünde bağımlılık (kütüphane, uygulamaya bağımlı olmamalı)
pub const BOUNDARY_RULES: &[(&str, &str)] = &[
    ("packages::", "apps::"), // Kütüphane → uygulama (yasak yön)
    ("lib::", "apps::"),      // lib → apps (yasak yön)
    ("core::", "apps::"),     // core → apps (yasak yön)
    ("shared::", "apps::"),   // shared → apps (yasak yön)
    ("packages::", "cmd::"),  // packages → cmd (yasak yön)
    ("lib::", "cmd::"),       // lib → cmd (yasak yön)
];

/// Bir dependency graph için mimari drift skoru hesaplar.
///
/// Önceki commit'in graf'ı verilmişse delta analizi yapılır;
/// yoksa baseline (50) skoru ile mutlak metrikler hesaplanır.
///
/// # Parametreler
/// - `graph`: Mevcut commit'in bağımlılık grafi
/// - `prev_graph`: Bir önceki commit'in grafi (ilk commit için `None`)
/// - `nodes`: Mevcut graftaki modül adları (boundary kontrolü için)
/// - `edges_raw`: Ham kenar listesi (from_module, to_module çiftleri)
/// - `timestamp`: Commit'in zaman damgası
///
/// # Dönüş
/// `DriftScore` — toplam skor (0-100) ve alt metrikler
///
/// # Algoritma
/// 1. Fan-in/fan-out hesapla, önceki graf varsa delta bul
/// 2. SCC ile döngü sayısını hesapla
/// 3. Boundary violation kontrolü yap
/// 4. Cognitive complexity proxy hesapla
/// 5. Normalize et ve [0, 100] aralığına clamp'le
pub fn calculate_drift(
    graph: &DiGraph<String, ()>,
    prev_graph: Option<&DiGraph<String, ()>>,
    _nodes: &[String],
    edges_raw: &[(String, String)],
    timestamp: i64,
) -> DriftScore {
    let node_count = graph.node_count();
    let edge_count = graph.edge_count();

    // ── 1. Fan-in / Fan-out hesapla ──
    let (current_fan_in, current_fan_out) = compute_fan_metrics(graph);

    let (fan_in_delta, fan_out_delta) = if let Some(prev) = prev_graph {
        let (prev_fan_in, prev_fan_out) = compute_fan_metrics(prev);
        (
            current_fan_in as i32 - prev_fan_in as i32,
            current_fan_out as i32 - prev_fan_out as i32,
        )
    } else {
        // İlk commit — delta yok
        (0, 0)
    };

    // ── 2. Döngüsel bağımlılık sayısı (SCC analizi) ──
    let current_cycles = count_cycles(graph);
    let prev_cycles = prev_graph.map_or(0, count_cycles);
    let new_cycles = current_cycles.saturating_sub(prev_cycles);

    // ── 3. Boundary violation kontrolü ──
    let boundary_violations = count_boundary_violations(edges_raw);

    // ── 4. Cognitive complexity proxy ──
    let complexity = if node_count > 0 {
        (edge_count as f64 / node_count as f64) * 10.0 + current_cycles as f64 * 5.0
    } else {
        0.0
    };

    // ── 5. Toplam skor hesapla ve normalize et ──
    let total = if prev_graph.is_some() {
        compute_total_score(
            fan_in_delta,
            fan_out_delta,
            new_cycles,
            boundary_violations,
            complexity,
        )
    } else {
        // İlk commit — baseline skor
        BASELINE_SCORE
    };

    debug!(
        total,
        fan_in_delta,
        fan_out_delta,
        new_cycles,
        boundary_violations,
        complexity,
        "Drift skoru hesaplandı"
    );

    DriftScore {
        total,
        fan_in_delta,
        fan_out_delta,
        new_cycles,
        boundary_violations,
        cognitive_complexity: (complexity * 100.0).round() / 100.0,
        timestamp,
    }
}

/// İki ardışık commit'in graf'ını karşılaştırarak temporal delta hesaplar.
///
/// Hangi modüllerin eklendiği/kaldırıldığı, yeni/çözülen döngüler ve
/// drift skoru değişimini raporlar.
///
/// # Parametreler
/// - `current_graph`: Mevcut commit'in grafi
/// - `prev_graph`: Önceki commit'in grafi
/// - `current_nodes`: Mevcut graftaki modül adları seti
/// - `prev_nodes`: Önceki graftaki modül adları seti
/// - `current_edges`: Mevcut kenar sayısı
/// - `prev_edges`: Önceki kenar sayısı
/// - `current_score`: Mevcut drift skoru
/// - `prev_score`: Önceki drift skoru
/// - `current_hash`: Mevcut commit hash'i
/// - `prev_hash`: Önceki commit hash'i
///
/// # Dönüş
/// `TemporalDelta` — iki commit arası tüm değişimlerin özeti
#[allow(dead_code, clippy::too_many_arguments)] // Gelecek sprintlerde detaylı temporal analiz için kullanılacak
pub fn compare_graphs(
    current_graph: &DiGraph<String, ()>,
    prev_graph: &DiGraph<String, ()>,
    current_nodes: &HashSet<String>,
    prev_nodes: &HashSet<String>,
    current_edges: usize,
    prev_edges: usize,
    current_score: u8,
    prev_score: u8,
    current_hash: &str,
    prev_hash: &str,
) -> TemporalDelta {
    let nodes_added = current_nodes.difference(prev_nodes).count();
    let nodes_removed = prev_nodes.difference(current_nodes).count();

    let edges_added = current_edges.saturating_sub(prev_edges);
    let edges_removed = prev_edges.saturating_sub(current_edges);

    let current_cycles = count_cycles(current_graph);
    let prev_cycles = count_cycles(prev_graph);

    let new_cycles = current_cycles.saturating_sub(prev_cycles);
    let resolved_cycles = prev_cycles.saturating_sub(current_cycles);

    let score_delta = current_score as i32 - prev_score as i32;

    TemporalDelta {
        prev_commit_hash: prev_hash.to_string(),
        current_commit_hash: current_hash.to_string(),
        score_delta,
        nodes_added,
        nodes_removed,
        edges_added,
        edges_removed,
        new_cycles,
        resolved_cycles,
    }
}

// =============================================================================
// Yardımcı fonksiyonlar — dahili kullanım
// =============================================================================

/// Her düğüm için fan-in (gelen kenar) ve fan-out (giden kenar) sayılarının
/// toplamını hesaplar.
///
/// # Dönüş
/// `(total_fan_in, total_fan_out)` — tüm düğümlerin toplam gelen/giden kenar sayıları
fn compute_fan_metrics(graph: &DiGraph<String, ()>) -> (usize, usize) {
    let mut total_fan_in: usize = 0;
    let mut total_fan_out: usize = 0;

    for node_idx in graph.node_indices() {
        // Fan-in: bu düğüme gelen kenar sayısı
        total_fan_in += graph
            .neighbors_directed(node_idx, petgraph::Direction::Incoming)
            .count();
        // Fan-out: bu düğümden giden kenar sayısı
        total_fan_out += graph
            .neighbors_directed(node_idx, petgraph::Direction::Outgoing)
            .count();
    }

    (total_fan_in, total_fan_out)
}

/// Graftaki döngüsel bağımlılık sayısını hesaplar.
///
/// Kosaraju'nun SCC (Strongly Connected Components) algoritmasını kullanır.
/// Tek elemanlı SCC'ler döngü değildir; 2+ elemanlı SCC'ler döngüdür.
///
/// # Dönüş
/// Döngüsel bağımlılık grubu sayısı (2+ elemanlı SCC sayısı)
fn count_cycles(graph: &DiGraph<String, ()>) -> usize {
    let sccs = kosaraju_scc(graph);
    sccs.iter().filter(|scc| scc.len() > 1).count()
}

/// Paket sınırı ihlallerini sayar.
///
/// Monorepo konvansiyonuna göre: kütüphane katmanı (packages/, lib/, core/)
/// uygulama katmanına (apps/, cmd/) bağımlı olmamalıdır.
///
/// # Parametreler
/// - `edges`: `(from_module, to_module)` çiftleri
///
/// # Dönüş
/// İhlal eden kenar sayısı
fn count_boundary_violations(edges: &[(String, String)]) -> usize {
    edges
        .iter()
        .filter(|(from, to)| {
            BOUNDARY_RULES.iter().any(|(from_prefix, to_prefix)| {
                from.starts_with(from_prefix) && to.starts_with(to_prefix)
            })
        })
        .count()
}

/// Alt metriklerden toplam drift skorunu hesaplar ve [0, 100] aralığına normalize eder.
///
/// # Formül
/// ```text
/// fan_component   = |fan_in_delta| + |fan_out_delta|  (max katkı: ~30)
/// cycle_penalty   = new_cycles * 15                   (her yeni döngü +15)
/// boundary_penalty= violations * 10                   (her ihlal +10)
/// complexity_comp = complexity * 1.5                   (karmaşıklık ağırlığı)
/// raw_score       = baseline(50) + fan_comp/2 + cycle_pen + boundary_pen + complexity_comp/3
/// ```
///
/// # Determinizm
/// Aynı girdiler her zaman aynı skoru üretir.
fn compute_total_score(
    fan_in_delta: i32,
    fan_out_delta: i32,
    new_cycles: usize,
    boundary_violations: usize,
    cognitive_complexity: f64,
) -> u8 {
    let fan_component = (fan_in_delta.unsigned_abs() + fan_out_delta.unsigned_abs()) as f64;
    let cycle_penalty = new_cycles as f64 * 15.0;
    let boundary_penalty = boundary_violations as f64 * 10.0;
    let complexity_component = cognitive_complexity * 1.5;

    // Baseline'dan başla, ceza ekle
    let raw = BASELINE_SCORE as f64
        + fan_component / 2.0
        + cycle_penalty
        + boundary_penalty
        + complexity_component / 3.0;

    // [0, 100] aralığına clamp'le
    raw.round().clamp(0.0, 100.0) as u8
}

/// Graftaki döngüsel bağımlılık sayısını public olarak döndürür.
///
/// `commands::analyze` modülü tarafından kullanılır.
pub fn count_cycles_public(graph: &DiGraph<String, ()>) -> usize {
    count_cycles(graph)
}

/// Kenar listesinden (from, to) çiftlerini oluşturur.
///
/// `commands::scan` modülünden çağrılır. DependencyEdge vektöründen
/// sadece modül adlarını içeren tuple vektörü üretir.
pub fn edges_to_pairs(edges: &[crate::models::DependencyEdge]) -> Vec<(String, String)> {
    edges
        .iter()
        .map(|e| (e.from_module.clone(), e.to_module.clone()))
        .collect()
}

// =============================================================================
// Testler
// =============================================================================
#[cfg(test)]
mod tests {
    use super::*;

    /// Test yardımcısı: basit bir DiGraph oluşturur.
    ///
    /// Düğümler: A, B, C
    /// Kenarlar: A→B, A→C, B→C
    fn make_simple_graph() -> DiGraph<String, ()> {
        let mut g = DiGraph::new();
        let a = g.add_node("A".to_string());
        let b = g.add_node("B".to_string());
        let c = g.add_node("C".to_string());
        g.add_edge(a, b, ());
        g.add_edge(a, c, ());
        g.add_edge(b, c, ());
        g
    }

    /// Test yardımcısı: döngüsel bir DiGraph oluşturur.
    ///
    /// Düğümler: A, B, C
    /// Kenarlar: A→B, B→C, C→A (tam döngü)
    fn make_cyclic_graph() -> DiGraph<String, ()> {
        let mut g = DiGraph::new();
        let a = g.add_node("A".to_string());
        let b = g.add_node("B".to_string());
        let c = g.add_node("C".to_string());
        g.add_edge(a, b, ());
        g.add_edge(b, c, ());
        g.add_edge(c, a, ());
        g
    }

    #[test]
    fn test_calculate_drift_baseline() {
        // İlk commit — önceki graf yok → baseline 50 olmalı
        let graph = make_simple_graph();
        let nodes = vec!["A".to_string(), "B".to_string(), "C".to_string()];
        let edges = vec![
            ("A".to_string(), "B".to_string()),
            ("A".to_string(), "C".to_string()),
            ("B".to_string(), "C".to_string()),
        ];

        let score = calculate_drift(&graph, None, &nodes, &edges, 1_000_000);

        assert_eq!(score.total, BASELINE_SCORE, "İlk commit baseline olmalı");
        assert_eq!(score.fan_in_delta, 0, "Delta yoksa fan_in_delta 0 olmalı");
        assert_eq!(score.fan_out_delta, 0, "Delta yoksa fan_out_delta 0 olmalı");
        assert_eq!(score.new_cycles, 0, "Basit grafta döngü olmamalı");
    }

    #[test]
    fn test_calculate_drift_with_previous() {
        // Önceki graf: basit, mevcut graf: döngüsel → skor artmalı
        let prev = make_simple_graph();
        let current = make_cyclic_graph();
        let nodes = vec!["A".to_string(), "B".to_string(), "C".to_string()];
        let edges = vec![
            ("A".to_string(), "B".to_string()),
            ("B".to_string(), "C".to_string()),
            ("C".to_string(), "A".to_string()),
        ];

        let score = calculate_drift(&current, Some(&prev), &nodes, &edges, 2_000_000);

        assert!(
            score.total > BASELINE_SCORE,
            "Döngü eklenince skor artmalı, ama {} geldi",
            score.total
        );
        assert_eq!(score.new_cycles, 1, "Bir yeni döngü eklendi");
    }

    #[test]
    fn test_cycle_detection() {
        let simple = make_simple_graph();
        assert_eq!(count_cycles(&simple), 0, "Basit grafta döngü yok");

        let cyclic = make_cyclic_graph();
        assert_eq!(count_cycles(&cyclic), 1, "Tam döngülü grafta 1 SCC");

        let empty: DiGraph<String, ()> = DiGraph::new();
        assert_eq!(count_cycles(&empty), 0, "Boş grafta döngü yok");
    }

    #[test]
    fn test_boundary_violations() {
        let edges = vec![
            // İhlal: packages → apps
            (
                "packages::ui::button".to_string(),
                "apps::web::home".to_string(),
            ),
            // Normal: apps → packages (izin verilen yön)
            (
                "apps::web::home".to_string(),
                "packages::ui::button".to_string(),
            ),
            // İhlal: lib → apps
            ("lib::utils".to_string(), "apps::api::routes".to_string()),
            // Normal: dahili referans
            (
                "packages::ui::button".to_string(),
                "packages::ui::theme".to_string(),
            ),
        ];

        let violations = count_boundary_violations(&edges);
        assert_eq!(violations, 2, "2 ihlal olmalı (packages→apps, lib→apps)");
    }

    #[test]
    fn test_fan_metrics() {
        let graph = make_simple_graph();
        let (fan_in, fan_out) = compute_fan_metrics(&graph);

        // A→B, A→C, B→C
        // Fan-in: A=0, B=1, C=2 → total=3
        // Fan-out: A=2, B=1, C=0 → total=3
        assert_eq!(fan_in, 3, "Toplam fan-in 3 olmalı");
        assert_eq!(fan_out, 3, "Toplam fan-out 3 olmalı");
    }

    #[test]
    fn test_compare_graphs_temporal() {
        let prev = make_simple_graph();
        let current = make_cyclic_graph();

        let prev_nodes: HashSet<String> = ["A", "B", "C"].iter().map(|s| s.to_string()).collect();
        let current_nodes: HashSet<String> =
            ["A", "B", "C", "D"].iter().map(|s| s.to_string()).collect();

        let delta = compare_graphs(
            &current,
            &prev,
            &current_nodes,
            &prev_nodes,
            3, // current edges
            3, // prev edges
            65,
            50,
            "commit2",
            "commit1",
        );

        assert_eq!(delta.score_delta, 15, "Skor farkı 65-50=15 olmalı");
        assert_eq!(delta.nodes_added, 1, "D eklendi → 1 yeni düğüm");
        assert_eq!(delta.nodes_removed, 0, "Hiç düğüm kaldırılmadı");
        assert_eq!(delta.new_cycles, 1, "1 yeni döngü");
        assert_eq!(delta.resolved_cycles, 0, "Çözülen döngü yok");
    }

    #[test]
    fn test_compute_total_score_deterministic() {
        // Aynı girdiler her zaman aynı çıktıyı vermeli
        let s1 = compute_total_score(5, 3, 1, 2, 15.0);
        let s2 = compute_total_score(5, 3, 1, 2, 15.0);
        assert_eq!(s1, s2, "Deterministic olmalı");

        // Skor [0, 100] aralığında olmalı
        assert!(s1 <= 100, "Skor 100'den büyük olamaz");

        // Çok büyük değerler 100'e clamp'lenmeli
        let extreme = compute_total_score(100, 100, 10, 20, 500.0);
        assert_eq!(extreme, 100, "Aşırı değerler 100'e clamp'lenmeli");
    }

    #[test]
    fn test_empty_graph_drift() {
        let empty: DiGraph<String, ()> = DiGraph::new();
        let nodes: Vec<String> = vec![];
        let edges: Vec<(String, String)> = vec![];

        let score = calculate_drift(&empty, None, &nodes, &edges, 0);
        assert_eq!(score.total, BASELINE_SCORE, "Boş graf baseline olmalı");
        assert_eq!(score.cognitive_complexity, 0.0, "Boş grafta karmaşıklık 0");
    }
}
