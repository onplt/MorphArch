// =============================================================================
// graph_builder.rs — petgraph ile bağımlılık grafi oluşturma
// =============================================================================
//
// Sorumluluklar:
//   1. Parser'dan gelen düğüm (modül) ve kenar (bağımlılık) listesinden
//      yönlü graf (DiGraph) oluşturma
//   2. Düğüm deduplication — aynı modül adı tek düğüm olur
//   3. Kenar ekleme — from → to yönünde
//   4. Graf istatistiklerini hesaplama (node_count, edge_count)
//
// petgraph::graph::DiGraph kullanılır:
//   - Düğüm ağırlığı: String (modül adı)
//   - Kenar ağırlığı: () (sadece bağlantı bilgisi, ağırlıksız)
// =============================================================================

use petgraph::graph::{DiGraph, NodeIndex};
use std::collections::{HashMap, HashSet};

use crate::models::DependencyEdge;

/// Parser'dan gelen düğüm ve kenar listesinden yönlü bağımlılık grafi oluşturur.
///
/// # Parametreler
/// - `nodes`: Benzersiz modül/paket adları seti
/// - `edges`: Modüller arası bağımlılık kenarları
///
/// # Dönüş
/// petgraph `DiGraph<String, ()>` — düğümler modül adları, kenarlar bağımlılıklar.
///
/// # Davranış
/// - Her benzersiz modül adı için tek düğüm oluşturulur
/// - Aynı (from, to) çifti için birden fazla kenar eklenebilir
///   (farklı dosyalardan aynı modüle bağımlılık)
/// - edges'teki modül adı nodes'ta yoksa sessizce atlanır
pub fn build_graph(nodes: &HashSet<String>, edges: &[DependencyEdge]) -> DiGraph<String, ()> {
    let mut graph = DiGraph::new();
    let mut node_indices: HashMap<&str, NodeIndex> = HashMap::new();

    // Düğümleri ekle — her modül adı için benzersiz bir NodeIndex
    for node_name in nodes {
        let idx = graph.add_node(node_name.clone());
        node_indices.insert(node_name, idx);
    }

    // Kenarları ekle — from → to yönünde
    for edge in edges {
        if let (Some(&from_idx), Some(&to_idx)) = (
            node_indices.get(edge.from_module.as_str()),
            node_indices.get(edge.to_module.as_str()),
        ) {
            graph.add_edge(from_idx, to_idx, ());
        }
    }

    graph
}

// =============================================================================
// Testler
// =============================================================================
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_graph_basic() {
        let mut nodes = HashSet::new();
        nodes.insert("main".to_string());
        nodes.insert("serde".to_string());
        nodes.insert("std".to_string());

        let edges = vec![
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
        ];

        let graph = build_graph(&nodes, &edges);

        assert_eq!(graph.node_count(), 3, "3 düğüm olmalı");
        assert_eq!(graph.edge_count(), 2, "2 kenar olmalı");
    }

    #[test]
    fn test_build_graph_empty() {
        let nodes = HashSet::new();
        let edges: Vec<DependencyEdge> = vec![];

        let graph = build_graph(&nodes, &edges);

        assert_eq!(graph.node_count(), 0);
        assert_eq!(graph.edge_count(), 0);
    }

    #[test]
    fn test_build_graph_missing_node_edge_skipped() {
        let mut nodes = HashSet::new();
        nodes.insert("main".to_string());
        // "serde" düğümü yok — kenar atlanmalı

        let edges = vec![DependencyEdge {
            from_module: "main".to_string(),
            to_module: "serde".to_string(),
            file_path: "src/main.rs".to_string(),
            line: 1,
        }];

        let graph = build_graph(&nodes, &edges);

        assert_eq!(graph.node_count(), 1, "Sadece 'main' düğümü olmalı");
        assert_eq!(graph.edge_count(), 0, "Hedef düğüm yok — kenar olmamalı");
    }
}
