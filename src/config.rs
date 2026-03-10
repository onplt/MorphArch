//! Configuration management for MorphArch.
//!
//! Two layers:
//! - [`MorphArchConfig`] — Runtime config (data directory, database path).
//! - [`ProjectConfig`] — Per-repository scoring/ignore config loaded from `morpharch.toml`.

use anyhow::{Context, Result};
use globset::{Glob, GlobSet, GlobSetBuilder};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tracing::{info, warn};

// ═════════════════════════════════════════════════════════════════════════════
// Runtime Configuration (data directory / database)
// ═════════════════════════════════════════════════════════════════════════════

/// Runtime configuration for the MorphArch application.
#[derive(Debug)]
pub struct MorphArchConfig {
    /// Full path to the SQLite database file
    pub db_path: PathBuf,
}

impl MorphArchConfig {
    /// Loads the default configuration.
    ///
    /// Creates ~/.morpharch/ if needed and sets the database path.
    pub fn load() -> Result<Self> {
        let home = dirs::home_dir().context(
            "Home directory not found. \
             Check your HOME (Linux/macOS) or USERPROFILE (Windows) environment variable.",
        )?;

        let morpharch_dir = home.join(".morpharch");
        std::fs::create_dir_all(&morpharch_dir).with_context(|| {
            format!(
                "Failed to create MorphArch data directory: {}",
                morpharch_dir.display()
            )
        })?;

        let db_path = morpharch_dir.join("morpharch.db");
        info!(path = %db_path.display(), "Configuration loaded");

        Ok(Self { db_path })
    }
}

// ═════════════════════════════════════════════════════════════════════════════
// Project Configuration (morpharch.toml)
// ═════════════════════════════════════════════════════════════════════════════

/// Top-level project configuration loaded from `morpharch.toml`.
///
/// If the file is missing, all fields use sensible defaults that reproduce
/// the exact behavior of the hardcoded scoring engine.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProjectConfig {
    #[serde(default)]
    pub ignore: IgnoreConfig,
    #[serde(default)]
    pub scoring: ScoringConfig,

    /// Compiled glob set for ignore paths — not serialized.
    #[serde(skip)]
    ignore_globs: Option<GlobSet>,
}

impl ProjectConfig {
    /// Loads project config from `morpharch.toml` at the given repository root.
    ///
    /// Returns `ProjectConfig::default()` if the file does not exist.
    /// Returns an error if the file exists but contains invalid TOML.
    pub fn load(repo_root: &Path) -> Result<Self> {
        let config_path = repo_root.join("morpharch.toml");
        if !config_path.exists() {
            info!("No morpharch.toml found, using default configuration");
            return Ok(Self::default());
        }

        let content = std::fs::read_to_string(&config_path)
            .with_context(|| format!("Failed to read config file: {}", config_path.display()))?;

        let mut config: Self = toml::from_str(&content).with_context(|| {
            format!(
                "Failed to parse {}: check TOML syntax and field names",
                config_path.display()
            )
        })?;

        config.compile_ignore_globs()?;
        config.scoring.weights.validate();

        info!(path = %config_path.display(), "Project configuration loaded");
        Ok(config)
    }

    /// Compiles ignore path patterns into a `GlobSet` for fast matching.
    fn compile_ignore_globs(&mut self) -> Result<()> {
        if self.ignore.paths.is_empty() {
            self.ignore_globs = None;
            return Ok(());
        }

        let mut builder = GlobSetBuilder::new();
        for pattern in &self.ignore.paths {
            builder.add(
                Glob::new(pattern)
                    .with_context(|| format!("Invalid ignore glob pattern: {pattern}"))?,
            );
        }
        self.ignore_globs = Some(builder.build().context("Failed to compile ignore globs")?);
        Ok(())
    }

    /// Returns the compiled ignore glob set (if any patterns were configured).
    pub fn ignore_globs(&self) -> Option<&GlobSet> {
        self.ignore_globs.as_ref()
    }
}

