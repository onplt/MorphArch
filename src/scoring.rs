//! Absolute Architecture Health scoring engine.
//!
//! Computes a 0-100 "Debt" score based on structural and maintenance metrics.
//! Health = 100 - Debt.
//!
//! # Scientific Philosophy
//! 1. **Correctness First**: Cycles and boundary violations are major architectural
//!    flaws that trigger heavy penalties.
//! 2. **Contextual Complexity**: Large projects are naturally dense. We use a
//!    forgiving density threshold (3.5) to avoid penalizing necessary complexity.
//! 3. **Scale Grace**: Small projects are exempt from density penalties.

use petgraph::algo::kosaraju_scc;
use petgraph::graph::DiGraph;
use std::collections::HashSet;
use tracing::debug;

use crate::models::{DriftScore, TemporalDelta};

/// Boundary violation rules: dependencies between these prefix pairs are violations.
pub const BOUNDARY_RULES: &[(&str, &str)] = &[
    ("packages::", "apps::"),
    ("lib::", "apps::"),
    ("core::", "apps::"),
    ("shared::", "apps::"),
    ("packages::", "cmd::"),
    ("lib::", "cmd::"),
    ("packages/", "apps/"),
    ("libs/", "apps/"),
    ("libs/", "cli/"),
    ("ext/", "cli/"),
    ("runtime/", "cli/"),
    ("libs/", "runtime/"),
];

/// Computes per-node instability index (I = Ce / (Ca + Ce)).
pub fn compute_instability_metrics(graph: &DiGraph<String, ()>) -> Vec<(String, f64)> {
    let mut metrics = Vec::new();
    for node_idx in graph.node_indices() {
        let ca = graph
            .neighbors_directed(node_idx, petgraph::Direction::Incoming)
            .count();
        let ce = graph
            .neighbors_directed(node_idx, petgraph::Direction::Outgoing)
            .count();
        let instability = if ca + ce > 0 {
            ce as f64 / (ca + ce) as f64
        } else {
            0.0
        };
        metrics.push((graph[node_idx].clone(), instability));
    }
    metrics.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    metrics
}

/// Calculates the absolute architectural debt score (0-100).
/// 0 debt = 100% Health.
pub fn calculate_drift(
    graph: &DiGraph<String, ()>,
    prev_graph: Option<&DiGraph<String, ()>>,
    _nodes: &[String],
    edges_raw: &[(String, String)],
    timestamp: i64,
) -> DriftScore {
    let node_count = graph.node_count();
    let edge_count = graph.edge_count();

    // ── 1. Contextual Density Analysis ──
    let density = if node_count > 0 {
        edge_count as f64 / node_count as f64
    } else {
        0.0
    };

    // Threshold of 3.5 is chosen as a healthy upper bound for complex systems.
    // Above this, every 1.0 unit of density adds 5 points of architectural debt.
    let density_debt = if node_count < 10 {
        0.0
    } else {
        (density - 3.5).max(0.0) * 5.0
    };

    // ── 2. Structural Debt (Fatal Flaws) ──
    let cycles = count_cycles(graph);
    let violations = count_boundary_violations(edges_raw);

    // Cycles are catastrophic for modularity. -25 health points per cycle group.
    let cycle_debt = cycles as f64 * 25.0;

    // Violations break layer contracts. -15 health points per unique violation.
    let violation_debt = violations as f64 * 15.0;

    // ── 3. Final Aggregation ──
    let total_debt = (cycle_debt + violation_debt + density_debt)
        .round()
        .min(100.0) as u8;

    // ── 4. Delta Metrics (for timeline visualization) ──
    let (current_fan_in, current_fan_out) = compute_fan_metrics(graph);
    let (fan_in_delta, fan_out_delta) = if let Some(prev) = prev_graph {
        let (prev_fan_in, prev_fan_out) = compute_fan_metrics(prev);
        (
            current_fan_in as i32 - prev_fan_in as i32,
            current_fan_out as i32 - prev_fan_out as i32,
        )
    } else {
        (0, 0)
    };

    debug!(
        total_debt,
        cycles, violations, density, "Architectural Health assessment complete"
    );

    DriftScore {
        total: total_debt,
        fan_in_delta,
        fan_out_delta,
        new_cycles: cycles,
        boundary_violations: violations,
        cognitive_complexity: (density * 10.0).round() / 10.0,
        timestamp,
    }
}

#[allow(clippy::too_many_arguments)]
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

// ── Helpers ──

fn compute_fan_metrics(graph: &DiGraph<String, ()>) -> (usize, usize) {
    let mut max_fan_in: usize = 0;
    let mut max_fan_out: usize = 0;
    for node_idx in graph.node_indices() {
        let fan_in = graph
            .neighbors_directed(node_idx, petgraph::Direction::Incoming)
            .count();
        let fan_out = graph
            .neighbors_directed(node_idx, petgraph::Direction::Outgoing)
            .count();
        if fan_in > max_fan_in {
            max_fan_in = fan_in;
        }
        if fan_out > max_fan_out {
            max_fan_out = fan_out;
        }
    }
    (max_fan_in, max_fan_out)
}

fn count_cycles(graph: &DiGraph<String, ()>) -> usize {
    kosaraju_scc(graph)
        .iter()
        .filter(|scc| scc.len() > 1)
        .count()
}

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

pub fn count_cycles_public(graph: &DiGraph<String, ()>) -> usize {
    count_cycles(graph)
}
pub fn edges_to_pairs(edges: &[crate::models::DependencyEdge]) -> Vec<(String, String)> {
    edges
        .iter()
        .map(|e| (e.from_module.clone(), e.to_module.clone()))
        .collect()
}

// =============================================================================
// Tests
// =============================================================================
#[cfg(test)]
mod tests {
    use super::*;

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
    fn test_calculate_health_clean_small() {
        let graph = make_simple_graph();
        let score = calculate_drift(&graph, None, &[], &[], 0);
        // Small project, no cycles -> 0 debt (100% health)
        assert_eq!(score.total, 0);
    }

    #[test]
    fn test_calculate_health_with_cycle() {
        let graph = make_cyclic_graph();
        let score = calculate_drift(&graph, None, &[], &[], 0);
        // 1 Cycle = 25 debt
        assert_eq!(score.total, 25);
    }

    #[test]
    fn test_calculate_health_density() {
        let mut g = DiGraph::new();
        for i in 0..20 {
            g.add_node(i.to_string());
        }
        // Add 100 edges -> Density = 5.0
        // This also creates a large cycle group (SCC).
        for i in 0..20 {
            for j in 1..6 {
                g.add_edge(
                    petgraph::graph::NodeIndex::new(i),
                    petgraph::graph::NodeIndex::new((i + j) % 20),
                    (),
                );
            }
        }
        let score = calculate_drift(&g, None, &[], &[], 0);
        // Density 5.0. Threshold 3.5. Excess 1.5. Debt = 1.5 * 5 = 7.5 -> 8
        // PLUS 1 Cycle group = 25 debt. Total = 33 debt.
        assert_eq!(score.total, 33);
    }
}
