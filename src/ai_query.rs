// =============================================================================
// ai_query.rs — AI-Powered Natural Language Query Engine
// =============================================================================
//
// Builds a compact JSON context from the current architecture state and sends
// natural language queries to an LLM (OpenAI-compatible or local Ollama).
//
// Supports both streaming (SSE) and non-streaming responses. The streaming
// path sends incremental text deltas via a channel for real-time TUI display.
// =============================================================================

use futures::StreamExt;
use serde::Serialize;
use serde_json::json;

use crate::config::AiConfig;
use crate::models::{BlastRadiusReport, DriftScore};
use crate::tui::ai_panel::StreamChunk;
use crate::tui::app::{FocusedPanel, InsightTab, ViewContext};

/// Compact architecture context sent to the LLM alongside the user's question.
#[derive(Debug, Serialize)]
pub struct ArchitectureContext {
    pub graph_summary: GraphSummary,
    pub drift_score: Option<DriftBreakdown>,
    pub blast_radius: Option<BlastSummary>,
    pub module_distribution: Option<ModuleDistribution>,
    pub top_brittle_modules: Vec<BrittleModule>,
    pub clusters: Vec<ClusterSummary>,
    pub cluster_couplings: Vec<ClusterCoupling>,
    pub cycle_summary: Option<CycleSummary>,
    pub cycle_groups: Vec<CycleGroup>,
    pub edge_distribution: Option<EdgeDistribution>,
    pub heaviest_edges: Vec<HeavyEdge>,
    pub diagnostics: Vec<String>,
    pub current_commit: Option<CommitContext>,
    pub trend: TrendData,
    pub scoring_context: Option<ScoringContext>,
    // Phase 2 enrichments
    pub cognitive_detail: Option<CognitiveDetail>,
    pub god_modules: Vec<GodModule>,
    pub boundary_violations: Vec<BoundaryViolation>,
    pub layer_topology: Option<LayerTopology>,
    // Phase 3 enrichments
    pub module_file_metrics: Vec<ModuleFileMetrics>,
    pub churn_hotspots: Vec<ChurnHotspot>,
    pub bus_factor_risks: Vec<BusFactorRisk>,
    // Phase 4B: temporal intelligence
    pub recent_diff: Option<SnapshotDiff>,
    // Phase 4C: focused module context
    pub focused_module_detail: Option<FocusedModuleDetail>,
}

#[derive(Debug, Serialize)]
pub struct GraphSummary {
    pub node_count: usize,
    pub edge_count: usize,
    pub top_hubs: Vec<HubModule>,
}

#[derive(Debug, Serialize)]
pub struct HubModule {
    pub name: String,
    pub fan_in: usize,
    pub fan_out: usize,
}

#[derive(Debug, Serialize)]
pub struct DriftBreakdown {
    pub total: u8,
    pub health_percent: u8,
    pub cycle_debt: f64,
    pub layering_debt: f64,
    pub hub_debt: f64,
    pub coupling_debt: f64,
    pub cognitive_debt: f64,
    pub instability_debt: f64,
    pub cycle_count: usize,
    pub boundary_violations: usize,
    pub layering_violations: usize,
    pub fan_in_delta: i32,
    pub fan_out_delta: i32,
    pub cognitive_complexity: f64,
}

#[derive(Debug, Serialize)]
pub struct BlastSummary {
    pub articulation_point_count: usize,
    pub most_impactful_module: String,
    pub max_blast_score: f64,
    pub mean_blast_score: f64,
    pub longest_chain_depth: usize,
    pub p50_blast: f64,
    pub p75_blast: f64,
    pub p90_blast: f64,
    pub modules_above_03: usize,
    pub modules_above_05: usize,
    pub top_impacts: Vec<ImpactEntry>,
    pub articulation_points: Vec<ArticulationPointEntry>,
    pub critical_chains: Vec<CriticalChainEntry>,
}

#[derive(Debug, Serialize)]
pub struct ImpactEntry {
    pub module: String,
    pub blast_score: f64,
    pub downstream_count: usize,
    pub weighted_reach: f64,
    pub is_keystone: bool,
}

#[derive(Debug, Serialize)]
pub struct ArticulationPointEntry {
    pub module: String,
    pub components_bridged: usize,
    pub fan_in: usize,
    pub fan_out: usize,
}

#[derive(Debug, Serialize)]
pub struct CriticalChainEntry {
    pub chain: Vec<String>,
    pub total_weight: u32,
    pub depth: usize,
}

#[derive(Debug, Serialize)]
pub struct BrittleModule {
    pub name: String,
    pub instability: f64,
    pub fan_in: usize,
    pub fan_out: usize,
}

// ─── Phase 1 enrichment structs ─────────────────────────────────────────────

/// Statistical distribution of module instability across the entire graph.
#[derive(Debug, Serialize)]
pub struct ModuleDistribution {
    pub total_modules: usize,
    pub brittle_count: usize,
    pub stable_count: usize,
    pub mean_instability: f64,
    pub median_instability: f64,
    pub p90_instability: f64,
}

/// Statistical distribution of edge weights across the entire graph.
#[derive(Debug, Serialize)]
pub struct EdgeDistribution {
    pub total_edges: usize,
    pub mean_weight: f64,
    pub median_weight: f64,
    pub p95_weight: f64,
    pub max_weight: u32,
    pub heavy_edge_count: usize,
}

/// Summary metadata about strongly-connected components (cycles).
#[derive(Debug, Serialize)]
pub struct CycleSummary {
    pub total_scc_count: usize,
    pub total_cyclic_nodes: usize,
    pub largest_scc_size: usize,
    pub scc_sizes: Vec<usize>,
}