// ── Ignore Configuration ──

/// Paths to exclude from AST parsing and scoring.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct IgnoreConfig {
    /// Glob patterns for paths to ignore (e.g., `["tests/**", "benches/**"]`).
    #[serde(default)]
    pub paths: Vec<String>,
}

// ── Scoring Configuration ──

/// Complete scoring engine configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ScoringConfig {
    #[serde(default)]
    pub weights: Weights,
    #[serde(default)]
    pub thresholds: Thresholds,
    #[serde(default)]
    pub boundaries: Vec<BoundaryRule>,
    #[serde(default)]
    pub exemptions: Exemptions,
}

// ── Weights ──

/// Component weights for the 6-component scoring algorithm.
///
/// Values are relative — they are normalized to sum to 1.0 before use.
/// Defaults match the current hardcoded weights.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Weights {
    /// Cycle debt weight (default: 30)
    #[serde(default = "default_w_cycle")]
    pub cycle: f64,
    /// Layering debt weight (default: 25)
    #[serde(default = "default_w_layering")]
    pub layering: f64,
    /// Hub debt weight (default: 15)
    #[serde(default = "default_w_hub")]
    pub hub: f64,
    /// Coupling debt weight (default: 12)
    #[serde(default = "default_w_coupling")]
    pub coupling: f64,
    /// Cognitive debt weight (default: 10)
    #[serde(default = "default_w_cognitive")]
    pub cognitive: f64,
    /// Instability debt weight (default: 8)
    #[serde(default = "default_w_instability")]
    pub instability: f64,
}

fn default_w_cycle() -> f64 {
    30.0
}
fn default_w_layering() -> f64 {
    25.0
}
fn default_w_hub() -> f64 {
    15.0
}
fn default_w_coupling() -> f64 {
    12.0
}
fn default_w_cognitive() -> f64 {
    10.0
}
fn default_w_instability() -> f64 {
    8.0
}

impl Default for Weights {
    fn default() -> Self {
        Self {
            cycle: 30.0,
            layering: 25.0,
            hub: 15.0,
            coupling: 12.0,
            cognitive: 10.0,
            instability: 8.0,
        }
    }
}

/// Normalized weights that sum to 1.0.
#[derive(Debug, Clone, Copy)]
pub struct NormalizedWeights {
    pub cycle: f64,
    pub layering: f64,
    pub hub: f64,
    pub coupling: f64,
    pub cognitive: f64,
    pub instability: f64,
}

impl Weights {
    /// Validates weights, warning on degenerate values.
    pub fn validate(&self) {
        let total = self.total();
        if total <= 0.0 {
            warn!(
                "All scoring weights are zero or negative — falling back to defaults. \
                 Check [scoring.weights] in morpharch.toml."
            );
        }
    }

    /// Sum of all raw weights.
    pub fn total(&self) -> f64 {
        self.cycle + self.layering + self.hub + self.coupling + self.cognitive + self.instability
    }

    /// Returns weights normalized to sum to 1.0.
    ///
    /// If total is zero or negative, returns the default weights normalized.
    pub fn normalized(&self) -> NormalizedWeights {
        let total = self.total();
        if total <= 0.0 {
            let defaults = Self::default();
            let dt = defaults.total();
            return NormalizedWeights {
                cycle: defaults.cycle / dt,
                layering: defaults.layering / dt,
                hub: defaults.hub / dt,
                coupling: defaults.coupling / dt,
                cognitive: defaults.cognitive / dt,
                instability: defaults.instability / dt,
            };
        }
        NormalizedWeights {
            cycle: self.cycle / total,
            layering: self.layering / total,
            hub: self.hub / total,
            coupling: self.coupling / total,
            cognitive: self.cognitive / total,
            instability: self.instability / total,
        }
    }
}

// ── Thresholds ──

