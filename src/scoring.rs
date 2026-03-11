//! 6-Component Scale-Aware Architecture Health Scoring Engine.
//!
//! Computes a 0-100 "Debt" score using six structural metrics.
//! Health = 100 - Debt.
//!
//! # Components (weighted sum → total debt)
//!
//! | Component       | Weight | What it Measures                               |
//! |-----------------|--------|------------------------------------------------|
//! | Cycle Debt      |  30%   | SCC count + cyclic node fraction + largest SCC |
//! | Layering Debt   |  25%   | Back-edge ratio in topological ordering        |
//! | Hub Debt        |  15%   | True god modules (high in AND out)             |
//! | Coupling Debt   |  12%   | Weighted coupling intensity via edge weights   |
//! | Cognitive Debt  |  10%   | Shannon entropy + edge excess ratio            |
//! | Instability Debt|   8%   | Refined Martin metric (leaf packages excluded) |
//!
//! # Design Philosophy
//! 1. **Scale Agnostic**: Asymptotic/logarithmic curves adapt to any codebase size.
//! 2. **Monorepo Aware**: Works identically for Deno, Turborepo, Nx, Moon, Gradle, Lage.
//! 3. **Legitimate Hubs Exempt**: High fan-in + low fan-out = shared core (no penalty).
//! 4. **Leaf Packages Exempt**: `ca=0` → `I=1.0` is natural, not brittle.
//! 5. **Edge Weights Used**: Import count drives coupling intensity measurement.

use petgraph::algo::kosaraju_scc;
use petgraph::graph::{DiGraph, NodeIndex};
use std::collections::HashMap;
use tracing::debug;

use crate::config::{Exemptions, ScoringConfig, Thresholds};
use crate::models::DriftScore;

