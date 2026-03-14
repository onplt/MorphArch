//! Dependency graph construction with petgraph.
//!
//! Builds a directed graph (`DiGraph<String, u32>`) from parsed dependency edges.
//! Nodes are module/package names; edges represent import relationships with weights.
//! Edge weights reflect coupling intensity (number of import statements).
//! Self-edges and duplicate edges are automatically filtered (weights are summed).

use petgraph::graph::{DiGraph, NodeIndex};
use std::collections::{HashMap, HashSet};

use crate::models::DependencyEdge;

/// Builds a directed dependency graph from node and edge lists.
///
/// # Parameters
/// - `nodes`: Set of unique module/package names
/// - `edges`: Dependency edges between modules
///
/// # Returns
/// petgraph `DiGraph<String, u32>` — nodes are package names, edges are weighted dependencies.
/// Edge weight = number of import statements for that (from → to) pair.
///
/// # Behavior
/// - One node is created for each unique package name
/// - `from_module` and `to_module` from edges are used directly as node labels
///   (they should already contain clean package names from scan.rs)
/// - Duplicate edges are merged: their weights are summed
pub fn build_graph(_nodes: &HashSet<String>, edges: &[DependencyEdge]) -> DiGraph<String, u32> {
    let mut graph = DiGraph::new();
    let mut node_indices: HashMap<String, NodeIndex> = HashMap::new();

    let mut ordered_nodes: Vec<_> = _nodes.iter().cloned().collect();
    ordered_nodes.sort();
    for node in ordered_nodes {
        node_indices.insert(node.clone(), graph.add_node(node));
    }

    // Process edges and dynamically add nodes (deduplicated)
    for edge in edges {
        // Both from_module and to_module are already clean package names.
        // scan.rs::collect_edges() normalizes path-like imports via
        // extract_package_name(), so we use them directly here.
        let from_pkg = edge.from_module.clone();
        let to_pkg = edge.to_module.clone();

        // Skip self-dependencies
        if from_pkg == to_pkg {
            continue;
        }

        let from_idx = *node_indices
            .entry(from_pkg.clone())
            .or_insert_with(|| graph.add_node(from_pkg));
        let to_idx = *node_indices
            .entry(to_pkg.clone())
            .or_insert_with(|| graph.add_node(to_pkg));

        // Merge duplicate edges by summing weights
        if let Some(existing) = graph.find_edge(from_idx, to_idx) {
            let w = graph[existing];
            graph[existing] = w + edge.weight;
        } else {
            graph.add_edge(from_idx, to_idx, edge.weight);
        }
    }

    graph
}

// =============================================================================
// Tests
// =============================================================================
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_graph_basic() {
        let nodes = HashSet::new();

        let edges = vec![
            DependencyEdge {
                from_module: "main".to_string(),
                to_module: "serde".to_string(),
                file_path: "src/main.rs".to_string(),
                line: Some(1),
                weight: 1,
                sample_origins: Vec::new(),
            },
            DependencyEdge {
                from_module: "main".to_string(),
                to_module: "std".to_string(),
                file_path: "src/main.rs".to_string(),
                line: Some(2),
                weight: 1,
                sample_origins: Vec::new(),
            },
        ];

        let graph = build_graph(&nodes, &edges);

        // "main" → "serde", "main" → "std"
        assert_eq!(
            graph.node_count(),
            3,
            "should have 3 nodes (main, serde, std)"
        );
        assert_eq!(graph.edge_count(), 2, "should have 2 edges");
    }

    #[test]
    fn test_build_graph_deduplication_and_weight_merge() {
        let nodes = HashSet::new();
        let edges = vec![
            DependencyEdge {
                from_module: "web".to_string(),
                to_module: "core".to_string(),
                file_path: "apps/web/src/app.ts".to_string(),
                line: Some(1),
                weight: 2,
                sample_origins: Vec::new(),
            },
            DependencyEdge {
                from_module: "web".to_string(),
                to_module: "core".to_string(),
                file_path: "apps/web/src/index.ts".to_string(),
                line: Some(1),
                weight: 3,
                sample_origins: Vec::new(),
            },
        ];

        let graph = build_graph(&nodes, &edges);

        assert_eq!(graph.node_count(), 2, "should have 2 nodes (web, core)");
        assert_eq!(
            graph.edge_count(),
            1,
            "same package pair should have one edge"
        );
        // Weight should be summed: 2 + 3 = 5
        let web_idx = graph.node_indices().find(|&n| graph[n] == "web").unwrap();
        let core_idx = graph.node_indices().find(|&n| graph[n] == "core").unwrap();
        let edge = graph.find_edge(web_idx, core_idx).unwrap();
        assert_eq!(graph[edge], 5, "duplicate edge weights should be summed");
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
    fn test_build_graph_preserves_isolated_nodes() {
        let nodes = HashSet::from(["isolated".to_string(), "connected".to_string()]);
        let edges = vec![];

        let graph = build_graph(&nodes, &edges);

        assert_eq!(graph.node_count(), 2);
        assert_eq!(graph.edge_count(), 0);
        assert!(graph.node_indices().any(|idx| graph[idx] == "isolated"));
    }
}
