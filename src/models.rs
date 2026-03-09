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

use serde::{Deserialize, Serialize};

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
    pub line: usize,
    /// Number of import statements for this edge (1 = single import, N = N files import this)
    #[serde(default = "default_weight")]
    pub weight: u32,
}

/// Default edge weight for backwards-compatible deserialization of old snapshots.
fn default_weight() -> u32 {
    1
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
    pub drift: Option<DriftScore>,
}

/// Lighter version of GraphSnapshot for UI lists and timelines.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotMetadata {
    pub commit_hash: String,
    pub timestamp: i64,
    pub drift: Option<DriftScore>,
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
    /// Back-edge count in topological layering (real boundary violations)
    pub boundary_violations: usize,
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