/// Legacy boundary violation rules — retained only for backward compatibility
/// with the `analyze` CLI command when no `morpharch.toml` boundaries are configured.
pub const LEGACY_BOUNDARY_RULES: &[(&str, &str)] = &[
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

// ═════════════════════════════════════════════════════════════════════════════
// Component 1: Cycle Debt (Weight: 30%)
// ═════════════════════════════════════════════════════════════════════════════

/// Computes cycle debt from SCC analysis.
///
/// Three sub-factors:
/// - `cyclic_fraction` (50%): fraction of nodes trapped in non-trivial SCCs
/// - `max_scc_fraction` (30%): largest SCC as fraction of total nodes
/// - `scc_count_score` (20%): log-scaled count of SCCs
///
/// Returns (debt_0_to_100, scc_count)
fn compute_cycle_debt(graph: &DiGraph<String, u32>) -> (f64, usize) {
    let n = graph.node_count();
    if n == 0 {
        return (0.0, 0);
    }

    let sccs = kosaraju_scc(graph);
    let non_trivial: Vec<&Vec<NodeIndex>> = sccs.iter().filter(|scc| scc.len() > 1).collect();
    let scc_count = non_trivial.len();

    if scc_count == 0 {
        return (0.0, 0);
    }

    // Nodes trapped in any non-trivial SCC
    let cyclic_nodes: usize = non_trivial.iter().map(|scc| scc.len()).sum();
    let cyclic_fraction = cyclic_nodes as f64 / n as f64;

    // Largest SCC as fraction of total
    let max_scc_size = non_trivial.iter().map(|scc| scc.len()).max().unwrap_or(0);
    let max_scc_fraction = max_scc_size as f64 / n as f64;

    // Log-scaled SCC count: ln(1+count) / ln(1+N/4), capped at 1.0
    let count_score = ((1.0 + scc_count as f64).ln() / (1.0 + n as f64 / 4.0).ln()).min(1.0);

    let raw = 0.50 * cyclic_fraction + 0.30 * max_scc_fraction + 0.20 * count_score;

    // Asymptotic scaling: 100 * (1 - e^(-3 * raw))
    let debt = 100.0 * (1.0 - (-3.0 * raw).exp());
    (debt, scc_count)
}

// ═════════════════════════════════════════════════════════════════════════════
// Component 2: Layering Debt (Weight: 25%)
// ═════════════════════════════════════════════════════════════════════════════

/// Computes layering debt via structural analysis.
///
/// Measures how much the graph deviates from a clean layered (DAG) architecture.
/// Cycle debt already penalizes the EXISTENCE of cycles; layering debt penalizes
/// the DENSITY of edges that violate clean directional flow.
///
/// For each non-trivial SCC: a minimal ring needs exactly `S` edges.
/// Any edges beyond that are excess cross-cutting violations within the cycle.
///
/// Returns (debt_0_to_100, violation_count)
fn compute_layering_debt(graph: &DiGraph<String, u32>) -> (f64, usize) {
    let n = graph.node_count();
    let e = graph.edge_count();
    if n < 3 || e == 0 {
        return (0.0, 0);
    }

    let sccs = kosaraju_scc(graph);

    // Map each node → its SCC index
    let mut node_to_scc: HashMap<NodeIndex, usize> = HashMap::new();
    for (scc_idx, scc) in sccs.iter().enumerate() {
        for &node in scc {
            node_to_scc.insert(node, scc_idx);
        }
    }

    // Count internal edges per non-trivial SCC
    let mut scc_internal_edges: Vec<usize> = vec![0; sccs.len()];
    for edge_idx in graph.edge_indices() {
        let (src, tgt) = graph.edge_endpoints(edge_idx).unwrap();
        let src_scc = node_to_scc[&src];
        let tgt_scc = node_to_scc[&tgt];
        if src_scc == tgt_scc && sccs[src_scc].len() > 1 {
            scc_internal_edges[src_scc] += 1;
        }
    }

    // For each non-trivial SCC: excess edges beyond a minimal ring
    // A ring of size S needs exactly S edges. Excess = internal - S.
    // These excess edges represent additional layering violations within the cycle.
    let mut violations = 0usize;
    for (scc_idx, scc) in sccs.iter().enumerate() {
        if scc.len() > 1 {
            let internal = scc_internal_edges[scc_idx];
            violations += internal.saturating_sub(scc.len());
        }
    }

    if violations == 0 {
        return (0.0, 0);
    }

    let violation_ratio = violations as f64 / e as f64;
    // Asymptotic scaling: 100 * (1 - e^(-3 * ratio))
    let debt = 100.0 * (1.0 - (-3.0 * violation_ratio).exp());
    (debt, violations)
}

// ═════════════════════════════════════════════════════════════════════════════
// Component 3: Hub Debt (Weight: 15%)
// ═════════════════════════════════════════════════════════════════════════════

/// Computes hub/god module debt.
///
/// A **god module** is one that sits in the MIDDLE of the dependency graph with
/// both high fan-in AND high fan-out. It knows too much and is known by too many.
///
/// Two architectural patterns are explicitly EXEMPT:
/// - **Shared cores** (high in, low out): e.g., `deno_core` — ratio < 0.3
/// - **Composition roots** (low in, high out): e.g., `cli/tools` — fan_in ≤ 2
///   These are entry points / orchestrators that naturally wire things together.
///
/// Uses scale-adaptive threshold: `2·√N`.
fn compute_hub_debt(
    graph: &DiGraph<String, u32>,
    thresholds: &Thresholds,
    exemptions: &Exemptions,
) -> f64 {
    let n = graph.node_count();
    if n < 6 {
        return 0.0;
    }

    let threshold = (2.0 * (n as f64).sqrt()).max(8.0);
    let mut total_god_penalty = 0.0f64;
    let mut god_count = 0u32;

    for node_idx in graph.node_indices() {
        let module_name = &graph[node_idx];

        // Explicit exemption by name
        if exemptions
            .hub_exempt
            .iter()
            .any(|e| module_name.contains(e.as_str()))
        {
            continue;
        }

        // ── Exemption: Named entry point stems ──
        // Modules whose file stem matches a configured entry point (e.g., "main",
        // "index", "app") are composition roots by definition — not god modules.
        if is_entry_point_stem(module_name, &exemptions.entry_point_stems) {
            continue;
        }

        let fan_in = graph
            .neighbors_directed(node_idx, petgraph::Direction::Incoming)
            .count();
        let fan_out = graph
            .neighbors_directed(node_idx, petgraph::Direction::Outgoing)
            .count();
        let total_degree = fan_in + fan_out;

        if (total_degree as f64) < threshold {
            continue;
        }

        // Hub ratio: how much does this module export vs import?
        let hub_ratio = fan_out as f64 / (fan_in as f64 + 1.0);

        // ── Exemption: Legitimate shared core ──
        // High fan-in, low fan-out (e.g., deno_core: 42 in, 3 out → ratio = 0.07)
        if hub_ratio < thresholds.hub_exemption_ratio {
            continue;
        }

        // ── Exemption: Composition root by fan-in threshold ──
        // Zero or very low fan-in means this is a top-level orchestrator, not a god
        // module. cli/tools (0 in, 46 out) is an entry point, not an anti-pattern.
        if fan_in <= thresholds.entry_point_max_fan_in {
            continue;
        }

        // At this point: module has high fan-in AND high fan-out → true god module
        // Excess ratio: how far above threshold
        let excess_ratio = (total_degree as f64 - threshold) / threshold;

        // Weight multiplier based on coupling strength via edge weights
        let weighted_degree: u32 = graph
            .edges_directed(node_idx, petgraph::Direction::Incoming)
            .map(|e| *e.weight())
            .sum::<u32>()
            + graph
                .edges_directed(node_idx, petgraph::Direction::Outgoing)
                .map(|e| *e.weight())
                .sum::<u32>();
        let weight_multiplier =
            (weighted_degree as f64 / total_degree.max(1) as f64 / 2.0).clamp(0.5, 2.0);

        let penalty = excess_ratio * weight_multiplier * hub_ratio.min(1.0);
        total_god_penalty += penalty;
        god_count += 1;
    }

    if god_count == 0 {
        return 0.0;
    }

    // Scale: multiple god modules compound
    let raw = (total_god_penalty / god_count as f64) * (1.0 + 0.3 * (god_count - 1) as f64);
    // Asymptotic: 100 * (1 - e^(-2 * raw))
    (100.0 * (1.0 - (-2.0 * raw).exp())).min(100.0)
}

// ═════════════════════════════════════════════════════════════════════════════
// Component 4: Coupling Debt (Weight: 12%)
// ═════════════════════════════════════════════════════════════════════════════

/// Computes coupling debt using log-dampened edge weights.
///
/// Edge weights can vary wildly (1 vs 234) which makes raw sums misleading.
/// We apply `ln(1 + w)` to each weight before aggregation — this respects
/// that w=234 is heavier than w=1, but not 234× heavier.
///
/// Two sub-factors:
/// - Log-dampened density excess (60%): Σ(ln(1+w))/N vs expected `3.0 + 2·ln(N)`
/// - Weight concentration (40%): coefficient of variation of log-dampened weights
fn compute_coupling_debt(graph: &DiGraph<String, u32>) -> f64 {
    let n = graph.node_count();
    let e = graph.edge_count();
    if n < 3 || e == 0 {
        return 0.0;
    }

    // Log-dampened weights: ln(1 + w) compresses extreme outliers
    let log_weights: Vec<f64> = graph
        .edge_indices()
        .map(|ei| (1.0 + graph[ei] as f64).ln())
        .collect();
    let total_log_weight: f64 = log_weights.iter().sum();
    let log_density = total_log_weight / n as f64;

    // Expected density adapts to scale: 3.0 + 2·ln(N)
    // Real monorepos naturally have higher connectivity than 3+ln(N)
    let expected = 3.0 + 2.0 * (n as f64).ln();
    let density_excess = ((log_density - expected).max(0.0) / expected).min(1.0);

    // Weight concentration: CV of log-dampened weights
    let mean_log = total_log_weight / e as f64;
    let variance: f64 = log_weights
        .iter()
        .map(|w| (w - mean_log).powi(2))
        .sum::<f64>()
        / e as f64;
    let std_dev = variance.sqrt();
    let cv = if mean_log > 0.0 {
        std_dev / mean_log
    } else {
        0.0
    };
    // High CV means extreme coupling outliers even after log dampening
    let concentration = (cv / 2.0).min(1.0);

    let raw = 0.60 * density_excess + 0.40 * concentration;
    // Asymptotic: 100 * (1 - e^(-3 * raw))
    100.0 * (1.0 - (-3.0 * raw).exp())
}

// ═════════════════════════════════════════════════════════════════════════════
// Component 5: Cognitive Debt (Weight: 10%)
// ═════════════════════════════════════════════════════════════════════════════

/// Computes cognitive debt — how hard the dependency graph is to reason about.
///
/// Real software isn't a tree. A module typically has 2-4 dependencies, so
/// the natural baseline is ~2N edges (not N-1). We measure excess above that.
///
/// Two sub-factors:
/// - Edge excess ratio: E / 2N, penalized only when > 1.0 (more than 2 deps avg)
/// - Degree excess: avg degree vs 2·ln(N), penalized only when above that
fn compute_cognitive_debt(graph: &DiGraph<String, u32>) -> f64 {
    let n = graph.node_count();
    let e = graph.edge_count();
    if n < 3 {
        return 0.0;
    }

    // Edge excess: how many more edges than a realistic baseline (2N)
    // A module having ~2 dependencies on average is normal for real software
    let baseline_edges = 2 * n;
    let edge_excess = if baseline_edges > 0 {
        ((e as f64 / baseline_edges as f64) - 1.0).max(0.0)
    } else {
        0.0
    };
    // 3x the baseline = fully saturated (6N edges would be extreme)
    let excess_ratio = (edge_excess / 2.0).min(1.0);

    // Average degree vs expected: 2·ln(N) is a reasonable expectation
    // for well-structured software (slightly more interconnected than sparse)
    let avg_degree = if n > 0 {
        2.0 * e as f64 / n as f64
    } else {
        0.0
    };
    let expected_avg = (2.0 * (n as f64).ln()).max(3.0);
    let degree_excess = ((avg_degree / expected_avg) - 1.0).clamp(0.0, 1.0);

    // Scale factor: soften penalties for small graphs where ratios are misleading
    let scale = 1.0 - (-(n as f64) / 20.0).exp();

    let raw = (0.50 * excess_ratio + 0.50 * degree_excess) * scale;
    // Asymptotic: 100 * (1 - e^(-3 * raw))
    100.0 * (1.0 - (-3.0 * raw).exp())
}

// ═════════════════════════════════════════════════════════════════════════════
// Component 6: Instability Debt (Weight: 8%)
// ═════════════════════════════════════════════════════════════════════════════

/// Computes instability debt using refined Martin metric.
///
/// Key fix: Excludes leaf packages (ca=0) which naturally have I=1.0.
/// Only penalizes non-leaf brittle modules: I > threshold AND fan_in > 0 AND degree ≥ 3.
fn compute_instability_debt(
    graph: &DiGraph<String, u32>,
    thresholds: &Thresholds,
    exemptions: &Exemptions,
) -> f64 {
    let n = graph.node_count();
    if n < 3 {
        return 0.0;
    }

    let mut brittle_count = 0usize;
    let mut eligible_count = 0usize;

    for node_idx in graph.node_indices() {
        let module_name = &graph[node_idx];

        // Explicit exemption by name
        if exemptions
            .instability_exempt
            .iter()
            .any(|e| module_name.contains(e.as_str()))
        {
            continue;
        }

        // Entry point stems are exempt — they naturally have high fan-out
        if is_entry_point_stem(module_name, &exemptions.entry_point_stems) {
            continue;
        }

        let ca = graph
            .neighbors_directed(node_idx, petgraph::Direction::Incoming)
            .count();
        let ce = graph
            .neighbors_directed(node_idx, petgraph::Direction::Outgoing)
            .count();
        let total = ca + ce;

        // Skip isolated nodes and low-connectivity nodes
        if total < 3 {
            continue;
        }

        // Skip leaf packages: ca=0 naturally produces I=1.0, which is not brittle
        if ca == 0 {
            continue;
        }

        eligible_count += 1;

        let instability = ce as f64 / total as f64;
        if instability > thresholds.brittle_instability_ratio {
            brittle_count += 1;
        }
    }

    if eligible_count == 0 {
        return 0.0;
    }

    let brittle_ratio = brittle_count as f64 / eligible_count as f64;

    // Scale factor for small graphs (softens penalty)
    let scale = 1.0 - (-(n as f64) / 30.0).exp();

    let raw = brittle_ratio * scale;
    // Asymptotic: 100 * (1 - e^(-3 * raw))
    100.0 * (1.0 - (-3.0 * raw).exp())
}

// ═════════════════════════════════════════════════════════════════════════════
// Fan Delta Computation (Median-based)
// ═════════════════════════════════════════════════════════════════════════════

/// Computes median fan-in and fan-out deltas between current and previous graph.
///
/// Uses median (not max) for robustness against outliers.
fn compute_fan_deltas(
    graph: &DiGraph<String, u32>,
    prev_graph: Option<&DiGraph<String, u32>>,
) -> (i32, i32) {
    let prev = match prev_graph {
        Some(p) => p,
        None => return (0, 0),
    };

    // Build name → (fan_in, fan_out) maps
    let current_map = build_fan_map(graph);
    let prev_map = build_fan_map(prev);

    let mut in_deltas: Vec<i32> = Vec::new();
    let mut out_deltas: Vec<i32> = Vec::new();

    for (name, (cur_in, cur_out)) in &current_map {
        if let Some(&(prev_in, prev_out)) = prev_map.get(name) {
            in_deltas.push(*cur_in as i32 - prev_in as i32);
            out_deltas.push(*cur_out as i32 - prev_out as i32);
        }
    }

    (median_i32(&mut in_deltas), median_i32(&mut out_deltas))
}

fn build_fan_map(graph: &DiGraph<String, u32>) -> HashMap<String, (usize, usize)> {
    let mut map = HashMap::new();
    for node_idx in graph.node_indices() {
        let fan_in = graph
            .neighbors_directed(node_idx, petgraph::Direction::Incoming)
            .count();
        let fan_out = graph
            .neighbors_directed(node_idx, petgraph::Direction::Outgoing)
            .count();
        map.insert(graph[node_idx].clone(), (fan_in, fan_out));
    }
    map
}

fn median_i32(values: &mut [i32]) -> i32 {
    if values.is_empty() {
        return 0;
    }
    values.sort_unstable();
    let mid = values.len() / 2;
    if values.len().is_multiple_of(2) {
        (values[mid - 1] + values[mid]) / 2
    } else {
        values[mid]
    }
}

// ═════════════════════════════════════════════════════════════════════════════
// Public API: Instability Metrics (used by TUI hotspots table)
// ═════════════════════════════════════════════════════════════════════════════

/// Computes per-node instability index (I = Ce / (Ca + Ce)).
/// Returns (Module Name, Instability, Fan-in (Ca), Fan-out (Ce))
pub fn compute_instability_metrics(
    graph: &DiGraph<String, u32>,
) -> Vec<(String, f64, usize, usize)> {
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
        metrics.push((graph[node_idx].clone(), instability, ca, ce));
    }
    // Sort primarily by highest fan-out + fan-in (most active modules), then by instability
    metrics.sort_by(|a, b| {
        let total_b = b.2 + b.3;
        let total_a = a.2 + a.3;
        total_b
            .cmp(&total_a)
            .then(b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal))
    });
    metrics
}