/// Configurable thresholds for scoring sub-components.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Thresholds {
    /// Hub exemption ratio — modules with `fan_out / (fan_in + 1)` below this
    /// are treated as legitimate shared cores (default: 0.3).
    #[serde(default = "default_hub_exemption_ratio")]
    pub hub_exemption_ratio: f64,

    /// Modules with fan-in at or below this are treated as entry-point
    /// composition roots and exempt from hub debt (default: 2).
    #[serde(default = "default_entry_point_max_fan_in")]
    pub entry_point_max_fan_in: usize,

    /// Instability threshold — modules with `I > this` are flagged as
    /// brittle (default: 0.8).
    #[serde(default = "default_brittle_instability_ratio")]
    pub brittle_instability_ratio: f64,
}

fn default_hub_exemption_ratio() -> f64 {
    0.3
}
fn default_entry_point_max_fan_in() -> usize {
    2
}
fn default_brittle_instability_ratio() -> f64 {
    0.8
}

impl Default for Thresholds {
    fn default() -> Self {
        Self {
            hub_exemption_ratio: 0.3,
            entry_point_max_fan_in: 2,
            brittle_instability_ratio: 0.8,
        }
    }
}

// ── Boundary Rules ──

/// A boundary rule that forbids dependencies from one set of modules to another.
///
/// ```toml
/// [[scoring.boundaries]]
/// from = "packages/**"
/// deny = ["apps/**", "cmd/**"]
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BoundaryRule {
    /// Glob pattern matching the source module path.
    pub from: String,
    /// Glob patterns matching denied target module paths.
    pub deny: Vec<String>,
}

// ── Exemptions ──

/// Modules exempted from specific debt calculations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Exemptions {
    /// Modules exempted from hub/god-module debt.
    #[serde(default)]
    pub hub_exempt: Vec<String>,

    /// Modules exempted from instability debt.
    #[serde(default)]
    pub instability_exempt: Vec<String>,

    /// File stems treated as entry points (exempt from fragility penalties).
    /// Defaults: `["main", "index", "app", "lib", "mod"]`.
    #[serde(default = "default_entry_point_stems")]
    pub entry_point_stems: Vec<String>,
}

fn default_entry_point_stems() -> Vec<String> {
    vec![
        "main".into(),
        "index".into(),
        "app".into(),
        "lib".into(),
        "mod".into(),
    ]
}

impl Default for Exemptions {
    fn default() -> Self {
        Self {
            hub_exempt: Vec::new(),
            instability_exempt: Vec::new(),
            entry_point_stems: default_entry_point_stems(),
        }
    }
}

