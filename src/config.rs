//! Configuration management for MorphArch.
//!
//! Two layers:
//! - [`MorphArchConfig`] — Runtime config (data directory, database path).
//! - [`ProjectConfig`] — Per-repository scoring/ignore config loaded from `morpharch.toml`.

use anyhow::{Context, Result, bail};
use globset::{Glob, GlobSet, GlobSetBuilder};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use tracing::{info, warn};

const DEFAULT_IGNORE_PRESETS: &[&str] = &["tooling", "artifacts", "generated"];
const BUILTIN_IGNORE_PRESETS: &[(&str, &[&str])] = &[
    (
        "tooling",
        &[
            ".github/**",
            ".husky/**",
            ".vscode/**",
            ".idea/**",
            ".devcontainer/**",
        ],
    ),
    (
        "artifacts",
        &[
            "coverage/**",
            "**/coverage/**",
            "dist/**",
            "**/dist/**",
            "build/**",
            "**/build/**",
            "target/**",
            "**/target/**",
            ".next/**",
            "**/.next/**",
            ".turbo/**",
            "**/.turbo/**",
            ".cache/**",
            "**/.cache/**",
            "tmp/**",
            "**/tmp/**",
            "temp/**",
            "**/temp/**",
        ],
    ),
    (
        "generated",
        &[
            "**/__generated__/**",
            "**/generated/**",
            "**/*.generated.*",
            "**/*.d.ts",
        ],
    ),
];

// ═════════════════════════════════════════════════════════════════════════════
// Runtime Configuration (data directory / database)
// ═════════════════════════════════════════════════════════════════════════════

/// Runtime configuration for the MorphArch application.
#[derive(Debug)]
pub struct MorphArchConfig {
    /// Full path to the SQLite database file
    pub db_path: PathBuf,
    /// Directory for persistent scan caches
    pub cache_dir: PathBuf,
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

        let cache_dir = morpharch_dir.join("subtree-cache");
        std::fs::create_dir_all(&cache_dir).with_context(|| {
            format!(
                "Failed to create MorphArch cache directory: {}",
                cache_dir.display()
            )
        })?;

        let db_path = morpharch_dir.join("morpharch.db");
        info!(path = %db_path.display(), "Configuration loaded");

        Ok(Self { db_path, cache_dir })
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
    pub scan: ScanConfig,
    #[serde(default)]
    pub scoring: ScoringConfig,
    #[serde(default)]
    pub clustering: ClusteringConfig,

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
        let mut config = if !config_path.exists() {
            info!("No morpharch.toml found, using default configuration");
            Self::default()
        } else {
            let content = std::fs::read_to_string(&config_path).with_context(|| {
                format!("Failed to read config file: {}", config_path.display())
            })?;

            toml::from_str(&content).with_context(|| {
                format!(
                    "Failed to parse {}: check TOML syntax and field names",
                    config_path.display()
                )
            })?
        };

        config.compile_ignore_globs()?;
        config.scan.validate()?;
        config.compile_boundary_rules()?;
        config.compile_clustering_rules()?;
        config.scoring.weights.validate();

        info!(path = %config_path.display(), "Project configuration loaded");
        Ok(config)
    }

    /// Compiles ignore path patterns into a `GlobSet` for fast matching.
    fn compile_ignore_globs(&mut self) -> Result<()> {
        let patterns = self.ignore.resolve_patterns()?;
        if patterns.is_empty() {
            self.ignore_globs = None;
            return Ok(());
        }

        let mut builder = GlobSetBuilder::new();
        for pattern in &patterns {
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

    fn compile_boundary_rules(&mut self) -> Result<()> {
        for rule in &mut self.scoring.boundaries {
            rule.compile()?;
        }
        Ok(())
    }

    pub fn config_fingerprint(&self) -> Result<String> {
        serde_json::to_string(&json!({
            "ignore": stable_ignore_config_value(&self.ignore),
            "scan": stable_scan_config_value(&self.scan),
            "scoring": stable_scoring_config_value(&self.scoring),
        }))
        .context("Failed to serialize project scan fingerprint")
    }

    pub fn ignore_fingerprint(&self) -> Result<String> {
        serde_json::to_string(&json!({
            "ignore": stable_ignore_config_value(&self.ignore),
            "scan": stable_scan_config_value(&self.scan),
        }))
        .context("Failed to serialize ignore fingerprint")
    }

    fn compile_clustering_rules(&mut self) -> Result<()> {
        for family in &mut self.clustering.families {
            family.compile()?;
            if let Some(kind) = family.kind {
                self.clustering
                    .cluster_kinds
                    .entry(family.name.clone())
                    .or_insert(kind);
            }
        }
        for constraint in &mut self.clustering.constraints {
            constraint.compile()?;
        }
        for rule in &mut self.clustering.rules {
            rule.compile()?;
            if let Some(kind) = rule.kind {
                self.clustering
                    .cluster_kinds
                    .entry(rule.name.clone())
                    .or_insert(kind);
            }
        }
        Ok(())
    }
}

// ── Ignore Configuration ──

/// Paths to exclude from AST parsing and scoring.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IgnoreConfig {
    /// Glob patterns for paths to ignore (e.g., `["tests/**", "benches/**"]`).
    #[serde(default)]
    pub paths: Vec<String>,
    /// Built-in or user-defined preset names to enable in addition to defaults.
    #[serde(default)]
    pub presets: Vec<String>,
    /// Whether MorphArch's built-in recommended presets are enabled by default.
    #[serde(default = "default_use_ignore_defaults")]
    pub use_defaults: bool,
    /// User-defined reusable preset groups local to this repository.
    #[serde(default)]
    pub custom_presets: HashMap<String, Vec<String>>,
}

