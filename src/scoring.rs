// =============================================================================
// scoring.rs — Architecture Drift Score calculation engine
// =============================================================================
//
// Core module for Sprint 3. Measures the architectural "health" of the
// dependency graph for each commit and produces a 0-100 score.
//
// Metrics:
//   1. Fan-in / Fan-out change (incoming/outgoing edge count per node)
//   2. Cyclic dependency count (petgraph SCC analysis)
//   3. Boundary violations (package boundary crossings: apps/ <-> packages/)
//   4. Cognitive complexity proxy = (edges/nodes)*10 + cycles*5
//   5. Total score = normalize(0-100)
//
// Temporal analysis:
//   compare_graphs() compares two consecutive commits' graphs and
//   produces a TemporalDelta.
//
// Determinism:
//   All calculations produce the same output for the same input.
//   round() is used for consistent floating-point rounding.
// =============================================================================

use petgraph::algo::kosaraju_scc;
use petgraph::graph::DiGraph;
use std::collections::HashSet;
use tracing::debug;

use crate::models::{DriftScore, TemporalDelta};

/// Baseline drift score — used when no previous graph exists.
const BASELINE_SCORE: u8 = 50;

/// Boundary violation rules: dependencies between these prefix pairs are violations.
///
/// Standard monorepo convention:
///   - `apps/` → applications (user-facing)
///   - `packages/` → shared libraries
///
/// Violation: dependency from `packages/` → `apps/` (library depending on app)
pub const BOUNDARY_RULES: &[(&str, &str)] = &[
    ("packages::", "apps::"), // Library → app (forbidden direction)
    ("lib::", "apps::"),      // lib → apps (forbidden direction)
    ("core::", "apps::"),     // core → apps (forbidden direction)
    ("shared::", "apps::"),   // shared → apps (forbidden direction)
    ("packages::", "cmd::"),  // packages → cmd (forbidden direction)
    ("lib::", "cmd::"),       // lib → cmd (forbidden direction)
];