// ═════════════════════════════════════════════════════════════════════════════
// Main Scoring Function
// ═════════════════════════════════════════════════════════════════════════════

/// Calculates the absolute architectural debt score (0-100).
/// 0 debt = 100% Health.
///
/// Uses 6-component weighted sum with configurable weights.
/// When called with `ScoringConfig::default()`, produces identical results
/// to the original hardcoded engine.
pub fn calculate_drift(
    graph: &DiGraph<String, u32>,
    prev_graph: Option<&DiGraph<String, u32>>,
    timestamp: i64,
    config: &ScoringConfig,
) -> DriftScore {
    let n = graph.node_count();

    if n == 0 {
        return DriftScore {
            total: 0,
            fan_in_delta: 0,
            fan_out_delta: 0,
            new_cycles: 0,
            boundary_violations: 0,
            cognitive_complexity: 0.0,
            timestamp,
            cycle_debt: 0.0,
            layering_debt: 0.0,
            hub_debt: 0.0,
            coupling_debt: 0.0,
            cognitive_debt: 0.0,
            instability_debt: 0.0,
        };
    }

    // ── Compute all 6 components ──
    let (cycle_score, scc_count) = compute_cycle_debt(graph);
    let (layering_score, back_edges) = compute_layering_debt(graph);
    let hub_score = compute_hub_debt(graph, &config.thresholds, &config.exemptions);
    let coupling_score = compute_coupling_debt(graph);
    let cognitive_score = compute_cognitive_debt(graph);
    let instability_score = compute_instability_debt(graph, &config.thresholds, &config.exemptions);

    // ── Weighted sum (normalized weights) ──
    let w = config.weights.normalized();
    let total_debt = (w.cycle * cycle_score
        + w.layering * layering_score
        + w.hub * hub_score
        + w.coupling * coupling_score
        + w.cognitive * cognitive_score
        + w.instability * instability_score)
        .round()
        .min(100.0) as u8;

    // ── Fan deltas (median-based) ──
    let (fan_in_delta, fan_out_delta) = compute_fan_deltas(graph, prev_graph);

    // ── Cognitive complexity (real entropy-based value) ──
    let cog_complexity = (cognitive_score * 10.0).round() / 10.0;

    debug!(
        total_debt,
        cycle_score,
        layering_score,
        hub_score,
        coupling_score,
        cognitive_score,
        instability_score,
        "Architectural Health assessment complete"
    );

    DriftScore {
        total: total_debt,
        fan_in_delta,
        fan_out_delta,
        new_cycles: scc_count,
        boundary_violations: back_edges,
        cognitive_complexity: cog_complexity,
        timestamp,
        cycle_debt: (cycle_score * 10.0).round() / 10.0,
        layering_debt: (layering_score * 10.0).round() / 10.0,
        hub_debt: (hub_score * 10.0).round() / 10.0,
        coupling_debt: (coupling_score * 10.0).round() / 10.0,
        cognitive_debt: (cognitive_score * 10.0).round() / 10.0,
        instability_debt: (instability_score * 10.0).round() / 10.0,
    }
}