impl IgnoreConfig {
    fn resolve_patterns(&self) -> Result<Vec<String>> {
        let mut resolved = Vec::new();

        if self.use_defaults {
            for preset in DEFAULT_IGNORE_PRESETS {
                if let Some(patterns) = resolve_builtin_ignore_preset(preset) {
                    resolved.extend(patterns.iter().map(|pattern| (*pattern).to_string()));
                }
            }
        }

        for preset in &self.presets {
            if let Some(patterns) = self.custom_presets.get(preset) {
                resolved.extend(patterns.iter().cloned());
                continue;
            }

            if let Some(patterns) = resolve_builtin_ignore_preset(preset) {
                resolved.extend(patterns.iter().map(|pattern| (*pattern).to_string()));
                continue;
            }

            let available = available_ignore_presets(&self.custom_presets);
            bail!(
                "Unknown ignore preset '{preset}'. Available presets: {}",
                available.join(", ")
            );
        }

        resolved.extend(self.paths.iter().cloned());

        let mut seen = HashSet::new();
        resolved.retain(|pattern| seen.insert(pattern.clone()));
        Ok(resolved)
    }
}

impl Default for IgnoreConfig {
    fn default() -> Self {
        Self {
            paths: Vec::new(),
            presets: Vec::new(),
            use_defaults: default_use_ignore_defaults(),
            custom_presets: HashMap::new(),
        }
    }
}

// —— Scan Configuration ——

const DEFAULT_TEST_PATH_PATTERNS: &[&str] = &[
    "/test/",
    "/tests/",
    "/testdata/",
    "/test_data/",
    "/__tests__/",
    "/spec/",
    "/fixtures/",
    "/fixture/",
    "/snapshots/",
    "/e2e/",
];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanConfig {
    #[serde(default = "default_scan_package_depth")]
    pub package_depth: usize,
    #[serde(default = "default_scan_external_min_importers")]
    pub external_min_importers: usize,
    #[serde(default = "default_scan_test_path_patterns")]
    pub test_path_patterns: Vec<String>,
}

impl ScanConfig {
    fn validate(&self) -> Result<()> {
        if self.package_depth == 0 {
            bail!("scan.package_depth must be at least 1");
        }
        Ok(())
    }

    pub fn normalized_test_path_patterns(&self) -> Vec<String> {
        self.test_path_patterns
            .iter()
            .map(|pattern| pattern.to_ascii_lowercase().replace('\\', "/"))
            .collect()
    }
}

impl Default for ScanConfig {
    fn default() -> Self {
        Self {
            package_depth: default_scan_package_depth(),
            external_min_importers: default_scan_external_min_importers(),
            test_path_patterns: default_scan_test_path_patterns(),
        }
    }
}

fn default_scan_package_depth() -> usize {
    2
}

fn default_scan_external_min_importers() -> usize {
    3
}

fn default_scan_test_path_patterns() -> Vec<String> {
    DEFAULT_TEST_PATH_PATTERNS
        .iter()
        .map(|pattern| (*pattern).to_string())
        .collect()
}

fn default_use_ignore_defaults() -> bool {
    true
}

fn resolve_builtin_ignore_preset(name: &str) -> Option<&'static [&'static str]> {
    BUILTIN_IGNORE_PRESETS
        .iter()
        .find_map(|(preset, patterns)| (*preset == name).then_some(*patterns))
}

fn available_ignore_presets(custom_presets: &HashMap<String, Vec<String>>) -> Vec<String> {
    let mut names: Vec<String> = BUILTIN_IGNORE_PRESETS
        .iter()
        .map(|(name, _)| (*name).to_string())
        .collect();
    names.extend(custom_presets.keys().cloned());
    names.sort();
    names.dedup();
    names
}

fn stable_ignore_config_value(ignore: &IgnoreConfig) -> serde_json::Value {
    let mut paths = ignore.paths.clone();
    paths.sort();

    let mut presets = ignore.presets.clone();
    presets.sort();

    let custom_presets: BTreeMap<String, Vec<String>> = ignore
        .custom_presets
        .iter()
        .map(|(name, patterns)| {
            let mut sorted_patterns = patterns.clone();
            sorted_patterns.sort();
            (name.clone(), sorted_patterns)
        })
        .collect();

    json!({
        "paths": paths,
        "presets": presets,
        "use_defaults": ignore.use_defaults,
        "custom_presets": custom_presets,
    })
}

