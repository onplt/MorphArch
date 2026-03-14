use std::collections::HashSet;
use std::time::{Duration, Instant};

use petgraph::graph::DiGraph;

use crate::blast_radius;
use crate::config::ScoringConfig;
use crate::graph_builder;
use crate::models::{BlastRadiusReport, DependencyEdge, DriftScore, InstabilityMetric};
use crate::scoring;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapshotAnalysisDetail {
    Core,
    Full,
}

pub struct SnapshotAnalysisArtifacts {
    pub graph: DiGraph<String, u32>,
    pub drift: DriftScore,
    pub blast_radius: Option<BlastRadiusReport>,
    pub instability_metrics: Vec<InstabilityMetric>,
    pub diagnostics: Vec<String>,
    pub timings: AnalysisTimings,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct AnalysisTimings {
    pub graph_build: Duration,
    pub drift: Duration,
    pub drift_cycle: Duration,
    pub drift_layering: Duration,
    pub drift_boundary_rules: Duration,
    pub drift_hub: Duration,
    pub drift_coupling: Duration,
    pub drift_cognitive: Duration,
    pub drift_instability: Duration,
    pub drift_fan_deltas: Duration,
    pub blast_radius: Duration,
    pub instability: Duration,
    pub diagnostics: Duration,
    pub graph_clone: Duration,
}

pub fn build_snapshot_artifacts(
    nodes: &HashSet<String>,
    edges: &[DependencyEdge],
    prev_graph: Option<&DiGraph<String, u32>>,
    timestamp: i64,
    scoring_config: &ScoringConfig,
    detail: SnapshotAnalysisDetail,
) -> SnapshotAnalysisArtifacts {
    let mut timings = AnalysisTimings::default();
    let graph_build_start = Instant::now();
    let graph = graph_builder::build_graph(nodes, edges);
    timings.graph_build += graph_build_start.elapsed();
    let mut artifacts = analyze_graph(&graph, prev_graph, timestamp, scoring_config, detail);
    artifacts.timings.graph_build += timings.graph_build;
    artifacts
}

pub fn analyze_graph(
    graph: &DiGraph<String, u32>,
    prev_graph: Option<&DiGraph<String, u32>>,
    timestamp: i64,
    scoring_config: &ScoringConfig,
    detail: SnapshotAnalysisDetail,
) -> SnapshotAnalysisArtifacts {
    let mut timings = AnalysisTimings::default();

    let drift_start = Instant::now();
    let (drift, drift_timings) =
        scoring::calculate_drift_profiled(graph, prev_graph, timestamp, scoring_config);
    timings.drift += drift_start.elapsed();
    timings.drift_cycle += drift_timings.cycle;
    timings.drift_layering += drift_timings.layering;
    timings.drift_boundary_rules += drift_timings.boundary_rules;
    timings.drift_hub += drift_timings.hub;
    timings.drift_coupling += drift_timings.coupling;
    timings.drift_cognitive += drift_timings.cognitive;
    timings.drift_instability += drift_timings.instability;
    timings.drift_fan_deltas += drift_timings.fan_deltas;

    let blast_radius = match detail {
        SnapshotAnalysisDetail::Core => None,
        SnapshotAnalysisDetail::Full => {
            let blast_start = Instant::now();
            let report = blast_radius::compute_blast_radius_report(
                graph,
                scoring_config.thresholds.blast_max_critical_paths,
            );
            timings.blast_radius += blast_start.elapsed();
            Some(report)
        }
    };
    let instability_metrics = match detail {
        SnapshotAnalysisDetail::Core => Vec::new(),
        SnapshotAnalysisDetail::Full => {
            let instability_start = Instant::now();
            let metrics = scoring::compute_instability_metrics(graph)
                .into_iter()
                .map(
                    |(module_name, instability, fan_in, fan_out)| InstabilityMetric {
                        module_name,
                        instability,
                        fan_in,
                        fan_out,
                    },
                )
                .collect();
            timings.instability += instability_start.elapsed();
            metrics
        }
    };
    let diagnostics = match detail {
        SnapshotAnalysisDetail::Core => Vec::new(),
        SnapshotAnalysisDetail::Full => {
            let diagnostics_start = Instant::now();
            let lines = scoring::generate_diagnostics(graph, &drift, scoring_config);
            timings.diagnostics += diagnostics_start.elapsed();
            lines
        }
    };
    let graph_clone_start = Instant::now();
    let graph = graph.clone();
    timings.graph_clone += graph_clone_start.elapsed();

    SnapshotAnalysisArtifacts {
        graph,
        drift,
        blast_radius,
        instability_metrics,
        diagnostics,
        timings,
    }
}