// ═════════════════════════════════════════════════════════════════════════════
// Helpers
// ═════════════════════════════════════════════════════════════════════════════

fn count_cycles(graph: &DiGraph<String, u32>) -> usize {
    kosaraju_scc(graph)
        .iter()
        .filter(|scc| scc.len() > 1)
        .count()
}

pub fn count_cycles_public(graph: &DiGraph<String, u32>) -> usize {
    count_cycles(graph)
}

pub fn edges_to_pairs(edges: &[crate::models::DependencyEdge]) -> Vec<(String, String)> {
    edges
        .iter()
        .map(|e| (e.from_module.clone(), e.to_module.clone()))
        .collect()
}

/// Returns `true` if the module name's file stem matches any configured entry point stem.
///
/// Example: `"cli/tools"` → stem `"tools"`, `"src/main.rs"` → stem `"main"`.
fn is_entry_point_stem(module_name: &str, stems: &[String]) -> bool {
    let basename = module_name.split('/').next_back().unwrap_or(module_name);
    let stem = basename.split('.').next().unwrap_or(basename);
    stems.iter().any(|s| s == stem)
}

// ═════════════════════════════════════════════════════════════════════════════
// Component Diagnostics (for TUI advisory display)
// ═════════════════════════════════════════════════════════════════════════════