/// Scoring algorithm configuration context — lets the AI explain scoring decisions.
#[derive(Debug, Serialize)]
pub struct ScoringContext {
    pub weights: ScoringWeights,
    pub hub_exempt: Vec<String>,
    pub instability_exempt: Vec<String>,
    pub entry_point_stems: Vec<String>,
    pub boundary_rules_count: usize,
    pub hub_exemption_ratio: f64,
    pub entry_point_max_fan_in: usize,
    pub brittle_instability_ratio: f64,
}

#[derive(Debug, Serialize)]
pub struct ScoringWeights {
    pub cycle: f64,
    pub layering: f64,
    pub hub: f64,
    pub coupling: f64,
    pub cognitive: f64,
    pub instability: f64,
}

/// Per-component trend decomposition over time.
#[derive(Debug, Serialize)]
pub struct TrendData {
    pub total: Vec<u8>,
    pub cycle_debt: Vec<f64>,
    pub layering_debt: Vec<f64>,
    pub hub_debt: Vec<f64>,
    pub coupling_debt: Vec<f64>,
    pub cognitive_debt: Vec<f64>,
    pub instability_debt: Vec<f64>,
}

// ─── Phase 2 enrichment structs ─────────────────────────────────────────────

/// Breakdown of cognitive debt sub-factors.
#[derive(Debug, Serialize)]
pub struct CognitiveDetail {
    pub edge_excess_ratio: f64,
    pub degree_excess: f64,
    pub avg_degree: f64,
    pub expected_avg_degree: f64,
    pub baseline_edges: usize,
    pub actual_edges: usize,
    pub scale_factor: f64,
}

/// A god module flagged by the hub debt algorithm.
#[derive(Debug, Serialize)]
pub struct GodModule {
    pub name: String,
    pub fan_in: usize,
    pub fan_out: usize,
    pub hub_ratio: f64,
    pub excess_ratio: f64,
}

/// A boundary rule violation with edge and rule details.
#[derive(Debug, Serialize)]
pub struct BoundaryViolation {
    pub from_module: String,
    pub to_module: String,
    pub rule_from_pattern: String,
    pub rule_deny_patterns: Vec<String>,
}

/// Simplified DAG of architectural layers with upward violations.
#[derive(Debug, Serialize)]
pub struct LayerTopology {
    pub layers: Vec<LayerLevel>,
    pub upward_violations: Vec<LayerViolation>,
}