fn stable_scan_config_value(scan: &ScanConfig) -> serde_json::Value {
    let mut test_path_patterns = scan.test_path_patterns.clone();
    test_path_patterns.sort();

    json!({
        "package_depth": scan.package_depth,
        "external_min_importers": scan.external_min_importers,
        "test_path_patterns": test_path_patterns,
    })
}

fn stable_scoring_config_value(scoring: &ScoringConfig) -> serde_json::Value {
    let mut boundaries: Vec<serde_json::Value> = scoring
        .boundaries
        .iter()
        .map(|rule| {
            let mut deny = rule.deny.clone();
            deny.sort();
            json!({
                "from": rule.from,
                "deny": deny,
            })
        })
        .collect();
    boundaries.sort_by(|a, b| {
        let a_from = a.get("from").and_then(|value| value.as_str()).unwrap_or("");
        let b_from = b.get("from").and_then(|value| value.as_str()).unwrap_or("");
        a_from.cmp(b_from)
    });

    let mut hub_exempt = scoring.exemptions.hub_exempt.clone();
    hub_exempt.sort();
    let mut instability_exempt = scoring.exemptions.instability_exempt.clone();
    instability_exempt.sort();
    let mut entry_point_stems = scoring.exemptions.entry_point_stems.clone();
    entry_point_stems.sort();

    json!({
        "weights": &scoring.weights,
        "thresholds": &scoring.thresholds,
        "boundaries": boundaries,
        "exemptions": {
            "hub_exempt": hub_exempt,
            "instability_exempt": instability_exempt,
            "entry_point_stems": entry_point_stems,
        },
    })
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

// —— Clustering Configuration ——

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ClusteringStrategy {
    Hybrid,
    Namespace,
    Structural,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ClusterKindHint {
    Workspace,
    Deps,
    Entry,
    External,
    Infra,
    Domain,
    Group,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ClusterKindMode {
    ExplicitThenHeuristic,
    ExplicitOnly,
}

impl Default for ClusterKindMode {
    fn default() -> Self {
        default_cluster_kind_mode()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClusterColorMode {
    Semantic,
    Minimal,
}

impl Default for ClusterColorMode {
    fn default() -> Self {
        default_cluster_color_mode()
    }
}

fn default_clustering_strategy() -> ClusteringStrategy {
    ClusteringStrategy::Hybrid
}

fn default_cluster_min_size() -> usize {
    2
}

fn default_root_token_min_repeats() -> usize {
    2
}

fn default_workspace_split_threshold() -> usize {
    6
}

fn default_workspace_max_share() -> f64 {
    0.45
}

fn default_collapse_external() -> bool {
    true
}

fn default_structural_enabled() -> bool {
    true
}

fn default_cluster_kind_mode() -> ClusterKindMode {
    ClusterKindMode::ExplicitThenHeuristic
}

fn default_cluster_color_mode() -> ClusterColorMode {
    ClusterColorMode::Minimal
}

fn default_preserve_family_purity() -> bool {
    true
}

fn default_post_merge_small_clusters() -> bool {
    true
}

fn default_disambiguate_duplicate_names() -> bool {
    true
}

fn default_include_exact_roots_for_known_heads() -> bool {
    true
}

fn default_fallback_family() -> String {
    "workspace".to_string()
}

fn default_family_priority() -> i32 {
    0
}

fn default_merge_small_into_family() -> bool {
    true
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ClusteringConstraintType {
    MustGroup,
    MustSeparate,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FamilySplitMode {
    Never,
    #[default]
    Allow,
    Prefer,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClusteringSemanticConfig {
    #[serde(default = "default_collapse_external")]
    pub collapse_external: bool,
    #[serde(default = "default_root_token_min_repeats")]
    pub root_token_min_repeats: usize,
    #[serde(default = "default_fallback_family")]
    pub fallback_family: String,
    #[serde(default = "default_include_exact_roots_for_known_heads")]
    pub include_exact_roots_for_known_heads: bool,
}

impl Default for ClusteringSemanticConfig {
    fn default() -> Self {
        Self {
            collapse_external: default_collapse_external(),
            root_token_min_repeats: default_root_token_min_repeats(),
            fallback_family: default_fallback_family(),
            include_exact_roots_for_known_heads: default_include_exact_roots_for_known_heads(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClusteringStructuralConfig {
    #[serde(default = "default_structural_enabled")]
    pub enabled: bool,
    #[serde(default = "default_cluster_min_size")]
    pub min_cluster_size: usize,
    #[serde(default = "default_workspace_split_threshold")]
    pub split_threshold: usize,
    #[serde(default = "default_workspace_max_share")]
    pub max_cluster_share: f64,
    #[serde(default = "default_preserve_family_purity")]
    pub preserve_family_purity: bool,
    #[serde(default = "default_post_merge_small_clusters")]
    pub post_merge_small_clusters: bool,
    #[serde(default = "default_disambiguate_duplicate_names")]
    pub disambiguate_duplicate_names: bool,
}

impl Default for ClusteringStructuralConfig {
    fn default() -> Self {
        Self {
            enabled: default_structural_enabled(),
            min_cluster_size: default_cluster_min_size(),
            split_threshold: default_workspace_split_threshold(),
            max_cluster_share: default_workspace_max_share(),
            preserve_family_purity: default_preserve_family_purity(),
            post_merge_small_clusters: default_post_merge_small_clusters(),
            disambiguate_duplicate_names: default_disambiguate_duplicate_names(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ClusteringPresentationConfig {
    #[serde(default)]
    pub aliases: HashMap<String, String>,
    #[serde(default)]
    pub kinds: HashMap<String, ClusterKindHint>,
    #[serde(default = "default_cluster_kind_mode")]
    pub kind_mode: ClusterKindMode,
    #[serde(default = "default_cluster_color_mode")]
    pub color_mode: ClusterColorMode,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClusteringFamily {
    pub name: String,
    #[serde(default, alias = "match")]
    pub include: Vec<String>,
    #[serde(default)]
    pub exclude: Vec<String>,
    #[serde(default)]
    pub kind: Option<ClusterKindHint>,
    #[serde(default)]
    pub split: FamilySplitMode,
    #[serde(default = "default_family_priority")]
    pub priority: i32,
    #[serde(default = "default_merge_small_into_family")]
    pub merge_small_into_family: bool,
    #[serde(skip)]
    include_compiled: Option<GlobSet>,
    #[serde(skip)]
    exclude_compiled: Option<GlobSet>,
}

impl ClusteringFamily {
    fn compile(&mut self) -> Result<()> {
        self.include_compiled = compile_optional_globset(&self.include)
            .context("Failed to compile clustering family include patterns")?;
        self.exclude_compiled = compile_optional_globset(&self.exclude)
            .context("Failed to compile clustering family exclude patterns")?;
        Ok(())
    }

    pub fn matches(&self, label: &str) -> bool {
        let included = self
            .include_compiled
            .as_ref()
            .is_some_and(|matcher| matcher.is_match(label));
        if !included {
            return false;
        }
        !self
            .exclude_compiled
            .as_ref()
            .is_some_and(|matcher| matcher.is_match(label))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClusteringConstraint {
    #[serde(rename = "type")]
    pub constraint_type: ClusteringConstraintType,
    #[serde(default)]
    pub members: Vec<String>,
    #[serde(default)]
    pub left: Vec<String>,
    #[serde(default)]
    pub right: Vec<String>,
    #[serde(skip)]
    members_compiled: Option<GlobSet>,
    #[serde(skip)]
    left_compiled: Option<GlobSet>,
    #[serde(skip)]
    right_compiled: Option<GlobSet>,
}

impl ClusteringConstraint {
    fn compile(&mut self) -> Result<()> {
        self.members_compiled = compile_optional_globset(&self.members)
            .context("Failed to compile clustering constraint member patterns")?;
        self.left_compiled = compile_optional_globset(&self.left)
            .context("Failed to compile clustering constraint left patterns")?;
        self.right_compiled = compile_optional_globset(&self.right)
            .context("Failed to compile clustering constraint right patterns")?;
        Ok(())
    }

    pub fn must_group(members: Vec<String>) -> Result<Self> {
        let mut constraint = Self {
            constraint_type: ClusteringConstraintType::MustGroup,
            members,
            left: Vec::new(),
            right: Vec::new(),
            members_compiled: None,
            left_compiled: None,
            right_compiled: None,
        };
        constraint.compile()?;
        Ok(constraint)
    }

    pub fn must_separate(left: Vec<String>, right: Vec<String>) -> Result<Self> {
        let mut constraint = Self {
            constraint_type: ClusteringConstraintType::MustSeparate,
            members: Vec::new(),
            left,
            right,
            members_compiled: None,
            left_compiled: None,
            right_compiled: None,
        };
        constraint.compile()?;
        Ok(constraint)
    }

    pub fn matches_members(&self, label: &str) -> bool {
        self.members_compiled
            .as_ref()
            .is_some_and(|matcher| matcher.is_match(label))
    }

    pub fn matches_left(&self, label: &str) -> bool {
        self.left_compiled
            .as_ref()
            .is_some_and(|matcher| matcher.is_match(label))
    }

    pub fn matches_right(&self, label: &str) -> bool {
        self.right_compiled
            .as_ref()
            .is_some_and(|matcher| matcher.is_match(label))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClusteringConfig {
    #[serde(default = "default_clustering_strategy")]
    pub strategy: ClusteringStrategy,
    #[serde(default)]
    pub semantic: Option<ClusteringSemanticConfig>,
    #[serde(default)]
    pub structural: Option<ClusteringStructuralConfig>,
    #[serde(default)]
    pub presentation: Option<ClusteringPresentationConfig>,
    #[serde(default)]
    pub families: Vec<ClusteringFamily>,
    #[serde(default)]
    pub constraints: Vec<ClusteringConstraint>,
    // Legacy flat fields kept for backward compatibility with older morpharch.toml files.
    #[serde(default = "default_cluster_min_size")]
    pub min_cluster_size: usize,
    #[serde(default = "default_root_token_min_repeats")]
    pub root_token_min_repeats: usize,
    #[serde(default = "default_workspace_split_threshold")]
    pub workspace_split_threshold: usize,
    #[serde(default = "default_workspace_max_share")]
    pub workspace_max_share: f64,
    #[serde(default = "default_collapse_external")]
    pub collapse_external: bool,
    #[serde(default)]
    pub cluster_aliases: HashMap<String, String>,
    #[serde(default)]
    pub cluster_kinds: HashMap<String, ClusterKindHint>,
    #[serde(default)]
    pub rules: Vec<ClusterRule>,
}

impl Default for ClusteringConfig {
    fn default() -> Self {
        Self {
            strategy: default_clustering_strategy(),
            semantic: None,
            structural: None,
            presentation: None,
            families: Vec::new(),
            constraints: Vec::new(),
            min_cluster_size: default_cluster_min_size(),
            root_token_min_repeats: default_root_token_min_repeats(),
            workspace_split_threshold: default_workspace_split_threshold(),
            workspace_max_share: default_workspace_max_share(),
            collapse_external: default_collapse_external(),
            cluster_aliases: HashMap::new(),
            cluster_kinds: HashMap::new(),
            rules: Vec::new(),
        }
    }
}

impl ClusteringConfig {
    pub fn effective_strategy(&self) -> ClusteringStrategy {
        if !self.structural_enabled() {
            ClusteringStrategy::Namespace
        } else {
            self.strategy
        }
    }

    pub fn structural_enabled(&self) -> bool {
        self.structural
            .as_ref()
            .map(|cfg| cfg.enabled)
            .unwrap_or(true)
    }

    pub fn effective_min_cluster_size(&self) -> usize {
        self.structural
            .as_ref()
            .map(|cfg| cfg.min_cluster_size)
            .unwrap_or(self.min_cluster_size)
    }

    pub fn effective_root_token_min_repeats(&self) -> usize {
        self.semantic
            .as_ref()
            .map(|cfg| cfg.root_token_min_repeats)
            .unwrap_or(self.root_token_min_repeats)
    }

    pub fn effective_split_threshold(&self) -> usize {
        self.structural
            .as_ref()
            .map(|cfg| cfg.split_threshold)
            .unwrap_or(self.workspace_split_threshold)
    }

    pub fn effective_max_cluster_share(&self) -> f64 {
        self.structural
            .as_ref()
            .map(|cfg| cfg.max_cluster_share)
            .unwrap_or(self.workspace_max_share)
    }

    pub fn effective_collapse_external(&self) -> bool {
        self.semantic
            .as_ref()
            .map(|cfg| cfg.collapse_external)
            .unwrap_or(self.collapse_external)
    }

    pub fn effective_fallback_family(&self) -> &str {
        self.semantic
            .as_ref()
            .map(|cfg| cfg.fallback_family.as_str())
            .unwrap_or("workspace")
    }

    pub fn effective_kind_mode(&self) -> ClusterKindMode {
        self.presentation
            .as_ref()
            .map(|cfg| cfg.kind_mode)
            .unwrap_or_default()
    }

    pub fn effective_color_mode(&self) -> ClusterColorMode {
        self.presentation
            .as_ref()
            .map(|cfg| cfg.color_mode)
            .unwrap_or_default()
    }

    pub fn include_exact_roots_for_known_heads(&self) -> bool {
        self.semantic
            .as_ref()
            .map(|cfg| cfg.include_exact_roots_for_known_heads)
            .unwrap_or(true)
    }

    pub fn preserve_family_purity(&self) -> bool {
        self.structural
            .as_ref()
            .map(|cfg| cfg.preserve_family_purity)
            .unwrap_or(true)
    }

    pub fn post_merge_small_clusters(&self) -> bool {
        self.structural
            .as_ref()
            .map(|cfg| cfg.post_merge_small_clusters)
            .unwrap_or(true)
    }

    pub fn disambiguate_duplicate_names(&self) -> bool {
        self.structural
            .as_ref()
            .map(|cfg| cfg.disambiguate_duplicate_names)
            .unwrap_or(true)
    }

    pub fn display_name_for(&self, name: &str) -> String {
        self.presentation
            .as_ref()
            .and_then(|cfg| cfg.aliases.get(name))
            .or_else(|| self.cluster_aliases.get(name))
            .cloned()
            .unwrap_or_else(|| name.to_string())
    }

    pub fn kind_hint_for(&self, name: &str) -> Option<&ClusterKindHint> {
        let lower = name.to_lowercase();
        self.presentation
            .as_ref()
            .and_then(|cfg| cfg.kinds.get(name).or_else(|| cfg.kinds.get(&lower)))
            .or_else(|| {
                self.cluster_kinds
                    .get(name)
                    .or_else(|| self.cluster_kinds.get(&lower))
            })
            .or_else(|| {
                self.families
                    .iter()
                    .find(|family| family.name == name)
                    .and_then(|family| family.kind.as_ref())
            })
    }

    pub fn matching_family_name(&self, label: &str) -> Option<String> {
        self.families
            .iter()
            .enumerate()
            .filter(|(_, family)| family.matches(label))
            .max_by(|(idx_a, family_a), (idx_b, family_b)| {
                family_a
                    .priority
                    .cmp(&family_b.priority)
                    .then_with(|| family_a.include.len().cmp(&family_b.include.len()))
                    .then_with(|| idx_b.cmp(idx_a))
            })
            .map(|(_, family)| family.name.clone())
            .or_else(|| matching_rule_name(label, &self.rules))
    }

    pub fn family_split_mode(&self, family_name: &str) -> FamilySplitMode {
        self.families
            .iter()
            .find(|family| family.name == family_name)
            .map(|family| family.split)
            .unwrap_or(FamilySplitMode::Allow)
    }

    pub fn family_prefers_small_merge(&self, family_name: &str) -> bool {
        self.families
            .iter()
            .find(|family| family.name == family_name)
            .map(|family| family.merge_small_into_family)
            .unwrap_or(true)
    }

    pub fn constraints_of_type(
        &self,
        constraint_type: ClusteringConstraintType,
    ) -> impl Iterator<Item = &ClusteringConstraint> {
        self.constraints
            .iter()
            .filter(move |constraint| constraint.constraint_type == constraint_type)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClusterRule {
    pub name: String,
    #[serde(default)]
    pub kind: Option<ClusterKindHint>,
    #[serde(default, rename = "match")]
    pub patterns: Vec<String>,
    #[serde(skip)]
    compiled: Option<GlobSet>,
}

impl ClusterRule {
    fn compile(&mut self) -> Result<()> {
        if self.patterns.is_empty() {
            self.compiled = None;
            return Ok(());
        }

        let mut builder = GlobSetBuilder::new();
        for pattern in &self.patterns {
            builder.add(
                Glob::new(pattern)
                    .with_context(|| format!("Invalid clustering glob pattern: {pattern}"))?,
            );
        }
        self.compiled = Some(
            builder
                .build()
                .context("Failed to compile clustering glob patterns")?,
        );
        Ok(())
    }

    pub fn matches(&self, label: &str) -> bool {
        self.compiled
            .as_ref()
            .is_some_and(|matcher| matcher.is_match(label))
    }
}

fn compile_optional_globset(patterns: &[String]) -> Result<Option<GlobSet>> {
    if patterns.is_empty() {
        return Ok(None);
    }

    let mut builder = GlobSetBuilder::new();
    for pattern in patterns {
        builder.add(
            Glob::new(pattern)
                .with_context(|| format!("Invalid clustering glob pattern: {pattern}"))?,
        );
    }
    Ok(Some(
        builder
            .build()
            .context("Failed to compile clustering glob patterns")?,
    ))
}

fn matching_rule_name(label: &str, rules: &[ClusterRule]) -> Option<String> {
    rules
        .iter()
        .find(|rule| !rule.name.is_empty() && rule.matches(label))
        .map(|rule| rule.name.clone())
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

    /// Blast radius score threshold for "high impact" module classification (default: 0.3).
    #[serde(default = "default_blast_high_impact_threshold")]
    pub blast_high_impact_threshold: f64,

    /// Maximum number of critical paths to compute (default: 5).
    #[serde(default = "default_blast_max_critical_paths")]
    pub blast_max_critical_paths: usize,
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
fn default_blast_high_impact_threshold() -> f64 {
    0.3
}
fn default_blast_max_critical_paths() -> usize {
    5
}

impl Default for Thresholds {
    fn default() -> Self {
        Self {
            hub_exemption_ratio: 0.3,
            entry_point_max_fan_in: 2,
            brittle_instability_ratio: 0.8,
            blast_high_impact_threshold: 0.3,
            blast_max_critical_paths: 5,
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
    #[serde(skip)]
    from_compiled: Option<GlobSet>,
    #[serde(skip)]
    deny_compiled: Option<GlobSet>,
}

impl BoundaryRule {
    fn compile(&mut self) -> Result<()> {
        self.from_compiled = compile_optional_globset(std::slice::from_ref(&self.from))
            .context("Failed to compile boundary source pattern")?;
        self.deny_compiled = compile_optional_globset(&self.deny)
            .context("Failed to compile boundary deny patterns")?;
        Ok(())
    }

    pub fn matches_from(&self, from: &str) -> bool {
        if let Some(matcher) = self.from_compiled.as_ref() {
            return matcher.is_match(from);
        }
        Glob::new(&self.from)
            .ok()
            .is_some_and(|glob| glob.compile_matcher().is_match(from))
    }

    pub fn matches_to(&self, to: &str) -> bool {
        if let Some(matcher) = self.deny_compiled.as_ref() {
            return matcher.is_match(to);
        }
        self.deny.iter().any(|pattern| {
            Glob::new(pattern)
                .ok()
                .is_some_and(|glob| glob.compile_matcher().is_match(to))
        })
    }

    pub fn matches(&self, from: &str, to: &str) -> bool {
        self.matches_from(from) && self.matches_to(to)
    }
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
        assert!((t.blast_high_impact_threshold - 0.3).abs() < f64::EPSILON);
        assert_eq!(t.blast_max_critical_paths, 5);
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

[scan]
package_depth = 1
external_min_importers = 0
test_path_patterns = ["/examples/", "/benchmarks/"]

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
        assert_eq!(config.scan.package_depth, 1);
        assert_eq!(config.scan.external_min_importers, 0);
        assert_eq!(
            config.scan.test_path_patterns,
            vec!["/examples/", "/benchmarks/"]
        );
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
        assert!(config.ignore.use_defaults);
        assert_eq!(config.scan.package_depth, 2);
        assert_eq!(config.scan.external_min_importers, 3);
        assert_eq!(config.scan.test_path_patterns, DEFAULT_TEST_PATH_PATTERNS);
        assert!((config.scoring.weights.total() - 100.0).abs() < f64::EPSILON);
        assert!(config.scoring.boundaries.is_empty());
        assert!(config.scoring.exemptions.hub_exempt.is_empty());
        assert_eq!(config.scoring.exemptions.entry_point_stems.len(), 5);
    }

    #[test]
    fn test_scan_config_validation_rejects_zero_package_depth() {
        let toml_str = r#"
[scan]
package_depth = 0
"#;
        let config: ProjectConfig = toml::from_str(toml_str).unwrap();
        let err = config.scan.validate().unwrap_err();
        assert!(err.to_string().contains("scan.package_depth"));
    }

    #[test]
    fn test_ignore_fingerprint_includes_scan_settings() {
        let base = ProjectConfig::default().ignore_fingerprint().unwrap();
        let changed = ProjectConfig {
            scan: ScanConfig {
                external_min_importers: 0,
                ..ScanConfig::default()
            },
            ..ProjectConfig::default()
        }
        .ignore_fingerprint()
        .unwrap();

        assert_ne!(base, changed);
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
        assert!(globs.is_match(".github/workflows/ci.generate.ts"));
        assert!(globs.is_match("cli/tsc/dts/node/internal.d.ts"));
        assert!(!globs.is_match("src/main.rs"));
    }

    #[test]
    fn test_ignore_config_can_disable_default_presets() {
        let toml_str = r#"
[ignore]
use_defaults = false
paths = ["tests/**"]
"#;
        let mut config: ProjectConfig = toml::from_str(toml_str).unwrap();
        config.compile_ignore_globs().unwrap();
        let globs = config.ignore_globs().unwrap();
        assert!(globs.is_match("tests/unit/foo.rs"));
        assert!(!globs.is_match(".github/workflows/ci.generate.ts"));
        assert!(!globs.is_match("cli/tsc/dts/node/internal.d.ts"));
    }

    #[test]
    fn test_ignore_config_supports_custom_presets() {
        let toml_str = r#"
[ignore]
use_defaults = false
presets = ["repo_noise"]
paths = ["custom/**"]

[ignore.custom_presets]
repo_noise = [".github/**", "scripts/dev/**"]
"#;
        let mut config: ProjectConfig = toml::from_str(toml_str).unwrap();
        config.compile_ignore_globs().unwrap();
        let globs = config.ignore_globs().unwrap();
        assert!(globs.is_match(".github/workflows/ci.generate.ts"));
        assert!(globs.is_match("scripts/dev/bootstrap.ts"));
        assert!(globs.is_match("custom/file.rs"));
        assert!(!globs.is_match("src/main.rs"));
    }

    #[test]
    fn test_ignore_config_unknown_preset_is_error() {
        let toml_str = r#"
[ignore]
presets = ["unknown"]
"#;
        let mut config: ProjectConfig = toml::from_str(toml_str).unwrap();
        let err = config.compile_ignore_globs().unwrap_err();
        assert!(err.to_string().contains("Unknown ignore preset"));
    }

    #[test]
    fn test_clustering_aliases_deserialize() {
        let toml_str = r#"
[clustering]
strategy = "hybrid"

[clustering.cluster_aliases]
workspace = "platform"
deps = "third_party"
"#;
        let config: ProjectConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(
            config.clustering.cluster_aliases.get("workspace"),
            Some(&"platform".to_string())
        );
        assert_eq!(
            config.clustering.cluster_aliases.get("deps"),
            Some(&"third_party".to_string())
        );
    }

    #[test]
    fn test_clustering_kinds_deserialize() {
        let toml_str = r#"
[clustering.cluster_kinds]
deps = "deps"
platform = "infra"
website = "entry"
"#;
        let config: ProjectConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(
            config.clustering.cluster_kinds.get("deps"),
            Some(&ClusterKindHint::Deps)
        );
        assert_eq!(
            config.clustering.cluster_kinds.get("platform"),
            Some(&ClusterKindHint::Infra)
        );
        assert_eq!(
            config.clustering.cluster_kinds.get("website"),
            Some(&ClusterKindHint::Entry)
        );
    }

    #[test]
    fn test_clustering_presentation_kind_mode_deserialize() {
        let toml_str = r#"
[clustering.presentation]
kind_mode = "explicit_only"
"#;
        let config: ProjectConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(
            config.clustering.effective_kind_mode(),
            ClusterKindMode::ExplicitOnly
        );
    }

    #[test]
    fn test_clustering_presentation_color_mode_deserialize() {
        let toml_str = r#"
[clustering.presentation]
color_mode = "semantic"
"#;
        let config: ProjectConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(
            config.clustering.effective_color_mode(),
            ClusterColorMode::Semantic
        );
    }

    #[test]
    fn test_clustering_presentation_color_mode_defaults_to_minimal() {
        let config = ProjectConfig::default();
        assert_eq!(
            config.clustering.effective_color_mode(),
            ClusterColorMode::Minimal
        );
    }

    #[test]
    fn test_clustering_rule_kind_populates_name_override() {
        let toml_str = r#"
[[clustering.rules]]
name = "git"
kind = "infra"
match = ["git_*", "git/**"]
"#;
        let mut config: ProjectConfig = toml::from_str(toml_str).unwrap();
        config.compile_clustering_rules().unwrap();
        assert_eq!(
            config.clustering.cluster_kinds.get("git"),
            Some(&ClusterKindHint::Infra)
        );
    }

    #[test]
    fn test_nested_clustering_tables_deserialize() {
        let toml_str = r#"
[clustering]
strategy = "hybrid"

[clustering.semantic]
collapse_external = false
fallback_family = "misc"
include_exact_roots_for_known_heads = false

[clustering.structural]
enabled = true
split_threshold = 9
preserve_family_purity = true

[clustering.presentation.aliases]
workspace = "platform"

[[clustering.families]]
name = "runtime"
include = ["runtime", "runtime/**"]
kind = "infra"
split = "never"
"#;
        let mut config: ProjectConfig = toml::from_str(toml_str).unwrap();
        config.compile_clustering_rules().unwrap();

        assert_eq!(
            config.clustering.semantic.as_ref().unwrap().fallback_family,
            "misc"
        );
        assert!(!config.clustering.effective_collapse_external());
        assert_eq!(config.clustering.effective_split_threshold(), 9);
        assert_eq!(config.clustering.display_name_for("workspace"), "platform");
        assert_eq!(
            config.clustering.kind_hint_for("runtime"),
            Some(&ClusterKindHint::Infra)
        );
        assert_eq!(
            config.clustering.family_split_mode("runtime"),
            FamilySplitMode::Never
        );
    }

    #[test]
    fn test_nested_clustering_fields_override_legacy_values() {
        let toml_str = r#"
[clustering]
min_cluster_size = 2
collapse_external = true

[clustering.semantic]
collapse_external = false
root_token_min_repeats = 5

[clustering.structural]
min_cluster_size = 7
enabled = false
"#;
        let config: ProjectConfig = toml::from_str(toml_str).unwrap();
        assert!(!config.clustering.effective_collapse_external());
        assert_eq!(config.clustering.effective_root_token_min_repeats(), 5);
        assert_eq!(config.clustering.effective_min_cluster_size(), 7);
        assert_eq!(
            config.clustering.effective_strategy(),
            ClusteringStrategy::Namespace
        );
    }

    #[test]
    fn test_clustering_constraints_deserialize() {
        let toml_str = r#"
[[clustering.constraints]]
type = "must_group"
members = ["core", "core/**"]

[[clustering.constraints]]
type = "must_separate"
left = ["deps"]
right = ["runtime/**"]
"#;
        let mut config: ProjectConfig = toml::from_str(toml_str).unwrap();
        config.compile_clustering_rules().unwrap();

        assert_eq!(config.clustering.constraints.len(), 2);
        assert_eq!(
            config.clustering.constraints[0].constraint_type,
            ClusteringConstraintType::MustGroup
        );
        assert!(config.clustering.constraints[0].matches_members("core/service"));
        assert_eq!(
            config.clustering.constraints[1].constraint_type,
            ClusteringConstraintType::MustSeparate
        );
        assert!(config.clustering.constraints[1].matches_right("runtime/ops"));
    }
}