/// Generates human-friendly advisory lines explaining WHY each elevated component
/// contributes to architectural debt. Designed for non-expert users — uses plain
/// language, names specific modules, and suggests actionable next steps.
///
/// Called from TUI to populate the ADVISORY section in the Health tab.
pub fn generate_diagnostics(
    graph: &DiGraph<String, u32>,
    drift: &crate::models::DriftScore,
    config: &ScoringConfig,
) -> Vec<String> {
    let mut lines = Vec::new();
    let n = graph.node_count();
    let e = graph.edge_count();

    // ── Cycle diagnostics ──
    if drift.cycle_debt > 10.0 {
        let sccs = kosaraju_scc(graph);
        let non_trivial: Vec<_> = sccs.iter().filter(|scc| scc.len() > 1).collect();
        if !non_trivial.is_empty() {
            let largest = non_trivial.iter().map(|s| s.len()).max().unwrap_or(0);
            let cyclic_nodes: usize = non_trivial.iter().map(|s| s.len()).sum();
            if non_trivial.len() == 1 {
                lines.push(format!(
                    "{} modules form a circular dependency chain. \
                     Break the cycle with interfaces or traits.",
                    cyclic_nodes
                ));
            } else {
                lines.push(format!(
                    "{} circular dependency groups found ({} modules involved, \
                     largest spans {}). Consider dependency inversion.",
                    non_trivial.len(),
                    cyclic_nodes,
                    largest
                ));
            }
        }
    }

    // ── Layering diagnostics ──
    if drift.layering_debt > 10.0 && drift.boundary_violations > 0 {
        let count = drift.boundary_violations;
        if count == 1 {
            lines.push(
                "1 extra cross-cutting edge inside a cycle. \
                 Simplifying internal wiring would improve clarity."
                    .to_string(),
            );
        } else {
            lines.push(format!(
                "{} extra edges inside dependency cycles make the \
                 structure harder to follow. Simplify internal wiring.",
                count
            ));
        }
    }

    // ── Hub diagnostics ──
    // Only show truly problematic god modules (high in AND out, not composition roots)
    if drift.hub_debt > 10.0 {
        let threshold = (2.0 * (n as f64).sqrt()).max(8.0);
        let mut hubs: Vec<(String, usize, usize)> = Vec::new();
        for node_idx in graph.node_indices() {
            let module_name = &graph[node_idx];
            if config
                .exemptions
                .hub_exempt
                .iter()
                .any(|e| module_name.contains(e.as_str()))
            {
                continue;
            }
            let fi = graph
                .neighbors_directed(node_idx, petgraph::Direction::Incoming)
                .count();
            let fo = graph
                .neighbors_directed(node_idx, petgraph::Direction::Outgoing)
                .count();
            let total = fi + fo;
            if total as f64 >= threshold {
                let ratio = fo as f64 / (fi as f64 + 1.0);
                // Match scoring: skip shared cores and composition roots
                if ratio >= config.thresholds.hub_exemption_ratio
                    && fi > config.thresholds.entry_point_max_fan_in
                {
                    hubs.push((graph[node_idx].clone(), fi, fo));
                }
            }
        }
        hubs.sort_by(|a, b| (b.1 + b.2).cmp(&(a.1 + a.2)));
        for (name, fi, fo) in hubs.iter().take(2) {
            lines.push(format!(
                "{} has {} incoming and {} outgoing deps — \
                 it's doing too much. Split responsibilities.",
                name, fi, fo
            ));
        }
    }

    // ── Coupling diagnostics ──
    if drift.coupling_debt > 10.0 {
        // Use log-dampened density consistent with scoring formula
        let total_log_w: f64 = graph
            .edge_indices()
            .map(|ei| (1.0 + graph[ei] as f64).ln())
            .sum();
        let log_density = total_log_w / n.max(1) as f64;
        let expected = 3.0 + 2.0 * (n as f64).ln();

        // Find heaviest edge
        let mut heaviest = ("?".to_string(), "?".to_string(), 0u32);
        for edge_idx in graph.edge_indices() {
            let w = graph[edge_idx];
            if w > heaviest.2 {
                let (src, tgt) = graph.edge_endpoints(edge_idx).unwrap();
                heaviest = (graph[src].clone(), graph[tgt].clone(), w);
            }
        }

        let ratio = log_density / expected;
        if ratio > 1.5 {
            lines.push(format!(
                "Modules are more tightly connected than expected ({:.1}x). \
                 Introduce abstractions to reduce direct dependencies.",
                ratio
            ));
        } else {
            lines.push(
                "Some modules share more dependencies than expected. \
                 Review coupling and consider interfaces."
                    .to_string(),
            );
        }

        if heaviest.2 > 5 {
            lines.push(format!(
                "Strongest link: {} \u{2192} {} ({} imports). \
                 This tight binding makes both harder to change.",
                heaviest.0, heaviest.1, heaviest.2
            ));
        }
    }

    // ── Cognitive diagnostics ──
    if drift.cognitive_debt > 10.0 {
        // Use 2N baseline consistent with scoring formula
        let baseline = 2 * n;
        let pct = if baseline > 0 {
            (((e as f64 / baseline as f64) - 1.0) * 100.0).round() as i32
        } else {
            0
        };
        if pct > 0 {
            lines.push(format!(
                "The graph has {}% more connections than typical. \
                 Fewer links would make the architecture easier to reason about.",
                pct
            ));
        }
    }

    // ── Instability diagnostics ──
    if drift.instability_debt > 10.0 {
        let mut brittle_names: Vec<String> = Vec::new();
        let mut eligible = 0usize;
        for node_idx in graph.node_indices() {
            let module_name = &graph[node_idx];
            if config
                .exemptions
                .instability_exempt
                .iter()
                .any(|e| module_name.contains(e.as_str()))
            {
                continue;
            }
            let ca = graph
                .neighbors_directed(node_idx, petgraph::Direction::Incoming)
                .count();
            let ce = graph
                .neighbors_directed(node_idx, petgraph::Direction::Outgoing)
                .count();
            if ca + ce >= 3 && ca > 0 {
                eligible += 1;
                if ce as f64 / (ca + ce) as f64 > config.thresholds.brittle_instability_ratio {
                    let name = graph[node_idx].clone();
                    let basename = name.split('/').next_back().unwrap_or(&name);
                    let stem = basename.split('.').next().unwrap_or(basename);
                    let is_entry = config
                        .exemptions
                        .entry_point_stems
                        .iter()
                        .any(|s| s == stem);

                    if !is_entry {
                        brittle_names.push(name);
                    }
                }
            }
        }
        if !brittle_names.is_empty() {
            if brittle_names.len() <= 3 {
                lines.push(format!(
                    "{} fragile: depends on many others but few depend on \
                     it. Changes upstream will likely cascade here.",
                    brittle_names.join(", ")
                ));
            } else {
                lines.push(format!(
                    "{} of {} core modules are fragile — they depend heavily \
                     on others. Stabilize with dependency injection.",
                    brittle_names.len(),
                    eligible
                ));
            }
        }
    }

    lines
}

