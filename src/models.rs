//! Core data models for MorphArch.
//!
//! This module defines the primary data structures:
//!
//! - [`CommitInfo`] — Git commit metadata
//! - [`DependencyEdge`] — A directed dependency between two modules
//! - [`GraphSnapshot`] — Complete dependency graph at a specific commit
//! - [`DriftScore`] — Architecture drift score (0–100) with sub-metrics
//! - [`TemporalDelta`] — Drift comparison between consecutive commits
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

/// Architecture drift score — measures graph "health" (0-100).
///
/// Score 0 = perfect architecture, 100 = fully chaotic.
/// Baseline (first commit or no previous graph) = 50.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DriftScore {
    /// Total drift score (0 = clean, 100 = chaotic)
    pub total: u8,
    /// Average fan-in change (positive = increasing dependencies)
    pub fan_in_delta: i32,
    /// Average fan-out change (positive = increasing external deps)
    pub fan_out_delta: i32,
    /// New circular dependency count
    pub new_cycles: usize,
    /// Package boundary violation count
    pub boundary_violations: usize,
    /// Cognitive complexity proxy: (edges/nodes)*10 + cycles*5
    pub cognitive_complexity: f64,
    /// Score computation timestamp
    pub timestamp: i64,
}

/// Drift comparison between two consecutive commits.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct TemporalDelta {
    pub prev_commit_hash: String,
    pub current_commit_hash: String,
    pub score_delta: i32,
    pub nodes_added: usize,
    pub nodes_removed: usize,
    pub edges_added: usize,
    pub edges_removed: usize,
    pub new_cycles: usize,
    pub resolved_cycles: usize,
}