#[derive(Debug, Serialize)]
pub struct LayerLevel {
    pub level: usize,
    pub cluster_names: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct LayerViolation {
    pub from_cluster: String,
    pub from_layer: usize,
    pub to_cluster: String,
    pub to_layer: usize,
}

// ─── Phase 3 enrichment structs ─────────────────────────────────────────────

/// Aggregated file-level metrics per module (function/type counts from tree-sitter).
#[derive(Debug, Serialize)]
pub struct ModuleFileMetrics {
    pub module_name: String,
    pub file_count: usize,
    pub total_functions: u32,
    pub total_types: u32,
    pub avg_functions_per_file: f64,
    pub avg_complexity: f32,
}

/// A churn hotspot — module with high change frequency and instability.
#[derive(Debug, Serialize)]
pub struct ChurnHotspot {
    pub module_name: String,
    pub touch_count: u32,
    pub instability: f64,
    /// risk = churn × instability (higher = more dangerous)
    pub risk_score: f64,
}

/// A module at bus factor risk — few unique contributors.
#[derive(Debug, Serialize)]
pub struct BusFactorRisk {
    pub module_name: String,
    pub unique_authors: usize,
    pub top_author: String,
    pub fan_in: usize,
}

// ─── Phase 4B: Temporal Intelligence ─────────────────────────────────────────

/// Compact diff between current and previous snapshot for temporal AI analysis.
#[derive(Debug, Serialize)]
pub struct SnapshotDiff {
    pub added_modules: Vec<String>,
    pub removed_modules: Vec<String>,
    pub added_edges: Vec<AddedEdge>,
    pub removed_edges: Vec<RemovedEdge>,
    pub weight_changes: Vec<WeightChange>,
    pub drift_deltas: Option<DriftComponentDeltas>,
    pub modules_changed_count: usize,
}

#[derive(Debug, Serialize)]
pub struct AddedEdge {
    pub from: String,
    pub to: String,
    pub weight: u32,
}

#[derive(Debug, Serialize)]
pub struct RemovedEdge {
    pub from: String,
    pub to: String,
}

#[derive(Debug, Serialize)]
pub struct WeightChange {
    pub from: String,
    pub to: String,
    pub old_weight: u32,
    pub new_weight: u32,
    pub delta: i32,
}

#[derive(Debug, Serialize)]
pub struct DriftComponentDeltas {
    pub total: i8,
    pub cycle_debt: f64,
    pub layering_debt: f64,
    pub hub_debt: f64,
    pub coupling_debt: f64,
    pub cognitive_debt: f64,
    pub instability_debt: f64,
}

// ─── Phase 4C: Focused Module Context ────────────────────────────────────────

/// Detailed structured data about the module the user is currently inspecting.
/// Provides ALL edges and module-specific metrics for deep AI analysis.
#[derive(Debug, Serialize)]
pub struct FocusedModuleDetail {
    pub module_name: String,
    pub instability: f64,
    pub fan_in: usize,
    pub fan_out: usize,
    pub blast_score: Option<f64>,
    pub is_keystone: bool,
    pub cluster: Option<String>,
    pub all_inbound_edges: Vec<ModuleEdge>,
    pub all_outbound_edges: Vec<ModuleEdge>,
    pub churn_count: Option<u32>,
    pub bus_factor: Option<BusFactorInfo>,
    pub in_cycle: bool,
    pub cycle_partners: Vec<String>,
    pub function_count: Option<u32>,
    pub type_count: Option<u32>,
    pub complexity: Option<u32>,
}

#[derive(Debug, Serialize)]
pub struct ModuleEdge {
    pub module: String,
    pub weight: u32,
}

#[derive(Debug, Serialize)]
pub struct BusFactorInfo {
    pub unique_authors: usize,
    pub top_author: String,
}

// ─── Existing structs ───────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct ClusterSummary {
    pub name: String,
    pub kind: String,
    pub layer: usize,
    pub role: String,
    pub member_count: usize,
    pub internal_count: usize,
    pub external_count: usize,
    pub inbound_weight: u32,
    pub outbound_weight: u32,
    pub internal_weight: u32,
    pub top_members: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ClusterCoupling {
    pub from: String,
    pub to: String,
    pub total_weight: u32,
    pub edge_count: usize,
}

#[derive(Debug, Serialize)]
pub struct CycleGroup {
    pub members: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct HeavyEdge {
    pub from: String,
    pub to: String,
    pub weight: u32,
}

#[derive(Debug, Serialize)]
pub struct CommitContext {
    pub short_hash: String,
    pub message: String,
    pub timestamp: i64,
}

/// Simplified cluster info passed from the TUI layer.
#[derive(Debug, Clone)]
pub struct ClusterInfo {
    pub name: String,
    pub kind: String,
    pub layer: usize,
    pub role: String,
    pub member_count: usize,
    pub internal_count: usize,
    pub external_count: usize,
    pub inbound_weight: u32,
    pub outbound_weight: u32,
    pub internal_weight: u32,
    pub top_members: Vec<String>,
}

/// What the user is currently looking at in the TUI.
#[derive(Debug, Serialize)]
pub struct FocusedContext {
    pub current_view: String,
    pub focused_panel: String,
    pub insight_tab: String,
    pub hovered_node: Option<String>,
    pub selected_cluster: Option<String>,
}

// ─── Percentile helper ──────────────────────────────────────────────────────

fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let idx = (p / 100.0 * (sorted.len() - 1) as f64).round() as usize;
    let idx = idx.min(sorted.len() - 1);
    (sorted[idx] * 100.0).round() / 100.0
}

fn percentile_u32(sorted: &[u32], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let idx = (p / 100.0 * (sorted.len() - 1) as f64).round() as usize;
    let idx = idx.min(sorted.len() - 1);
    sorted[idx] as f64
}

pub fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

// ─── Snapshot diff computation ───────────────────────────────────────────────

/// Compute the diff between two graph snapshots for temporal AI analysis.
pub fn compute_snapshot_diff(
    current: &crate::models::GraphSnapshot,
    previous: &crate::models::GraphSnapshot,
) -> SnapshotDiff {
    // 1. Module diff: compare node sets
    use std::collections::HashSet;
    let cur_nodes: HashSet<&str> = current.nodes.iter().map(|s| s.as_str()).collect();
    let prev_nodes: HashSet<&str> = previous.nodes.iter().map(|s| s.as_str()).collect();

    let added_modules: Vec<String> = cur_nodes
        .difference(&prev_nodes)
        .map(|s| s.to_string())
        .collect();
    let removed_modules: Vec<String> = prev_nodes
        .difference(&cur_nodes)
        .map(|s| s.to_string())
        .collect();

    // 2. Edge diff: compare (from, to) -> weight maps
    use std::collections::HashMap;
    let cur_edges: HashMap<(&str, &str), u32> = current
        .edges
        .iter()
        .map(|e| ((e.from_module.as_str(), e.to_module.as_str()), e.weight))
        .collect();
    let prev_edges: HashMap<(&str, &str), u32> = previous
        .edges
        .iter()
        .map(|e| ((e.from_module.as_str(), e.to_module.as_str()), e.weight))
        .collect();

    let mut added_edges = Vec::new();
    let mut removed_edges = Vec::new();
    let mut weight_changes = Vec::new();

    for (&(from, to), &cur_w) in &cur_edges {
        match prev_edges.get(&(from, to)) {
            None => added_edges.push(AddedEdge {
                from: from.to_string(),
                to: to.to_string(),
                weight: cur_w,
            }),
            Some(&prev_w) if prev_w != cur_w => {
                weight_changes.push(WeightChange {
                    from: from.to_string(),
                    to: to.to_string(),
                    old_weight: prev_w,
                    new_weight: cur_w,
                    delta: cur_w as i32 - prev_w as i32,
                });
            }
            _ => {}
        }
    }
    for &(from, to) in prev_edges.keys() {
        if !cur_edges.contains_key(&(from, to)) {
            removed_edges.push(RemovedEdge {
                from: from.to_string(),
                to: to.to_string(),
            });
        }
    }

    // Sort by significance
    added_edges.sort_by(|a, b| b.weight.cmp(&a.weight));
    weight_changes.sort_by(|a, b| b.delta.abs().cmp(&a.delta.abs()));

    // Truncate for token budget
    added_edges.truncate(15);
    removed_edges.truncate(15);
    weight_changes.truncate(10);

    // 3. Drift deltas
    let drift_deltas = match (current.drift.as_ref(), previous.drift.as_ref()) {
        (Some(cur), Some(prev)) => Some(DriftComponentDeltas {
            total: cur.total as i8 - prev.total as i8,
            cycle_debt: round2(cur.cycle_debt - prev.cycle_debt),
            layering_debt: round2(cur.layering_debt - prev.layering_debt),
            hub_debt: round2(cur.hub_debt - prev.hub_debt),
            coupling_debt: round2(cur.coupling_debt - prev.coupling_debt),
            cognitive_debt: round2(cur.cognitive_debt - prev.cognitive_debt),
            instability_debt: round2(cur.instability_debt - prev.instability_debt),
        }),
        _ => None,
    };

    let modules_changed_count = added_modules.len() + removed_modules.len();

    SnapshotDiff {
        added_modules,
        removed_modules,
        added_edges,
        removed_edges,
        weight_changes,
        drift_deltas,
        modules_changed_count,
    }
}

// ─── Context builder ────────────────────────────────────────────────────────

/// Phase 2 enrichments passed from the TUI layer.
pub struct Phase2Enrichments {
    pub cognitive_detail: Option<CognitiveDetail>,
    pub god_modules: Vec<GodModule>,
    pub boundary_violations: Vec<BoundaryViolation>,
    pub layer_topology: Option<LayerTopology>,
}

/// Phase 3 enrichments passed from the TUI layer.
pub struct Phase3Enrichments {
    pub module_file_metrics: Vec<ModuleFileMetrics>,
    pub churn_hotspots: Vec<ChurnHotspot>,
    pub bus_factor_risks: Vec<BusFactorRisk>,
}

/// Builds a compact JSON context from the current architecture state.
#[allow(clippy::too_many_arguments)]
pub fn build_architecture_context(
    node_count: usize,
    edge_count: usize,
    drift: Option<&DriftScore>,
    blast_radius: Option<&BlastRadiusReport>,
    instability_metrics: &[(String, f64, usize, usize)],
    clusters: &[ClusterInfo],
    cluster_couplings: &[ClusterCoupling],
    cycle_groups: &[Vec<String>],
    heaviest_edges: &[HeavyEdge],
    diagnostics: &[String],
    current_commit: Option<CommitContext>,
    trend: TrendData,
    edge_weights: &[u32],
    scoring_context: Option<ScoringContext>,
    phase2: Phase2Enrichments,
    phase3: Phase3Enrichments,
    recent_diff: Option<SnapshotDiff>,
    focused_module_detail: Option<FocusedModuleDetail>,
) -> ArchitectureContext {
    let mut top_hubs: Vec<HubModule> = instability_metrics
        .iter()
        .map(|(name, _, fan_in, fan_out)| HubModule {
            name: name.clone(),
            fan_in: *fan_in,
            fan_out: *fan_out,
        })
        .take(10)
        .collect();
    top_hubs.sort_by(|a, b| (b.fan_in + b.fan_out).cmp(&(a.fan_in + a.fan_out)));
    top_hubs.truncate(10);

    let drift_breakdown = drift.map(|d| DriftBreakdown {
        total: d.total,
        health_percent: 100u8.saturating_sub(d.total),
        cycle_debt: d.cycle_debt,
        layering_debt: d.layering_debt,
        hub_debt: d.hub_debt,
        coupling_debt: d.coupling_debt,
        cognitive_debt: d.cognitive_debt,
        instability_debt: d.instability_debt,
        cycle_count: d.new_cycles,
        boundary_violations: d.boundary_violations,
        layering_violations: d.layering_violations,
        fan_in_delta: d.fan_in_delta,
        fan_out_delta: d.fan_out_delta,
        cognitive_complexity: round2(d.cognitive_complexity),
    });

    // ── Blast radius with distribution stats (Phase 1.7) ──
    let blast_summary = blast_radius.map(|br| {
        let top_impacts: Vec<ImpactEntry> = br
            .impacts
            .iter()
            .take(15)
            .map(|m| ImpactEntry {
                module: m.module_name.clone(),
                blast_score: round2(m.blast_score),
                downstream_count: m.downstream_count,
                weighted_reach: round2(m.weighted_reach),
                is_keystone: m.is_articulation_point,
            })
            .collect();

        let articulation_points: Vec<ArticulationPointEntry> = br
            .articulation_points
            .iter()
            .take(8)
            .map(|ap| ArticulationPointEntry {
                module: ap.module_name.clone(),
                components_bridged: ap.components_bridged,
                fan_in: ap.fan_in,
                fan_out: ap.fan_out,
            })
            .collect();

        let critical_chains: Vec<CriticalChainEntry> = br
            .critical_paths
            .iter()
            .take(8)
            .map(|p| CriticalChainEntry {
                chain: p.chain.clone(),
                total_weight: p.total_weight,
                depth: p.depth,
            })
            .collect();

        // Percentile distribution from all blast scores
        let mut blast_scores: Vec<f64> = br.impacts.iter().map(|m| m.blast_score).collect();
        blast_scores.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let modules_above_03 = blast_scores.iter().filter(|&&s| s > 0.3).count();
        let modules_above_05 = blast_scores.iter().filter(|&&s| s > 0.5).count();

        BlastSummary {
            articulation_point_count: br.summary.articulation_point_count,
            most_impactful_module: br.summary.most_impactful_module.clone(),
            max_blast_score: round2(br.summary.max_blast_score),
            mean_blast_score: round2(br.summary.mean_blast_score),
            longest_chain_depth: br.summary.longest_chain_depth,
            p50_blast: percentile(&blast_scores, 50.0),
            p75_blast: percentile(&blast_scores, 75.0),
            p90_blast: percentile(&blast_scores, 90.0),
            modules_above_03,
            modules_above_05,
            top_impacts,
            articulation_points,
            critical_chains,
        }
    });

    // ── Module distribution stats (Phase 1.1) ──
    let module_distribution = if instability_metrics.is_empty() {
        None
    } else {
        let total = instability_metrics.len();
        let mut inst_vals: Vec<f64> = instability_metrics.iter().map(|(_, i, _, _)| *i).collect();
        inst_vals.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let brittle_count = inst_vals.iter().filter(|&&i| i > 0.5).count();
        let stable_count = inst_vals.iter().filter(|&&i| i <= 0.3).count();
        let mean = inst_vals.iter().sum::<f64>() / total as f64;

        Some(ModuleDistribution {
            total_modules: total,
            brittle_count,
            stable_count,
            mean_instability: round2(mean),
            median_instability: percentile(&inst_vals, 50.0),
            p90_instability: percentile(&inst_vals, 90.0),
        })
    };

    // ── Brittle modules (increased to top 15) ──
    let top_brittle: Vec<BrittleModule> = instability_metrics
        .iter()
        .filter(|(_, inst, fan_in, _)| *inst > 0.5 && *fan_in > 0)
        .take(15)
        .map(|(name, inst, fan_in, fan_out)| BrittleModule {
            name: name.clone(),
            instability: round2(*inst),
            fan_in: *fan_in,
            fan_out: *fan_out,
        })
        .collect();

    // ── Edge weight distribution (Phase 1.2) ──
    let edge_distribution = if edge_weights.is_empty() {
        None
    } else {
        let mut sorted_weights = edge_weights.to_vec();
        sorted_weights.sort_unstable();
        let total = sorted_weights.len();
        let sum: u64 = sorted_weights.iter().map(|&w| w as u64).sum();
        let mean = sum as f64 / total as f64;
        let heavy_count = sorted_weights.iter().filter(|&&w| w > 3).count();

        Some(EdgeDistribution {
            total_edges: total,
            mean_weight: round2(mean),
            median_weight: percentile_u32(&sorted_weights, 50.0),
            p95_weight: percentile_u32(&sorted_weights, 95.0),
            max_weight: sorted_weights.last().copied().unwrap_or(0),
            heavy_edge_count: heavy_count,
        })
    };

    // ── Cluster summaries with members and roles (Phase 1.6) ──
    let cluster_summaries: Vec<ClusterSummary> = clusters
        .iter()
        .map(|c| ClusterSummary {
            name: c.name.clone(),
            kind: c.kind.clone(),
            layer: c.layer,
            role: c.role.clone(),
            member_count: c.member_count,
            internal_count: c.internal_count,
            external_count: c.external_count,
            inbound_weight: c.inbound_weight,
            outbound_weight: c.outbound_weight,
            internal_weight: c.internal_weight,
            top_members: c.top_members.clone(),
        })
        .collect();

    // ── Cycle summary metadata (Phase 1.3) ──
    let cycle_summary = if cycle_groups.is_empty() {
        None
    } else {
        let mut scc_sizes: Vec<usize> = cycle_groups.iter().map(|g| g.len()).collect();
        scc_sizes.sort_unstable_by(|a, b| b.cmp(a)); // descending
        let total_cyclic: usize = scc_sizes.iter().sum();
        let largest = scc_sizes.first().copied().unwrap_or(0);

        Some(CycleSummary {
            total_scc_count: scc_sizes.len(),
            total_cyclic_nodes: total_cyclic,
            largest_scc_size: largest,
            scc_sizes: scc_sizes.into_iter().take(20).collect(),
        })
    };

    // Cycle groups: increased to 10, members still capped at 10
    let cycle_group_entries: Vec<CycleGroup> = cycle_groups
        .iter()
        .take(10)
        .map(|members| CycleGroup {
            members: members.iter().take(10).cloned().collect(),
        })
        .collect();

    ArchitectureContext {
        graph_summary: GraphSummary {
            node_count,
            edge_count,
            top_hubs,
        },
        drift_score: drift_breakdown,
        blast_radius: blast_summary,
        module_distribution,
        top_brittle_modules: top_brittle,
        clusters: cluster_summaries,
        cluster_couplings: cluster_couplings.to_vec(),
        cycle_summary,
        cycle_groups: cycle_group_entries,
        edge_distribution,
        heaviest_edges: heaviest_edges.to_vec(),
        diagnostics: diagnostics.iter().take(25).cloned().collect(),
        current_commit,
        trend,
        scoring_context,
        // Phase 2
        cognitive_detail: phase2.cognitive_detail,
        god_modules: phase2.god_modules,
        boundary_violations: phase2.boundary_violations,
        layer_topology: phase2.layer_topology,
        // Phase 3
        module_file_metrics: phase3.module_file_metrics,
        churn_hotspots: phase3.churn_hotspots,
        bus_factor_risks: phase3.bus_factor_risks,
        // Phase 4B
        recent_diff,
        // Phase 4C
        focused_module_detail,
    }
}

/// Estimate token count from JSON byte length (~4 chars per token for JSON).
fn estimate_tokens(json_bytes: usize) -> usize {
    json_bytes / 4
}

/// Trims lower-priority context fields to stay within the token budget.
/// Priority (lowest trimmed first): bus_factor, churn, file_metrics, diagnostics,
/// boundary_violations, cycle_groups, heaviest_edges.
pub fn compress_context(ctx: &mut ArchitectureContext, max_tokens: usize) {
    fn ctx_tokens(ctx: &ArchitectureContext) -> usize {
        serde_json::to_string(ctx)
            .map(|s| estimate_tokens(s.len()))
            .unwrap_or(0)
    }

    if ctx_tokens(ctx) <= max_tokens {
        return;
    }

    // Tier 1: trim low-priority Phase 3 data
    ctx.bus_factor_risks.truncate(5);
    ctx.churn_hotspots.truncate(8);
    ctx.module_file_metrics.truncate(10);
    if ctx_tokens(ctx) <= max_tokens {
        return;
    }

    // Tier 2: trim supporting detail
    ctx.diagnostics.truncate(10);
    ctx.boundary_violations.truncate(5);
    ctx.heaviest_edges.truncate(8);
    if let Some(ref mut diff) = ctx.recent_diff {
        diff.added_edges.truncate(8);
        diff.removed_edges.truncate(8);
        diff.weight_changes.truncate(5);
    }
    if ctx_tokens(ctx) <= max_tokens {
        return;
    }

    // Tier 3: trim more aggressively
    ctx.cycle_groups.truncate(5);
    ctx.bus_factor_risks.clear();
    ctx.module_file_metrics.truncate(5);
    ctx.churn_hotspots.truncate(5);
    if let Some(ref mut blast) = ctx.blast_radius {
        blast.critical_chains.truncate(3);
        blast.top_impacts.truncate(8);
        blast.articulation_points.truncate(5);
    }
    // Truncate focused module edges in tight budgets
    if let Some(ref mut detail) = ctx.focused_module_detail {
        detail.all_inbound_edges.truncate(20);
        detail.all_outbound_edges.truncate(20);
    }
    if ctx_tokens(ctx) <= max_tokens {
        return;
    }

    // Tier 4: clear remaining bulky data
    ctx.module_file_metrics.clear();
    ctx.churn_hotspots.clear();
    ctx.recent_diff = None;
    ctx.diagnostics.truncate(5);
    if let Some(ref mut detail) = ctx.focused_module_detail {
        detail.all_inbound_edges.truncate(10);
        detail.all_outbound_edges.truncate(10);
        detail.cycle_partners.truncate(5);
    }
    ctx.clusters
        .iter_mut()
        .for_each(|c| c.top_members.truncate(5));

    fn trim_tail<T>(v: &mut Vec<T>, max: usize) {
        if v.len() > max {
            *v = v.split_off(v.len() - max);
        }
    }
    trim_tail(&mut ctx.trend.total, 20);
    trim_tail(&mut ctx.trend.cycle_debt, 20);
    trim_tail(&mut ctx.trend.layering_debt, 20);
    trim_tail(&mut ctx.trend.hub_debt, 20);
    trim_tail(&mut ctx.trend.coupling_debt, 20);
    trim_tail(&mut ctx.trend.cognitive_debt, 20);
    trim_tail(&mut ctx.trend.instability_debt, 20);
}

/// Builds focused context from the current TUI view state.
pub fn build_focused_context(
    nav_stack: &[ViewContext],
    focused_panel: &FocusedPanel,
    insight_tab: &InsightTab,
    hovered_node: Option<String>,
    selected_cluster: Option<String>,
) -> FocusedContext {
    let current_view = match nav_stack.last() {
        Some(ViewContext::PackageDetail(name)) => format!("Cluster: {}", name),
        Some(ViewContext::ModuleInspect(name)) => format!("Module: {}", name),
        _ => "Overview".to_string(),
    };

    FocusedContext {
        current_view,
        focused_panel: format!("{:?}", focused_panel),
        insight_tab: format!("{:?}", insight_tab),
        hovered_node,
        selected_cluster,
    }
}

const SYSTEM_PROMPT: &str = "\
You are MorphArch AI, an expert software architecture analyst embedded in a terminal UI.

You receive a JSON snapshot of a codebase's dependency graph analysis and answer user questions about it.

RULES:
1. Answer in the SAME LANGUAGE as the user's question (Turkish -> Turkish, English -> English, etc.).
2. Be thorough but concise. Use 2-5 sentences for simple questions, more for complex analysis.
3. Always reference SPECIFIC module names, cluster names, and numeric values from the data.
4. When discussing risk, cite blast_score, instability, fan_in, fan_out values.
5. For trend questions, analyze the per-component trend arrays (cycle_debt, layering_debt, etc.) not just total scores.
6. DO NOT invent data not in the context. Say \"no data available\" if missing.
7. DO NOT use markdown formatting (no **, ##, etc.) - output plain text only.
8. Use simple numbered lists (1. 2. 3.) when listing items.
9. When identifying problems, explain WHY they are problems and suggest SPECIFIC refactoring steps.
10. Prioritize actionable advice: what to change, where, and why.
11. Use module_distribution stats to contextualize individual module metrics (e.g. \"this module's instability 0.85 is in the top 10% — p90 is 0.72\").
12. Use edge_distribution to identify coupling outliers (e.g. \"weight 15 is extreme — p95 is only 5\").
13. Use scoring_context to explain WHY certain modules are exempt from penalties (entry points, shared cores).
14. Use cluster roles and top_members to explain architectural organization.
15. Use cycle_summary to quantify cycle severity (total cyclic nodes as % of graph, largest SCC size).
16. Use blast_radius percentiles (p50, p75, p90) to contextualize individual blast scores.
17. Use cognitive_detail to explain WHY cognitive debt is high (edge excess vs degree excess, actual vs baseline edges).
18. Use god_modules to name the specific modules causing hub debt and explain their fan_in/fan_out/hub_ratio.
19. Use boundary_violations to show which specific edges violate configured architectural boundaries.
20. Use layer_topology to explain the layering structure and identify upward violations (dependencies going the wrong way).
21. Use module_file_metrics to contextualize module complexity (function/type counts, file density).
22. Use churn_hotspots to identify high-risk modules (frequently changed AND unstable = highest refactoring priority).
23. Use bus_factor_risks to flag knowledge concentration risks (modules with <=2 contributors AND high fan_in).
24. Use recent_diff to explain what changed between the current and previous snapshot — added/removed modules, new/removed edges, weight changes, and drift component deltas.
25. When focused_module_detail is present, provide DEEP analysis of that specific module: explain its role based on edges, identify whether it's a hub/leaf/bridge, analyze its blast radius context, and give targeted refactoring advice.

CONTEXT DATA:
- graph_summary: node/edge counts, top hub modules (by fan_in + fan_out)
- drift_score: 0-100 debt score with 6 sub-components, fan_in/fan_out deltas, layering violations, cognitive complexity
- blast_radius: articulation points (keystones), top impact modules, critical chains, PLUS percentile distribution (p50/p75/p90 blast scores, count of modules above 0.3 and 0.5 thresholds)
- module_distribution: statistical summary of ALL modules — total count, brittle/stable counts, mean/median/p90 instability
- top_brittle_modules: top 15 modules with instability > 0.5 and fan_in > 0
- edge_distribution: edge weight stats — mean, median, p95, max, count of heavy edges (weight > 3)
- clusters: architectural groupings with kind, layer, role (PrimaryArchitecture/SupportCluster/ExternalSink), top member modules
- cluster_couplings: inter-cluster dependency edges (from, to, weight, edge_count)
- cycle_summary: total SCC count, total cyclic nodes, largest SCC size, all SCC sizes
- cycle_groups: specific modules forming each circular dependency (up to 10 SCCs, 10 members each)
- heaviest_edges: top 15 highest-weight dependency edges
- diagnostics: pre-computed analysis advisories naming specific problematic modules
- scoring_context: scoring algorithm weights (cycle:30%, layering:25%, etc.), exempted modules, entry point stems, boundary rule count, thresholds
- cognitive_detail: edge_excess_ratio, degree_excess, avg_degree vs expected, baseline vs actual edges, scale factor — explains WHY cognitive debt score is what it is
- god_modules: specific modules flagged as god modules by hub debt — name, fan_in, fan_out, hub_ratio, excess_ratio
- boundary_violations: specific edges crossing configured architectural boundaries — from_module, to_module, rule patterns
- layer_topology: cluster DAG organized by layer level, plus upward violations (edges from higher to lower layer)
- module_file_metrics: per-module aggregated file complexity — file_count, total_functions, total_types, avg_functions_per_file (top 20 by total_functions)
- churn_hotspots: modules with high change frequency AND instability — touch_count, instability, risk_score (risk = churn × instability, top 15)
- bus_factor_risks: modules with <=2 unique contributors AND fan_in > 0 — unique_authors, top_author, fan_in (knowledge concentration risks)
- recent_diff: diff between current and previous snapshot — added/removed modules, added/removed/changed edges, drift component deltas (positive = worsening)
- focused_module_detail: when user inspects a specific module — full edge lists (inbound/outbound with weights), instability, blast_score, cycle membership, churn, bus_factor, function/type counts
- current_commit: hash and message of the currently viewed commit
- trend: per-component historical trends (total, cycle_debt, layering_debt, hub_debt, coupling_debt, cognitive_debt, instability_debt — oldest first)
- user_focus: what the user is currently looking at in the TUI (view, panel, tab, hovered node, selected cluster)

EXAMPLE:
Q: What causes the most cycle debt?
A: The primary cycle debt contributor is the strongly-connected component containing modules X, Y, and Z (cycle_debt: 45.2 out of max 30 weight). These form a circular dependency chain: X imports Y for type definitions, Y imports Z for utility functions, and Z imports X for configuration constants. To break this cycle, extract the shared types and constants into a separate module (e.g., types.rs or shared.rs) that all three can import without creating back-edges. This would eliminate the largest SCC and reduce cycle debt by approximately 60%.";

/// Builds the messages array including conversation history and focused context.
fn build_messages(
    question: &str,
    context: &ArchitectureContext,
    focused: &FocusedContext,
    history: &[(String, String)],
) -> Vec<serde_json::Value> {
    let context_json = serde_json::to_string(context).unwrap_or_else(|_| "{}".to_string());
    let focus_json = serde_json::to_string(focused).unwrap_or_else(|_| "{}".to_string());

    let mut messages = vec![json!({ "role": "system", "content": SYSTEM_PROMPT })];

    // Include last N conversation turns for continuity
    for (q, a) in history {
        messages.push(json!({ "role": "user", "content": q }));
        messages.push(json!({ "role": "assistant", "content": a }));
    }

    let user_prompt = format!(
        "Architecture Data:\n{}\n\nUser Focus:\n{}\n\nQuestion: {}",
        context_json, focus_json, question
    );
    messages.push(json!({ "role": "user", "content": user_prompt }));

    messages
}

/// Sends a streaming query to the AI endpoint, sending chunks via the channel.
/// Falls back to non-streaming if streaming fails.
pub async fn send_query_streaming(
    question: &str,
    context: &ArchitectureContext,
    focused: &FocusedContext,
    history: &[(String, String)],
    config: &AiConfig,
    tx: tokio::sync::mpsc::UnboundedSender<StreamChunk>,
) {
    let messages = build_messages(question, context, focused, history);
    let api_key = std::env::var(&config.api_key_env).unwrap_or_default();

    let client = match reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(30))
        .read_timeout(std::time::Duration::from_secs(300))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            let _ = tx.send(StreamChunk::Error(format!("HTTP client error: {}", e)));
            return;
        }
    };

    // Try streaming first
    if config.stream {
        let body = json!({
            "model": config.model,
            "messages": messages,
            "temperature": config.temperature,
            "max_tokens": config.max_tokens,
            "stream": true
        });

        match client
            .post(&config.endpoint)
            .header("Authorization", format!("Bearer {}", api_key))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
        {
            Ok(res) if res.status().is_success() => {
                let mut stream = res.bytes_stream();
                let mut buffer = String::new();
                let mut got_done = false;
                let mut truncated_by_length = false;

                while let Some(chunk_result) = stream.next().await {
                    match chunk_result {
                        Ok(bytes) => {
                            buffer.push_str(&String::from_utf8_lossy(&bytes));

                            // Process complete SSE lines
                            while let Some(newline_pos) = buffer.find('\n') {
                                let line = buffer[..newline_pos].trim().to_string();
                                buffer = buffer[newline_pos + 1..].to_string();

                                if line.is_empty() {
                                    continue;
                                }

                                if let Some(data) = line.strip_prefix("data: ") {
                                    let data = data.trim();
                                    if data == "[DONE]" {
                                        got_done = true;
                                        break;
                                    }

                                    if let Ok(parsed) =
                                        serde_json::from_str::<serde_json::Value>(data)
                                    {
                                        // Check finish_reason for token limit
                                        if parsed
                                            .get("choices")
                                            .and_then(|c| c.get(0))
                                            .and_then(|c| c.get("finish_reason"))
                                            .and_then(|r| r.as_str())
                                            == Some("length")
                                        {
                                            truncated_by_length = true;
                                        }

                                        if let Some(content) = parsed
                                            .get("choices")
                                            .and_then(|c| c.get(0))
                                            .and_then(|c| c.get("delta"))
                                            .and_then(|d| d.get("content"))
                                            .and_then(|c| c.as_str())
                                            && !content.is_empty()
                                        {
                                            let _ =
                                                tx.send(StreamChunk::Delta(content.to_string()));
                                        }
                                    }
                                }
                            }
                            if got_done {
                                break;
                            }
                        }
                        Err(e) => {
                            let _ =
                                tx.send(StreamChunk::Error(format!("Stream read error: {}", e)));
                            return;
                        }
                    }
                }

                // Process any remaining data in the buffer (last chunk may lack trailing \n)
                let remaining = buffer.trim().to_string();
                if let Some(data) = remaining.strip_prefix("data: ") {
                    let data = data.trim();
                    if data == "[DONE]" {
                        got_done = true;
                    } else if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(data) {
                        if parsed
                            .get("choices")
                            .and_then(|c| c.get(0))
                            .and_then(|c| c.get("finish_reason"))
                            .and_then(|r| r.as_str())
                            == Some("length")
                        {
                            truncated_by_length = true;
                        }
                        if let Some(content) = parsed
                            .get("choices")
                            .and_then(|c| c.get(0))
                            .and_then(|c| c.get("delta"))
                            .and_then(|d| d.get("content"))
                            .and_then(|c| c.as_str())
                            && !content.is_empty()
                        {
                            let _ = tx.send(StreamChunk::Delta(content.to_string()));
                        }
                    }
                }

                // Append truncation notice if model hit max_tokens
                if truncated_by_length {
                    let _ = tx.send(StreamChunk::Delta(
                        "\n\n⚠ *Yanıt, token limiti (max_tokens) nedeniyle kesildi. \
                         Daha kısa veya daha spesifik bir soru sorun.*"
                            .to_string(),
                    ));
                }

                if got_done || truncated_by_length {
                    let _ = tx.send(StreamChunk::Done);
                } else {
                    // Stream ended without [DONE] — connection may have dropped
                    let _ = tx.send(StreamChunk::Delta(
                        "\n\n⚠ *Yanıt akışı beklenmedik şekilde sonlandı. \
                         Bağlantı kopmuş olabilir.*"
                            .to_string(),
                    ));
                    let _ = tx.send(StreamChunk::Done);
                }
                return;
            }
            Ok(res) => {
                let status = res.status();
                let err = res.text().await.unwrap_or_default();
                tracing::warn!(
                    "Streaming failed ({}), falling back to non-streaming",
                    status
                );
                // Fall through to non-streaming
                let _ = (status, err);
            }
            Err(e) => {
                tracing::warn!("Streaming request failed: {}, falling back", e);
            }
        }
    }

    // Non-streaming fallback
    let body = json!({
        "model": config.model,
        "messages": messages,
        "temperature": config.temperature,
        "max_tokens": config.max_tokens
    });

    match client
        .post(&config.endpoint)
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
    {
        Ok(res) if res.status().is_success() => {
            #[derive(serde::Deserialize)]
            struct AiResponse {
                choices: Vec<Choice>,
            }
            #[derive(serde::Deserialize)]
            struct Choice {
                message: Message,
                #[serde(default)]
                finish_reason: Option<String>,
            }
            #[derive(serde::Deserialize)]
            struct Message {
                content: String,
            }

            match res.json::<AiResponse>().await {
                Ok(parsed) => {
                    if let Some(choice) = parsed.choices.into_iter().next() {
                        let _ = tx.send(StreamChunk::Delta(
                            choice.message.content.trim().to_string(),
                        ));
                        if choice.finish_reason.as_deref() == Some("length") {
                            let _ = tx.send(StreamChunk::Delta(
                                "\n\n⚠ *Yanıt, token limiti (max_tokens) nedeniyle kesildi. \
                                 Daha kısa veya daha spesifik bir soru sorun.*"
                                    .to_string(),
                            ));
                        }
                    }
                    let _ = tx.send(StreamChunk::Done);
                }
                Err(e) => {
                    let _ = tx.send(StreamChunk::Error(format!("Parse error: {}", e)));
                }
            }
        }
        Ok(res) => {
            let status = res.status();
            let err_text = res.text().await.unwrap_or_default();
            let truncated = if err_text.len() > 200 {
                format!("{}...", &err_text[..200])
            } else {
                err_text
            };
            let _ = tx.send(StreamChunk::Error(format!(
                "AI API error ({}): {}",
                status, truncated
            )));
        }
        Err(e) => {
            let _ = tx.send(StreamChunk::Error(format!(
                "Request failed: {}. Is the endpoint running?",
                e
            )));
        }
    }
}