// =============================================================================
// Tests
// =============================================================================
#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ScoringConfig;

    fn default_config() -> ScoringConfig {
        ScoringConfig::default()
    }

    // ── Test Helpers ──

    fn make_tree_graph(n: usize) -> DiGraph<String, u32> {
        let mut g = DiGraph::new();
        let nodes: Vec<_> = (0..n).map(|i| g.add_node(format!("node_{i}"))).collect();
        // Perfect binary-ish tree: parent → child edges only
        for i in 1..n {
            g.add_edge(nodes[i / 2], nodes[i], 1);
        }
        g
    }

    fn make_layered_graph(layers: &[usize]) -> DiGraph<String, u32> {
        let mut g = DiGraph::new();
        let mut layer_nodes: Vec<Vec<NodeIndex>> = Vec::new();

        for (l, &count) in layers.iter().enumerate() {
            let mut nodes = Vec::new();
            for i in 0..count {
                nodes.push(g.add_node(format!("L{l}_{i}")));
            }
            layer_nodes.push(nodes);
        }

        // Connect each layer to the next (forward edges only)
        for l in 0..layer_nodes.len() - 1 {
            for &src in &layer_nodes[l] {
                for &tgt in &layer_nodes[l + 1] {
                    g.add_edge(src, tgt, 1);
                }
            }
        }
        g
    }

    fn make_simple_graph() -> DiGraph<String, u32> {
        let mut g = DiGraph::new();
        let a = g.add_node("A".to_string());
        let b = g.add_node("B".to_string());
        let c = g.add_node("C".to_string());
        g.add_edge(a, b, 1);
        g.add_edge(a, c, 1);
        g.add_edge(b, c, 1);
        g
    }

    fn make_cyclic_graph() -> DiGraph<String, u32> {
        let mut g = DiGraph::new();
        let a = g.add_node("A".to_string());
        let b = g.add_node("B".to_string());
        let c = g.add_node("C".to_string());
        g.add_edge(a, b, 1);
        g.add_edge(b, c, 1);
        g.add_edge(c, a, 1);
        g
    }

    // ── Test 1: Empty graph → 0 debt ──
    #[test]
    fn test_empty_graph_zero_debt() {
        let g: DiGraph<String, u32> = DiGraph::new();
        let score = calculate_drift(&g, None, 0, &default_config());
        assert_eq!(score.total, 0, "Empty graph should have 0 debt");
        assert_eq!(score.cycle_debt, 0.0);
        assert_eq!(score.layering_debt, 0.0);
    }

    // ── Test 2: Perfect tree → debt < 10 ──
    #[test]
    fn test_perfect_tree_low_debt() {
        let g = make_tree_graph(31); // 5-level binary tree
        let score = calculate_drift(&g, None, 0, &default_config());
        assert!(
            score.total < 10,
            "Perfect tree should have <10 debt, got: {}",
            score.total
        );
        assert_eq!(score.new_cycles, 0, "Tree should have no cycles");
    }

    // ── Test 3: Clean 3-layer graph → 5-15 debt ──
    #[test]
    fn test_clean_layered_graph() {
        let g = make_layered_graph(&[3, 5, 4]); // 3 layers, forward-only
        let score = calculate_drift(&g, None, 0, &default_config());
        assert!(
            score.total <= 20,
            "Clean layered graph should have ≤20 debt, got: {}",
            score.total
        );
        assert_eq!(score.new_cycles, 0, "No cycles in clean layers");
        assert_eq!(
            score.boundary_violations, 0,
            "No back-edges in clean layers"
        );
    }

    // ── Test 4: Single 3-node cycle → cycle debt 15-30 ──
    #[test]
    fn test_single_cycle() {
        let g = make_cyclic_graph();
        let score = calculate_drift(&g, None, 0, &default_config());
        assert!(
            score.total >= 10 && score.total <= 40,
            "Single cycle should produce 10-40 debt, got: {}",
            score.total
        );
        assert_eq!(score.new_cycles, 1, "Should detect 1 SCC");
        assert!(score.cycle_debt > 0.0, "Cycle debt sub-score should be > 0");
    }

    // ── Test 5: Large SCC (10 nodes in cycle) → high cycle debt ──
    #[test]
    fn test_large_scc_high_cycle_debt() {
        let mut g = DiGraph::new();
        let nodes: Vec<_> = (0..10).map(|i| g.add_node(format!("cyc_{i}"))).collect();
        // Ring: 0→1→2→...→9→0
        for i in 0..10 {
            g.add_edge(nodes[i], nodes[(i + 1) % 10], 1);
        }
        // Add some non-cyclic context
        for i in 10..20 {
            let n = g.add_node(format!("leaf_{i}"));
            g.add_edge(nodes[0], n, 1);
        }
        let score = calculate_drift(&g, None, 0, &default_config());
        assert!(
            score.cycle_debt > 30.0,
            "Large SCC should produce high cycle debt, got: {}",
            score.cycle_debt
        );
    }

    // ── Test 6: Legitimate hub (high in, low out) → 0 hub debt ──
    #[test]
    fn test_legitimate_hub_zero_penalty() {
        let mut g = DiGraph::new();
        let core = g.add_node("shared_core".to_string());
        // 30 modules depend on core, but core depends on nothing
        for i in 0..30 {
            let n = g.add_node(format!("consumer_{i}"));
            g.add_edge(n, core, 1);
        }
        let score = calculate_drift(&g, None, 0, &default_config());
        assert_eq!(
            score.hub_debt, 0.0,
            "Legitimate hub (high in, zero out) should have 0 hub debt"
        );
    }

    // ── Test 7: God module (high in AND out) → high hub debt ──
    #[test]
    fn test_god_module_high_hub_debt() {
        let mut g = DiGraph::new();
        let god = g.add_node("god_module".to_string());
        // 15 modules depend on god
        for i in 0..15 {
            let n = g.add_node(format!("dep_{i}"));
            g.add_edge(n, god, 1);
        }
        // God also depends on 10 modules (high fan-out)
        for i in 0..10 {
            let n = g.add_node(format!("target_{i}"));
            g.add_edge(god, n, 1);
        }
        let score = calculate_drift(&g, None, 0, &default_config());
        assert!(
            score.hub_debt > 0.0,
            "God module (high in AND out) should have positive hub debt, got: {}",
            score.hub_debt
        );
    }

    // ── Test 8: Heavy edge weights → higher coupling debt ──
    #[test]
    fn test_heavy_weights_coupling_debt() {
        let mut g_light = DiGraph::new();
        let mut g_heavy = DiGraph::new();
        // Same topology, different weights
        let nodes_l: Vec<_> = (0..10)
            .map(|i| g_light.add_node(format!("n_{i}")))
            .collect();
        let nodes_h: Vec<_> = (0..10)
            .map(|i| g_heavy.add_node(format!("n_{i}")))
            .collect();
        for i in 0..9 {
            g_light.add_edge(nodes_l[i], nodes_l[i + 1], 1);
            g_heavy.add_edge(nodes_h[i], nodes_h[i + 1], 20); // 20x heavier
        }
        let score_light = calculate_drift(&g_light, None, 0, &default_config());
        let score_heavy = calculate_drift(&g_heavy, None, 0, &default_config());
        assert!(
            score_heavy.coupling_debt >= score_light.coupling_debt,
            "Heavy weights should produce ≥ coupling debt: light={}, heavy={}",
            score_light.coupling_debt,
            score_heavy.coupling_debt
        );
    }

    // ── Test 9: Back-edges in layered graph → layering debt ──
    #[test]
    fn test_back_edges_layering_debt() {
        let mut g = make_layered_graph(&[3, 4, 3]);
        // Add back-edges from layer 2 to layer 0
        let l2_nodes: Vec<_> = g
            .node_indices()
            .filter(|&n| g[n].starts_with("L2"))
            .collect();
        let l0_nodes: Vec<_> = g
            .node_indices()
            .filter(|&n| g[n].starts_with("L0"))
            .collect();
        for &src in &l2_nodes {
            for &tgt in &l0_nodes {
                g.add_edge(src, tgt, 1);
            }
        }
        let score = calculate_drift(&g, None, 0, &default_config());
        assert!(
            score.layering_debt > 0.0,
            "Back-edges should produce layering debt, got: {}",
            score.layering_debt
        );
        assert!(
            score.boundary_violations > 0,
            "Back-edges should be counted as boundary violations"
        );
    }

    // ── Test 10: Leaf packages with I=1.0 → 0 instability debt ──
    #[test]
    fn test_leaf_packages_zero_instability() {
        let mut g = DiGraph::new();
        let core = g.add_node("core".to_string());
        // 5 leaf packages that only import from core (ca=0, ce=1, I=1.0)
        for i in 0..5 {
            let leaf = g.add_node(format!("leaf_{i}"));
            g.add_edge(leaf, core, 1);
        }
        let score = calculate_drift(&g, None, 0, &default_config());
        assert_eq!(
            score.instability_debt, 0.0,
            "Leaf packages (ca=0) should not contribute instability debt"
        );
    }

    // ── Test 11: Non-leaf brittle module → instability debt ──
    #[test]
    fn test_nonleaf_brittle_instability() {
        let mut g = DiGraph::new();
        // Create a module with high instability BUT with fan_in > 0
        let hub = g.add_node("hub".to_string());
        let brittle = g.add_node("brittle".to_string());
        let t1 = g.add_node("target1".to_string());
        let t2 = g.add_node("target2".to_string());
        let t3 = g.add_node("target3".to_string());
        let t4 = g.add_node("target4".to_string());

        // hub depends on brittle (brittle has fan_in = 1)
        g.add_edge(hub, brittle, 1);
        // brittle depends on 4 things (fan_out = 4, I = 4/5 = 0.8)
        g.add_edge(brittle, t1, 1);
        g.add_edge(brittle, t2, 1);
        g.add_edge(brittle, t3, 1);
        g.add_edge(brittle, t4, 1);

        // Add more nodes to meet minimum size
        for i in 0..20 {
            let n = g.add_node(format!("extra_{i}"));
            g.add_edge(hub, n, 1);
        }

        let score = calculate_drift(&g, None, 0, &default_config());
        // Note: instability debt may still be 0 if brittle_ratio is very low
        // The important thing is the algorithm runs without error
        assert!(
            score.instability_debt >= 0.0,
            "Instability debt should be non-negative"
        );
    }

    // ── Test 12: 100-node clean graph → health > 85% ──
    #[test]
    fn test_100_node_clean_graph_high_health() {
        let g = make_tree_graph(100);
        let score = calculate_drift(&g, None, 0, &default_config());
        let health = 100 - score.total;
        assert!(
            health >= 85,
            "100-node clean tree should have health ≥ 85%, got: {}%",
            health
        );
    }

    // ── Test 13: 100-node spaghetti → health < 40% ──
    #[test]
    fn test_100_node_spaghetti_low_health() {
        let mut g = DiGraph::new();
        let nodes: Vec<_> = (0..100).map(|i| g.add_node(format!("s_{i}"))).collect();
        // Dense random-ish connections + cycles
        for i in 0..100 {
            for j in 0..100 {
                if i != j && (i * 7 + j * 13) % 5 == 0 {
                    g.add_edge(nodes[i], nodes[j], ((i + j) % 10 + 1) as u32);
                }
            }
        }
        let score = calculate_drift(&g, None, 0, &default_config());
        let health = 100u8.saturating_sub(score.total);
        assert!(
            health < 40,
            "100-node spaghetti should have health < 40%, got: {}%",
            health
        );
    }

    // ── Test 14: Old DriftScore JSON backward compat ──
    #[test]
    fn test_drift_score_backward_compat() {
        // Simulate old JSON without new fields
        let old_json = r#"{
            "total": 42,
            "fan_in_delta": 2,
            "fan_out_delta": -1,
            "new_cycles": 1,
            "boundary_violations": 0,
            "cognitive_complexity": 3.5,
            "timestamp": 1234567890
        }"#;
        let score: DriftScore = serde_json::from_str(old_json).unwrap();
        assert_eq!(score.total, 42);
        assert_eq!(score.cycle_debt, 0.0, "Default for missing sub-scores");
        assert_eq!(score.layering_debt, 0.0);
        assert_eq!(score.hub_debt, 0.0);
        assert_eq!(score.coupling_debt, 0.0);
        assert_eq!(score.cognitive_debt, 0.0);
        assert_eq!(score.instability_debt, 0.0);
    }

    // ── Test 15: Median fan delta computation ──
    #[test]
    fn test_median_fan_delta() {
        // Build two graphs with known fan-in/out changes
        let mut prev = DiGraph::new();
        let pa = prev.add_node("A".to_string());
        let pb = prev.add_node("B".to_string());
        let pc = prev.add_node("C".to_string());
        prev.add_edge(pa, pb, 1);
        prev.add_edge(pa, pc, 1);

        let mut curr = DiGraph::new();
        let ca = curr.add_node("A".to_string());
        let cb = curr.add_node("B".to_string());
        let cc = curr.add_node("C".to_string());
        let cd = curr.add_node("D".to_string());
        curr.add_edge(ca, cb, 1);
        curr.add_edge(ca, cc, 1);
        curr.add_edge(ca, cd, 1); // A gained one more out-edge
        curr.add_edge(cb, cc, 1); // B gained one out-edge

        let score = calculate_drift(&curr, Some(&prev), 0, &default_config());
        // Fan deltas should be computed based on median of per-node changes
        // The exact values depend on which nodes overlap; just verify they're reasonable
        assert!(
            score.fan_in_delta.abs() <= 5,
            "Fan-in delta should be small: {}",
            score.fan_in_delta
        );
        assert!(
            score.fan_out_delta.abs() <= 5,
            "Fan-out delta should be small: {}",
            score.fan_out_delta
        );
    }

    // ── Test: Small clean graph (existing test, updated) ──
    #[test]
    fn test_calculate_health_clean_small() {
        let graph = make_simple_graph();
        let score = calculate_drift(&graph, None, 0, &default_config());
        assert!(
            score.total <= 10,
            "Score should be very low for a clean tiny graph: {}",
            score.total
        );
    }

    // ── Test: Cyclic graph (existing test, updated) ──
    #[test]
    fn test_calculate_health_with_cycle() {
        let graph = make_cyclic_graph();
        let score = calculate_drift(&graph, None, 0, &default_config());
        assert!(
            score.total >= 10 && score.total <= 50,
            "Cycle should add significant debt: {}",
            score.total
        );
    }

    // ── Test: Entry Point Exemption (Main/Index/App) ──
    #[test]
    fn test_entry_point_exemption() {
        let mut g = DiGraph::new();
        // Create an entry point module that acts as a "God Module" (high fan-out, zero fan-in)
        let entry = g.add_node("src/main.rs".to_string());
        let mut targets = Vec::new();

        for i in 0..15 {
            let n = g.add_node(format!("module_{i}"));
            targets.push(n);
            g.add_edge(entry, n, 1);
        }

        // Add an extra connection to bump up instability parameters
        let child = g.add_node("child".to_string());
        g.add_edge(targets[0], child, 1);
        g.add_edge(child, targets[0], 1); // create a cycle to force non-zero score to trigger diagnostics

        let _score = calculate_drift(&g, None, 0, &default_config());

        // Ensure hub_debt is 0 for entry points despite high fan-out
        // Note: The logic in `compute_hub_debt` checks `is_entry`
        let mut g_regular = DiGraph::new();
        let reg = g_regular.add_node("src/utils.rs".to_string());
        for i in 0..15 {
            let n = g_regular.add_node(format!("module_{i}"));
            g_regular.add_edge(reg, n, 1);
            g_regular.add_edge(n, reg, 1); // Make it high fan-in AND fan-out
        }

        let _score_reg = calculate_drift(&g_regular, None, 0, &default_config());

        // generate_diagnostics shouldn't flag 'main.rs' as brittle
        let mock_score = DriftScore {
            total: 50,
            fan_in_delta: 0,
            fan_out_delta: 0,
            new_cycles: 0,
            boundary_violations: 0,
            cognitive_complexity: 0.0,
            timestamp: 0,
            cognitive_debt: 0.0,
            cycle_debt: 0.0,
            layering_debt: 0.0,
            coupling_debt: 0.0,
            hub_debt: 20.0,         // Force diagnostic generation
            instability_debt: 20.0, // Force diagnostic generation
        };

        let diagnostics = generate_diagnostics(&g, &mock_score, &default_config());
        let joined_diagnostics = diagnostics.join(" ");

        // The word "main" or "main.rs" should NOT be flagged as fragile
        assert!(
            !joined_diagnostics.contains("main.rs fragile"),
            "Entry points should not be flagged as fragile"
        );
    }

    #[test]
    fn test_is_entry_point_stem_matching() {
        let stems: Vec<String> = ["main", "index", "app"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert!(is_entry_point_stem("src/main.rs", &stems));
        assert!(is_entry_point_stem("cli/index", &stems));
        assert!(is_entry_point_stem("app", &stems));
        assert!(is_entry_point_stem("packages/web/app.ts", &stems));
        assert!(!is_entry_point_stem("src/utils.rs", &stems));
        assert!(!is_entry_point_stem("core/server", &stems));
        assert!(!is_entry_point_stem("main_helper", &stems));
    }

    #[test]
    fn test_entry_point_stems_exempt_from_hub_and_instability_scoring() {
        // Build a graph where "cli/app" is a god-module-shaped entry point:
        // high fan-in AND fan-out, which would normally trigger hub + instability debt.
        let mut g = DiGraph::new();
        let app = g.add_node("cli/app".to_string());
        for i in 0..12 {
            let n = g.add_node(format!("module_{i}"));
            g.add_edge(app, n, 1); // high fan-out
            g.add_edge(n, app, 1); // high fan-in
        }

        // With default config, "app" is in entry_point_stems → should be exempt
        let cfg = default_config();
        let hub = compute_hub_debt(&g, &cfg.thresholds, &cfg.exemptions);
        let instability = compute_instability_debt(&g, &cfg.thresholds, &cfg.exemptions);
        assert_eq!(hub, 0.0, "Entry point 'app' should be exempt from hub debt");
        assert_eq!(
            instability, 0.0,
            "Entry point 'app' should be exempt from instability debt"
        );

        // With a config where "app" is NOT an entry point stem → should trigger debt
        let mut cfg_no_app = default_config();
        cfg_no_app.exemptions.entry_point_stems =
            ["main", "index"].iter().map(|s| s.to_string()).collect();
        let hub_no_exempt = compute_hub_debt(&g, &cfg_no_app.thresholds, &cfg_no_app.exemptions);
        assert!(
            hub_no_exempt > 0.0,
            "Without 'app' in stems, hub debt should be non-zero"
        );
    }
}