/// Calculates the architecture drift score for a dependency graph.
///
/// If a previous commit's graph is provided, delta analysis is performed;
/// otherwise a baseline (50) score with absolute metrics is calculated.
///
/// # Parameters
/// - `graph`: Current commit's dependency graph
/// - `prev_graph`: Previous commit's graph (`None` for first commit)
/// - `nodes`: Module names in the current graph (for boundary checks)
/// - `edges_raw`: Raw edge list (from_module, to_module pairs)
/// - `timestamp`: Commit timestamp
///
/// # Returns
/// `DriftScore` — total score (0-100) and sub-metrics
///
/// # Algorithm
/// 1. Compute fan-in/fan-out, find delta if previous graph exists
/// 2. Count cycles with SCC
/// 3. Check boundary violations
/// 4. Compute cognitive complexity proxy
/// 5. Normalize and clamp to [0, 100]
pub fn calculate_drift(
    graph: &DiGraph<String, ()>,
    prev_graph: Option<&DiGraph<String, ()>>,
    _nodes: &[String],
    edges_raw: &[(String, String)],
    timestamp: i64,
) -> DriftScore {
    let node_count = graph.node_count();
    let edge_count = graph.edge_count();

    // ── 1. Fan-in / Fan-out calculation ──
    let (current_fan_in, current_fan_out) = compute_fan_metrics(graph);

    let (fan_in_delta, fan_out_delta) = if let Some(prev) = prev_graph {
        let (prev_fan_in, prev_fan_out) = compute_fan_metrics(prev);
        (
            current_fan_in as i32 - prev_fan_in as i32,
            current_fan_out as i32 - prev_fan_out as i32,
        )
    } else {
        // First commit — no delta
        (0, 0)
    };

    // ── 2. Cyclic dependency count (SCC analysis) ──
    let current_cycles = count_cycles(graph);
    let prev_cycles = prev_graph.map_or(0, count_cycles);
    let new_cycles = current_cycles.saturating_sub(prev_cycles);

    // ── 3. Boundary violation check ──
    let boundary_violations = count_boundary_violations(edges_raw);

    // ── 4. Cognitive complexity proxy ──
    let complexity = if node_count > 0 {
        (edge_count as f64 / node_count as f64) * 10.0 + current_cycles as f64 * 5.0
    } else {
        0.0
    };

    // ── 5. Compute total score and normalize ──
    let total = if prev_graph.is_some() {
        compute_total_score(
            fan_in_delta,
            fan_out_delta,
            new_cycles,
            boundary_violations,
            complexity,
        )
    } else {
        // First commit — baseline score
        BASELINE_SCORE
    };

    debug!(
        total,
        fan_in_delta,
        fan_out_delta,
        new_cycles,
        boundary_violations,
        complexity,
        "Drift score calculated"
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

/// Compares two consecutive commits' graphs to compute a temporal delta.
#[allow(dead_code, clippy::too_many_arguments)]
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
// Helper functions — internal use
// =============================================================================

fn compute_fan_metrics(graph: &DiGraph<String, ()>) -> (usize, usize) {
    let mut total_fan_in: usize = 0;
    let mut total_fan_out: usize = 0;

    for node_idx in graph.node_indices() {
        total_fan_in += graph
            .neighbors_directed(node_idx, petgraph::Direction::Incoming)
            .count();
        total_fan_out += graph
            .neighbors_directed(node_idx, petgraph::Direction::Outgoing)
            .count();
    }

    (total_fan_in, total_fan_out)
}

fn count_cycles(graph: &DiGraph<String, ()>) -> usize {
    let sccs = kosaraju_scc(graph);
    sccs.iter().filter(|scc| scc.len() > 1).count()
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

    let raw = BASELINE_SCORE as f64
        + fan_component / 2.0
        + cycle_penalty
        + boundary_penalty
        + complexity_component / 3.0;

    raw.round().clamp(0.0, 100.0) as u8
}

/// Returns the cyclic dependency count as a public function.
pub fn count_cycles_public(graph: &DiGraph<String, ()>) -> usize {
    count_cycles(graph)
}

/// Creates (from, to) pairs from an edge list.
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
    fn test_calculate_drift_baseline() {
        let graph = make_simple_graph();
        let nodes = vec!["A".to_string(), "B".to_string(), "C".to_string()];
        let edges = vec![
            ("A".to_string(), "B".to_string()),
            ("A".to_string(), "C".to_string()),
            ("B".to_string(), "C".to_string()),
        ];

        let score = calculate_drift(&graph, None, &nodes, &edges, 1_000_000);

        assert_eq!(score.total, BASELINE_SCORE, "First commit should be baseline");
        assert_eq!(score.fan_in_delta, 0);
        assert_eq!(score.fan_out_delta, 0);
        assert_eq!(score.new_cycles, 0);
    }

    #[test]
    fn test_calculate_drift_with_previous() {
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
            "Adding a cycle should increase score, but got {}",
            score.total
        );
        assert_eq!(score.new_cycles, 1);
    }

    #[test]
    fn test_cycle_detection() {
        let simple = make_simple_graph();
        assert_eq!(count_cycles(&simple), 0);

        let cyclic = make_cyclic_graph();
        assert_eq!(count_cycles(&cyclic), 1);

        let empty: DiGraph<String, ()> = DiGraph::new();
        assert_eq!(count_cycles(&empty), 0);
    }

    #[test]
    fn test_boundary_violations() {
        let edges = vec![
            ("packages::ui::button".to_string(), "apps::web::home".to_string()),
            ("apps::web::home".to_string(), "packages::ui::button".to_string()),
            ("lib::utils".to_string(), "apps::api::routes".to_string()),
            ("packages::ui::button".to_string(), "packages::ui::theme".to_string()),
        ];

        let violations = count_boundary_violations(&edges);
        assert_eq!(violations, 2, "Should have 2 violations (packages->apps, lib->apps)");
    }

    #[test]
    fn test_fan_metrics() {
        let graph = make_simple_graph();
        let (fan_in, fan_out) = compute_fan_metrics(&graph);
        assert_eq!(fan_in, 3);
        assert_eq!(fan_out, 3);
    }

    #[test]
    fn test_compare_graphs_temporal() {
        let prev = make_simple_graph();
        let current = make_cyclic_graph();

        let prev_nodes: HashSet<String> = ["A", "B", "C"].iter().map(|s| s.to_string()).collect();
        let current_nodes: HashSet<String> =
            ["A", "B", "C", "D"].iter().map(|s| s.to_string()).collect();

        let delta = compare_graphs(
            &current, &prev, &current_nodes, &prev_nodes,
            3, 3, 65, 50, "commit2", "commit1",
        );

        assert_eq!(delta.score_delta, 15);
        assert_eq!(delta.nodes_added, 1);
        assert_eq!(delta.nodes_removed, 0);
        assert_eq!(delta.new_cycles, 1);
        assert_eq!(delta.resolved_cycles, 0);
    }

    #[test]
    fn test_compute_total_score_deterministic() {
        let s1 = compute_total_score(5, 3, 1, 2, 15.0);
        let s2 = compute_total_score(5, 3, 1, 2, 15.0);
        assert_eq!(s1, s2, "Should be deterministic");
        assert!(s1 <= 100);

        let extreme = compute_total_score(100, 100, 10, 20, 500.0);
        assert_eq!(extreme, 100, "Extreme values should clamp to 100");
    }

    #[test]
    fn test_empty_graph_drift() {
        let empty: DiGraph<String, ()> = DiGraph::new();
        let nodes: Vec<String> = vec![];
        let edges: Vec<(String, String)> = vec![];

        let score = calculate_drift(&empty, None, &nodes, &edges, 0);
        assert_eq!(score.total, BASELINE_SCORE);
        assert_eq!(score.cognitive_complexity, 0.0);
    }
}
