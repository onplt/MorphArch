//! Core data models for MorphArch.
//!
//! This module defines the primary data structures:
//!
//! - [`CommitInfo`] — Git commit metadata
//! - [`DependencyEdge`] — A directed dependency between two modules
//! - [`GraphSnapshot`] — Complete dependency graph at a specific commit
//! - [`DriftScore`] — Architecture drift score (0–100) with sub-metrics
//!
//! All types implement `Serialize` and `Deserialize` for JSON persistence.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

pub const CURRENT_ANALYSIS_VERSION: u32 = 3;

/// Metadata for a single Git commit.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommitInfo {
    pub hash: String,
    pub author_name: String,
    pub author_email: String,
    pub message: String,
    pub timestamp: i64,
    pub tree_id: String,
}

/// A dependency edge between two modules/packages.
///
/// `weight` counts how many import statements exist for this (from → to) pair.
/// Higher weight = stronger coupling between modules.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DependencyEdge {
    pub from_module: String,
    pub to_module: String,
    pub file_path: String,
    #[serde(default)]
    pub line: Option<usize>,
    /// Number of import statements for this edge (1 = single import, N = N files import this)
    #[serde(default = "default_weight")]
    pub weight: u32,
    #[serde(default)]
    pub sample_origins: Vec<EdgeOrigin>,
}