// ═════════════════════════════════════════════════════════════════════════════
// Tests
// ═════════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_weights_sum_to_100() {
        let w = Weights::default();
        assert!((w.total() - 100.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_normalized_weights_sum_to_1() {
        let w = Weights::default();
        let n = w.normalized();
        let sum = n.cycle + n.layering + n.hub + n.coupling + n.cognitive + n.instability;
        assert!((sum - 1.0).abs() < 1e-10);
    }

    #[test]
    fn test_zero_weights_fallback_to_defaults() {
        let w = Weights {
            cycle: 0.0,
            layering: 0.0,
            hub: 0.0,
            coupling: 0.0,
            cognitive: 0.0,
            instability: 0.0,
        };
        let n = w.normalized();
        let sum = n.cycle + n.layering + n.hub + n.coupling + n.cognitive + n.instability;
        assert!((sum - 1.0).abs() < 1e-10);
        assert!(n.cycle > 0.0, "Should fall back to default cycle weight");
    }

    #[test]
    fn test_custom_weights_normalize() {
        let w = Weights {
            cycle: 50.0,
            layering: 50.0,
            hub: 0.0,
            coupling: 0.0,
            cognitive: 0.0,
            instability: 0.0,
        };
        let n = w.normalized();
        assert!((n.cycle - 0.5).abs() < 1e-10);
        assert!((n.layering - 0.5).abs() < 1e-10);
        assert!((n.hub).abs() < 1e-10);
    }

    #[test]
    fn test_default_thresholds() {
        let t = Thresholds::default();
        assert!((t.hub_exemption_ratio - 0.3).abs() < f64::EPSILON);
        assert_eq!(t.entry_point_max_fan_in, 2);
        assert!((t.brittle_instability_ratio - 0.8).abs() < f64::EPSILON);
    }

    #[test]
    fn test_default_entry_point_stems() {
        let e = Exemptions::default();
        assert!(e.entry_point_stems.contains(&"main".to_string()));
        assert!(e.entry_point_stems.contains(&"index".to_string()));
        assert!(e.entry_point_stems.contains(&"app".to_string()));
    }

    #[test]
    fn test_toml_deserialization_full() {
        let toml_str = r#"
[ignore]
paths = ["tests/**", "benches/**"]

[scoring.weights]
cycle = 40
layering = 20
hub = 15
coupling = 10
cognitive = 10
instability = 5

[scoring.thresholds]
hub_exemption_ratio = 0.4
entry_point_max_fan_in = 3
brittle_instability_ratio = 0.7

[[scoring.boundaries]]
from = "packages/**"
deny = ["apps/**", "cmd/**"]

[[scoring.boundaries]]
from = "libs/shared/**"
deny = ["libs/feature_*/**"]

[scoring.exemptions]
hub_exempt = ["src/utils.rs"]
instability_exempt = ["packages/ui-kit/src/index.ts"]
entry_point_stems = ["main", "index", "app", "lib", "mod", "server"]
"#;
        let config: ProjectConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.ignore.paths.len(), 2);
        assert!((config.scoring.weights.cycle - 40.0).abs() < f64::EPSILON);
        assert!((config.scoring.thresholds.hub_exemption_ratio - 0.4).abs() < f64::EPSILON);
        assert_eq!(config.scoring.thresholds.entry_point_max_fan_in, 3);
        assert_eq!(config.scoring.boundaries.len(), 2);
        assert_eq!(config.scoring.boundaries[0].from, "packages/**");
        assert_eq!(config.scoring.boundaries[0].deny, vec!["apps/**", "cmd/**"]);
        assert_eq!(config.scoring.exemptions.hub_exempt, vec!["src/utils.rs"]);
        assert_eq!(config.scoring.exemptions.entry_point_stems.len(), 6);
    }

    #[test]
    fn test_toml_deserialization_minimal() {
        // Empty config should produce all defaults
        let config: ProjectConfig = toml::from_str("").unwrap();
        assert!(config.ignore.paths.is_empty());
        assert!((config.scoring.weights.total() - 100.0).abs() < f64::EPSILON);
        assert!(config.scoring.boundaries.is_empty());
        assert!(config.scoring.exemptions.hub_exempt.is_empty());
        assert_eq!(config.scoring.exemptions.entry_point_stems.len(), 5);
    }

    #[test]
    fn test_toml_deserialization_partial() {
        // Only override one weight — rest should default
        let toml_str = r#"
[scoring.weights]
cycle = 50
"#;
        let config: ProjectConfig = toml::from_str(toml_str).unwrap();
        assert!((config.scoring.weights.cycle - 50.0).abs() < f64::EPSILON);
        assert!((config.scoring.weights.layering - 25.0).abs() < f64::EPSILON);
        assert!((config.scoring.weights.hub - 15.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_ignore_glob_compilation() {
        let toml_str = r#"
[ignore]
paths = ["tests/**", "**/generated_*.rs"]
"#;
        let mut config: ProjectConfig = toml::from_str(toml_str).unwrap();
        config.compile_ignore_globs().unwrap();
        let globs = config.ignore_globs().unwrap();
        assert!(globs.is_match("tests/unit/foo.rs"));
        assert!(globs.is_match("src/generated_schema.rs"));
        assert!(!globs.is_match("src/main.rs"));
    }
}