/// Default edge weight for backwards-compatible deserialization of old snapshots.
fn default_weight() -> u32 {
    1
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EdgeOrigin {
    pub file_path: String,
    #[serde(default)]
    pub line: Option<usize>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum NodeKind {
    Internal,
    External,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NodeMetadata {
    pub kind: NodeKind,
    #[serde(default)]
    pub importer_count: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FilteredExternalSample {
    pub module_name: String,
    pub importer_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FileImportTarget {
    pub module_name: String,
    pub weight: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FileDependencyState {
    pub package_name: String,
    #[serde(default)]
    pub imports: Vec<FileImportTarget>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct RepoScanState {
    #[serde(default)]
    pub files: HashMap<String, FileDependencyState>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct GraphDelta {
    #[serde(default)]
    pub upserts: Vec<(String, FileDependencyState)>,
    #[serde(default)]
    pub deletes: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ScanMetadata {
    #[serde(default)]
    pub external_min_importers: u32,
    #[serde(default)]
    pub included_external_count: usize,
    #[serde(default)]
    pub filtered_external_count: usize,
    #[serde(default)]
    pub filtered_external_samples: Vec<FilteredExternalSample>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct InstabilityMetric {
    pub module_name: String,
    pub instability: f64,
    pub fan_in: usize,
    pub fan_out: usize,
}

/// Full dependency graph at a specific commit.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphSnapshot {
    pub commit_hash: String,
    pub nodes: Vec<String>,
    pub edges: Vec<DependencyEdge>,
    pub node_count: usize,
    pub edge_count: usize,
    pub timestamp: i64,
    #[serde(default)]
    pub analysis_version: u32,
    #[serde(default)]
    pub config_fingerprint: String,
    #[serde(default)]
    pub node_metadata: HashMap<String, NodeMetadata>,
    #[serde(default)]
    pub scan_metadata: ScanMetadata,
    #[serde(default)]
    pub drift: Option<DriftScore>,
    /// Blast radius analysis (computed during scan, None for old snapshots)
    #[serde(default)]
    pub blast_radius: Option<BlastRadiusReport>,
    #[serde(default)]
    pub instability_metrics: Vec<InstabilityMetric>,
    #[serde(default)]
    pub diagnostics: Vec<String>,
}

/// Lighter version of GraphSnapshot for UI lists and timelines.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotMetadata {
    pub commit_hash: String,
    pub scan_order: i64,
    pub timestamp: i64,
    pub drift: Option<DriftScore>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotFrame {
    pub commit_hash: String,
    pub scan_order: i64,
    pub timestamp: i64,
    pub node_count: usize,
    pub edge_count: usize,
    pub analysis_version: u32,
    pub config_fingerprint: String,
    pub drift: Option<DriftScore>,
    pub scan_metadata: ScanMetadata,
    pub delta: GraphDelta,
    pub has_full_artifacts: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeavySnapshotArtifacts {
    pub blast_radius: BlastRadiusReport,
    pub instability_metrics: Vec<InstabilityMetric>,
    pub diagnostics: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphCheckpoint {
    pub commit_hash: String,
    pub scan_order: i64,
    pub state: RepoScanState,
    #[serde(default)]
    pub full_artifacts: Option<HeavySnapshotArtifacts>,
}

impl GraphSnapshot {
    pub fn requires_core_recompute(&self) -> bool {
        self.analysis_version < CURRENT_ANALYSIS_VERSION
            || self.config_fingerprint.is_empty()
            || self.node_metadata.is_empty()
            || self.drift.is_none()
    }

    pub fn needs_runtime_insights(&self) -> bool {
        self.instability_metrics.is_empty() || self.diagnostics.is_empty()
    }

    pub fn needs_full_analysis(&self) -> bool {
        self.blast_radius.is_none()
    }

    pub fn requires_artifact_recompute(&self) -> bool {
        self.requires_core_recompute()
            || self.needs_runtime_insights()
            || self.needs_full_analysis()
    }
}

/// Architecture drift score — measures graph "health" (0-100).
///
/// Score 0 = perfect architecture, 100 = fully chaotic.
/// Uses a 6-component scale-aware algorithm:
///   - Cycle Debt (30%): SCC count + cyclic node fraction + largest SCC
///   - Layering Debt (25%): Back-edge ratio in topological ordering
///   - Hub Debt (15%): True god modules (high in AND out) detection
///   - Coupling Debt (12%): Weighted coupling intensity using edge weights
///   - Cognitive Debt (10%): Shannon entropy + edge excess ratio
///   - Instability Debt (8%): Refined Martin metric (leaf packages excluded)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DriftScore {
    /// Total drift score (0 = clean, 100 = chaotic)
    pub total: u8,
    /// Median fan-in change (positive = increasing dependencies)
    pub fan_in_delta: i32,
    /// Median fan-out change (positive = increasing external deps)
    pub fan_out_delta: i32,
    /// Circular dependency (SCC) count
    pub new_cycles: usize,
    /// Configured package-boundary rule violations.
    pub boundary_violations: usize,
    /// Extra cross-cutting edges inside SCCs.
    #[serde(default)]
    pub layering_violations: usize,
    /// Cognitive complexity: Shannon entropy + edge excess ratio
    pub cognitive_complexity: f64,
    /// Score computation timestamp
    pub timestamp: i64,

    // ── 6-Component Sub-Scores (0.0 - 100.0 each) ──
    /// Cycle debt sub-score (weight: 30%)
    #[serde(default)]
    pub cycle_debt: f64,
    /// Layering debt sub-score (weight: 25%)
    #[serde(default)]
    pub layering_debt: f64,
    /// Hub debt sub-score (weight: 15%)
    #[serde(default)]
    pub hub_debt: f64,
    /// Coupling debt sub-score (weight: 12%)
    #[serde(default)]
    pub coupling_debt: f64,
    /// Cognitive debt sub-score (weight: 10%)
    #[serde(default)]
    pub cognitive_debt: f64,
    /// Instability debt sub-score (weight: 8%)
    #[serde(default)]
    pub instability_debt: f64,
}

// ── Blast Radius Cartography ──

/// Blast radius analysis for a single graph snapshot.
///
/// Contains per-module impact scores, articulation points (structural
/// keystones), and critical dependency chains.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlastRadiusReport {
    /// Per-module impact scores, sorted by blast_score descending
    pub impacts: Vec<ModuleImpact>,
    /// Articulation points — nodes whose removal fragments the graph
    pub articulation_points: Vec<ArticulationPoint>,
    /// Top critical dependency chains (longest weighted paths)
    pub critical_paths: Vec<CascadePath>,
    /// Graph-level summary statistics
    pub summary: BlastRadiusSummary,
}

/// Blast radius impact score for a single module.
///
/// `blast_score` is 0.0–1.0: fraction of downstream graph reachable,
/// weighted by inverse-square distance decay and coupling intensity.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModuleImpact {
    /// Module name (matches graph node label)
    pub module_name: String,
    /// Normalized blast score (0.0 = leaf, 1.0 = affects everything)
    pub blast_score: f64,
    /// Count of transitively reachable downstream modules
    pub downstream_count: usize,
    /// Sum of decay-weighted reachability (raw, before normalization)
    pub weighted_reach: f64,
    /// Whether this module is an articulation point
    pub is_articulation_point: bool,
}

/// A structural keystone whose removal would fragment the dependency graph.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArticulationPoint {
    /// Module name
    pub module_name: String,
    /// Number of biconnected components this point bridges
    pub components_bridged: usize,
    /// Fan-in count (how many depend on it)
    pub fan_in: usize,
    /// Fan-out count (how many it depends on)
    pub fan_out: usize,
}

/// A critical dependency chain — longest weighted path through the graph.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CascadePath {
    /// Ordered list of module names from root to leaf
    pub chain: Vec<String>,
    /// Total coupling weight along the path (sum of edge weights)
    pub total_weight: u32,
    /// Chain length (number of modules)
    pub depth: usize,
}

/// Graph-level blast radius summary statistics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlastRadiusSummary {
    /// Number of articulation points in the graph
    pub articulation_point_count: usize,
    /// Maximum blast score across all modules
    pub max_blast_score: f64,
    /// Module with the highest blast score
    pub most_impactful_module: String,
    /// Average blast score
    pub mean_blast_score: f64,
    /// Length of the longest critical path
    pub longest_chain_depth: usize,
}
