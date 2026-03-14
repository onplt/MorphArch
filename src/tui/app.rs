// =============================================================================
// tui/app.rs — Main TUI application state and event loop
// =============================================================================

use std::collections::{HashMap, HashSet};
use std::num::NonZeroUsize;
use std::time::{Duration, Instant};

use crossterm::event::{
    self, Event, KeyCode, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use lru::LruCache;
use petgraph::graph::DiGraph;
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::canvas::Canvas;
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};

use super::architecture_map::{ArchitectureMap, ClusterEdge, ClusterNode, ClusterOverviewRole};
use crate::analysis;
use crate::blast_radius;
use crate::config::{ClusterColorMode, ClusteringConfig, ScoringConfig};
use crate::db::Database;
use crate::graph_builder;
use crate::models::{BlastRadiusReport, DriftScore, GraphSnapshot, NodeKind, SnapshotMetadata};
use crate::scoring;

/// Which panel currently has keyboard focus
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusedPanel {
    Packages,
    Graph,
    Insights,
    Timeline,
}

/// Navigation context — forms a stack for drill-down
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ViewContext {
    Overview,
    PackageDetail(String),
    ModuleInspect(String),
}

/// Active tab in the insights panel
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InsightTab {
    Overview,
    Hotspots,
    Blast,
}

// Keep ActiveView for backward compat during transition
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActiveView {
    Dashboard,
    Inspect(String),
    MacroGraph,
}

#[derive(Debug, Clone)]
enum SidebarEntry {
    Cluster { id: usize, label: String },
    Member { id: usize, label: String },
}

use super::graph_renderer::{
    ACCENT_BLUE, ACCENT_LAVENDER, ACCENT_MAUVE, BG_BASE, BG_SURFACE, ClusterMapSemantic,
    FG_OVERLAY, FG_TEXT, GraphLayout, GraphRelationSemantic, OverviewEdgeSemantic,
    cluster_map_color, graph_relation_color, overview_edge_color, weighted_edge_color,
};
use super::insight_panel::render_insight_panel;
use super::timeline::{TimelineState, render_timeline};

fn adaptive_steps(n_nodes: usize, base: usize, min: usize) -> usize {
    if n_nodes <= 100 {
        base
    } else {
        (base * 100 / n_nodes).clamp(min, base)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotspotsSort {
    Instability,
    FanIn,
    FanOut,
}

pub struct App {
    pub db: Option<Database>,
    pub repo_id: String,
    pub graph_layout: GraphLayout,
    pub timeline: TimelineState,
    pub snapshots_metadata: Vec<SnapshotMetadata>,
    pub snapshot_cache: LruCache<String, GraphSnapshot>,
    pub current_drift: Option<DriftScore>,
    pub is_playing: bool,
    pub should_quit: bool,
    pub last_auto_advance: Instant,
    pub auto_play_interval: Duration,
    pub tick_rate: Duration,
    pub last_tick: Instant,
    pub frame_count: u64,
    pub dragging_node: Option<usize>,
    pub graph_area: Rect,
    pub needs_warmup: bool,
    pub pkg_scroll_offset: usize,
    pub pkg_area: Rect,
    pub timeline_area: Rect,
    pub pkg_panel_width: u16,
    pub resizing_pkg: bool,
    pub dragging_timeline: bool,
    pub hovered_node: Option<usize>,
    pub graph_scale: f64,
    pub graph_pan_x: f64,
    pub graph_pan_y: f64,
    pub dragging_pan: bool,
    pub last_mouse_pos: Option<(u16, u16)>,
    pending_graph_focus: bool,
    render_cache: Option<GraphRenderCache>,
    pub architecture_map: Option<ArchitectureMap>,
    pub internal_node_indices: HashSet<usize>,
    pub selected_cluster: Option<usize>,
    pub hovered_cluster: Option<usize>,
    /// Commit hash currently being loaded from DB
    pub loading_hash: Option<String>,
    /// List of (package_name, instability_score) for the current graph
    pub brittle_packages: Vec<(String, f64, usize, usize)>,
    pub hotspots_state: ratatui::widgets::TableState,
    pub hotspots_sort: HotspotsSort,
    pub active_view: ActiveView,

    // ── New TUI redesign fields ──
    /// Which panel currently has keyboard focus
    pub focused_panel: FocusedPanel,
    /// Navigation drill-down stack
    pub nav_stack: Vec<ViewContext>,
    /// Active tab in the insights panel
    pub insight_tab: InsightTab,
    /// Help overlay visible
    pub show_help: bool,
    /// Left sidebar visible (toggle with 'b')
    pub sidebar_visible: bool,
    /// Right insights panel visible (toggle with 'i')
    pub insights_visible: bool,
    /// Filter bar currently active (typing mode)
    pub filter_active: bool,
    /// Current filter text (replaces search_query/show_search)
    pub filter_text: String,
    /// Selected package index in sidebar
    pub selected_pkg_index: Option<usize>,
    /// Repo name for breadcrumb
    pub repo_name: String,
    /// Insights panel width
    pub insights_panel_width: u16,
    /// Insights panel area (for mouse interaction)
    pub insights_area: Rect,
    /// Is the user currently dragging the insights panel border?
    pub resizing_insights: bool,
    /// Whether the sidebar was visible in the last rendered layout
    effective_sidebar_visible: bool,
    /// Whether the insights panel was visible in the last rendered layout
    effective_insights_visible: bool,
    /// Component diagnostic advisory lines (computed from current graph)
    pub advisory_lines: Vec<String>,
    /// Scoring configuration for diagnostics generation
    pub scoring_config: ScoringConfig,
    pub clustering_config: ClusteringConfig,

    // ── Blast Radius Cartography ──
    /// Blast radius overlay mode active (toggle with 'x')
    pub blast_overlay_active: bool,
    /// Cached blast radius report for the current snapshot
    pub current_blast_radius: Option<BlastRadiusReport>,
    /// Per-node blast scores indexed by graph layout position
    pub node_blast_scores: Vec<f64>,
    /// Single-node cascade highlight: (layout_idx, distance, impact)
    pub cascade_highlight: Option<Vec<(usize, u32, f64)>>,
    /// Scroll offset for the Blast tab TOP IMPACT list
    pub blast_impact_scroll: usize,
    /// Number of snapshots skipped during initial load due to corruption
    pub skipped_snapshot_count: usize,
    /// Whether the current snapshot required legacy artifact recomputation
    pub legacy_snapshot_recomputed: bool,
}

struct GraphRenderCache {
    label_visible: HashSet<usize>,
    sorted_edge_indices: Vec<usize>,
}

struct ResolvedSnapshotAnalysis {
    drift: Option<DriftScore>,
    blast_radius: Option<BlastRadiusReport>,
    instability_metrics: Vec<(String, f64, usize, usize)>,
    diagnostics: Vec<String>,
    legacy_recomputed: bool,
}

impl App {
    pub fn new(
        db: Option<Database>,
        repo_id: String,
        snapshots_metadata: Vec<SnapshotMetadata>,
        initial_snapshot: Option<GraphSnapshot>,
    ) -> Self {
        let timeline_commits: Vec<(String, String, i64)> = snapshots_metadata
            .iter()
            .map(|meta| (meta.commit_hash.clone(), String::new(), meta.timestamp))
            .collect();

        let timeline = TimelineState::new(timeline_commits);
        let mut snapshot_cache = LruCache::new(NonZeroUsize::new(8).expect("non-zero cache size"));
        if let Some(snapshot) = initial_snapshot {
            snapshot_cache.put(snapshot.commit_hash.clone(), snapshot);
        }

        let (labels, edges, weights, internal_node_indices) = snapshots_metadata
            .first()
            .and_then(|first_meta| snapshot_cache.peek(&first_meta.commit_hash))
            .map(|first| {
                let (labels, edges, weights) = snapshot_to_layout_data(first);
                let internal = snapshot_internal_nodes(first, &labels);
                (labels, edges, weights, internal)
            })
            .unwrap_or_else(|| (vec![], vec![], vec![], HashSet::new()));

        let graph_layout = GraphLayout::new(labels, edges, weights, 500.0, 500.0);
        let current_drift = snapshots_metadata.first().and_then(|m| m.drift.clone());

        let now = Instant::now();

        let mut app = Self {
            db,
            repo_id,
            graph_layout,
            timeline,
            snapshots_metadata,
            snapshot_cache,
            current_drift,
            is_playing: false,
            should_quit: false,
            last_auto_advance: now,
            auto_play_interval: Duration::from_millis(1500),
            tick_rate: Duration::from_millis(33),
            last_tick: now,
            frame_count: 0,
            dragging_node: None,
            graph_area: Rect::default(),
            needs_warmup: true,
            pkg_scroll_offset: 0,
            pkg_area: Rect::default(),
            timeline_area: Rect::default(),
            pkg_panel_width: 20,
            resizing_pkg: false,
            dragging_timeline: false,
            hovered_node: None,
            graph_scale: 1.0,
            graph_pan_x: 0.0,
            graph_pan_y: 0.0,
            dragging_pan: false,
            last_mouse_pos: None,
            pending_graph_focus: false,
            render_cache: None,
            architecture_map: None,
            internal_node_indices,
            selected_cluster: None,
            hovered_cluster: None,
            loading_hash: None,
            brittle_packages: Vec::new(),
            hotspots_state: ratatui::widgets::TableState::default(),
            hotspots_sort: HotspotsSort::Instability,
            active_view: ActiveView::Dashboard,
            // New TUI redesign fields
            focused_panel: FocusedPanel::Graph,
            nav_stack: vec![ViewContext::Overview],
            insight_tab: InsightTab::Overview,
            show_help: false,
            sidebar_visible: true,
            insights_visible: true,
            filter_active: false,
            filter_text: String::new(),
            selected_pkg_index: None,
            repo_name: String::new(),
            insights_panel_width: 36,
            insights_area: Rect::default(),
            resizing_insights: false,
            effective_sidebar_visible: true,
            effective_insights_visible: true,
            advisory_lines: Vec::new(),
            scoring_config: ScoringConfig::default(),
            clustering_config: ClusteringConfig::default(),
            // Blast radius cartography
            blast_overlay_active: false,
            current_blast_radius: None,
            node_blast_scores: Vec::new(),
            cascade_highlight: None,
            blast_impact_scroll: 0,
            skipped_snapshot_count: 0,
            legacy_snapshot_recomputed: false,
        };

        if let Some(first_meta) = app.snapshots_metadata.first() {
            let hash = first_meta.commit_hash.clone();
            app.refresh_render_cache(&hash);
            app.compute_insights();
            app.sync_sidebar_selection();
            app.focus_current_graph_view();
        }

        app
    }

    /// Computes architectural insights like instability for the current graph.
    pub fn compute_insights(&mut self) {
        self.architecture_map = ArchitectureMap::build(
            &self.graph_layout.labels,
            &self.graph_layout.edges,
            &self.graph_layout.edge_weights,
            Some(&self.internal_node_indices),
            &self.clustering_config,
        );

        let Some(snapshot) = self.current_snapshot().cloned() else {
            self.current_drift = None;
            self.brittle_packages.clear();
            self.advisory_lines.clear();
            self.current_blast_radius = None;
            self.node_blast_scores.clear();
            self.legacy_snapshot_recomputed = false;
            return;
        };

        let resolved = self.resolve_snapshot_analysis(&snapshot);
        self.current_drift = resolved.drift;
        self.brittle_packages = resolved.instability_metrics;
        self.apply_hotspots_sort();
        self.advisory_lines = resolved.diagnostics;
        self.legacy_snapshot_recomputed = resolved.legacy_recomputed;
        self.current_blast_radius = resolved.blast_radius;
        self.node_blast_scores = self
            .current_blast_radius
            .as_ref()
            .map(|blast_report| {
                self.graph_layout
                    .labels
                    .iter()
                    .map(|label| {
                        blast_report
                            .impacts
                            .iter()
                            .find(|m| m.module_name == *label)
                            .map(|m| m.blast_score)
                            .unwrap_or(0.0)
                    })
                    .collect()
            })
            .unwrap_or_else(|| vec![0.0; self.graph_layout.labels.len()]);
        self.cascade_highlight = None;
        self.blast_impact_scroll = 0;
    }

    pub fn apply_hotspots_sort(&mut self) {
        match self.hotspots_sort {
            HotspotsSort::Instability => {
                self.brittle_packages.sort_by(|a, b| {
                    let total_b = b.2 + b.3;
                    let total_a = a.2 + a.3;
                    total_b
                        .cmp(&total_a)
                        .then(b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal))
                });
            }
            HotspotsSort::FanIn => {
                self.brittle_packages.sort_by(|a, b| b.2.cmp(&a.2));
            }
            HotspotsSort::FanOut => {
                self.brittle_packages.sort_by(|a, b| b.3.cmp(&a.3));
            }
        }
        // After sort, select the first item so the highlight follows the new order
        if !self.brittle_packages.is_empty() {
            self.hotspots_state.select(Some(0));
        }
    }

    pub fn select_next_hotspot(&mut self) {
        if self.brittle_packages.is_empty() {
            return;
        }
        let max = self.brittle_packages.len().saturating_sub(1);
        let i = match self.hotspots_state.selected() {
            Some(i) => (i + 1).min(max),
            None => 0,
        };
        self.hotspots_state.select(Some(i));
        self.focus_selected_hotspot();
    }

    pub fn select_prev_hotspot(&mut self) {
        if self.brittle_packages.is_empty() {
            return;
        }
        let i = match self.hotspots_state.selected() {
            Some(i) => i.saturating_sub(1),
            None => 0,
        };
        self.hotspots_state.select(Some(i));
        self.focus_selected_hotspot();
    }

    fn focus_selected_hotspot(&mut self) {
        if let Some(i) = self.hotspots_state.selected()
            && let Some(pkg) = self.brittle_packages.get(i)
        {
            // Focus the graph on the selected node if it exists
            if let Some(idx) = self.graph_layout.labels.iter().position(|l| l == &pkg.0) {
                self.hovered_node = Some(idx);
            }
        }
    }

    pub fn refresh_render_cache(&mut self, _hash: &str) {
        // ... (existing logic) ...
        let n_nodes = self.graph_layout.positions.len();
        let mut degrees = vec![0; n_nodes];
        for &(from, to) in &self.graph_layout.edges {
            if from < n_nodes {
                degrees[from] += 1;
            }
            if to < n_nodes {
                degrees[to] += 1;
            }
        }

        let max_labels = if n_nodes > 200 {
            (n_nodes / 7).clamp(20, 60)
        } else if n_nodes > 80 {
            (n_nodes / 3).max(20)
        } else {
            n_nodes
        };

        let mut ranked: Vec<(usize, usize)> = degrees.iter().copied().enumerate().collect();
        ranked.sort_by(|a, b| b.1.cmp(&a.1));
        let label_visible = ranked
            .into_iter()
            .take(max_labels)
            .map(|(i, _)| i)
            .collect();

        let mut sorted_edge_indices: Vec<usize> = (0..self.graph_layout.edges.len()).collect();
        sorted_edge_indices
            .sort_by_key(|&i| self.graph_layout.edge_weights.get(i).copied().unwrap_or(1));

        self.render_cache = Some(GraphRenderCache {
            label_visible,
            sorted_edge_indices,
        });
    }

    pub fn set_timeline_commits(&mut self, commits: Vec<(String, String, i64)>) {
        self.timeline = TimelineState::new(commits);
    }

    fn update_graph_for_current_commit(&mut self) {
        let idx = self.timeline.current_index;
        let meta = match self.snapshots_metadata.get(idx) {
            Some(m) => m,
            None => return,
        };

        let hash = meta.commit_hash.clone();

        // Clone from cache to avoid borrow checker conflicts
        let cached_snapshot = self.snapshot_cache.peek(&hash).cloned();

        if let Some(snapshot) = cached_snapshot {
            self.apply_snapshot(&snapshot);
            self.loading_hash = None;
        } else {
            self.loading_hash = Some(hash);
        }
    }

    fn apply_snapshot(&mut self, snapshot: &GraphSnapshot) {
        let (labels, edges, weights) = snapshot_to_layout_data(snapshot);
        self.internal_node_indices = snapshot_internal_nodes(snapshot, &labels);
        self.graph_layout.update_graph(labels, edges, weights);
        self.refresh_render_cache(&snapshot.commit_hash);
        self.compute_insights();
        self.dragging_node = None;
        self.hovered_node = None;
        self.hovered_cluster = None;
        self.selected_cluster = None;
        self.active_view = ActiveView::Dashboard;
        self.nav_stack = vec![ViewContext::Overview];
        self.selected_pkg_index = None;
        self.pkg_scroll_offset = 0;
        for pos in &mut self.graph_layout.positions {
            pos.prev_x = pos.x;
            pos.prev_y = pos.y;
        }
        let n = self.graph_layout.positions.len();
        let steps = adaptive_steps(n, 300, 3);
        self.graph_layout.multi_step(steps);
        self.graph_layout.center_layout();
        self.graph_layout.temperature = 0.01;
        self.sync_sidebar_selection();
        self.focus_current_graph_view();
    }

    pub fn next_commit(&mut self) {
        let old_idx = self.timeline.current_index;
        self.timeline.next();
        if self.timeline.current_index != old_idx {
            self.update_graph_for_current_commit();
        }
    }

    pub fn prev_commit(&mut self) {
        let old_idx = self.timeline.current_index;
        self.timeline.prev();
        if self.timeline.current_index != old_idx {
            self.update_graph_for_current_commit();
        }
    }

    pub fn jump_commit(&mut self, n: isize) {
        let old_idx = self.timeline.current_index;
        self.timeline.jump_by(n);
        if self.timeline.current_index != old_idx {
            self.update_graph_for_current_commit();
        }
    }

    pub fn jump_to_first(&mut self) {
        let old_idx = self.timeline.current_index;
        self.timeline.jump_to_start();
        if self.timeline.current_index != old_idx {
            self.update_graph_for_current_commit();
        }
    }

    pub fn jump_to_last(&mut self) {
        let old_idx = self.timeline.current_index;
        self.timeline.jump_to_end();
        if self.timeline.current_index != old_idx {
            self.update_graph_for_current_commit();
        }
    }

    pub fn seek_to(&mut self, index: usize) {
        let old_idx = self.timeline.current_index;
        self.timeline.jump_to(index);
        if self.timeline.current_index != old_idx {
            self.update_graph_for_current_commit();
        }
    }

    pub fn reheat_layout(&mut self) {
        self.graph_layout.reheat();
    }

    /// Get current view context (top of nav stack)
    pub fn current_view(&self) -> &ViewContext {
        self.nav_stack.last().unwrap_or(&ViewContext::Overview)
    }

    /// Push a new view context onto the navigation stack.
    /// If we're already drilled into a module, replace it instead of stacking.
    pub fn push_view(&mut self, ctx: ViewContext) {
        // If the top of the stack is already a ModuleInspect, replace it
        // instead of appending (prevents infinite breadcrumb stacking)
        if let Some(last) = self.nav_stack.last()
            && matches!(last, ViewContext::ModuleInspect(_))
            && matches!(ctx, ViewContext::ModuleInspect(_))
        {
            self.nav_stack.pop();
        }
        if let Some(last) = self.nav_stack.last()
            && matches!(last, ViewContext::PackageDetail(_))
            && matches!(ctx, ViewContext::PackageDetail(_))
        {
            self.nav_stack.pop();
        }
        self.nav_stack.push(ctx);
    }

    /// Pop the navigation stack (go back). Returns false if already at root.
    pub fn pop_view(&mut self) -> bool {
        if self.nav_stack.len() > 1 {
            self.nav_stack.pop();
            // Sync active_view for graph rendering compatibility
            match self.current_view() {
                ViewContext::Overview => {
                    self.active_view = ActiveView::Dashboard;
                    self.selected_cluster = None;
                    self.hovered_cluster = None;
                    self.selected_pkg_index = None;
                    self.pkg_scroll_offset = 0;
                }
                ViewContext::PackageDetail(_) => {
                    self.active_view = ActiveView::Dashboard;
                }
                ViewContext::ModuleInspect(name) => {
                    self.active_view = ActiveView::Inspect(name.clone());
                }
            }
            self.sync_sidebar_selection();
            self.focus_current_graph_view();
            true
        } else {
            false
        }
    }

    /// Cycle focus to the next visible panel
    pub fn focus_next(&mut self) {
        self.focused_panel = match self.focused_panel {
            FocusedPanel::Packages => FocusedPanel::Graph,
            FocusedPanel::Graph => {
                if self.effective_insights_visible {
                    FocusedPanel::Insights
                } else {
                    FocusedPanel::Timeline
                }
            }
            FocusedPanel::Insights => FocusedPanel::Timeline,
            FocusedPanel::Timeline => {
                if self.effective_sidebar_visible {
                    FocusedPanel::Packages
                } else {
                    FocusedPanel::Graph
                }
            }
        };
    }

    /// Cycle focus to the previous visible panel
    pub fn focus_prev(&mut self) {
        self.focused_panel = match self.focused_panel {
            FocusedPanel::Packages => FocusedPanel::Timeline,
            FocusedPanel::Graph => {
                if self.effective_sidebar_visible {
                    FocusedPanel::Packages
                } else {
                    FocusedPanel::Timeline
                }
            }
            FocusedPanel::Insights => FocusedPanel::Graph,
            FocusedPanel::Timeline => {
                if self.effective_insights_visible {
                    FocusedPanel::Insights
                } else {
                    FocusedPanel::Graph
                }
            }
        };
    }

    fn sync_visible_panels(&mut self, sidebar: bool, insights: bool) {
        self.effective_sidebar_visible = sidebar;
        self.effective_insights_visible = insights;

        if (matches!(self.focused_panel, FocusedPanel::Packages) && !sidebar)
            || (matches!(self.focused_panel, FocusedPanel::Insights) && !insights)
        {
            self.focused_panel = FocusedPanel::Graph;
        }
    }

    /// Compute the set of visible node indices for the current inspect/filter state.
    /// Returns `None` when no filtering is active (all nodes visible).
    fn visible_node_set(&self) -> Option<HashSet<usize>> {
        let inspect_center = if let ActiveView::Inspect(ref name) = self.active_view {
            self.graph_layout.labels.iter().position(|l| l == name)
        } else {
            None
        };

        if !self.filter_text.is_empty() {
            let q = self.filter_text.to_lowercase();
            let matched: HashSet<usize> = self
                .graph_layout
                .labels
                .iter()
                .enumerate()
                .filter(|(_, l)| l.to_lowercase().contains(&q))
                .map(|(i, _)| i)
                .collect();
            let mut visible = matched.clone();
            for &(f, t) in &self.graph_layout.edges {
                if matched.contains(&f) {
                    visible.insert(t);
                }
                if matched.contains(&t) {
                    visible.insert(f);
                }
            }
            Some(visible)
        } else if let Some(center) = inspect_center {
            let mut visible = HashSet::new();
            visible.insert(center);
            for &(f, t) in &self.graph_layout.edges {
                if f == center || t == center {
                    visible.insert(f);
                    visible.insert(t);
                }
            }
            Some(visible)
        } else {
            None
        }
    }

    fn should_show_architecture_overview(&self) -> bool {
        self.selected_cluster.is_none()
            && !matches!(self.active_view, ActiveView::Inspect(_))
            && self.architecture_map.is_some()
    }

    fn reset_graph_viewport(&mut self) {
        self.graph_scale = 1.0;
        self.graph_pan_x = 0.0;
        self.graph_pan_y = 0.0;
    }

    fn focus_current_graph_view(&mut self) {
        self.reset_graph_viewport();
        self.pending_graph_focus = true;
        if self.selected_cluster.is_none() && !self.should_show_architecture_overview() {
            self.graph_layout.center_layout();
        }
    }

    fn apply_pending_graph_focus(
        &mut self,
        canvas_w: f64,
        canvas_h: f64,
        center_idx: Option<usize>,
    ) {
        if !self.pending_graph_focus {
            return;
        }
        self.pending_graph_focus = false;

        let Some(center_idx) = center_idx else {
            return;
        };
        let Some(pos) = self.graph_layout.positions.get(center_idx) else {
            return;
        };

        let visible_w = canvas_w.max(1.0) / self.graph_scale.max(0.1);
        let visible_h = canvas_h.max(1.0) / self.graph_scale.max(0.1);
        self.graph_pan_x = pos.x - visible_w / 2.0;
        self.graph_pan_y = pos.y - visible_h / 2.0;
    }

    fn node_total_weight(&self, node_id: usize) -> u32 {
        self.graph_layout
            .edges
            .iter()
            .enumerate()
            .filter_map(|(edge_idx, &(from, to))| {
                if from == node_id || to == node_id {
                    Some(
                        self.graph_layout
                            .edge_weights
                            .get(edge_idx)
                            .copied()
                            .unwrap_or(1),
                    )
                } else {
                    None
                }
            })
            .sum()
    }

    fn set_selected_sidebar_index(&mut self, idx: usize) {
        self.selected_pkg_index = Some(idx);
        self.sync_sidebar_selection();
    }

    fn sidebar_visible_capacity(&self, total_entries: usize) -> usize {
        let inner_rows = self.pkg_area.height.saturating_sub(2) as usize;
        if inner_rows == 0 {
            0
        } else if total_entries > inner_rows {
            inner_rows.saturating_sub(1).max(1)
        } else {
            inner_rows
        }
    }

    fn normalize_sidebar_scroll(&mut self, total_entries: usize) {
        let visible_capacity = self.sidebar_visible_capacity(total_entries);
        if visible_capacity == 0 || total_entries <= visible_capacity {
            self.pkg_scroll_offset = 0;
            return;
        }

        let max_offset = total_entries.saturating_sub(visible_capacity);
        self.pkg_scroll_offset = self.pkg_scroll_offset.min(max_offset);
    }

    fn ensure_sidebar_index_visible(&mut self, idx: usize, total_entries: usize) {
        let visible_capacity = self.sidebar_visible_capacity(total_entries);
        if visible_capacity == 0 || total_entries <= visible_capacity {
            self.pkg_scroll_offset = 0;
            return;
        }

        if idx < self.pkg_scroll_offset {
            self.pkg_scroll_offset = idx;
        } else if idx >= self.pkg_scroll_offset + visible_capacity {
            self.pkg_scroll_offset = idx.saturating_sub(visible_capacity.saturating_sub(1));
        }
        self.normalize_sidebar_scroll(total_entries);
    }

    fn open_member_inspect(&mut self, node_id: usize) {
        let Some(label) = self.graph_layout.labels.get(node_id).cloned() else {
            return;
        };

        self.hovered_node = Some(node_id);
        self.active_view = ActiveView::Inspect(label.clone());
        self.push_view(ViewContext::ModuleInspect(label));
        self.focus_current_graph_view();
    }

    fn sync_active_inspect_with_sidebar(&mut self) {
        if self.blast_overlay_active || !matches!(self.active_view, ActiveView::Inspect(_)) {
            return;
        }

        if let Some(node_id) = self.selected_sidebar_member() {
            self.open_member_inspect(node_id);
        }
    }

    fn sync_sidebar_selection(&mut self) {
        let entries = self.sidebar_entries();
        if entries.is_empty() {
            self.selected_pkg_index = None;
            self.hovered_node = None;
            self.hovered_cluster = None;
            self.pkg_scroll_offset = 0;
            return;
        }

        let preferred_idx = if let Some(cluster_id) = self.hovered_cluster {
            entries.iter().position(|entry| match entry {
                SidebarEntry::Cluster { id, .. } => *id == cluster_id,
                SidebarEntry::Member { .. } => false,
            })
        } else if let Some(node_id) = self.hovered_node {
            entries.iter().position(|entry| match entry {
                SidebarEntry::Member { id, .. } => *id == node_id,
                SidebarEntry::Cluster { .. } => false,
            })
        } else {
            None
        };

        let idx = self
            .selected_pkg_index
            .or(preferred_idx)
            .unwrap_or(0)
            .min(entries.len() - 1);
        self.selected_pkg_index = Some(idx);
        self.ensure_sidebar_index_visible(idx, entries.len());

        match &entries[idx] {
            SidebarEntry::Cluster { id, .. } => {
                self.hovered_cluster = Some(*id);
                self.hovered_node = None;
            }
            SidebarEntry::Member { id, .. } => {
                self.hovered_node = Some(*id);
                self.hovered_cluster = self.selected_cluster;
            }
        }

        self.sync_active_inspect_with_sidebar();
    }

    fn sidebar_entries(&self) -> Vec<SidebarEntry> {
        let filter = self.filter_text.to_lowercase();
        let overview_mode = self.should_show_architecture_overview();
        let mut entries = if overview_mode {
            self.architecture_map
                .as_ref()
                .map(|map| {
                    let mut clusters = map.clusters.iter().collect::<Vec<_>>();
                    clusters.sort_by(|a, b| {
                        cluster_overview_rank(b)
                            .cmp(&cluster_overview_rank(a))
                            .then_with(|| a.name.cmp(&b.name))
                    });
                    clusters
                        .into_iter()
                        .map(|cluster| SidebarEntry::Cluster {
                            id: cluster.id,
                            label: format!("{} ({})", cluster.name, cluster.members.len()),
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
        } else if let Some(cluster_id) = self.selected_cluster {
            self.architecture_map
                .as_ref()
                .and_then(|map| map.clusters.get(cluster_id))
                .map(|cluster| {
                    let mut members = cluster
                        .members
                        .iter()
                        .filter_map(|&member| {
                            self.graph_layout
                                .labels
                                .get(member)
                                .map(|label| SidebarEntry::Member {
                                    id: member,
                                    label: label.clone(),
                                })
                        })
                        .collect::<Vec<_>>();

                    members.sort_by(|a, b| match (a, b) {
                        (
                            SidebarEntry::Member {
                                id: a_id,
                                label: a_label,
                            },
                            SidebarEntry::Member {
                                id: b_id,
                                label: b_label,
                            },
                        ) => {
                            let a_internal = self.internal_node_indices.contains(a_id);
                            let b_internal = self.internal_node_indices.contains(b_id);
                            b_internal
                                .cmp(&a_internal)
                                .then_with(|| {
                                    self.node_total_weight(*b_id)
                                        .cmp(&self.node_total_weight(*a_id))
                                })
                                .then_with(|| a_label.cmp(b_label))
                        }
                        _ => std::cmp::Ordering::Equal,
                    });
                    members
                })
                .unwrap_or_default()
        } else {
            self.graph_layout
                .labels
                .iter()
                .enumerate()
                .map(|(idx, label)| SidebarEntry::Member {
                    id: idx,
                    label: label.clone(),
                })
                .collect::<Vec<_>>()
        };

        if self.selected_cluster.is_none() && !overview_mode {
            entries.sort_by(|a, b| {
                sidebar_label(a)
                    .to_lowercase()
                    .cmp(&sidebar_label(b).to_lowercase())
            });
        }
        if filter.is_empty() {
            entries
        } else if overview_mode {
            entries
                .into_iter()
                .filter(|entry| match entry {
                    SidebarEntry::Cluster { id, label } => {
                        label.to_lowercase().contains(&filter)
                            || self
                                .architecture_map
                                .as_ref()
                                .and_then(|map| map.clusters.get(*id))
                                .is_some_and(|cluster| {
                                    cluster.members.iter().any(|member| {
                                        self.graph_layout.labels[*member]
                                            .to_lowercase()
                                            .contains(&filter)
                                    })
                                })
                    }
                    SidebarEntry::Member { label, .. } => label.to_lowercase().contains(&filter),
                })
                .collect()
        } else {
            entries
                .into_iter()
                .filter(|entry| sidebar_label(entry).to_lowercase().contains(&filter))
                .collect()
        }
    }

    fn sidebar_title(&self, shown: usize) -> String {
        if self.should_show_architecture_overview() {
            let total = self
                .architecture_map
                .as_ref()
                .map(|map| map.clusters.len())
                .unwrap_or(0);
            if shown < total {
                format!(" Clusters ({}/{}) ", shown, total)
            } else {
                format!(" Clusters ({}) ", total)
            }
        } else if let Some(cluster_id) = self.selected_cluster {
            let (name, total) = self
                .architecture_map
                .as_ref()
                .and_then(|map| map.clusters.get(cluster_id))
                .map(|cluster| (cluster.name.clone(), cluster.members.len()))
                .unwrap_or_else(|| ("Cluster".to_string(), 0));
            if shown < total {
                format!(" Members: {} ({}/{}) ", name, shown, total)
            } else {
                format!(" Members: {} ({}) ", name, total)
            }
        } else {
            let total = self.graph_layout.labels.len();
            if shown < total {
                format!(" Packages ({}/{}) ", shown, total)
            } else {
                format!(" Packages ({}) ", total)
            }
        }
    }

    fn selected_sidebar_member(&self) -> Option<usize> {
        if self.selected_cluster.is_none() {
            return self.selected_pkg_index.and_then(|idx| {
                self.sidebar_entries()
                    .get(idx)
                    .and_then(|entry| match entry {
                        SidebarEntry::Member { id, .. } => Some(*id),
                        SidebarEntry::Cluster { .. } => None,
                    })
            });
        }

        self.selected_pkg_index
            .and_then(|idx| {
                self.sidebar_entries()
                    .get(idx)
                    .and_then(|entry| match entry {
                        SidebarEntry::Member { id, .. } => Some(*id),
                        SidebarEntry::Cluster { .. } => None,
                    })
            })
            .or_else(|| {
                self.selected_cluster.and_then(|cluster_id| {
                    self.architecture_map
                        .as_ref()
                        .and_then(|map| map.clusters.get(cluster_id))
                        .and_then(|cluster| cluster.members.first().copied())
                })
            })
    }

    fn enter_cluster_detail(&mut self, cluster_id: usize) {
        let Some(map) = &self.architecture_map else {
            return;
        };
        let Some(cluster) = map.clusters.get(cluster_id) else {
            return;
        };

        self.selected_cluster = Some(cluster_id);
        self.hovered_cluster = Some(cluster_id);
        self.active_view = ActiveView::Dashboard;
        self.selected_pkg_index = None;
        self.pkg_scroll_offset = 0;
        self.hovered_node = cluster.members.iter().copied().max_by(|a, b| {
            let a_anchor = app_label_match(&self.graph_layout.labels[*a], &cluster.anchor_label);
            let b_anchor = app_label_match(&self.graph_layout.labels[*b], &cluster.anchor_label);
            a_anchor
                .cmp(&b_anchor)
                .then_with(|| self.node_total_weight(*a).cmp(&self.node_total_weight(*b)))
        });
        self.push_view(ViewContext::PackageDetail(cluster.name.clone()));
        self.sync_sidebar_selection();
        self.focus_current_graph_view();
    }

    /// Set repo name for breadcrumb display
    pub fn set_repo_name(&mut self, name: String) {
        self.repo_name = name;
    }

    pub fn set_scoring_config(&mut self, config: ScoringConfig) {
        self.scoring_config = config;
        self.compute_insights();
        self.sync_sidebar_selection();
    }

    pub fn set_clustering_config(&mut self, config: ClusteringConfig) {
        self.clustering_config = config;
        self.compute_insights();
        self.sync_sidebar_selection();
    }

    pub fn set_skipped_snapshot_count(&mut self, count: usize) {
        self.skipped_snapshot_count = count;
    }

    fn current_snapshot(&self) -> Option<&GraphSnapshot> {
        self.snapshots_metadata
            .get(self.timeline.current_index)
            .and_then(|meta| self.snapshot_cache.peek(&meta.commit_hash))
    }

    fn previous_snapshot_graph(&self) -> Option<DiGraph<String, u32>> {
        let prev_hash = self
            .snapshots_metadata
            .get(self.timeline.current_index + 1)
            .map(|meta| meta.commit_hash.clone())?;

        let snapshot = self.snapshot_cache.peek(&prev_hash).cloned().or_else(|| {
            self.db.as_ref().and_then(|db| {
                db.get_graph_snapshot(&self.repo_id, &prev_hash)
                    .ok()
                    .flatten()
            })
        })?;

        let nodes: HashSet<String> = snapshot.nodes.into_iter().collect();
        Some(graph_builder::build_graph(&nodes, &snapshot.edges))
    }

    fn current_metric_graph(&self) -> Option<DiGraph<String, u32>> {
        let snapshot = self.current_snapshot()?;
        let nodes: HashSet<String> = snapshot.nodes.iter().cloned().collect();
        Some(graph_builder::build_graph(&nodes, &snapshot.edges))
    }

    fn resolve_snapshot_analysis(&self, snapshot: &GraphSnapshot) -> ResolvedSnapshotAnalysis {
        if snapshot.requires_core_recompute() {
            let nodes: HashSet<String> = snapshot.nodes.iter().cloned().collect();
            let prev_graph = self.previous_snapshot_graph();
            let artifacts = analysis::build_snapshot_artifacts(
                &nodes,
                &snapshot.edges,
                prev_graph.as_ref(),
                snapshot.timestamp,
                &self.scoring_config,
                analysis::SnapshotAnalysisDetail::Core,
            );
            let drift = artifacts.drift.clone();

            return ResolvedSnapshotAnalysis {
                drift: Some(drift.clone()),
                blast_radius: snapshot.blast_radius.clone(),
                instability_metrics: scoring::compute_instability_metrics(&artifacts.graph)
                    .into_iter()
                    .map(|metric| (metric.0, metric.1, metric.2, metric.3))
                    .collect(),
                diagnostics: scoring::generate_diagnostics(
                    &artifacts.graph,
                    &drift,
                    &self.scoring_config,
                ),
                legacy_recomputed: true,
            };
        }

        if snapshot.needs_runtime_insights() {
            let nodes: HashSet<String> = snapshot.nodes.iter().cloned().collect();
            let graph = graph_builder::build_graph(&nodes, &snapshot.edges);
            let drift = snapshot.drift.clone();
            let instability_metrics = scoring::compute_instability_metrics(&graph)
                .into_iter()
                .map(|metric| (metric.0, metric.1, metric.2, metric.3))
                .collect();
            let diagnostics = drift
                .as_ref()
                .map(|drift| scoring::generate_diagnostics(&graph, drift, &self.scoring_config))
                .unwrap_or_default();

            return ResolvedSnapshotAnalysis {
                drift,
                blast_radius: snapshot.blast_radius.clone(),
                instability_metrics,
                diagnostics,
                legacy_recomputed: true,
            };
        }

        ResolvedSnapshotAnalysis {
            drift: snapshot.drift.clone(),
            blast_radius: snapshot.blast_radius.clone(),
            instability_metrics: snapshot
                .instability_metrics
                .iter()
                .map(|metric| {
                    (
                        metric.module_name.clone(),
                        metric.instability,
                        metric.fan_in,
                        metric.fan_out,
                    )
                })
                .collect(),
            diagnostics: snapshot.diagnostics.clone(),
            legacy_recomputed: false,
        }
    }

    fn ensure_current_blast_radius(&mut self) -> bool {
        if self.current_blast_radius.is_some() {
            return true;
        }

        let Some(snapshot) = self.current_snapshot().cloned() else {
            return false;
        };
        let nodes: HashSet<String> = snapshot.nodes.iter().cloned().collect();
        let graph = graph_builder::build_graph(&nodes, &snapshot.edges);
        let blast_report = blast_radius::compute_blast_radius_report(
            &graph,
            self.scoring_config.thresholds.blast_max_critical_paths,
        );

        self.node_blast_scores = self
            .graph_layout
            .labels
            .iter()
            .map(|label| {
                blast_report
                    .impacts
                    .iter()
                    .find(|m| m.module_name == *label)
                    .map(|m| m.blast_score)
                    .unwrap_or(0.0)
            })
            .collect();
        self.current_blast_radius = Some(blast_report.clone());
        if let Some(cached) = self.snapshot_cache.get_mut(&snapshot.commit_hash) {
            cached.blast_radius = Some(blast_report);
        }
        true
    }

    fn current_scan_metadata(&self) -> Option<&crate::models::ScanMetadata> {
        self.current_snapshot().and_then(|snapshot| {
            if snapshot.scan_metadata.external_min_importers > 0 {
                Some(&snapshot.scan_metadata)
            } else {
                None
            }
        })
    }

    /// Get sorted and filtered package list for sidebar
    pub fn get_sorted_packages(&self) -> Vec<String> {
        self.sidebar_entries()
            .into_iter()
            .map(|entry| sidebar_label(&entry).to_string())
            .collect()
    }

    fn context_advisory_lines(&self) -> Vec<String> {
        let mut lines = Vec::new();

        if let Some(node_id) = self.selected_sidebar_member()
            && matches!(self.active_view, ActiveView::Inspect(_))
        {
            let label = self
                .graph_layout
                .labels
                .get(node_id)
                .cloned()
                .unwrap_or_default();
            let mut inbound = 0u32;
            let mut outbound = 0u32;
            for (edge_idx, &(from, to)) in self.graph_layout.edges.iter().enumerate() {
                let weight = self
                    .graph_layout
                    .edge_weights
                    .get(edge_idx)
                    .copied()
                    .unwrap_or(1);
                if to == node_id {
                    inbound += weight;
                }
                if from == node_id {
                    outbound += weight;
                }
            }
            lines.push(format!(
                "Inspecting `{}` with {} incoming and {} outgoing dependency weight.",
                label, inbound, outbound
            ));
            if let Some(cluster_id) = self.selected_cluster
                && let Some(map) = &self.architecture_map
                && let Some(cluster) = map.clusters.get(cluster_id)
            {
                lines.push(format!("Scoped inside cluster `{}`.", cluster.name));
            }
        } else if let Some(cluster_id) = self.selected_cluster
            && let Some(map) = &self.architecture_map
            && let Some(cluster) = map.clusters.get(cluster_id)
        {
            lines.push(format!(
                "Cluster `{}` is a {} with {} members.",
                cluster.name,
                cluster_summary_type_label(cluster),
                cluster.members.len()
            ));
            lines.push(format!(
                "Incoming links: {}. Outgoing links: {}. Internal links: {}.",
                cluster.inbound_weight, cluster.outbound_weight, cluster.internal_weight
            ));
            if cluster.inbound_weight == 0 && cluster.outbound_weight == 0 {
                lines.push("This cluster is isolated from the rest of the map.".to_string());
            }
            let note = cluster_summary_note(cluster);
            if !note.is_empty() {
                lines.push(note.to_string());
            }
        }

        lines
    }

    pub fn handle_key(&mut self, code: KeyCode, modifiers: KeyModifiers) {
        // ── Filter bar input mode ──
        if self.filter_active {
            match code {
                KeyCode::Esc => {
                    self.filter_active = false;
                    self.filter_text.clear();
                    self.sync_sidebar_selection();
                    self.focus_current_graph_view();
                }
                KeyCode::Enter => {
                    self.filter_active = false;
                    // Keep filter_text active (shown as badge)
                    self.sync_sidebar_selection();
                    self.focus_current_graph_view();
                }
                KeyCode::Backspace => {
                    self.filter_text.pop();
                }
                KeyCode::Char(c) => {
                    self.filter_text.push(c);
                }
                _ => {}
            }
            return;
        }

        // ── Help overlay ──
        if self.show_help {
            match code {
                KeyCode::Char('?') | KeyCode::Esc => {
                    self.show_help = false;
                }
                _ => {}
            }
            return;
        }

        // ── Global keys (always work) ──
        match code {
            KeyCode::Char('q') => {
                self.should_quit = true;
                return;
            }
            KeyCode::Char('c') if modifiers.contains(KeyModifiers::CONTROL) => {
                self.should_quit = true;
                return;
            }
            KeyCode::Char('?') => {
                self.show_help = true;
                return;
            }
            KeyCode::Char('/') => {
                self.filter_active = true;
                self.filter_text.clear();
                return;
            }
            KeyCode::Tab => {
                if modifiers.contains(KeyModifiers::SHIFT) {
                    self.focus_prev();
                } else {
                    self.focus_next();
                }
                return;
            }
            KeyCode::BackTab => {
                self.focus_prev();
                return;
            }
            KeyCode::Char('1') => {
                if self.effective_sidebar_visible {
                    self.focused_panel = FocusedPanel::Packages;
                }
                return;
            }
            KeyCode::Char('2') => {
                self.focused_panel = FocusedPanel::Graph;
                return;
            }
            KeyCode::Char('3') => {
                if self.effective_insights_visible {
                    self.focused_panel = FocusedPanel::Insights;
                }
                return;
            }
            KeyCode::Char('4') => {
                self.focused_panel = FocusedPanel::Timeline;
                return;
            }
            KeyCode::Char('b') => {
                self.sidebar_visible = !self.sidebar_visible;
                if !self.sidebar_visible && self.focused_panel == FocusedPanel::Packages {
                    self.focused_panel = FocusedPanel::Graph;
                }
                return;
            }
            KeyCode::Char('i') => {
                self.insights_visible = !self.insights_visible;
                if !self.insights_visible && self.focused_panel == FocusedPanel::Insights {
                    self.focused_panel = FocusedPanel::Graph;
                }
                return;
            }
            KeyCode::Char('r') => {
                self.reheat_layout();
                return;
            }
            KeyCode::Char('x') => {
                self.blast_overlay_active = !self.blast_overlay_active;
                if self.blast_overlay_active {
                    self.ensure_current_blast_radius();
                } else {
                    self.cascade_highlight = None;
                }
                return;
            }
            KeyCode::Char('p') | KeyCode::Char(' ') => {
                self.is_playing = !self.is_playing;
                self.last_auto_advance = Instant::now();
                return;
            }
            KeyCode::Esc => {
                // Cascading escape: cascade → filter → nav stack → deselect → quit
                if self.cascade_highlight.is_some() {
                    self.cascade_highlight = None;
                } else if !self.filter_text.is_empty() {
                    self.filter_text.clear();
                    self.sync_sidebar_selection();
                    self.focus_current_graph_view();
                } else if self.nav_stack.len() > 1 {
                    self.pop_view();
                } else if self.hotspots_state.selected().is_some() {
                    self.hotspots_state.select(None);
                    self.hovered_node = None;
                } else {
                    self.should_quit = true;
                }
                return;
            }
            // Global timeline navigation: Left/Right always control timeline
            KeyCode::Left if self.focused_panel != FocusedPanel::Insights => {
                self.prev_commit();
                return;
            }
            KeyCode::Right if self.focused_panel != FocusedPanel::Insights => {
                self.next_commit();
                return;
            }
            _ => {}
        }

        // ── Focus-specific keys ──
        match self.focused_panel {
            FocusedPanel::Packages => {
                self.handle_packages_key(code);
            }
            FocusedPanel::Graph => {
                self.handle_graph_key(code);
            }
            FocusedPanel::Insights => {
                self.handle_insights_key(code);
            }
            FocusedPanel::Timeline => {
                self.handle_timeline_key(code);
            }
        }
    }

    fn handle_packages_key(&mut self, code: KeyCode) {
        let entries = self.sidebar_entries();
        match code {
            KeyCode::Char('j') | KeyCode::Down => {
                if entries.is_empty() {
                    return;
                }
                let next = self
                    .selected_pkg_index
                    .map(|i| (i + 1).min(entries.len() - 1))
                    .unwrap_or(0);
                self.selected_pkg_index = Some(next);
                self.ensure_sidebar_index_visible(next, entries.len());
                self.sync_sidebar_selection();
            }
            KeyCode::Char('k') | KeyCode::Up => {
                if entries.is_empty() {
                    return;
                }
                let next = self
                    .selected_pkg_index
                    .map(|i| i.saturating_sub(1))
                    .unwrap_or(0);
                self.selected_pkg_index = Some(next);
                self.ensure_sidebar_index_visible(next, entries.len());
                self.sync_sidebar_selection();
            }
            KeyCode::Char('g') => {
                self.selected_pkg_index = Some(0);
                self.pkg_scroll_offset = 0;
                self.sync_sidebar_selection();
            }
            KeyCode::Char('G') => {
                if !entries.is_empty() {
                    self.selected_pkg_index = Some(entries.len() - 1);
                    self.ensure_sidebar_index_visible(entries.len() - 1, entries.len());
                    self.sync_sidebar_selection();
                }
            }
            KeyCode::Enter => {
                if let Some(idx) = self.selected_pkg_index
                    && let Some(entry) = entries.get(idx)
                {
                    match entry {
                        SidebarEntry::Cluster { id, .. } => {
                            self.enter_cluster_detail(*id);
                        }
                        SidebarEntry::Member { id, .. } => {
                            if self.blast_overlay_active {
                                self.ensure_current_blast_radius();
                                self.hovered_node = Some(*id);
                                self.compute_cascade_for_node(*id);
                            } else {
                                self.open_member_inspect(*id);
                            }
                        }
                    }
                }
            }
            KeyCode::Char('s') => {
                self.hotspots_sort = match self.hotspots_sort {
                    HotspotsSort::Instability => HotspotsSort::FanIn,
                    HotspotsSort::FanIn => HotspotsSort::FanOut,
                    HotspotsSort::FanOut => HotspotsSort::Instability,
                };
                self.apply_hotspots_sort();
            }
            _ => {}
        }
    }

    fn handle_graph_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Char('j') | KeyCode::Down
                if self.should_show_architecture_overview()
                    || (self.selected_cluster.is_some()
                        && !matches!(self.active_view, ActiveView::Inspect(_))) =>
            {
                let entries = self.sidebar_entries();
                if entries.is_empty() {
                    return;
                }
                let idx = self
                    .selected_pkg_index
                    .map(|i| (i + 1).min(entries.len() - 1))
                    .unwrap_or(0);
                self.set_selected_sidebar_index(idx);
            }
            KeyCode::Char('k') | KeyCode::Up
                if self.should_show_architecture_overview()
                    || (self.selected_cluster.is_some()
                        && !matches!(self.active_view, ActiveView::Inspect(_))) =>
            {
                let entries = self.sidebar_entries();
                if entries.is_empty() {
                    return;
                }
                let idx = self
                    .selected_pkg_index
                    .map(|i| i.saturating_sub(1))
                    .unwrap_or(0);
                if idx < entries.len() {
                    self.set_selected_sidebar_index(idx);
                }
            }
            KeyCode::Enter => {
                if self.should_show_architecture_overview() {
                    if let Some(cluster_id) = self.hovered_cluster.or_else(|| {
                        self.selected_pkg_index.and_then(|idx| {
                            self.sidebar_entries()
                                .get(idx)
                                .and_then(|entry| match entry {
                                    SidebarEntry::Cluster { id, .. } => Some(*id),
                                    SidebarEntry::Member { .. } => None,
                                })
                        })
                    }) {
                        self.enter_cluster_detail(cluster_id);
                    }
                } else if let Some(idx) = self.hovered_node {
                    if self.blast_overlay_active {
                        // In blast mode: compute cascade highlight for this node
                        self.ensure_current_blast_radius();
                        self.compute_cascade_for_node(idx);
                    } else {
                        // Normal: inspect hovered node
                        if let Some(label) = self.graph_layout.labels.get(idx) {
                            let name = label.clone();
                            self.active_view = ActiveView::Inspect(name.clone());
                            self.push_view(ViewContext::ModuleInspect(name));
                            self.focus_current_graph_view();
                        }
                    }
                }
            }
            KeyCode::Char('c') => {
                self.focus_current_graph_view();
            }
            _ => {}
        }
    }

    /// Computes the single-node blast radius cascade for TUI overlay.
    fn compute_cascade_for_node(&mut self, layout_idx: usize) {
        let Some(g) = self.current_metric_graph() else {
            return;
        };
        let node_map: HashMap<String, petgraph::graph::NodeIndex> =
            g.node_indices().map(|idx| (g[idx].clone(), idx)).collect();
        let label = &self.graph_layout.labels[layout_idx];
        if let Some(&ni) = node_map.get(label) {
            let blast_nodes = blast_radius::compute_single_node_blast(&g, ni);
            // Map NodeIndex back to layout indices
            let mapped: Vec<(usize, u32, f64)> = blast_nodes
                .iter()
                .filter_map(|(ni, dist, impact)| {
                    let name = &g[*ni];
                    self.graph_layout
                        .labels
                        .iter()
                        .position(|l| l == name)
                        .map(|layout_i| (layout_i, *dist, *impact))
                })
                .collect();
            self.cascade_highlight = Some(mapped);
        }
    }

    fn handle_insights_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Char('j') | KeyCode::Down => match self.insight_tab {
                InsightTab::Hotspots => self.select_next_hotspot(),
                InsightTab::Blast => {
                    self.ensure_current_blast_radius();
                    let max = self
                        .current_blast_radius
                        .as_ref()
                        .map(|br| br.impacts.len().saturating_sub(1))
                        .unwrap_or(0);
                    self.blast_impact_scroll = (self.blast_impact_scroll + 1).min(max);
                }
                _ => {}
            },
            KeyCode::Char('k') | KeyCode::Up => match self.insight_tab {
                InsightTab::Hotspots => self.select_prev_hotspot(),
                InsightTab::Blast => {
                    self.blast_impact_scroll = self.blast_impact_scroll.saturating_sub(1);
                }
                _ => {}
            },
            KeyCode::Left | KeyCode::Char('[') | KeyCode::Char('h') => {
                self.insight_tab = match self.insight_tab {
                    InsightTab::Overview => InsightTab::Blast,
                    InsightTab::Hotspots => InsightTab::Overview,
                    InsightTab::Blast => InsightTab::Hotspots,
                };
            }
            KeyCode::Right | KeyCode::Char(']') | KeyCode::Char('l') => {
                self.insight_tab = match self.insight_tab {
                    InsightTab::Overview => InsightTab::Hotspots,
                    InsightTab::Hotspots => InsightTab::Blast,
                    InsightTab::Blast => InsightTab::Overview,
                };
            }
            KeyCode::Enter => match self.insight_tab {
                InsightTab::Hotspots => {
                    if let Some(i) = self.hotspots_state.selected()
                        && let Some(pkg) = self.brittle_packages.get(i)
                    {
                        let name = pkg.0.clone();
                        self.active_view = ActiveView::Inspect(name.clone());
                        self.push_view(ViewContext::ModuleInspect(name));
                        self.focus_current_graph_view();
                    }
                }
                InsightTab::Blast => {
                    // Enter on Blast tab: compute cascade for the impact at scroll position
                    self.ensure_current_blast_radius();
                    if let Some(br) = &self.current_blast_radius
                        && let Some(impact) = br.impacts.get(self.blast_impact_scroll)
                    {
                        let module = impact.module_name.clone();
                        if let Some(idx) =
                            self.graph_layout.labels.iter().position(|l| *l == module)
                        {
                            self.blast_overlay_active = true;
                            self.hovered_node = Some(idx);
                            self.compute_cascade_for_node(idx);
                        }
                    }
                }
                _ => {}
            },
            KeyCode::Char('s') => {
                self.hotspots_sort = match self.hotspots_sort {
                    HotspotsSort::Instability => HotspotsSort::FanIn,
                    HotspotsSort::FanIn => HotspotsSort::FanOut,
                    HotspotsSort::FanOut => HotspotsSort::Instability,
                };
                self.apply_hotspots_sort();
            }
            _ => {}
        }
    }

    fn handle_timeline_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Char('j') | KeyCode::Right => {
                self.next_commit();
            }
            KeyCode::Char('k') | KeyCode::Left => {
                self.prev_commit();
            }
            KeyCode::Char('l') | KeyCode::PageDown => {
                self.jump_commit(10);
            }
            KeyCode::Char('h') | KeyCode::PageUp => {
                self.jump_commit(-10);
            }
            KeyCode::Home | KeyCode::Char('g') => {
                self.jump_to_first();
            }
            KeyCode::End | KeyCode::Char('G') => {
                self.jump_to_last();
            }
            KeyCode::Char('+') | KeyCode::Char('=') => {
                let ms = self.auto_play_interval.as_millis() as u64;
                let new_ms = if ms > 500 {
                    ms - 250
                } else {
                    (ms.saturating_sub(100)).max(200)
                };
                self.auto_play_interval = Duration::from_millis(new_ms);
            }
            KeyCode::Char('-') | KeyCode::Char('_') => {
                let ms = self.auto_play_interval.as_millis() as u64;
                let new_ms = if ms < 500 {
                    ms + 100
                } else {
                    (ms + 250).min(5000)
                };
                self.auto_play_interval = Duration::from_millis(new_ms);
            }
            _ => {}
        }
    }

    pub fn handle_mouse(&mut self, mouse: MouseEvent) {
        let col = mouse.column;
        let row = mouse.row;

        let pkg_right_border = self.pkg_area.x + self.pkg_area.width;
        let on_pkg_border = (col as i16 - pkg_right_border as i16).unsigned_abs() <= 1
            && row >= self.pkg_area.y
            && row < self.pkg_area.y + self.pkg_area.height;

        if self.resizing_pkg {
            match mouse.kind {
                MouseEventKind::Drag(MouseButton::Left)
                | MouseEventKind::Down(MouseButton::Left) => {
                    self.pkg_panel_width = col.saturating_sub(self.pkg_area.x).clamp(14, 60);
                    return;
                }
                MouseEventKind::Up(MouseButton::Left) => {
                    self.resizing_pkg = false;
                    return;
                }
                _ => {}
            }
        }

        if matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left)) && on_pkg_border {
            self.resizing_pkg = true;
            return;
        }

        let on_insights_border = (col as i16 - self.insights_area.x as i16).unsigned_abs() <= 1
            && row >= self.insights_area.y
            && row < self.insights_area.y + self.insights_area.height;

        if self.resizing_insights {
            match mouse.kind {
                MouseEventKind::Drag(MouseButton::Left)
                | MouseEventKind::Down(MouseButton::Left) => {
                    let right_edge = self.insights_area.x + self.insights_area.width;
                    self.insights_panel_width = right_edge.saturating_sub(col).clamp(20, 80);
                    return;
                }
                MouseEventKind::Up(MouseButton::Left) => {
                    self.resizing_insights = false;
                    return;
                }
                _ => {}
            }
        }

        if matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left)) && on_insights_border {
            self.resizing_insights = true;
            return;
        }

        // Click on a package in the sidebar
        let pkg_inner = Rect {
            x: self.pkg_area.x + 1,
            y: self.pkg_area.y + 1,
            width: self.pkg_area.width.saturating_sub(2),
            height: self.pkg_area.height.saturating_sub(2),
        };
        let in_pkg =
            pkg_inner.contains(ratatui::layout::Position::new(col, row)) && pkg_inner.width > 0;
        if matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left)) && in_pkg {
            self.focused_panel = FocusedPanel::Packages;
            let entries = self.sidebar_entries();
            let row_offset = row.saturating_sub(pkg_inner.y) as usize;
            let visible_capacity = self.sidebar_visible_capacity(entries.len());
            let list_height = visible_capacity.min(pkg_inner.height as usize);
            if row_offset >= list_height {
                return;
            }

            let clicked_idx = self.pkg_scroll_offset + row_offset;
            if clicked_idx < entries.len() {
                self.selected_pkg_index = Some(clicked_idx);
                let clicked_entry = entries.get(clicked_idx).cloned();
                if matches!(clicked_entry, Some(SidebarEntry::Member { .. }))
                    && matches!(self.active_view, ActiveView::Inspect(_))
                    && matches!(self.current_view(), ViewContext::ModuleInspect(_))
                {
                    self.pop_view();
                }
                self.sync_sidebar_selection();
                if let Some(entry) = clicked_entry {
                    match entry {
                        SidebarEntry::Cluster { id, .. } => {
                            self.enter_cluster_detail(id);
                        }
                        SidebarEntry::Member { id, .. } => {
                            self.hovered_node = Some(id);
                            self.hovered_cluster = self.selected_cluster;
                            if self.blast_overlay_active {
                                self.compute_cascade_for_node(id);
                            }
                        }
                    }
                }
            }
            return;
        }

        let area = if self.should_show_architecture_overview() {
            overview_map_rect(self.graph_area)
        } else {
            self.graph_area
        };
        let inner_x = area.x + 1;
        let inner_y = area.y + 1;
        let inner_w = area.width.saturating_sub(2);
        let inner_h = area.height.saturating_sub(2);
        if inner_w == 0 || inner_h == 0 {
            return;
        }

        let in_canvas =
            col >= inner_x && col < inner_x + inner_w && row >= inner_y && row < inner_y + inner_h;
        let overview_canvas_w = inner_w as f64 * 2.0;
        let overview_canvas_h = inner_h as f64 * 4.0;
        let tl = self.timeline_area;
        let tl_inner_x = tl.x + 1;
        let tl_inner_w = tl.width.saturating_sub(3);
        let in_timeline = col >= tl.x
            && col < tl.x + tl.width
            && row >= tl.y
            && row < tl.y + tl.height
            && !self.timeline.is_empty();

        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) if in_timeline => {
                self.dragging_timeline = true;
                self.is_playing = false;
                let ratio = ((col.saturating_sub(tl_inner_x) as f64) / tl_inner_w.max(1) as f64)
                    .clamp(0.0, 1.0);
                self.seek_to((ratio * (self.timeline.len() - 1) as f64).round() as usize);
            }
            MouseEventKind::Drag(MouseButton::Left) if self.dragging_timeline => {
                let ratio = ((col.saturating_sub(tl_inner_x) as f64) / tl_inner_w.max(1) as f64)
                    .clamp(0.0, 1.0);
                self.seek_to((ratio * (self.timeline.len() - 1) as f64).round() as usize);
            }
            MouseEventKind::Up(MouseButton::Left) if self.dragging_timeline => {
                self.dragging_timeline = false;
            }
            MouseEventKind::Down(MouseButton::Left) if in_canvas => {
                self.focused_panel = FocusedPanel::Graph;
                if self.should_show_architecture_overview() {
                    let (px, py) = self.terminal_to_canvas_space(
                        col,
                        row,
                        inner_x,
                        inner_y,
                        inner_w,
                        inner_h,
                        overview_canvas_w,
                        overview_canvas_h,
                    );
                    self.hovered_cluster = overview_hit_test_terminal(
                        self,
                        col,
                        row,
                        area,
                        overview_canvas_w,
                        overview_canvas_h,
                    )
                    .or_else(|| {
                        overview_hit_test(self, px, py, overview_canvas_w, overview_canvas_h)
                    });
                    if let Some(cluster_id) = self.hovered_cluster {
                        self.enter_cluster_detail(cluster_id);
                    }
                    return;
                }
                if self.selected_cluster.is_some()
                    && !matches!(self.active_view, ActiveView::Inspect(_))
                {
                    return;
                }
                if let Some(old_idx) = self.dragging_node.take()
                    && old_idx < self.graph_layout.positions.len()
                {
                    self.graph_layout.positions[old_idx].pinned = false;
                }
                let (px, py) =
                    self.terminal_to_physics(col, row, inner_x, inner_y, inner_w, inner_h);
                let diag =
                    (self.graph_layout.width.powi(2) + self.graph_layout.height.powi(2)).sqrt();
                let grab_radius = (diag * 0.06).max(30.0);
                let vis = self.visible_node_set();
                let mut closest: Option<(usize, f64)> = None;
                for (i, pos) in self.graph_layout.positions.iter().enumerate() {
                    // Skip nodes hidden by inspect/filter
                    if let Some(ref v) = vis
                        && !v.contains(&i)
                    {
                        continue;
                    }
                    let dx = pos.x - px;
                    let dy = pos.y - py;
                    let dist = (dx * dx + dy * dy).sqrt();
                    if dist < grab_radius && (closest.is_none() || dist < closest.unwrap().1) {
                        closest = Some((i, dist));
                    }
                }
                if let Some((idx, _)) = closest {
                    self.dragging_node = Some(idx);
                    self.graph_layout.positions[idx].pinned = true;
                } else {
                    self.dragging_pan = true;
                    self.last_mouse_pos = Some((col, row));
                }
            }
            MouseEventKind::Drag(MouseButton::Left) => {
                if self.selected_cluster.is_some()
                    && !matches!(self.active_view, ActiveView::Inspect(_))
                {
                    return;
                }
                if let Some(idx) = self.dragging_node
                    && idx < self.graph_layout.positions.len()
                {
                    let (px, py) =
                        self.terminal_to_physics(col, row, inner_x, inner_y, inner_w, inner_h);
                    self.graph_layout.positions[idx].x = px;
                    self.graph_layout.positions[idx].y = py;
                    self.graph_layout.positions[idx].prev_x = px;
                    self.graph_layout.positions[idx].prev_y = py;
                } else if self.dragging_pan {
                    if let Some((last_col, last_row)) = self.last_mouse_pos {
                        let (px_now, py_now) =
                            self.terminal_to_physics(col, row, inner_x, inner_y, inner_w, inner_h);
                        let (px_last, py_last) = self.terminal_to_physics(
                            last_col, last_row, inner_x, inner_y, inner_w, inner_h,
                        );

                        self.graph_pan_x -= px_now - px_last;
                        self.graph_pan_y -= py_now - py_last;
                    }
                    self.last_mouse_pos = Some((col, row));
                }
            }
            MouseEventKind::Up(MouseButton::Left) => {
                if self.selected_cluster.is_some()
                    && !matches!(self.active_view, ActiveView::Inspect(_))
                {
                    self.dragging_pan = false;
                    self.last_mouse_pos = None;
                    return;
                }
                if let Some(idx) = self.dragging_node.take()
                    && idx < self.graph_layout.positions.len()
                {
                    self.graph_layout.positions[idx].pinned = false;
                    self.graph_layout.temperature = 0.01;
                }
                self.dragging_pan = false;
                self.last_mouse_pos = None;
            }
            MouseEventKind::Moved => {
                // When a cascade highlight is active, lock hovered_node to the
                // cascade source so mouse movement doesn't break the visual.
                if self.cascade_highlight.is_some() {
                    // Do nothing — keep hovered_node pinned to cascade source
                } else if self.should_show_architecture_overview() && in_canvas {
                    let (px, py) = self.terminal_to_canvas_space(
                        col,
                        row,
                        inner_x,
                        inner_y,
                        inner_w,
                        inner_h,
                        overview_canvas_w,
                        overview_canvas_h,
                    );
                    self.hovered_cluster = overview_hit_test_terminal(
                        self,
                        col,
                        row,
                        area,
                        overview_canvas_w,
                        overview_canvas_h,
                    )
                    .or_else(|| {
                        overview_hit_test(self, px, py, overview_canvas_w, overview_canvas_h)
                    });
                    self.hovered_node = None;
                } else if self.selected_cluster.is_some()
                    && !matches!(self.active_view, ActiveView::Inspect(_))
                {
                    self.hovered_node = self.selected_sidebar_member();
                } else if in_canvas {
                    let (px, py) =
                        self.terminal_to_physics(col, row, inner_x, inner_y, inner_w, inner_h);
                    let vis = self.visible_node_set();
                    let mut closest: Option<(usize, f64)> = None;
                    for (i, pos) in self.graph_layout.positions.iter().enumerate() {
                        // Skip nodes hidden by inspect/filter
                        if let Some(ref v) = vis
                            && !v.contains(&i)
                        {
                            continue;
                        }
                        let dist = ((pos.x - px).powi(2) + (pos.y - py).powi(2)).sqrt();
                        if dist < 10.0 && (closest.is_none() || dist < closest.unwrap().1) {
                            closest = Some((i, dist));
                        }
                    }
                    self.hovered_node = closest.map(|(idx, _)| idx);
                } else {
                    self.hovered_node = None;
                    self.hovered_cluster = None;
                }
            }
            MouseEventKind::Down(MouseButton::Left)
                if self
                    .insights_area
                    .contains(ratatui::layout::Position::new(col, row)) =>
            {
                self.focused_panel = FocusedPanel::Insights;
                // Tab bar is at row y+1 (inside border)
                let tab_row = self.insights_area.y + 1;
                if row == tab_row {
                    // Click on tab bar — determine which tab by x position
                    let rel_x = col.saturating_sub(self.insights_area.x + 1);
                    let mut cursor = 0u16;
                    for (idx, (label, tab)) in insight_tab_specs(self).iter().enumerate() {
                        let tab_width = format!(" {} ", label).chars().count() as u16;
                        if rel_x < cursor + tab_width {
                            self.insight_tab = *tab;
                            break;
                        }
                        cursor += tab_width;
                        if idx < 3 {
                            cursor += 1;
                        }
                    }
                } else if self.insight_tab == InsightTab::Hotspots {
                    // Click on hotspots table row
                    // Content starts at y + 4 (1: panel border, 1: tab bar, 1: table title, 1: table header)
                    let row_offset = row.saturating_sub(self.insights_area.y + 4) as usize;
                    let clicked_idx = self.hotspots_state.offset() + row_offset;
                    if clicked_idx < self.brittle_packages.len() {
                        self.hotspots_state.select(Some(clicked_idx));
                        self.focus_selected_hotspot();
                        // Emulate Enter key
                        if let Some(pkg) = self.brittle_packages.get(clicked_idx) {
                            let name = pkg.0.clone();
                            self.active_view = ActiveView::Inspect(name.clone());
                            self.push_view(ViewContext::ModuleInspect(name));
                            self.focus_current_graph_view();
                        }
                    }
                } else if self.insight_tab == InsightTab::Blast {
                    // Click on blast impact row — trigger cascade for that module
                    // Summary(3) + Keystones(5) + TopImpact title(1) = offset 9
                    // Plus tab bar row(1) + border(1) = 11 from insights_area.y
                    self.ensure_current_blast_radius();
                    let content_start = self.insights_area.y + 11;
                    if row >= content_start {
                        let has_above = self.blast_impact_scroll > 0;
                        let indicator_offset = if has_above { 1u16 } else { 0 };
                        let row_idx = row.saturating_sub(content_start + indicator_offset) as usize
                            + self.blast_impact_scroll;
                        if let Some(br) = &self.current_blast_radius
                            && row_idx < br.impacts.len()
                        {
                            let module = br.impacts[row_idx].module_name.clone();
                            // Find this module's node index in the graph
                            if let Some(idx) =
                                self.graph_layout.labels.iter().position(|l| *l == module)
                            {
                                self.blast_overlay_active = true;
                                self.ensure_current_blast_radius();
                                self.hovered_node = Some(idx);
                                self.compute_cascade_for_node(idx);
                            }
                        }
                    }
                }
            }
            MouseEventKind::ScrollUp => {
                let pos = ratatui::layout::Position::new(col, row);
                if self.pkg_area.contains(pos) {
                    let entries_len = self.sidebar_entries().len();
                    self.normalize_sidebar_scroll(entries_len);
                    self.pkg_scroll_offset = self.pkg_scroll_offset.saturating_sub(3);
                } else if self.insights_area.contains(pos) {
                    // Scroll insights: navigate content or switch tabs
                    match self.insight_tab {
                        InsightTab::Hotspots => self.select_prev_hotspot(),
                        InsightTab::Blast => {
                            self.blast_impact_scroll = self.blast_impact_scroll.saturating_sub(1);
                        }
                        _ => {
                            self.insight_tab = match self.insight_tab {
                                InsightTab::Overview => InsightTab::Blast,
                                _ => unreachable!(),
                            };
                        }
                    }
                } else if in_canvas {
                    if self.selected_cluster.is_some()
                        && !matches!(self.active_view, ActiveView::Inspect(_))
                    {
                        return;
                    }
                    // Zoom in
                    let scale_factor = 1.1;

                    // Keep the physical point under the cursor in the same screen position
                    let (px_before, py_before) = if self.should_show_architecture_overview() {
                        self.terminal_to_canvas_space(
                            col,
                            row,
                            inner_x,
                            inner_y,
                            inner_w,
                            inner_h,
                            overview_canvas_w,
                            overview_canvas_h,
                        )
                    } else {
                        self.terminal_to_physics(col, row, inner_x, inner_y, inner_w, inner_h)
                    };

                    self.graph_scale = (self.graph_scale * scale_factor).min(10.0);

                    let (px_after, py_after) = if self.should_show_architecture_overview() {
                        self.terminal_to_canvas_space(
                            col,
                            row,
                            inner_x,
                            inner_y,
                            inner_w,
                            inner_h,
                            overview_canvas_w,
                            overview_canvas_h,
                        )
                    } else {
                        self.terminal_to_physics(col, row, inner_x, inner_y, inner_w, inner_h)
                    };

                    self.graph_pan_x -= px_after - px_before;
                    self.graph_pan_y -= py_after - py_before;
                }
            }
            MouseEventKind::ScrollDown => {
                let pos = ratatui::layout::Position::new(col, row);
                if self.pkg_area.contains(pos) {
                    let entries_len = self.sidebar_entries().len();
                    let visible_capacity = self.sidebar_visible_capacity(entries_len);
                    if visible_capacity > 0 && entries_len > visible_capacity {
                        let max_offset = entries_len.saturating_sub(visible_capacity);
                        self.pkg_scroll_offset =
                            self.pkg_scroll_offset.saturating_add(3).min(max_offset);
                    } else {
                        self.pkg_scroll_offset = 0;
                    }
                } else if self.insights_area.contains(pos) {
                    // Scroll insights: navigate content or switch tabs
                    match self.insight_tab {
                        InsightTab::Hotspots => self.select_next_hotspot(),
                        InsightTab::Blast => {
                            self.ensure_current_blast_radius();
                            let max = self
                                .current_blast_radius
                                .as_ref()
                                .map(|br| br.impacts.len().saturating_sub(1))
                                .unwrap_or(0);
                            self.blast_impact_scroll = (self.blast_impact_scroll + 1).min(max);
                        }
                        _ => {
                            self.insight_tab = match self.insight_tab {
                                InsightTab::Overview => InsightTab::Hotspots,
                                _ => unreachable!(),
                            };
                        }
                    }
                } else if in_canvas {
                    if self.selected_cluster.is_some()
                        && !matches!(self.active_view, ActiveView::Inspect(_))
                    {
                        return;
                    }
                    // Zoom out
                    let scale_factor = 1.1;

                    let (px_before, py_before) = if self.should_show_architecture_overview() {
                        self.terminal_to_canvas_space(
                            col,
                            row,
                            inner_x,
                            inner_y,
                            inner_w,
                            inner_h,
                            overview_canvas_w,
                            overview_canvas_h,
                        )
                    } else {
                        self.terminal_to_physics(col, row, inner_x, inner_y, inner_w, inner_h)
                    };

                    self.graph_scale = (self.graph_scale / scale_factor).max(0.1);

                    let (px_after, py_after) = if self.should_show_architecture_overview() {
                        self.terminal_to_canvas_space(
                            col,
                            row,
                            inner_x,
                            inner_y,
                            inner_w,
                            inner_h,
                            overview_canvas_w,
                            overview_canvas_h,
                        )
                    } else {
                        self.terminal_to_physics(col, row, inner_x, inner_y, inner_w, inner_h)
                    };

                    self.graph_pan_x -= px_after - px_before;
                    self.graph_pan_y -= py_after - py_before;
                }
            }
            _ => {}
        }
    }

    fn terminal_to_physics(
        &self,
        col: u16,
        row: u16,
        ix: u16,
        iy: u16,
        iw: u16,
        ih: u16,
    ) -> (f64, f64) {
        let nx = (col.saturating_sub(ix) as f64) / iw.max(1) as f64;
        let ny = (row.saturating_sub(iy) as f64) / ih.max(1) as f64;

        let visible_w = self.graph_layout.width / self.graph_scale;
        let visible_h = self.graph_layout.height / self.graph_scale;

        (
            self.graph_pan_x + nx * visible_w,
            self.graph_pan_y + (1.0 - ny) * visible_h,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn terminal_to_canvas_space(
        &self,
        col: u16,
        row: u16,
        ix: u16,
        iy: u16,
        iw: u16,
        ih: u16,
        canvas_w: f64,
        canvas_h: f64,
    ) -> (f64, f64) {
        let nx = ((col.saturating_sub(ix) as f64) + 0.5) / iw.max(1) as f64;
        let ny = ((row.saturating_sub(iy) as f64) + 0.5) / ih.max(1) as f64;
        let visible_w = canvas_w / self.graph_scale;
        let visible_h = canvas_h / self.graph_scale;
        (
            self.graph_pan_x + nx.clamp(0.0, 1.0) * visible_w,
            self.graph_pan_y + (1.0 - ny.clamp(0.0, 1.0)) * visible_h,
        )
    }

    pub fn tick_auto_play(&mut self) {
        if self.is_playing && self.last_auto_advance.elapsed() >= self.auto_play_interval {
            self.next_commit();
            self.last_auto_advance = Instant::now();
            if self.timeline.current_index + 1 >= self.timeline.len() {
                self.is_playing = false;
            }
        }
    }

    pub fn tick_physics(&mut self) {
        if self.should_show_architecture_overview()
            || (self.selected_cluster.is_some()
                && !matches!(self.active_view, ActiveView::Inspect(_)))
        {
            self.frame_count += 1;
            return;
        }
        if self.graph_layout.temperature >= 0.02 || self.dragging_node.is_some() {
            let n = self.graph_layout.positions.len();
            self.graph_layout.multi_step(adaptive_steps(n, 3, 1));
        }
        self.frame_count += 1;
    }
}

fn snapshot_to_layout_data(
    snapshot: &GraphSnapshot,
) -> (Vec<String>, Vec<(usize, usize)>, Vec<u32>) {
    let labels = snapshot.nodes.clone();
    let l2i: HashMap<&String, usize> = labels.iter().enumerate().map(|(i, l)| (l, i)).collect();
    let mut edges = Vec::new();
    let mut weights = Vec::new();
    for e in &snapshot.edges {
        if let (Some(f), Some(t)) = (l2i.get(&e.from_module), l2i.get(&e.to_module)) {
            edges.push((*f, *t));
            weights.push(e.weight);
        }
    }
    (labels, edges, weights)
}

fn snapshot_internal_nodes(snapshot: &GraphSnapshot, labels: &[String]) -> HashSet<usize> {
    if !snapshot.node_metadata.is_empty() {
        return labels
            .iter()
            .enumerate()
            .filter_map(|(idx, label)| {
                snapshot.node_metadata.get(label).and_then(|metadata| {
                    if matches!(metadata.kind, NodeKind::Internal) {
                        Some(idx)
                    } else {
                        None
                    }
                })
            })
            .collect();
    }

    let index_by_label = labels
        .iter()
        .enumerate()
        .map(|(idx, label)| (label.as_str(), idx))
        .collect::<HashMap<_, _>>();

    let mut internal = HashSet::new();
    for edge in &snapshot.edges {
        if let Some(idx) = index_by_label.get(edge.from_module.as_str()) {
            internal.insert(*idx);
        }
    }

    for (idx, label) in labels.iter().enumerate() {
        if label.contains('/')
            || matches!(label.as_str(), "cli" | "ext" | "libs" | "runtime")
            || label.starts_with("deno_")
            || label.starts_with("cli_")
            || label.starts_with("ext_")
            || label.starts_with("runtime_")
            || label.starts_with("libs_")
        {
            internal.insert(idx);
        }
    }

    internal
}

/// Additional color constants for the redesign
const BORDER_FOCUSED: Color = Color::Rgb(180, 190, 254); // Lavender
const BORDER_UNFOCUSED: Color = Color::Rgb(69, 71, 90); // Surface2
const BG_SURFACE1: Color = Color::Rgb(49, 50, 68);
const COLOR_HEALTHY: Color = Color::Rgb(166, 227, 161);
const COLOR_WARNING: Color = Color::Rgb(249, 226, 175);
const COLOR_DANGER: Color = Color::Rgb(243, 139, 168);
const FG_SUBTEXT: Color = Color::Rgb(166, 173, 200);

pub fn render_app(frame: &mut Frame, app: &mut App) {
    let size = frame.area();
    frame.render_widget(Block::default().style(Style::default().bg(BG_BASE)), size);

    // ── Responsive: check terminal width ──
    if size.width < 40 || size.height < 12 {
        let msg = Paragraph::new(vec![
            Line::from(""),
            Line::from(Span::styled(
                " Terminal too small ",
                Style::default()
                    .fg(COLOR_DANGER)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::styled(
                format!(" Min: 40x12, Current: {}x{} ", size.width, size.height),
                Style::default().fg(FG_OVERLAY),
            )),
        ])
        .alignment(ratatui::layout::Alignment::Center);
        frame.render_widget(msg, size);
        return;
    }

    // Auto-collapse panels based on terminal width
    let effective_sidebar = app.sidebar_visible && size.width >= 60;
    let effective_insights = app.insights_visible && size.width >= 100;
    app.sync_visible_panels(effective_sidebar, effective_insights);

    // ── Main vertical layout: header + content + timeline + filter? + footer ──
    let has_filter_bar = app.filter_active;
    let mut vert_constraints = vec![
        Constraint::Length(1), // Header/breadcrumb
        Constraint::Min(8),    // Main content area
        Constraint::Length(3), // Timeline
    ];
    if has_filter_bar {
        vert_constraints.push(Constraint::Length(1)); // Filter bar
    }
    vert_constraints.push(Constraint::Length(2)); // Footer (2 rows)

    let vert = Layout::default()
        .direction(Direction::Vertical)
        .constraints(vert_constraints)
        .split(size);

    let header_area = vert[0];
    let content_area = vert[1];
    let timeline_area = vert[2];
    let filter_area = if has_filter_bar { Some(vert[3]) } else { None };
    let footer_area = vert[vert.len() - 1];

    app.timeline_area = timeline_area;

    // ── Header: Breadcrumb ──
    render_breadcrumb(frame, header_area, app);

    // ── Content: Sidebar | Graph | Insights ──
    let mut h_constraints = Vec::new();
    if effective_sidebar {
        h_constraints.push(Constraint::Length(app.pkg_panel_width));
    }
    h_constraints.push(Constraint::Min(30)); // Graph always present
    if effective_insights {
        h_constraints.push(Constraint::Length(app.insights_panel_width));
    }

    let content_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(h_constraints)
        .split(content_area);

    let mut chunk_idx = 0;

    // Sidebar
    if effective_sidebar {
        app.pkg_area = content_chunks[chunk_idx];
        render_package_list_v2(frame, content_chunks[chunk_idx], app);
        chunk_idx += 1;
    } else {
        app.pkg_area = Rect::default();
    }

    // Graph (always)
    app.graph_area = content_chunks[chunk_idx];
    render_graph_canvas(frame, content_chunks[chunk_idx], app);
    chunk_idx += 1;

    // Insights
    if effective_insights {
        app.insights_area = content_chunks[chunk_idx];
        render_insights_tabbed(frame, content_chunks[chunk_idx], app);
    } else {
        app.insights_area = Rect::default();
    }

    // ── Timeline ──
    let tl_focused = app.focused_panel == FocusedPanel::Timeline;
    render_timeline(frame, timeline_area, &app.timeline, tl_focused);

    // ── Filter bar ──
    if let Some(fa) = filter_area {
        render_filter_bar(frame, fa, app);
    }

    // ── Footer ──
    render_footer(frame, footer_area, app);

    // ── Help overlay (rendered last, on top of everything) ──
    if app.show_help {
        render_help_overlay(frame, size);
    }
}

pub fn render_graph_canvas(frame: &mut Frame, area: Rect, app: &mut App) {
    let canvas_w = (area.width.saturating_sub(2) as f64) * 2.0;
    let canvas_h = (area.height.saturating_sub(2) as f64) * 4.0;
    if canvas_w > 80.0 && canvas_h > 50.0 {
        app.graph_layout.resize(canvas_w, canvas_h);
    }

    if app.needs_warmup {
        let n = app.graph_layout.positions.len();
        app.graph_layout.reinitialize_positions();
        app.graph_layout.multi_step(adaptive_steps(n, 300, 3));
        app.graph_layout.center_layout();
        app.graph_layout.temperature = 0.01;
        app.needs_warmup = false;
    }

    if app.should_show_architecture_overview() {
        render_architecture_overview(frame, area, app, canvas_w, canvas_h);
        return;
    }

    if app.selected_cluster.is_some() && !matches!(app.active_view, ActiveView::Inspect(_)) {
        render_cluster_workspace(frame, area, app);
        return;
    }

    let search_active = !app.filter_text.is_empty();
    let inspect_active = matches!(app.active_view, ActiveView::Inspect(_));
    let is_filtered = search_active || inspect_active;

    let mut inspect_center_idx = if let ActiveView::Inspect(ref inspected) = app.active_view {
        app.graph_layout.labels.iter().position(|l| l == inspected)
    } else {
        None
    };

    let (search_matched, search_visible) = if search_active {
        let q = app.filter_text.to_lowercase();
        let mut m = HashSet::new();
        for (i, l) in app.graph_layout.labels.iter().enumerate() {
            if l.to_lowercase().contains(&q) {
                m.insert(i);
            }
        }
        // When filter matches exactly one node, treat it as center for
        // directional coloring (same visual as inspect mode)
        if m.len() == 1 {
            inspect_center_idx = m.iter().copied().next();
        }
        let mut v = m.clone();
        for &(f, t) in &app.graph_layout.edges {
            if m.contains(&f) {
                v.insert(t);
            }
            if m.contains(&t) {
                v.insert(f);
            }
        }
        (m, v)
    } else if let Some(center_idx) = inspect_center_idx {
        let mut m = HashSet::new();
        let mut v = HashSet::new();

        m.insert(center_idx);
        v.insert(center_idx);
        for &(f, t) in &app.graph_layout.edges {
            if f == center_idx || t == center_idx {
                v.insert(f);
                v.insert(t);
            }
        }
        (m, v)
    } else {
        (HashSet::new(), HashSet::new())
    };

    if inspect_active {
        app.apply_pending_graph_focus(canvas_w, canvas_h, inspect_center_idx);
    } else if app.pending_graph_focus {
        app.pending_graph_focus = false;
    }

    let is_graph_focused = app.focused_panel == FocusedPanel::Graph;
    let graph_border = if is_graph_focused {
        BORDER_FOCUSED
    } else {
        BORDER_UNFOCUSED
    };

    let (view_title, display_nodes, display_edges) = match &app.active_view {
        ActiveView::Inspect(pkg) => {
            // Count only visible edges for the title
            let visible_edges = app
                .graph_layout
                .edges
                .iter()
                .filter(|&&(f, t)| search_visible.contains(&f) && search_visible.contains(&t))
                .count();
            (format!(" {} ", pkg), search_visible.len(), visible_edges)
        }
        _ => (
            " Dependency Graph ".to_string(),
            app.graph_layout.labels.len(),
            app.graph_layout.edges.len(),
        ),
    };

    let block = Block::default()
        .title(Span::styled(
            format!(
                "{}[{} nodes, {} edges] ",
                view_title, display_nodes, display_edges,
            ),
            if is_graph_focused {
                Style::default()
                    .fg(BORDER_FOCUSED)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(FG_OVERLAY)
            },
        ))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(graph_border))
        .style(Style::default().bg(BG_SURFACE));

    let layout = &app.graph_layout;
    let snapped: Vec<(f64, f64)> = layout
        .positions
        .iter()
        .map(|p| {
            // Provide raw physical coordinates (with slight anti-jitter rounding)
            (
                (p.x / 2.0).floor() * 2.0 + 1.0,
                (p.y / 4.0).floor() * 4.0 + 2.0,
            )
        })
        .collect();

    let cache = app.render_cache.as_ref();
    let n_nodes = layout.positions.len();
    let max_edges = if n_nodes > 200 {
        400
    } else if n_nodes > 100 {
        600
    } else {
        usize::MAX
    };

    let snapped_cloned = snapped.clone();
    let search_visible_cloned = search_visible.clone();

    // Clone blast state into the canvas closure for edge coloring
    let blast_active = app.blast_overlay_active;
    let blast_scores_cloned = app.node_blast_scores.clone();
    let cascade_cloned = app.cascade_highlight.clone();
    let hovered_cloned = app.hovered_node;

    let visible_w = canvas_w.max(1.0) / app.graph_scale;
    let visible_h = canvas_h.max(1.0) / app.graph_scale;
    let pan_x = app.graph_pan_x;
    let pan_y = app.graph_pan_y;

    let canvas = Canvas::default()
        .block(block)
        .marker(ratatui::symbols::Marker::Braille)
        .x_bounds([pan_x, pan_x + visible_w])
        .y_bounds([pan_y, pan_y + visible_h])
        .paint(move |ctx| {
            // Pre-compute whether cascade should affect edge rendering
            let edge_use_cascade = cascade_cloned.is_some()
                && (!is_filtered
                    || hovered_cloned.is_some_and(|h| search_visible_cloned.contains(&h)));

            if let Some(c) = cache {
                for (count, &idx) in c.sorted_edge_indices.iter().rev().enumerate() {
                    let &(f, t) = &layout.edges[idx];
                    if count >= max_edges {
                        break;
                    }

                    if is_filtered {
                        let is_f_visible = search_visible_cloned.contains(&f);
                        let is_t_visible = search_visible_cloned.contains(&t);
                        // In inspect/filter mode: completely skip edges not in the subgraph
                        if !is_f_visible || !is_t_visible {
                            continue;
                        }
                    }

                    let (x1, y1) = snapped_cloned[f];
                    let (x2, y2) = snapped_cloned[t];

                    let color = if blast_active {
                        if edge_use_cascade {
                            let cascade = cascade_cloned.as_ref().unwrap();
                            let f_in = cascade.iter().any(|(ci, _, _)| *ci == f)
                                || hovered_cloned == Some(f);
                            let t_in = cascade.iter().any(|(ci, _, _)| *ci == t)
                                || hovered_cloned == Some(t);
                            if f_in && t_in {
                                // Edge on the cascade path: bright
                                use crate::tui::graph_renderer::cascade_distance_color;
                                let max_dist = cascade
                                    .iter()
                                    .filter(|(ci, _, _)| *ci == f || *ci == t)
                                    .map(|(_, d, _)| *d)
                                    .min()
                                    .unwrap_or(1);
                                cascade_distance_color(max_dist)
                            } else {
                                // Not on cascade path: very dim
                                graph_relation_color(GraphRelationSemantic::CascadeDimmed)
                            }
                        } else {
                            // Blast heatmap: edge color = average blast score of endpoints
                            let s_f = blast_scores_cloned.get(f).copied().unwrap_or(0.0);
                            let s_t = blast_scores_cloned.get(t).copied().unwrap_or(0.0);
                            use crate::tui::graph_renderer::blast_color;
                            blast_color((s_f + s_t) / 2.0)
                        }
                    } else if is_filtered {
                        if let Some(center) = inspect_center_idx {
                            if f == center {
                                // Outbound: Peach — this module depends on target
                                graph_relation_color(GraphRelationSemantic::Outbound)
                            } else if t == center {
                                // Inbound: Teal — source depends on this module
                                graph_relation_color(GraphRelationSemantic::Inbound)
                            } else {
                                // Neighbor-to-neighbor: muted Sapphire
                                graph_relation_color(GraphRelationSemantic::Related)
                            }
                        } else {
                            // Filter mode (search): use weight-based but brighter
                            weighted_edge_color(layout.edge_weights[idx].max(2))
                        }
                    } else {
                        weighted_edge_color(layout.edge_weights[idx])
                    };

                    ctx.draw(&ratatui::widgets::canvas::Line {
                        x1,
                        y1,
                        x2,
                        y2,
                        color,
                    });
                }
            }
        });
    frame.render_widget(canvas, area);
    if area.height > 4 && area.width > 32 {
        let legend_area = Rect {
            x: area.x.saturating_add(2),
            y: area.y.saturating_add(1),
            width: area.width.saturating_sub(4),
            height: 1,
        };
        frame.render_widget(
            Paragraph::new(graph_legend_line(app)).style(Style::default().bg(BG_SURFACE)),
            legend_area,
        );
    }

    let buf = frame.buffer_mut();
    let label_max_len = if n_nodes > 80 { 12 } else { 14 };

    // When in inspect/filter mode, only apply cascade coloring if the source node
    // is within the visible subset — otherwise the cascade dims all visible nodes
    // because the source and its downstream neighbors are outside the view.
    let use_cascade = app.cascade_highlight.is_some()
        && (!is_filtered
            || app
                .hovered_node
                .is_some_and(|h| search_visible.contains(&h)));

    for (i, &(px, py)) in snapped.iter().enumerate() {
        let is_focus = inspect_center_idx == Some(i);
        let is_search_match =
            is_filtered && inspect_center_idx.is_none() && search_matched.contains(&i);
        let is_v = is_filtered && search_visible.contains(&i);
        let is_h = app.hovered_node == Some(i);

        // In inspect/filter mode: completely hide nodes not in the subgraph
        if is_filtered && !is_v {
            continue;
        }

        // Semantic Zooming: Hide labels if zoomed out too far, unless hovered or filtered
        let is_zoomed_out = app.graph_scale < 0.6;
        let show_l = is_h
            || (!is_zoomed_out
                && ((is_filtered && is_v)
                    || (!is_filtered && cache.is_some_and(|c| c.label_visible.contains(&i)))));

        let screen_x = (px - app.graph_pan_x) * app.graph_scale;
        let screen_y = (py - app.graph_pan_y) * app.graph_scale;

        // Skip if outside canvas
        if screen_x < 0.0 || screen_x > canvas_w || screen_y < 0.0 || screen_y > canvas_h {
            continue;
        }

        let col = area.x + 1 + (screen_x / 2.0) as u16;
        let row = area.y + 1 + ((canvas_h - screen_y) / 4.0) as u16;

        if col < area.x + area.width - 1 && row < area.y + area.height - 1 {
            let color = if app.blast_overlay_active {
                // Blast overlay mode: color by blast score or cascade distance
                if use_cascade {
                    let cascade = app.cascade_highlight.as_ref().unwrap();
                    if is_h {
                        graph_relation_color(GraphRelationSemantic::CascadeSource)
                    } else if let Some((_, dist, _)) = cascade.iter().find(|(ci, _, _)| *ci == i) {
                        use crate::tui::graph_renderer::cascade_distance_color;
                        cascade_distance_color(*dist)
                    } else {
                        graph_relation_color(GraphRelationSemantic::CascadeDimmed)
                    }
                } else if i < app.node_blast_scores.len() {
                    use crate::tui::graph_renderer::blast_color;
                    blast_color(app.node_blast_scores[i])
                } else {
                    default_graph_node_color(app, i)
                }
            } else if is_h {
                graph_relation_color(GraphRelationSemantic::Hover)
            } else if is_focus {
                graph_relation_color(GraphRelationSemantic::Focus)
            } else if is_search_match {
                graph_relation_color(GraphRelationSemantic::SearchMatch)
            } else if is_filtered && is_v {
                // Differentiate inbound vs outbound neighbors
                if let Some(center) = inspect_center_idx {
                    let is_inbound = layout.edges.iter().any(|&(f, t)| f == i && t == center);
                    let is_outbound = layout.edges.iter().any(|&(f, t)| f == center && t == i);
                    if is_inbound && is_outbound {
                        graph_relation_color(GraphRelationSemantic::Bidirectional)
                    } else if is_inbound {
                        graph_relation_color(GraphRelationSemantic::Inbound)
                    } else if is_outbound {
                        graph_relation_color(GraphRelationSemantic::Outbound)
                    } else {
                        graph_relation_color(GraphRelationSemantic::Related)
                    }
                } else {
                    graph_relation_color(GraphRelationSemantic::Related)
                }
            } else {
                default_graph_node_color(app, i)
            };

            // Articulation points get diamond ◆, others get circle ●
            let symbol = if app.blast_overlay_active {
                if let Some(ref br) = app.current_blast_radius {
                    let label = &layout.labels[i];
                    if br
                        .articulation_points
                        .iter()
                        .any(|a| a.module_name == *label)
                    {
                        "◆"
                    } else {
                        "●"
                    }
                } else {
                    "●"
                }
            } else if is_focus {
                "◆"
            } else if is_h {
                "◉"
            } else if is_search_match {
                "◎"
            } else {
                "●"
            };

            let cell = &mut buf[(col, row)];
            cell.set_symbol(symbol).set_fg(color);

            if show_l {
                let label = &layout.labels[i];
                let truncated_label =
                    (!is_h).then(|| super::widgets::truncate_str(label, label_max_len));
                let text = truncated_label.as_deref().unwrap_or(label.as_str());
                let text_len = text.chars().count() as u16;

                // Adaptive Label Placement:
                // If node is in the right 25% of the screen, flip label to the left.
                let is_right_edge = col > area.x + (area.width * 3 / 4);
                let (label_x, can_render) = if is_right_edge {
                    let lx = col.saturating_sub(text_len + 1);
                    (lx, lx > area.x)
                } else {
                    let lx = col + 2;
                    (lx, lx + text_len < area.x + area.width - 1)
                };

                if can_render {
                    let label_color = if is_focus || is_search_match || (is_filtered && is_v) {
                        color
                    } else if is_h {
                        Color::White
                    } else {
                        FG_OVERLAY
                    };
                    let label_style = if is_focus || is_h || is_search_match {
                        Style::default()
                            .fg(label_color)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(label_color)
                    };
                    buf.set_string(label_x, row, text, label_style);
                }
            }
        }
    }
}

fn sidebar_label(entry: &SidebarEntry) -> &str {
    match entry {
        SidebarEntry::Cluster { label, .. } | SidebarEntry::Member { label, .. } => label.as_str(),
    }
}

fn app_label_match(label: &str, anchor: &str) -> bool {
    label == anchor
}

fn architecture_positions(app: &App, width: f64, height: f64) -> Vec<(f64, f64)> {
    app.architecture_map
        .as_ref()
        .map(|map| {
            let mut sorted_ids = map
                .clusters
                .iter()
                .map(|cluster| cluster.id)
                .collect::<Vec<_>>();
            sorted_ids.sort_by(|a, b| {
                cluster_overview_rank(&map.clusters[*b])
                    .cmp(&cluster_overview_rank(&map.clusters[*a]))
                    .then_with(|| map.clusters[*a].name.cmp(&map.clusters[*b].name))
            });

            let center_x = width * 0.50;
            let center_y = height * 0.52;
            let radius_x = width * 0.24;
            let radius_y = height * 0.28;
            let count = sorted_ids.len().max(1) as f64;
            let mut positions = vec![(center_x, center_y); map.clusters.len()];

            for (idx, cluster_id) in sorted_ids.iter().enumerate() {
                let angle =
                    -std::f64::consts::FRAC_PI_2 + (idx as f64 / count) * std::f64::consts::TAU;
                let cluster = &map.clusters[*cluster_id];
                let mut x = center_x + radius_x * angle.cos();
                let y = center_y + radius_y * angle.sin();

                if cluster.is_dependency_sink() {
                    x = width * 0.78;
                } else if matches!(cluster.overview_role(), ClusterOverviewRole::SupportCluster) {
                    x = (x + width * 0.62) / 2.0;
                }

                positions[*cluster_id] = (x, y);
            }

            let min_x = width * 0.10;
            let max_x = width * 0.90;
            let min_y = height * 0.14;
            let max_y = height * 0.90;

            for _ in 0..36 {
                let mut forces = vec![(0.0, 0.0); map.clusters.len()];

                for left in 0..map.clusters.len() {
                    for right in left + 1..map.clusters.len() {
                        let (lx, ly) = positions[left];
                        let (rx, ry) = positions[right];
                        let dx = lx - rx;
                        let dy = ly - ry;
                        let dist_sq = (dx * dx + dy * dy).max(64.0);
                        let dist = dist_sq.sqrt();
                        let force = 4_200.0 / dist_sq;
                        let fx = (dx / dist) * force;
                        let fy = (dy / dist) * force;
                        forces[left].0 += fx;
                        forces[left].1 += fy;
                        forces[right].0 -= fx;
                        forces[right].1 -= fy;
                    }
                }

                for edge in &map.edges {
                    let (from_x, from_y) = positions[edge.from];
                    let (to_x, to_y) = positions[edge.to];
                    let dx = to_x - from_x;
                    let dy = to_y - from_y;
                    let dist = (dx * dx + dy * dy).sqrt().max(1.0);
                    let ideal = if is_dependency_sink_edge(map, edge) {
                        width * 0.22
                    } else {
                        width * 0.14
                    };
                    let pull = ((dist - ideal) / ideal)
                        * f64::from(edge.total_weight.max(1)).sqrt()
                        * 0.045;
                    let fx = (dx / dist) * pull;
                    let fy = (dy / dist) * pull;
                    forces[edge.from].0 += fx;
                    forces[edge.from].1 += fy;
                    forces[edge.to].0 -= fx;
                    forces[edge.to].1 -= fy;
                }

                for cluster in &map.clusters {
                    let idx = cluster.id;
                    let (x, y) = positions[idx];
                    let mut target_x = center_x;
                    if cluster.is_dependency_sink() {
                        target_x = width * 0.82;
                    } else if matches!(cluster.overview_role(), ClusterOverviewRole::SupportCluster)
                    {
                        target_x = width * 0.58;
                    }
                    let gravity = if cluster.is_dependency_sink() {
                        0.030
                    } else {
                        0.018
                    };
                    forces[idx].0 += (target_x - x) * gravity;
                    forces[idx].1 += (center_y - y) * gravity;
                }

                for (idx, cluster) in map.clusters.iter().enumerate() {
                    let x_step = 1.0 + (cluster.members.len() as f64).sqrt() * 0.08;
                    let y_step = 1.0 + (cluster.members.len() as f64).sqrt() * 0.05;
                    positions[idx].0 =
                        (positions[idx].0 + forces[idx].0 * x_step).clamp(min_x, max_x);
                    positions[idx].1 =
                        (positions[idx].1 + forces[idx].1 * y_step).clamp(min_y, max_y);
                }
            }

            positions
        })
        .unwrap_or_default()
}

fn cluster_overview_rank(cluster: &ClusterNode) -> (u8, u32, u32, usize, String) {
    let role_rank = match cluster.overview_role() {
        ClusterOverviewRole::PrimaryArchitecture => 2,
        ClusterOverviewRole::SupportCluster => 1,
        ClusterOverviewRole::ExternalSink => 0,
    };
    (
        role_rank,
        cluster.inbound_weight + cluster.outbound_weight + cluster.internal_weight,
        cluster.internal_weight,
        cluster.members.len(),
        cluster.name.clone(),
    )
}

fn preferred_overview_cluster(map: &ArchitectureMap) -> Option<&ClusterNode> {
    map.clusters.iter().max_by(|a, b| {
        cluster_overview_rank(a)
            .cmp(&cluster_overview_rank(b))
            .then_with(|| b.name.cmp(&a.name))
    })
}

fn overview_anchor_reason(cluster: &ClusterNode) -> &'static str {
    match cluster.overview_role() {
        ClusterOverviewRole::PrimaryArchitecture => {
            "chosen because it is the most connected internal cluster"
        }
        ClusterOverviewRole::SupportCluster => {
            "chosen because it is the strongest support-side connector"
        }
        ClusterOverviewRole::ExternalSink => {
            "chosen because it is the strongest third-party cluster"
        }
    }
}

fn is_dependency_sink_edge(map: &ArchitectureMap, edge: &ClusterEdge) -> bool {
    map.clusters
        .get(edge.from)
        .is_some_and(ClusterNode::is_dependency_sink)
        || map
            .clusters
            .get(edge.to)
            .is_some_and(ClusterNode::is_dependency_sink)
}

fn bridge_overview_rank(map: &ArchitectureMap, edge: &ClusterEdge) -> (u8, u32, usize) {
    let from_role = map.clusters[edge.from].overview_role();
    let to_role = map.clusters[edge.to].overview_role();
    let role_rank = if matches!(from_role, ClusterOverviewRole::PrimaryArchitecture)
        && matches!(to_role, ClusterOverviewRole::PrimaryArchitecture)
    {
        2
    } else if !matches!(from_role, ClusterOverviewRole::ExternalSink)
        && !matches!(to_role, ClusterOverviewRole::ExternalSink)
    {
        1
    } else {
        0
    };
    (role_rank, edge.total_weight, edge.edge_count)
}

fn preferred_overview_bridge(map: &ArchitectureMap) -> Option<&ClusterEdge> {
    map.edges.iter().max_by(|a, b| {
        bridge_overview_rank(map, a)
            .cmp(&bridge_overview_rank(map, b))
            .then_with(|| {
                map.clusters[b.from]
                    .name
                    .cmp(&map.clusters[a.from].name)
                    .then_with(|| map.clusters[b.to].name.cmp(&map.clusters[a.to].name))
            })
    })
}

fn architecture_summary_lines(
    app: &App,
    map: &ArchitectureMap,
    omitted_sink_edges: usize,
) -> Vec<String> {
    let mut lines = Vec::new();
    if map.clusters.is_empty() {
        lines.push("No clusters available for the current snapshot.".to_string());
        return lines;
    }

    let overview_anchor = preferred_overview_cluster(map);
    if let Some(cluster) = overview_anchor {
        lines.push(format!(
            "Overview anchor is `{}` ({} members, in:{} out:{}) - {}.",
            cluster.name,
            cluster.members.len(),
            cluster.inbound_weight,
            cluster.outbound_weight,
            overview_anchor_reason(cluster)
        ));
    }

    let raw_strongest_bridge = map
        .edges
        .iter()
        .max_by_key(|edge| (edge.total_weight, edge.edge_count));
    if let Some(edge) = preferred_overview_bridge(map).or(raw_strongest_bridge) {
        lines.push(format!(
            "Strongest link is `{}` -> `{}` (weight {}, {} links).",
            map.clusters[edge.from].name,
            map.clusters[edge.to].name,
            edge.total_weight,
            edge.edge_count
        ));
    } else {
        lines.push("No inter-cluster links were detected.".to_string());
    }

    if let Some(edge) = raw_strongest_bridge
        && is_dependency_sink_edge(map, edge)
        && preferred_overview_bridge(map)
            .is_some_and(|preferred| preferred.from != edge.from || preferred.to != edge.to)
    {
        lines.push(format!(
            "Strongest third-party link is `{}` -> `{}` (weight {}, {} links).",
            map.clusters[edge.from].name,
            map.clusters[edge.to].name,
            edge.total_weight,
            edge.edge_count
        ));
    }

    let deprioritized_sinks = map
        .clusters
        .iter()
        .filter(|cluster| cluster.is_dependency_sink())
        .count();
    if deprioritized_sinks > 0 && deprioritized_sinks < map.clusters.len() {
        lines.push(format!(
            "{} external-heavy cluster(s) are deprioritized in overview.",
            deprioritized_sinks
        ));
    }

    if omitted_sink_edges > 0 {
        lines.push(format!(
            "+{} lower-signal third-party link(s) omitted from the map.",
            omitted_sink_edges
        ));
    }

    if let Some(scan_metadata) = app.current_scan_metadata() {
        lines.push(format!(
            "Third-party view keeps dependencies with {}+ importers; {} lower-signal dependencies are hidden.",
            scan_metadata.external_min_importers,
            scan_metadata.filtered_external_count
        ));
    }

    let isolated = map
        .clusters
        .iter()
        .filter(|cluster| cluster.inbound_weight == 0 && cluster.outbound_weight == 0)
        .count();
    if isolated > 0 {
        lines.push(format!(
            "{} cluster(s) are isolated or weakly connected.",
            isolated
        ));
    }

    lines
}

fn cluster_kind_label(cluster: &ClusterNode) -> &'static str {
    match cluster.kind {
        super::architecture_map::ClusterKind::Workspace => "workspace",
        super::architecture_map::ClusterKind::Deps => "third-party",
        super::architecture_map::ClusterKind::Entry => "entrypoint",
        super::architecture_map::ClusterKind::External => "external",
        super::architecture_map::ClusterKind::Infra => "support",
        super::architecture_map::ClusterKind::Domain => "domain",
        super::architecture_map::ClusterKind::Group => "unclassified",
    }
}

fn cluster_summary_note(cluster: &ClusterNode) -> &'static str {
    if cluster.is_dependency_sink() {
        "Shared third-party packages grouped for the architecture view."
    } else {
        ""
    }
}

fn cluster_summary_type_label(cluster: &ClusterNode) -> &'static str {
    if cluster.is_dependency_sink() {
        "third-party cluster"
    } else {
        cluster_kind_label(cluster)
    }
}

fn panel_title(title: impl Into<String>) -> Span<'static> {
    Span::styled(
        format!(" {} ", title.into()),
        Style::default()
            .fg(ACCENT_LAVENDER)
            .add_modifier(Modifier::BOLD),
    )
}

fn legend_chip(label: &'static str, color: Color) -> Vec<Span<'static>> {
    vec![
        Span::styled("●", Style::default().fg(color)),
        Span::styled(format!(" {label}"), Style::default().fg(FG_OVERLAY)),
    ]
}

fn ui_color_mode(app: &App) -> ClusterColorMode {
    app.clustering_config.effective_color_mode()
}

fn edge_weight_legend_line() -> Line<'static> {
    let mut spans = vec![Span::styled("Edges ", Style::default().fg(FG_TEXT))];
    for (idx, (label, color)) in [
        ("light", weighted_edge_color(1)),
        ("normal", weighted_edge_color(4)),
        ("heavy", weighted_edge_color(8)),
        ("critical", weighted_edge_color(16)),
    ]
    .into_iter()
    .enumerate()
    {
        if idx > 0 {
            spans.push(Span::raw("  "));
        }
        spans.extend(legend_chip(label, color));
    }
    Line::from(spans)
}

fn inspect_relation_legend_line() -> Line<'static> {
    let mut spans = vec![Span::styled("Nodes ", Style::default().fg(FG_TEXT))];
    for (idx, (label, color)) in [
        (
            "◆ focus",
            graph_relation_color(GraphRelationSemantic::Focus),
        ),
        ("in", graph_relation_color(GraphRelationSemantic::Inbound)),
        ("out", graph_relation_color(GraphRelationSemantic::Outbound)),
        (
            "both",
            graph_relation_color(GraphRelationSemantic::Bidirectional),
        ),
        (
            "◉ hover",
            graph_relation_color(GraphRelationSemantic::Hover),
        ),
    ]
    .into_iter()
    .enumerate()
    {
        if idx > 0 {
            spans.push(Span::raw("  "));
        }
        spans.extend(legend_chip(label, color));
    }
    Line::from(spans)
}

fn search_relation_legend_line() -> Line<'static> {
    let mut spans = vec![Span::styled("Search ", Style::default().fg(FG_TEXT))];
    for (idx, (label, color)) in [
        (
            "◎ match",
            graph_relation_color(GraphRelationSemantic::SearchMatch),
        ),
        (
            "related",
            graph_relation_color(GraphRelationSemantic::Related),
        ),
        (
            "◉ hover",
            graph_relation_color(GraphRelationSemantic::Hover),
        ),
    ]
    .into_iter()
    .enumerate()
    {
        if idx > 0 {
            spans.push(Span::raw("  "));
        }
        spans.extend(legend_chip(label, color));
    }
    Line::from(spans)
}

fn blast_legend_line() -> Line<'static> {
    let mut spans = vec![Span::styled("Blast ", Style::default().fg(FG_TEXT))];
    for (idx, (label, color)) in [
        ("low", crate::tui::graph_renderer::blast_color(0.08)),
        ("mid", crate::tui::graph_renderer::blast_color(0.40)),
        ("high", crate::tui::graph_renderer::blast_color(0.72)),
        (
            "source",
            graph_relation_color(GraphRelationSemantic::CascadeSource),
        ),
    ]
    .into_iter()
    .enumerate()
    {
        if idx > 0 {
            spans.push(Span::raw("  "));
        }
        spans.extend(legend_chip(label, color));
    }
    Line::from(spans)
}

fn overview_legend_line(app: &App) -> Line<'static> {
    let mut spans = vec![Span::styled("Map ", Style::default().fg(FG_TEXT))];
    let items = if matches!(ui_color_mode(app), ClusterColorMode::Semantic) {
        vec![
            ("anchor", cluster_map_color(ClusterMapSemantic::Central)),
            (
                "third-party",
                cluster_map_color(ClusterMapSemantic::ThirdParty),
            ),
            ("entry", cluster_map_color(ClusterMapSemantic::Entrypoint)),
            ("support", cluster_map_color(ClusterMapSemantic::Support)),
            ("hover", cluster_map_color(ClusterMapSemantic::Hovered)),
        ]
    } else {
        vec![
            ("anchor", cluster_map_color(ClusterMapSemantic::Central)),
            (
                "third-party",
                cluster_map_color(ClusterMapSemantic::ThirdParty),
            ),
            ("other", cluster_map_color(ClusterMapSemantic::Neutral)),
            ("hover", cluster_map_color(ClusterMapSemantic::Hovered)),
        ]
    };
    for (idx, (label, color)) in items.into_iter().enumerate() {
        if idx > 0 {
            spans.push(Span::raw("  "));
        }
        spans.extend(legend_chip(label, color));
    }
    Line::from(spans)
}

fn overview_legend_line_for_help() -> Line<'static> {
    let mut spans = vec![Span::styled("Map ", Style::default().fg(FG_TEXT))];
    for (idx, (label, color)) in [
        ("anchor", cluster_map_color(ClusterMapSemantic::Central)),
        (
            "third-party",
            cluster_map_color(ClusterMapSemantic::ThirdParty),
        ),
        (
            "entry/support",
            cluster_map_color(ClusterMapSemantic::Entrypoint),
        ),
        ("other", cluster_map_color(ClusterMapSemantic::Neutral)),
        ("hover", cluster_map_color(ClusterMapSemantic::Hovered)),
    ]
    .into_iter()
    .enumerate()
    {
        if idx > 0 {
            spans.push(Span::raw("  "));
        }
        spans.extend(legend_chip(label, color));
    }
    Line::from(spans)
}

fn graph_legend_line(app: &App) -> Line<'static> {
    if app.blast_overlay_active {
        blast_legend_line()
    } else if matches!(app.active_view, ActiveView::Inspect(_)) {
        inspect_relation_legend_line()
    } else if !app.filter_text.is_empty() {
        search_relation_legend_line()
    } else {
        let mut spans = edge_weight_legend_line().spans;
        spans.push(Span::raw("  "));
        spans.push(Span::styled("nodes", Style::default().fg(FG_TEXT)));
        spans.push(Span::raw(" "));
        spans.push(Span::styled(
            if matches!(ui_color_mode(app), ClusterColorMode::Semantic) {
                "family hues"
            } else {
                "neutral"
            },
            Style::default().fg(
                if matches!(ui_color_mode(app), ClusterColorMode::Semantic) {
                    crate::tui::graph_renderer::palette_node_color(0)
                } else {
                    graph_relation_color(GraphRelationSemantic::Neutral)
                },
            ),
        ));
        Line::from(spans)
    }
}

fn default_graph_node_color(app: &App, index: usize) -> Color {
    match ui_color_mode(app) {
        ClusterColorMode::Semantic => crate::tui::graph_renderer::palette_node_color(index),
        ClusterColorMode::Minimal => graph_relation_color(GraphRelationSemantic::Neutral),
    }
}

fn overview_cluster_display_color(
    app: &App,
    cluster: &ClusterNode,
    is_hovered: bool,
    is_central: bool,
) -> Color {
    if is_hovered {
        return cluster_map_color(ClusterMapSemantic::Hovered);
    }
    if is_central {
        return cluster_map_color(ClusterMapSemantic::Central);
    }

    match ui_color_mode(app) {
        ClusterColorMode::Semantic => {
            if cluster.is_dependency_sink() {
                cluster_map_color(ClusterMapSemantic::ThirdParty)
            } else if matches!(cluster.kind, super::architecture_map::ClusterKind::Entry) {
                cluster_map_color(ClusterMapSemantic::Entrypoint)
            } else if matches!(cluster.kind, super::architecture_map::ClusterKind::Infra) {
                cluster_map_color(ClusterMapSemantic::Support)
            } else {
                cluster_map_color(ClusterMapSemantic::Neutral)
            }
        }
        ClusterColorMode::Minimal => {
            if cluster.is_dependency_sink() {
                cluster_map_color(ClusterMapSemantic::ThirdParty)
            } else {
                cluster_map_color(ClusterMapSemantic::Neutral)
            }
        }
    }
}

fn subsection_title(title: impl Into<String>) -> Span<'static> {
    Span::styled(
        format!(" {} ", title.into()),
        Style::default().fg(FG_TEXT).add_modifier(Modifier::BOLD),
    )
}

fn cluster_semantic_hint(cluster: &ClusterNode) -> Option<&'static str> {
    match cluster.kind {
        super::architecture_map::ClusterKind::Workspace => Some("workspace"),
        super::architecture_map::ClusterKind::Deps => Some("third-party"),
        super::architecture_map::ClusterKind::Entry => Some("entrypoint"),
        super::architecture_map::ClusterKind::External => Some("external"),
        super::architecture_map::ClusterKind::Infra => Some("support"),
        super::architecture_map::ClusterKind::Domain
        | super::architecture_map::ClusterKind::Group => None,
    }
}

#[derive(Debug, Default, Clone)]
struct OverviewEdgeSelection {
    indices: Vec<usize>,
    omitted_sink_edges: usize,
    primary_bridge: Option<usize>,
    external_bridge: Option<usize>,
}

fn edge_index_for(map: &ArchitectureMap, from: usize, to: usize) -> Option<usize> {
    map.edges
        .iter()
        .position(|edge| edge.from == from && edge.to == to)
}

fn overview_edge_selection(map: &ArchitectureMap) -> OverviewEdgeSelection {
    let mut outgoing_by_cluster = HashMap::<usize, Vec<(usize, u32, usize)>>::new();
    for (idx, edge) in map.edges.iter().enumerate() {
        outgoing_by_cluster.entry(edge.from).or_default().push((
            idx,
            edge.total_weight,
            edge.edge_count,
        ));
    }

    let mut chosen = HashSet::new();
    for (cluster_id, edges) in outgoing_by_cluster.iter_mut() {
        edges.sort_by(|a, b| {
            b.1.cmp(&a.1)
                .then_with(|| b.2.cmp(&a.2))
                .then_with(|| a.0.cmp(&b.0))
        });
        let per_cluster_limit = match map
            .clusters
            .get(*cluster_id)
            .map(ClusterNode::overview_role)
        {
            Some(ClusterOverviewRole::PrimaryArchitecture) => 2,
            Some(ClusterOverviewRole::SupportCluster) => 1,
            _ => 1,
        };
        for (idx, _, _) in edges.iter().take(per_cluster_limit) {
            chosen.insert(*idx);
        }
    }

    let primary_bridge =
        preferred_overview_bridge(map).and_then(|edge| edge_index_for(map, edge.from, edge.to));
    if let Some(idx) = primary_bridge {
        chosen.insert(idx);
    }

    let raw_strongest_bridge = map
        .edges
        .iter()
        .max_by_key(|edge| (edge.total_weight, edge.edge_count));
    let external_bridge = raw_strongest_bridge
        .filter(|edge| is_dependency_sink_edge(map, edge))
        .and_then(|edge| edge_index_for(map, edge.from, edge.to));
    if let Some(idx) = external_bridge {
        chosen.insert(idx);
    }

    let mut ranked = chosen.into_iter().collect::<Vec<_>>();
    ranked.sort_by(|a, b| {
        let left = &map.edges[*a];
        let right = &map.edges[*b];
        bridge_overview_rank(map, right)
            .cmp(&bridge_overview_rank(map, left))
            .then_with(|| right.total_weight.cmp(&left.total_weight))
            .then_with(|| right.edge_count.cmp(&left.edge_count))
            .then_with(|| a.cmp(b))
    });

    const OVERVIEW_EDGE_LIMIT: usize = 10;
    let omitted_sink_edges = ranked
        .iter()
        .skip(OVERVIEW_EDGE_LIMIT)
        .filter(|idx| is_dependency_sink_edge(map, &map.edges[**idx]))
        .count();
    ranked.truncate(OVERVIEW_EDGE_LIMIT);
    OverviewEdgeSelection {
        indices: ranked,
        omitted_sink_edges,
        primary_bridge,
        external_bridge,
    }
}

fn overview_hit_test(app: &App, px: f64, py: f64, width: f64, height: f64) -> Option<usize> {
    let Some(map) = &app.architecture_map else {
        return None;
    };
    let positions = architecture_positions(app, width, height);
    let mut closest = None;
    let mut best_dist = f64::MAX;
    for cluster in &map.clusters {
        let Some(&(cx, cy)) = positions.get(cluster.id) else {
            continue;
        };
        let radius = overview_cluster_hit_radius(cluster);
        let dist = ((cx - px).powi(2) + (cy - py).powi(2)).sqrt();
        if dist <= radius && dist < best_dist {
            best_dist = dist;
            closest = Some(cluster.id);
        }
    }
    closest
}

fn overview_hit_test_terminal(
    app: &App,
    col: u16,
    row: u16,
    map_area: Rect,
    width: f64,
    height: f64,
) -> Option<usize> {
    let Some(map) = &app.architecture_map else {
        return None;
    };

    let positions = architecture_positions(app, width, height);
    let mut closest = None;
    let mut best_score = (u16::MAX, f64::MAX);

    for cluster in &map.clusters {
        let Some(&(px, py)) = positions.get(cluster.id) else {
            continue;
        };
        let screen_x = (px - app.graph_pan_x) * app.graph_scale;
        let screen_y = (py - app.graph_pan_y) * app.graph_scale;
        if screen_x < 0.0 || screen_y < 0.0 || screen_x > width || screen_y > height {
            continue;
        }

        let anchor_col = map_area.x + 1 + (screen_x / 2.0) as u16;
        let anchor_row = map_area.y + 1 + ((height - screen_y) / 4.0) as u16;

        let label = format!("{} ({})", cluster.name, cluster.members.len());
        let text = super::widgets::truncate_str(&label, 24);
        let label_len = text.chars().count() as u16;
        let label_x = if anchor_col > map_area.x + (map_area.width * 3 / 4) {
            anchor_col.saturating_sub(label_len + 1)
        } else {
            anchor_col + 2
        };

        let on_anchor =
            row == anchor_row && col >= anchor_col.saturating_sub(1) && col <= anchor_col + 1;
        let on_label = row == anchor_row
            && label_x > map_area.x
            && label_x + label_len < map_area.x + map_area.width - 1
            && col >= label_x
            && col < label_x + label_len;

        if on_anchor || on_label {
            let dx = (col as i32 - anchor_col as i32).unsigned_abs() as u16;
            let dy = (row as i32 - anchor_row as i32).unsigned_abs() as u16;
            let score = (dx + dy, dx as f64);
            if score < best_score {
                best_score = score;
                closest = Some(cluster.id);
            }
        }
    }

    closest
}

fn overview_cluster_hit_radius(cluster: &ClusterNode) -> f64 {
    let member_factor = (cluster.members.len() as f64).sqrt() * 0.9;
    let role_bonus = match cluster.overview_role() {
        ClusterOverviewRole::PrimaryArchitecture => 0.0,
        ClusterOverviewRole::SupportCluster => 0.8,
        ClusterOverviewRole::ExternalSink => 1.2,
    };
    (7.0 + member_factor + role_bonus).clamp(7.5, 16.0)
}

fn overview_summary_rect(area: Rect) -> Rect {
    Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(6), Constraint::Min(8)])
        .split(area)[0]
}

fn overview_map_rect(area: Rect) -> Rect {
    Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(6), Constraint::Min(8)])
        .split(area)[1]
}

fn render_architecture_overview(
    frame: &mut Frame,
    area: Rect,
    app: &mut App,
    _canvas_w: f64,
    _canvas_h: f64,
) {
    let Some(map) = app.architecture_map.as_ref() else {
        return;
    };

    let summary_area = overview_summary_rect(area);
    let map_area = overview_map_rect(area);
    let map_canvas_w = (map_area.width.saturating_sub(2) as f64) * 2.0;
    let map_canvas_h = (map_area.height.saturating_sub(2) as f64) * 4.0;

    let hovered = app
        .hovered_cluster
        .and_then(|cluster_id| map.clusters.get(cluster_id))
        .map(|cluster| {
            let kind =
                cluster_semantic_hint(cluster).unwrap_or_else(|| cluster_kind_label(cluster));
            format!(
                "  [hint: {} | {} members | in:{} out:{}]",
                kind,
                cluster.members.len(),
                cluster.inbound_weight,
                cluster.outbound_weight
            )
        })
        .unwrap_or_default();

    let is_focused = app.focused_panel == FocusedPanel::Graph;
    let border_color = if is_focused {
        BORDER_FOCUSED
    } else {
        BORDER_UNFOCUSED
    };

    let summary_block = Block::default()
        .title(Span::styled(
            format!(
                " Map | Architecture [{} clusters, {} links]{} ",
                map.clusters.len(),
                map.edges.len(),
                hovered
            ),
            if is_focused {
                Style::default()
                    .fg(BORDER_FOCUSED)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(FG_OVERLAY)
            },
        ))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color))
        .style(Style::default().bg(BG_SURFACE));
    let summary_inner = summary_block.inner(summary_area);
    frame.render_widget(summary_block, summary_area);

    let edge_selection = overview_edge_selection(map);
    let summary_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(summary_inner);
    let summary_lines = architecture_summary_lines(app, map, edge_selection.omitted_sink_edges)
        .into_iter()
        .take(summary_chunks[0].height as usize)
        .map(|line| Line::from(Span::styled(line, Style::default().fg(FG_TEXT))))
        .collect::<Vec<_>>();
    frame.render_widget(
        Paragraph::new(summary_lines).wrap(Wrap { trim: true }),
        summary_chunks[0],
    );
    if summary_chunks[1].height > 0 {
        frame.render_widget(
            Paragraph::new(overview_legend_line(app)).style(Style::default().bg(BG_SURFACE)),
            summary_chunks[1],
        );
    }

    let positions = architecture_positions(app, map_canvas_w, map_canvas_h);
    let visible_w = map_canvas_w.max(1.0) / app.graph_scale;
    let visible_h = map_canvas_h.max(1.0) / app.graph_scale;
    let pan_x = app.graph_pan_x;
    let pan_y = app.graph_pan_y;
    let edge_indices = edge_selection.indices;
    let primary_bridge_idx = edge_selection.primary_bridge;
    let external_bridge_idx = edge_selection.external_bridge;
    let central_cluster_id = preferred_overview_cluster(map).map(|cluster| cluster.id);
    let positions_for_canvas = positions.clone();

    let map_block = Block::default()
        .title(Span::styled(
            " Cluster map ",
            if is_focused {
                Style::default()
                    .fg(BORDER_FOCUSED)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(FG_OVERLAY)
            },
        ))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color))
        .style(Style::default().bg(BG_SURFACE));

    let canvas = Canvas::default()
        .block(map_block)
        .marker(ratatui::symbols::Marker::Braille)
        .x_bounds([pan_x, pan_x + visible_w])
        .y_bounds([pan_y, pan_y + visible_h])
        .paint(move |ctx| {
            for edge_idx in &edge_indices {
                let edge = &map.edges[*edge_idx];
                let Some(&(x1, y1)) = positions_for_canvas.get(edge.from) else {
                    continue;
                };
                let Some(&(x2, y2)) = positions_for_canvas.get(edge.to) else {
                    continue;
                };
                let edge_color = if Some(*edge_idx) == primary_bridge_idx {
                    overview_edge_color(OverviewEdgeSemantic::PrimaryBridge)
                } else if Some(*edge_idx) == external_bridge_idx {
                    overview_edge_color(OverviewEdgeSemantic::ExternalBridge)
                } else if is_dependency_sink_edge(map, edge) {
                    overview_edge_color(OverviewEdgeSemantic::ExternalSink)
                } else {
                    weighted_edge_color(edge.total_weight.max(edge.edge_count as u32))
                };
                ctx.draw(&ratatui::widgets::canvas::Line {
                    x1,
                    y1,
                    x2,
                    y2,
                    color: edge_color,
                });
            }
        });
    frame.render_widget(canvas, map_area);

    let buf = frame.buffer_mut();
    for cluster in &map.clusters {
        let Some(&(px, py)) = positions.get(cluster.id) else {
            continue;
        };
        let screen_x = (px - app.graph_pan_x) * app.graph_scale;
        let screen_y = (py - app.graph_pan_y) * app.graph_scale;
        if screen_x < 0.0 || screen_y < 0.0 || screen_x > map_canvas_w || screen_y > map_canvas_h {
            continue;
        }
        let col = map_area.x + 1 + (screen_x / 2.0) as u16;
        let row = map_area.y + 1 + ((map_canvas_h - screen_y) / 4.0) as u16;
        if col >= map_area.x + map_area.width - 1 || row >= map_area.y + map_area.height - 1 {
            continue;
        }

        let is_hovered = app.hovered_cluster == Some(cluster.id);
        let is_central = central_cluster_id == Some(cluster.id);
        let color = overview_cluster_display_color(app, cluster, is_hovered, is_central);

        buf[(col, row)]
            .set_symbol(if is_hovered {
                "@"
            } else if is_central {
                "*"
            } else {
                "o"
            })
            .set_fg(color);

        let label = format!("{} ({})", cluster.name, cluster.members.len());
        let text = super::widgets::truncate_str(&label, 24);
        let label_len = text.chars().count() as u16;
        let label_x = if col > map_area.x + (map_area.width * 3 / 4) {
            col.saturating_sub(label_len + 1)
        } else {
            col + 2
        };
        if label_x > map_area.x && label_x + label_len < map_area.x + map_area.width - 1 {
            buf.set_string(
                label_x,
                row,
                text,
                Style::default()
                    .fg(if is_hovered { Color::White } else { color })
                    .add_modifier(if is_hovered || is_central {
                        Modifier::BOLD
                    } else {
                        Modifier::empty()
                    }),
            );
        }
    }
}

fn render_cluster_workspace(frame: &mut Frame, area: Rect, app: &mut App) {
    let Some(cluster_id) = app.selected_cluster else {
        return;
    };
    let Some(map) = app.architecture_map.as_ref() else {
        return;
    };
    let Some(cluster) = map.clusters.get(cluster_id) else {
        return;
    };

    let is_focused = app.focused_panel == FocusedPanel::Graph;
    let border_color = if is_focused {
        BORDER_FOCUSED
    } else {
        BORDER_UNFOCUSED
    };
    let block = Block::default()
        .title(Span::styled(
            format!(
                " {}: {} [{} members] ",
                if cluster.is_dependency_sink() {
                    "Third-party cluster"
                } else {
                    "Cluster details"
                },
                cluster.name,
                cluster.members.len(),
            ),
            if is_focused {
                Style::default()
                    .fg(BORDER_FOCUSED)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(FG_OVERLAY)
            },
        ))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color))
        .style(Style::default().bg(BG_SURFACE));

    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width < 12 || inner.height < 6 {
        return;
    }

    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(4)])
        .split(inner);
    let body_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length((sections[1].height * 2 / 5).max(11)),
            Constraint::Min(10),
        ])
        .split(sections[1]);

    let header = Line::from(vec![
        Span::styled(
            " Enter ",
            Style::default()
                .fg(BG_BASE)
                .bg(Color::Rgb(166, 227, 161))
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
        Span::styled(
            if cluster.is_dependency_sink() {
                "Inspect selected dependency"
            } else {
                "Inspect selected member"
            },
            Style::default().fg(FG_TEXT),
        ),
    ]);
    frame.render_widget(Paragraph::new(header), sections[0]);

    render_cluster_summary(frame, body_chunks[0], app, cluster_id);
    render_cluster_focus_preview(frame, body_chunks[1], app, cluster_id);
}

fn render_cluster_summary(frame: &mut Frame, area: Rect, app: &App, cluster_id: usize) {
    let Some(map) = app.architecture_map.as_ref() else {
        return;
    };
    let Some(cluster) = map.clusters.get(cluster_id) else {
        return;
    };

    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(34),
            Constraint::Length(1),
            Constraint::Percentage(38),
            Constraint::Length(1),
            Constraint::Percentage(28),
        ])
        .split(area);
    render_vertical_separator(frame, cols[1], 1);
    render_vertical_separator(frame, cols[3], 1);
    let summary_col = cols[0];
    let members_col = cols[2];
    let bridges_col = cols[4];

    let member_stats = cluster
        .members
        .iter()
        .map(|member| {
            let label = &app.graph_layout.labels[*member];
            let mut incoming = 0u32;
            let mut outgoing = 0u32;
            for (edge_idx, &(from, to)) in app.graph_layout.edges.iter().enumerate() {
                let weight = app
                    .graph_layout
                    .edge_weights
                    .get(edge_idx)
                    .copied()
                    .unwrap_or(1);
                if from == *member {
                    outgoing += weight;
                }
                if to == *member {
                    incoming += weight;
                }
            }
            (label.clone(), incoming + outgoing, incoming, outgoing)
        })
        .collect::<Vec<_>>();

    let mut ranked_members = member_stats;
    ranked_members.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));

    let mut inbound = map
        .edges
        .iter()
        .filter(|edge| edge.to == cluster_id)
        .map(|edge| {
            (
                map.clusters[edge.from].name.clone(),
                edge.total_weight,
                edge.edge_count,
            )
        })
        .collect::<Vec<_>>();
    inbound.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));

    let mut outbound = map
        .edges
        .iter()
        .filter(|edge| edge.from == cluster_id)
        .map(|edge| {
            (
                map.clusters[edge.to].name.clone(),
                edge.total_weight,
                edge.edge_count,
            )
        })
        .collect::<Vec<_>>();
    outbound.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let member_limit = usize::from(members_col.height.saturating_sub(2)).clamp(6, 18);
    let bridge_limit = usize::from(bridges_col.height.saturating_sub(5) / 2).clamp(3, 10);

    let internal_count = cluster.internal_member_count;
    let external_count = cluster.external_member_count;
    let summary_lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            format!(" Type: {}", cluster_summary_type_label(cluster)),
            Style::default().fg(ACCENT_BLUE),
        )),
        Line::from(Span::styled(
            format!(" Members: {}", cluster.members.len()),
            Style::default().fg(FG_TEXT),
        )),
        Line::from(Span::styled(
            format!(
                " Members: {} internal, {} third-party",
                internal_count, external_count
            ),
            Style::default().fg(FG_OVERLAY),
        )),
        Line::from(""),
        Line::from(Span::styled(
            format!(" Internal links: {}", cluster.internal_weight),
            Style::default().fg(ACCENT_LAVENDER),
        )),
        Line::from(Span::styled(
            format!(" Incoming links: {}", cluster.inbound_weight),
            Style::default().fg(Color::Rgb(148, 226, 213)),
        )),
        Line::from(Span::styled(
            format!(" Outgoing links: {}", cluster.outbound_weight),
            Style::default().fg(Color::Rgb(250, 179, 135)),
        )),
        Line::from(""),
        Line::from(Span::styled(
            cluster_summary_note(cluster),
            Style::default().fg(FG_OVERLAY),
        )),
    ];
    render_padded_section_paragraph(
        frame,
        summary_col,
        panel_title("Cluster details"),
        summary_lines,
    );

    let mut member_lines = ranked_members
        .iter()
        .take(member_limit)
        .map(|(label, total, incoming, outgoing)| {
            if cluster.is_dependency_sink() {
                let detail = if *incoming > 0 && *incoming != *total {
                    format!("  in:{}", incoming)
                } else {
                    String::new()
                };
                Line::from(vec![
                    Span::styled(
                        format!("{:>3} ", total),
                        Style::default()
                            .fg(ACCENT_MAUVE)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        super::widgets::truncate_str(label, 20),
                        Style::default().fg(FG_TEXT),
                    ),
                    Span::styled(detail, Style::default().fg(FG_OVERLAY)),
                ])
            } else {
                Line::from(vec![
                    Span::styled(
                        format!("{:>3} ", total),
                        Style::default()
                            .fg(ACCENT_MAUVE)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        super::widgets::truncate_str(label, 18),
                        Style::default().fg(FG_TEXT),
                    ),
                    Span::styled(
                        format!("  in:{} out:{}", incoming, outgoing),
                        Style::default().fg(FG_OVERLAY),
                    ),
                ])
            }
        })
        .collect::<Vec<_>>();
    member_lines.insert(0, Line::from(""));
    render_padded_section_paragraph(
        frame,
        members_col,
        panel_title(if cluster.is_dependency_sink() {
            "Top dependencies"
        } else {
            "Key members"
        }),
        member_lines,
    );

    let mut bridge_lines = Vec::new();
    if inbound.is_empty() {
        bridge_lines.push(Line::from(Span::styled(
            if cluster.is_dependency_sink() {
                "  No importing clusters"
            } else {
                "  No incoming links"
            },
            Style::default().fg(FG_OVERLAY),
        )));
    } else {
        bridge_lines.extend(
            inbound
                .iter()
                .take(bridge_limit)
                .map(|(name, weight, count)| {
                    Line::from(format!(
                        "  {}  {} weight, {} links",
                        super::widgets::truncate_str(name, 18),
                        weight,
                        count
                    ))
                }),
        );
    }
    if !outbound.is_empty() {
        bridge_lines.push(Line::from(""));
        bridge_lines.push(Line::from(Span::styled(
            "Outgoing",
            Style::default()
                .fg(Color::Rgb(250, 179, 135))
                .add_modifier(Modifier::BOLD),
        )));
        bridge_lines.extend(
            outbound
                .iter()
                .take(bridge_limit)
                .map(|(name, weight, count)| {
                    Line::from(format!(
                        "  {}  {} weight, {} links",
                        super::widgets::truncate_str(name, 18),
                        weight,
                        count
                    ))
                }),
        );
    }
    bridge_lines.insert(0, Line::from(""));
    render_padded_section_paragraph(
        frame,
        bridges_col,
        panel_title(if cluster.is_dependency_sink() {
            "Importing clusters"
        } else {
            "Cluster links"
        }),
        bridge_lines,
    );
}

fn render_relation_group_block(
    frame: &mut Frame,
    area: Rect,
    title: &str,
    rows: &[(String, (u32, usize))],
    accent: Color,
    empty_message: &str,
) {
    let block = Block::default().title(subsection_title(title));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.width == 0 || inner.height <= 1 {
        return;
    }

    let content = Rect {
        x: inner.x.saturating_add(1),
        y: inner.y.saturating_add(1),
        width: inner.width.saturating_sub(1),
        height: inner.height.saturating_sub(1),
    };
    if content.width == 0 || content.height == 0 {
        return;
    }

    if rows.is_empty() {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                format!("  {}", empty_message),
                Style::default().fg(FG_OVERLAY),
            ))),
            content,
        );
        return;
    }

    let rows_per_column = content.height.max(1) as usize;
    let max_columns = if content.width >= 42 { 2 } else { 1 };
    let total_capacity = rows_per_column.saturating_mul(max_columns);
    let needs_more = rows.len() > total_capacity;
    let visible_capacity = if needs_more {
        total_capacity.saturating_sub(1).max(1)
    } else {
        total_capacity
    };
    let visible_rows = rows.iter().take(visible_capacity).collect::<Vec<_>>();
    let hidden_count = rows.len().saturating_sub(visible_rows.len());
    let column_count = visible_rows.len().div_ceil(rows_per_column).max(1);
    let column_width = (content.width / column_count as u16).max(1);
    let buf = frame.buffer_mut();

    for (idx, (label, (weight, count))) in visible_rows.iter().enumerate() {
        let column = idx / rows_per_column;
        let row = idx % rows_per_column;
        let x = content.x + (column as u16 * column_width);
        let y = content.y + row as u16;
        if y >= content.y + content.height || x >= content.x + content.width {
            continue;
        }

        let available = content
            .width
            .saturating_sub((column as u16 * column_width) + 1)
            .min(column_width)
            .max(8) as usize;
        let prefix = format!("{:>3} ", weight);
        let suffix = if *count > 1 {
            format!("  x{}", count)
        } else {
            String::new()
        };
        let label_width = available.saturating_sub(prefix.len() + suffix.len());
        let clipped = super::widgets::truncate_str(label, label_width.max(4));
        let line = format!("{}{}{}", prefix, clipped, suffix);
        let display = super::widgets::truncate_str(&line, available);
        buf.set_string(
            x,
            y,
            display,
            Style::default().fg(if column == 0 { accent } else { FG_TEXT }),
        );
    }

    if hidden_count > 0 {
        let idx = visible_rows.len();
        let column = idx / rows_per_column;
        let row = idx % rows_per_column;
        let x = content.x + (column as u16 * column_width);
        let y = content.y + row as u16;
        if y < content.y + content.height && x < content.x + content.width {
            let message = format!("+{} more", hidden_count);
            let available = content
                .width
                .saturating_sub((column as u16 * column_width) + 1)
                .min(column_width)
                .max(8) as usize;
            buf.set_string(
                x,
                y,
                super::widgets::truncate_str(&message, available),
                Style::default().fg(FG_OVERLAY),
            );
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RelationLensData {
    member_id: usize,
    member_label: String,
    inbound_rows: Vec<(String, (u32, usize))>,
    outbound_rows: Vec<(String, (u32, usize))>,
    inbound_total: u32,
    outbound_total: u32,
    internal_relations: usize,
    role_label: &'static str,
}

fn selected_role_label(app: &App, node_id: usize) -> &'static str {
    if app.internal_node_indices.contains(&node_id) {
        "internal member"
    } else {
        "external dependency"
    }
}

fn build_selected_relation_lens(
    app: &App,
    map: &ArchitectureMap,
    cluster: &ClusterNode,
    member_id: usize,
) -> RelationLensData {
    let cluster_set = cluster.members.iter().copied().collect::<HashSet<_>>();
    let detailed_external_members = cluster.is_dependency_sink();
    let mut inbound = HashMap::<String, (u32, usize)>::new();
    let mut outbound = HashMap::<String, (u32, usize)>::new();

    for (edge_idx, &(from, to)) in app.graph_layout.edges.iter().enumerate() {
        let weight = app
            .graph_layout
            .edge_weights
            .get(edge_idx)
            .copied()
            .unwrap_or(1);
        if to == member_id {
            let source = if cluster_set.contains(&from) || detailed_external_members {
                app.graph_layout.labels[from].clone()
            } else {
                map.clusters[map.cluster_of_node[from]].name.clone()
            };
            let entry = inbound.entry(source).or_insert((0, 0));
            entry.0 += weight;
            entry.1 += 1;
        }
        if from == member_id {
            let target = if cluster_set.contains(&to) || detailed_external_members {
                app.graph_layout.labels[to].clone()
            } else {
                map.clusters[map.cluster_of_node[to]].name.clone()
            };
            let entry = outbound.entry(target).or_insert((0, 0));
            entry.0 += weight;
            entry.1 += 1;
        }
    }

    let mut inbound_rows = inbound.into_iter().collect::<Vec<_>>();
    inbound_rows.sort_by(|a, b| b.1.0.cmp(&a.1.0).then_with(|| a.0.cmp(&b.0)));
    let mut outbound_rows = outbound.into_iter().collect::<Vec<_>>();
    outbound_rows.sort_by(|a, b| b.1.0.cmp(&a.1.0).then_with(|| a.0.cmp(&b.0)));
    let inbound_total = inbound_rows.iter().map(|(_, (weight, _))| *weight).sum();
    let outbound_total = outbound_rows.iter().map(|(_, (weight, _))| *weight).sum();
    let internal_relations = app
        .graph_layout
        .edges
        .iter()
        .filter(|&&(from, to)| {
            (from == member_id || to == member_id)
                && cluster_set.contains(&from)
                && cluster_set.contains(&to)
        })
        .count();

    RelationLensData {
        member_id,
        member_label: app.graph_layout.labels[member_id].clone(),
        inbound_rows,
        outbound_rows,
        inbound_total,
        outbound_total,
        internal_relations,
        role_label: selected_role_label(app, member_id),
    }
}

fn render_cluster_focus_preview(frame: &mut Frame, area: Rect, app: &App, cluster_id: usize) {
    if area.width < 24 || area.height < 8 {
        return;
    }

    let Some(map) = app.architecture_map.as_ref() else {
        return;
    };
    let Some(cluster) = map.clusters.get(cluster_id) else {
        return;
    };
    let Some(focus_id) = app
        .selected_sidebar_member()
        .or_else(|| cluster.members.first().copied())
    else {
        return;
    };
    let lens = build_selected_relation_lens(app, map, cluster, focus_id);

    let is_focused = app.focused_panel == FocusedPanel::Graph;
    let border_color = if is_focused {
        BORDER_FOCUSED
    } else {
        BORDER_UNFOCUSED
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color))
        .style(Style::default().bg(BG_SURFACE));
    let block = block.title(panel_title(format!(
        "{} [{} inbound{}]",
        if cluster.is_dependency_sink() {
            "Dependency Lens"
        } else {
            "Member Lens"
        },
        lens.inbound_rows.len(),
        if lens.outbound_rows.is_empty() {
            String::new()
        } else {
            format!(", {} outbound", lens.outbound_rows.len())
        }
    )));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.width < 20 || inner.height < 6 {
        return;
    }

    let is_dependency_sink = cluster.is_dependency_sink();
    let has_outbound = !lens.outbound_rows.is_empty();
    let section_constraints = if has_outbound {
        vec![
            Constraint::Percentage(58),
            Constraint::Length(1),
            Constraint::Percentage(18),
            Constraint::Length(1),
            Constraint::Percentage(24),
        ]
    } else {
        vec![
            Constraint::Percentage(58),
            Constraint::Length(1),
            Constraint::Percentage(42),
        ]
    };
    let sections = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(section_constraints)
        .split(inner);
    if has_outbound {
        render_vertical_separator(frame, sections[1], 1);
        render_vertical_separator(frame, sections[3], 1);
    } else {
        render_vertical_separator(frame, sections[1], 1);
    }
    let inbound_col = sections[0];
    let center_col = sections[2];
    let outbound_col = if has_outbound {
        Some(sections[4])
    } else {
        None
    };
    let inbound_title = if is_dependency_sink {
        format!("Direct importers [{}]", lens.inbound_rows.len())
    } else {
        format!("Incoming links [{}]", lens.inbound_rows.len())
    };

    render_relation_group_block(
        frame,
        inbound_col,
        &inbound_title,
        &lens.inbound_rows,
        Color::Rgb(148, 226, 213),
        "No incoming links",
    );
    if let Some(outbound_col) = outbound_col {
        let outbound_title = format!("Outgoing links [{}]", lens.outbound_rows.len());
        render_relation_group_block(
            frame,
            outbound_col,
            &outbound_title,
            &lens.outbound_rows,
            Color::Rgb(250, 179, 135),
            "No outgoing links",
        );
    }

    let center_lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            super::widgets::truncate_str(
                &lens.member_label,
                center_col.width.saturating_sub(4) as usize,
            ),
            Style::default()
                .fg(Color::Rgb(255, 232, 115))
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(Span::styled(
            format!("Cluster: {}", cluster.name),
            Style::default().fg(FG_OVERLAY),
        )),
        Line::from(Span::styled(
            format!("Role: {}", lens.role_label),
            Style::default().fg(FG_OVERLAY),
        )),
        Line::from(""),
        Line::from(""),
        Line::from(Span::styled(
            format!("Incoming weight: {}", lens.inbound_total),
            Style::default().fg(Color::Rgb(148, 226, 213)),
        )),
        if has_outbound {
            Line::from(Span::styled(
                format!("Outgoing weight: {}", lens.outbound_total),
                Style::default().fg(Color::Rgb(250, 179, 135)),
            ))
        } else {
            Line::from("")
        },
        Line::from(Span::styled(
            if is_dependency_sink {
                format!("One-hop importers: {}", lens.inbound_rows.len())
            } else {
                format!("Internal relations: {}", lens.internal_relations)
            },
            Style::default().fg(if is_dependency_sink {
                ACCENT_BLUE
            } else {
                ACCENT_LAVENDER
            }),
        )),
    ];
    render_padded_section_paragraph(
        frame,
        center_col,
        subsection_title(if is_dependency_sink {
            "Dependency details"
        } else {
            "Member details"
        }),
        center_lines,
    );
}

fn render_padded_section_paragraph(
    frame: &mut Frame,
    area: Rect,
    title: Span<'static>,
    lines: Vec<Line<'static>>,
) {
    let block = Block::default().title(title);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let content = Rect {
        x: inner.x.saturating_add(1),
        y: inner.y,
        width: inner.width.saturating_sub(1),
        height: inner.height,
    };
    if content.width == 0 || content.height == 0 {
        return;
    }

    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), content);
}

fn render_vertical_separator(frame: &mut Frame, area: Rect, inset: u16) {
    if area.width == 0 || area.height == 0 {
        return;
    }

    let buf = frame.buffer_mut();
    let start_y = area.y.saturating_add(inset);
    let end_y = area.y + area.height.saturating_sub(inset);
    for y in start_y..end_y {
        buf[(area.x, y)]
            .set_symbol("|")
            .set_style(Style::default().fg(Color::Rgb(88, 91, 124)));
    }
}

/// Renders the breadcrumb header bar
fn render_breadcrumb(frame: &mut Frame, area: Rect, app: &App) {
    let mut spans = vec![Span::styled(
        " morpharch",
        Style::default()
            .fg(ACCENT_MAUVE)
            .add_modifier(Modifier::BOLD),
    )];

    if !app.repo_name.is_empty() {
        spans.push(Span::styled(" ❯ ", Style::default().fg(FG_OVERLAY)));
        spans.push(Span::styled(
            app.repo_name.as_str(),
            Style::default().fg(FG_TEXT),
        ));
    }

    // Current commit info
    if let Some(hash) = app.timeline.current_commit_hash() {
        let short = if hash.len() >= 7 { &hash[..7] } else { hash };
        spans.push(Span::styled(" ❯ ", Style::default().fg(FG_OVERLAY)));
        spans.push(Span::styled(short, Style::default().fg(ACCENT_BLUE)));
        if let Some(msg) = app.timeline.current_commit_message()
            && !msg.is_empty()
        {
            let truncated = super::widgets::truncate_str(msg, 40);
            spans.push(Span::styled(
                format!(" \"{}\"", truncated),
                Style::default().fg(FG_SUBTEXT),
            ));
        }
    }

    // Navigation context from stack
    for ctx in app.nav_stack.iter().skip(1) {
        spans.push(Span::styled(" ❯ ", Style::default().fg(FG_OVERLAY)));
        match ctx {
            ViewContext::Overview => {}
            ViewContext::PackageDetail(name) => {
                spans.push(Span::styled(
                    name.as_str(),
                    Style::default()
                        .fg(ACCENT_LAVENDER)
                        .add_modifier(Modifier::BOLD),
                ));
            }
            ViewContext::ModuleInspect(name) => {
                spans.push(Span::styled(
                    name.as_str(),
                    Style::default()
                        .fg(COLOR_WARNING)
                        .add_modifier(Modifier::BOLD),
                ));
            }
        }
    }

    // Filter indicator
    if !app.filter_text.is_empty() && !app.filter_active {
        spans.push(Span::styled("  ", Style::default()));
        spans.push(Span::styled(
            format!("[Filter: \"{}\"]", app.filter_text),
            Style::default()
                .fg(Color::Rgb(148, 226, 213))
                .add_modifier(Modifier::BOLD),
        ));
    }

    frame.render_widget(
        Paragraph::new(Line::from(spans)).style(Style::default().bg(BG_SURFACE)),
        area,
    );
}

/// Renders the package list sidebar with selection support
fn render_package_list_v2(frame: &mut Frame, area: Rect, app: &App) {
    let is_focused = app.focused_panel == FocusedPanel::Packages;
    let border_color = if is_focused {
        BORDER_FOCUSED
    } else {
        BORDER_UNFOCUSED
    };

    let entries = app.sidebar_entries();
    let shown = entries.len();
    let title_str = app.sidebar_title(shown);

    let block = Block::default()
        .title(Span::styled(
            title_str,
            if is_focused {
                Style::default()
                    .fg(BORDER_FOCUSED)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(FG_OVERLAY)
            },
        ))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color))
        .style(Style::default().bg(BG_SURFACE));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if entries.is_empty() {
        let empty = Paragraph::new("  (empty)").style(Style::default().fg(FG_OVERLAY));
        frame.render_widget(empty, inner);
        return;
    }

    let max_visible = inner.height as usize;
    let list_height = app.sidebar_visible_capacity(entries.len()).min(max_visible);
    let max_offset = entries.len().saturating_sub(list_height.max(1));
    let effective_offset = app.pkg_scroll_offset.min(max_offset);

    let filter_lower = app.filter_text.to_lowercase();
    let mut lines: Vec<Line> = Vec::new();

    for (rel_i, entry) in entries.iter().enumerate().skip(effective_offset) {
        if lines.len() >= list_height {
            break;
        }

        let label = sidebar_label(entry);
        let short = super::widgets::truncate_str(label, inner.width.saturating_sub(5) as usize);

        let is_selected = app.selected_pkg_index == Some(rel_i);
        let is_filter_match =
            !filter_lower.is_empty() && label.to_lowercase().contains(&filter_lower);

        let (fg, bg) = if is_selected && is_focused {
            (Color::White, BG_SURFACE1)
        } else if is_selected {
            (FG_TEXT, BG_SURFACE1)
        } else if is_filter_match {
            (ACCENT_LAVENDER, BG_SURFACE)
        } else {
            (FG_TEXT, BG_SURFACE)
        };

        let modifier = if is_selected || is_filter_match {
            Modifier::BOLD
        } else {
            Modifier::empty()
        };

        let prefix = if is_selected { " ❯ " } else { "   " };
        let marker = match entry {
            SidebarEntry::Cluster { .. } => "> ",
            SidebarEntry::Member { .. } => "- ",
        };

        lines.push(Line::from(vec![
            Span::styled(
                prefix,
                Style::default().fg(if is_selected {
                    ACCENT_MAUVE
                } else {
                    FG_OVERLAY
                }),
            ),
            Span::styled(marker, Style::default().fg(FG_OVERLAY).bg(bg)),
            Span::styled(short, Style::default().fg(fg).bg(bg).add_modifier(modifier)),
        ]));
    }

    // Scroll indicator
    if entries.len() > list_height {
        let visible_end = (effective_offset + list_height).min(entries.len());
        lines.push(Line::from(Span::styled(
            format!(
                " [{}-{}/{}]",
                effective_offset + 1,
                visible_end,
                entries.len()
            ),
            Style::default().fg(FG_OVERLAY),
        )));
    }

    frame.render_widget(Paragraph::new(lines), inner);
}

/// Renders the tabbed insights panel
fn render_insights_tabbed(frame: &mut Frame, area: Rect, app: &mut App) {
    let is_focused = app.focused_panel == FocusedPanel::Insights;
    let border_color = if is_focused {
        BORDER_FOCUSED
    } else {
        BORDER_UNFOCUSED
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color))
        .style(Style::default().bg(BG_SURFACE));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    // ── Tab bar (1 row) ──
    let tab_area = Rect::new(inner.x, inner.y, inner.width, 1);
    let content_area = Rect::new(
        inner.x,
        inner.y + 1,
        inner.width,
        inner.height.saturating_sub(1),
    );

    let tabs = insight_tab_specs(app);

    let mut tab_spans = Vec::new();
    for (i, (label, tab)) in tabs.iter().enumerate() {
        let is_active = app.insight_tab == *tab;
        if is_active {
            tab_spans.push(Span::styled(
                format!(" {} ", label),
                Style::default()
                    .fg(BG_BASE)
                    .bg(ACCENT_MAUVE)
                    .add_modifier(Modifier::BOLD),
            ));
        } else {
            tab_spans.push(Span::styled(
                format!(" {} ", label),
                Style::default().fg(FG_OVERLAY),
            ));
        }
        // No trailing separator after the last tab
        if i < tabs.len() - 1 {
            tab_spans.push(Span::styled(" ", Style::default()));
        }
    }
    frame.render_widget(Paragraph::new(Line::from(tab_spans)), tab_area);

    // ── Render active tab content ──
    match app.insight_tab {
        InsightTab::Overview => {
            if matches!(app.active_view, ActiveView::Inspect(_)) {
                super::insight_panel::render_module_inspector(frame, content_area, app);
            } else {
                let contextual_lines = app.context_advisory_lines();
                let trend_data =
                    build_trend_data(&app.snapshots_metadata, app.timeline.current_index);
                render_insight_panel(
                    frame,
                    content_area,
                    &app.current_drift,
                    &contextual_lines,
                    &app.advisory_lines,
                    &app.scoring_config.weights,
                    &trend_data,
                    app.timeline.current_index,
                    app.timeline.len(),
                );
            }
        }
        InsightTab::Hotspots => {
            render_hotspots_tab(frame, content_area, app);
        }
        InsightTab::Blast => {
            super::insight_panel::render_blast_radius_panel(
                frame,
                content_area,
                &app.current_blast_radius,
                app.blast_impact_scroll,
            );
        }
    }
}

fn insight_tab_specs(app: &App) -> Vec<(&'static str, InsightTab)> {
    let first_label = if matches!(app.active_view, ActiveView::Inspect(_)) {
        "Module"
    } else {
        "Overview"
    };

    vec![
        (first_label, InsightTab::Overview),
        ("Hotspots", InsightTab::Hotspots),
        ("Blast", InsightTab::Blast),
    ]
}

/// Renders the hotspots tab content
fn render_hotspots_tab(frame: &mut Frame, area: Rect, app: &mut App) {
    if app.brittle_packages.is_empty() {
        let spinner = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
        let spin_frame = spinner[(app.frame_count as usize / 2) % spinner.len()];
        frame.render_widget(
            Paragraph::new(format!("  {} Analyzing...", spin_frame))
                .style(Style::default().fg(ACCENT_LAVENDER)),
            area,
        );
        return;
    }

    use ratatui::widgets::{Cell, Row, Table};

    let sort_col = &app.hotspots_sort;
    let hdr_style = |is_sorted: bool| {
        let base = Style::default().add_modifier(Modifier::UNDERLINED);
        if is_sorted {
            base.fg(ACCENT_LAVENDER).add_modifier(Modifier::BOLD)
        } else {
            base.fg(FG_OVERLAY)
        }
    };
    let header_cells = vec![
        Cell::from(""),
        Cell::from("Module").style(hdr_style(false)),
        Cell::from("In").style(hdr_style(matches!(sort_col, HotspotsSort::FanIn))),
        Cell::from("Out").style(hdr_style(matches!(sort_col, HotspotsSort::FanOut))),
        Cell::from("Inst").style(hdr_style(matches!(sort_col, HotspotsSort::Instability))),
    ];
    let header = Row::new(header_cells).height(1);

    let max_name_len = area.width.saturating_sub(16) as usize;
    let mut rows = Vec::new();
    for (name, instability, fan_in, fan_out) in app.brittle_packages.iter() {
        let instab_color = if *instability > 0.8 {
            COLOR_DANGER
        } else if *instability > 0.5 {
            COLOR_WARNING
        } else {
            COLOR_HEALTHY
        };

        let in_color = if *fan_in > 15 {
            ACCENT_MAUVE
        } else if *fan_in > 5 {
            ACCENT_BLUE
        } else {
            FG_OVERLAY
        };
        let out_color = if *fan_out > 15 {
            ACCENT_MAUVE
        } else if *fan_out > 5 {
            ACCENT_BLUE
        } else {
            FG_OVERLAY
        };

        let marker = if *instability >= 0.95 || (*fan_in + *fan_out > 20) {
            "■"
        } else {
            ""
        };

        let display_name = super::widgets::truncate_str(name, max_name_len);

        rows.push(
            Row::new(vec![
                Cell::from(Span::styled(marker, Style::default().fg(instab_color))),
                Cell::from(Span::styled(display_name, Style::default().fg(FG_TEXT))),
                Cell::from(Span::styled(
                    format!("{:>3}", fan_in),
                    Style::default().fg(in_color),
                )),
                Cell::from(Span::styled(
                    format!("{:>3}", fan_out),
                    Style::default().fg(out_color),
                )),
                Cell::from(Span::styled(
                    format!("{:.2}", instability),
                    Style::default()
                        .fg(instab_color)
                        .add_modifier(Modifier::BOLD),
                )),
            ])
            .height(1),
        );
    }

    let sort_label = match app.hotspots_sort {
        HotspotsSort::Instability => "inst",
        HotspotsSort::FanIn => "in",
        HotspotsSort::FanOut => "out",
    };

    let t = Table::new(
        rows,
        [
            Constraint::Length(2),
            Constraint::Min(8),
            Constraint::Length(4),
            Constraint::Length(4),
            Constraint::Length(5),
        ],
    )
    .header(header)
    .block(
        Block::default().title(Span::styled(
            format!(" HOTSPOTS (repo, s:sort by {}) ", sort_label),
            Style::default()
                .fg(ACCENT_LAVENDER)
                .add_modifier(Modifier::BOLD),
        )),
    )
    .row_highlight_style(
        Style::default()
            .bg(BG_SURFACE1)
            .add_modifier(Modifier::BOLD),
    );

    frame.render_stateful_widget(t, area, &mut app.hotspots_state);
}

/// Build trend data from snapshot metadata
fn build_trend_data(snapshots: &[SnapshotMetadata], current_index: usize) -> Vec<u64> {
    if snapshots.is_empty() {
        return vec![];
    }
    let start = current_index.min(snapshots.len() - 1);
    let end = (start + 49).min(snapshots.len() - 1);
    let slice = &snapshots[start..=end];

    let mut data: Vec<u64> = slice
        .iter()
        .map(|s| s.drift.as_ref().map(|d| d.total as u64).unwrap_or(50))
        .collect();
    if data.len() > 1 {
        data.reverse();
    }
    data
}

/// Renders the filter bar
fn render_filter_bar(frame: &mut Frame, area: Rect, app: &App) {
    let total = if app.should_show_architecture_overview() {
        app.architecture_map
            .as_ref()
            .map(|map| map.clusters.len())
            .unwrap_or(0)
    } else if let Some(cluster_id) = app.selected_cluster {
        app.architecture_map
            .as_ref()
            .and_then(|map| map.clusters.get(cluster_id))
            .map(|cluster| cluster.members.len())
            .unwrap_or(0)
    } else {
        app.graph_layout.labels.len()
    };
    let matched = app.sidebar_entries().len();

    let line = Line::from(vec![
        Span::styled(
            " / ",
            Style::default()
                .fg(ACCENT_MAUVE)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(&app.filter_text),
        Span::styled("█", Style::default().fg(ACCENT_MAUVE)),
        Span::styled(
            format!("  {}/{}", matched, total),
            Style::default().fg(FG_OVERLAY),
        ),
    ]);
    frame.render_widget(
        Paragraph::new(line).style(Style::default().bg(BG_SURFACE)),
        area,
    );
}

/// Renders contextual footer (2 rows: hints + status)
fn render_footer(frame: &mut Frame, area: Rect, app: &App) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Length(1)])
        .split(area);

    let selected_item_noun = app
        .selected_cluster
        .and_then(|cluster_id| {
            app.architecture_map
                .as_ref()
                .and_then(|map| map.clusters.get(cluster_id))
        })
        .map(|cluster| {
            if cluster.is_dependency_sink() {
                "dependency"
            } else {
                "member"
            }
        })
        .unwrap_or("member");

    let hints = match app.focused_panel {
        FocusedPanel::Packages => {
            if app.should_show_architecture_overview() {
                "j/k:Navigate  Enter:Open cluster  <-/->:Timeline  /:Filter  ?:Help  q:Quit"
                    .to_string()
            } else if app.selected_cluster.is_some() {
                format!(
                    "j/k:Navigate  Enter:Inspect {}  <-/->:Timeline  /:Filter  ?:Help  q:Quit",
                    selected_item_noun
                )
            } else {
                "j/k:Navigate  Enter:Inspect  s:Sort  <-/->:Timeline  /:Filter  ?:Help  q:Quit"
                    .to_string()
            }
        }
        FocusedPanel::Graph => {
            if app.selected_cluster.is_some() && !matches!(app.active_view, ActiveView::Inspect(_))
            {
                format!(
                    "j/k:Navigate  Enter:Inspect selected {}  Esc:Back  <-/->:Timeline  /:Filter",
                    selected_item_noun
                )
            } else if app.should_show_architecture_overview() {
                "Enter:Open cluster  c:Reset  x:Blast  <-/->:Timeline  /:Filter  ?:Help  q:Quit"
                    .to_string()
            } else {
                "r:Reheat  c:Center  x:Blast  Enter:Inspect  <-/->:Timeline  /:Filter  ?:Help  q:Quit"
                    .to_string()
            }
        }
        FocusedPanel::Insights => {
            "j/k:Navigate  h/l:Tab  <-/->:Tab  Enter:Inspect  s:Sort  ?:Help  q:Quit".to_string()
        }
        FocusedPanel::Timeline => {
            "j/k:+/-1  h/l:+/-10  g/G:Start/End  Space:Play  +/-:Speed  ?:Help".to_string()
        }
    };
    frame.render_widget(
        Paragraph::new(Span::styled(
            format!(" {}", hints),
            Style::default().fg(FG_OVERLAY),
        ))
        .style(Style::default().bg(BG_BASE)),
        rows[0],
    );

    let view_label = match app.current_view() {
        ViewContext::Overview if app.should_show_architecture_overview() => "MAP".to_string(),
        ViewContext::Overview => "OVERVIEW".to_string(),
        ViewContext::PackageDetail(n) if app.selected_cluster.is_some() => {
            format!("CLUSTER: {}", n)
        }
        ViewContext::PackageDetail(n) => format!("PKG: {}", n),
        ViewContext::ModuleInspect(n) => format!("INSPECT: {}", n),
    };

    let health = app
        .current_drift
        .as_ref()
        .map(|d| 100u8.saturating_sub(d.total))
        .unwrap_or(0);
    let health_color = if health >= 70 {
        COLOR_HEALTHY
    } else if health >= 40 {
        COLOR_WARNING
    } else {
        COLOR_DANGER
    };

    let panel_name = match app.focused_panel {
        FocusedPanel::Packages => "PKG",
        FocusedPanel::Graph => "GRAPH",
        FocusedPanel::Insights => "INSIGHTS",
        FocusedPanel::Timeline => "TIMELINE",
    };
    let graph_summary = app
        .architecture_map
        .as_ref()
        .map(|map| {
            format!(
                "{} modules / {} clusters",
                app.graph_layout.labels.len(),
                map.clusters.len()
            )
        })
        .unwrap_or_else(|| format!("{} nodes", app.graph_layout.labels.len()));

    let mut status = Line::from(vec![
        Span::styled(
            format!(" [{}]", view_label),
            Style::default()
                .fg(ACCENT_MAUVE)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" | ", Style::default().fg(BORDER_UNFOCUSED)),
        Span::styled(
            format!("{}/{}", app.timeline.current_index + 1, app.timeline.len()),
            Style::default().fg(ACCENT_LAVENDER),
        ),
        Span::styled(" | ", Style::default().fg(BORDER_UNFOCUSED)),
        Span::styled(
            if app.is_playing { "PLAY" } else { "PAUSE" },
            Style::default().fg(if app.is_playing {
                COLOR_HEALTHY
            } else {
                FG_OVERLAY
            }),
        ),
        Span::styled(
            format!(
                " {:.1}s",
                app.auto_play_interval.as_millis() as f64 / 1000.0
            ),
            Style::default().fg(FG_OVERLAY),
        ),
        Span::styled(" | ", Style::default().fg(BORDER_UNFOCUSED)),
        Span::styled(
            format!("Score: {}%", health),
            Style::default()
                .fg(health_color)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" | ", Style::default().fg(BORDER_UNFOCUSED)),
        Span::styled(graph_summary, Style::default().fg(FG_OVERLAY)),
        Span::styled(" | ", Style::default().fg(BORDER_UNFOCUSED)),
        Span::styled(
            format!("[{}]", panel_name),
            Style::default().fg(ACCENT_BLUE),
        ),
        Span::styled(" | ", Style::default().fg(BORDER_UNFOCUSED)),
        Span::styled(
            "Tab:Panel  b:Sidebar  i:Detail",
            Style::default().fg(FG_OVERLAY),
        ),
    ]);

    if app.skipped_snapshot_count > 0 {
        status
            .spans
            .push(Span::styled(" | ", Style::default().fg(BORDER_UNFOCUSED)));
        status.spans.push(Span::styled(
            format!("{} snapshots unavailable", app.skipped_snapshot_count),
            Style::default().fg(COLOR_DANGER),
        ));
    }

    if app.blast_overlay_active {
        status.spans.push(Span::styled(
            " [BLAST]",
            Style::default()
                .fg(Color::Rgb(30, 30, 46))
                .bg(Color::Rgb(243, 139, 168))
                .add_modifier(Modifier::BOLD),
        ));
    }

    frame.render_widget(
        Paragraph::new(status).style(Style::default().bg(BG_BASE)),
        rows[1],
    );
}
/// Renders the help overlay
fn render_help_overlay(frame: &mut Frame, area: Rect) {
    frame.render_widget(Clear, area);
    let overlay = Block::default().style(Style::default().bg(Color::Rgb(20, 20, 35)));
    frame.render_widget(overlay, area);

    let w = 70.min(area.width.saturating_sub(4));
    let h = 34.min(area.height.saturating_sub(4));
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    let help_area = Rect::new(x, y, w, h);

    let block = Block::default()
        .title(Span::styled(
            " KEYBINDINGS ",
            Style::default()
                .fg(ACCENT_MAUVE)
                .add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(ACCENT_LAVENDER))
        .style(Style::default().bg(BG_SURFACE));

    let inner = block.inner(help_area);
    frame.render_widget(block, help_area);

    let help_text = vec![
        Line::from(""),
        Line::from(Span::styled(
            " GLOBAL",
            Style::default()
                .fg(ACCENT_LAVENDER)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from("  Tab / Shift+Tab   Next / previous panel"),
        Line::from("  1-4               Jump to Packages / Graph / Insights / Timeline"),
        Line::from("  /                 Focus filter input"),
        Line::from("  ?                 Open this help"),
        Line::from("  Esc               Back / clear / quit"),
        Line::from("  q / Ctrl+C        Quit"),
        Line::from("  Space / p         Play / pause timeline"),
        Line::from("  b / i             Toggle sidebar / insights"),
        Line::from(""),
        Line::from(Span::styled(
            " PACKAGES",
            Style::default()
                .fg(ACCENT_LAVENDER)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from("  j/k or Up/Down    Move selection"),
        Line::from("  g / G             First / last item"),
        Line::from("  Enter             Open cluster or inspect member"),
        Line::from(""),
        Line::from(Span::styled(
            " GRAPH",
            Style::default()
                .fg(ACCENT_LAVENDER)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from("  Overview          j/k move cluster, Enter opens cluster"),
        Line::from("  Cluster details   j/k moves selection, Enter opens inspect"),
        Line::from("  Cluster details   Esc returns to the architecture map"),
        Line::from("  Inspect           Enter opens raw node inspect"),
        Line::from("  c                 Reset / center current graph view"),
        Line::from("  r                 Reheat raw force layout"),
        Line::from("  x                 Toggle blast overlay"),
        Line::from(""),
        Line::from(Span::styled(
            " INSIGHTS",
            Style::default()
                .fg(ACCENT_LAVENDER)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from("  j/k or Up/Down    Move within active insight tab"),
        Line::from("  h/l or Left/Right Switch insight tab"),
        Line::from("  s                 Sort hotspots"),
        Line::from("  Enter             Inspect selected hotspot / blast item"),
        Line::from(""),
        Line::from(Span::styled(
            " TIMELINE",
            Style::default()
                .fg(ACCENT_LAVENDER)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from("  Left/Right        Previous / next commit from most panels"),
        Line::from("  j/k               Previous / next commit when timeline focused"),
        Line::from("  h/l               Jump -10 / +10 commits"),
        Line::from("  g / G             First / last commit"),
        Line::from("  + / -             Change autoplay speed"),
        Line::from(""),
        Line::from(Span::styled(
            " GRAPH COLORS",
            Style::default()
                .fg(ACCENT_LAVENDER)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from("  Default graph     Edge color = import weight"),
        edge_weight_legend_line(),
        Line::from("  Inspect state     ◆ focus   ◉ hover"),
        Line::from(vec![
            Span::raw("  Inspect mode      "),
            Span::styled(
                "●",
                Style::default().fg(graph_relation_color(GraphRelationSemantic::Inbound)),
            ),
            Span::styled(" incoming  ", Style::default().fg(FG_OVERLAY)),
            Span::styled(
                "●",
                Style::default().fg(graph_relation_color(GraphRelationSemantic::Outbound)),
            ),
            Span::styled(" outgoing  ", Style::default().fg(FG_OVERLAY)),
            Span::styled(
                "●",
                Style::default().fg(graph_relation_color(GraphRelationSemantic::Bidirectional)),
            ),
            Span::styled(" both ways  ", Style::default().fg(FG_OVERLAY)),
            Span::styled(
                "●",
                Style::default().fg(graph_relation_color(GraphRelationSemantic::Focus)),
            ),
            Span::styled(" focus", Style::default().fg(FG_OVERLAY)),
        ]),
        Line::from(vec![
            Span::raw("  Search mode       "),
            Span::styled(
                "◎",
                Style::default().fg(graph_relation_color(GraphRelationSemantic::SearchMatch)),
            ),
            Span::styled(" match  ", Style::default().fg(FG_OVERLAY)),
            Span::styled(
                "●",
                Style::default().fg(graph_relation_color(GraphRelationSemantic::Related)),
            ),
            Span::styled(" related  ", Style::default().fg(FG_OVERLAY)),
            Span::styled(
                "◉",
                Style::default().fg(graph_relation_color(GraphRelationSemantic::Hover)),
            ),
            Span::styled(" hover", Style::default().fg(FG_OVERLAY)),
        ]),
        Line::from(vec![
            Span::raw("  Blast mode        "),
            Span::styled(
                "●",
                Style::default().fg(crate::tui::graph_renderer::blast_color(0.08)),
            ),
            Span::styled(" low  ", Style::default().fg(FG_OVERLAY)),
            Span::styled(
                "●",
                Style::default().fg(crate::tui::graph_renderer::blast_color(0.72)),
            ),
            Span::styled(" high  ", Style::default().fg(FG_OVERLAY)),
            Span::styled(
                "●",
                Style::default().fg(graph_relation_color(GraphRelationSemantic::CascadeSource)),
            ),
            Span::styled(" source", Style::default().fg(FG_OVERLAY)),
        ]),
        Line::from("  Default nodes use family hues in semantic mode, neutral in minimal mode."),
        Line::from(""),
        Line::from(Span::styled(
            " MAP COLORS",
            Style::default()
                .fg(ACCENT_LAVENDER)
                .add_modifier(Modifier::BOLD),
        )),
        overview_legend_line_for_help(),
        Line::from(""),
        Line::from(Span::styled(
            "        Press ? or Esc to close",
            Style::default().fg(FG_OVERLAY),
        )),
    ];

    frame.render_widget(Paragraph::new(help_text).wrap(Wrap { trim: true }), inner);
}

pub async fn run_tui(mut app: App) -> anyhow::Result<()> {
    use crossterm::ExecutableCommand;
    use crossterm::terminal::{
        EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
    };
    use ratatui::backend::CrosstermBackend;
    use std::io;

    struct TerminalCleanupGuard;

    impl Drop for TerminalCleanupGuard {
        fn drop(&mut self) {
            let _ = disable_raw_mode();
            let mut stdout = io::stdout();
            let _ = stdout.execute(crossterm::event::DisableMouseCapture);
            let _ = stdout.execute(LeaveAlternateScreen);
        }
    }

    enable_raw_mode()?;
    std::io::stdout().execute(EnterAlternateScreen)?;
    std::io::stdout().execute(crossterm::event::EnableMouseCapture)?;
    let _cleanup = TerminalCleanupGuard;
    let mut terminal = ratatui::Terminal::new(CrosstermBackend::new(std::io::stdout()))?;
    terminal.clear()?;

    loop {
        if let Some(hash) = app.loading_hash.take()
            && let Some(db) = &app.db
            && let Ok(Some(snapshot)) = db.get_graph_snapshot(&app.repo_id, &hash)
        {
            app.snapshot_cache.put(hash.clone(), snapshot.clone());
            app.apply_snapshot(&snapshot);
        }

        terminal.draw(|f| {
            render_app(f, &mut app);
        })?;

        if event::poll(Duration::from_millis(5))? {
            match event::read()? {
                Event::Key(k) if k.kind == event::KeyEventKind::Press => {
                    app.handle_key(k.code, k.modifiers)
                }
                Event::Mouse(m) => {
                    app.handle_mouse(m);
                }
                _ => {}
            }
        }

        app.tick_physics();
        app.tick_auto_play();

        if app.should_quit {
            break;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[allow(clippy::too_many_arguments)]
    fn make_cluster(
        id: usize,
        name: &str,
        kind: super::super::architecture_map::ClusterKind,
        internal_member_count: usize,
        external_member_count: usize,
        inbound_weight: u32,
        outbound_weight: u32,
        internal_weight: u32,
    ) -> ClusterNode {
        ClusterNode {
            id,
            name: name.to_string(),
            kind,
            members: (0..internal_member_count + external_member_count).collect(),
            internal_member_count,
            external_member_count,
            anchor_label: format!("{name}/anchor"),
            layer: 0,
            x_ratio: 0.5,
            y_ratio: 0.5,
            inbound_weight,
            outbound_weight,
            internal_weight,
        }
    }

    fn make_map(clusters: Vec<ClusterNode>, edges: Vec<ClusterEdge>) -> ArchitectureMap {
        ArchitectureMap {
            cluster_of_node: Vec::new(),
            max_layer: 1,
            should_default_to_overview: true,
            clusters,
            edges,
        }
    }

    #[test]
    fn overview_prefers_internal_cluster_over_dependency_sink() {
        let map = make_map(
            vec![
                make_cluster(
                    0,
                    "deps",
                    super::super::architecture_map::ClusterKind::Deps,
                    0,
                    80,
                    6_000,
                    0,
                    0,
                ),
                make_cluster(
                    1,
                    "ext",
                    super::super::architecture_map::ClusterKind::Group,
                    30,
                    0,
                    1_000,
                    900,
                    120,
                ),
            ],
            vec![ClusterEdge {
                from: 1,
                to: 0,
                total_weight: 1_000,
                edge_count: 80,
            }],
        );

        let cluster = preferred_overview_cluster(&map).expect("overview cluster");
        assert_eq!(cluster.name, "ext");
    }

    #[test]
    fn overview_prefers_internal_bridge_over_dependency_sink_bridge() {
        let map = make_map(
            vec![
                make_cluster(
                    0,
                    "core",
                    super::super::architecture_map::ClusterKind::Group,
                    20,
                    0,
                    300,
                    280,
                    200,
                ),
                make_cluster(
                    1,
                    "runtime",
                    super::super::architecture_map::ClusterKind::Infra,
                    16,
                    0,
                    220,
                    240,
                    180,
                ),
                make_cluster(
                    2,
                    "deps",
                    super::super::architecture_map::ClusterKind::Deps,
                    0,
                    60,
                    2_500,
                    0,
                    0,
                ),
            ],
            vec![
                ClusterEdge {
                    from: 0,
                    to: 2,
                    total_weight: 2_500,
                    edge_count: 160,
                },
                ClusterEdge {
                    from: 0,
                    to: 1,
                    total_weight: 320,
                    edge_count: 24,
                },
            ],
        );

        let bridge = preferred_overview_bridge(&map).expect("overview bridge");
        assert_eq!((bridge.from, bridge.to), (0, 1));
    }

    #[test]
    fn overview_hit_radius_stays_tight_for_small_clusters() {
        let cluster = make_cluster(
            0,
            "cli",
            super::super::architecture_map::ClusterKind::Entry,
            4,
            0,
            24,
            18,
            12,
        );
        let radius = overview_cluster_hit_radius(&cluster);
        assert!(
            radius <= 10.0,
            "small clusters should not get oversized hover areas"
        );
    }

    #[test]
    fn overview_terminal_hit_detects_label_area() {
        let map = make_map(
            vec![make_cluster(
                0,
                "cli",
                super::super::architecture_map::ClusterKind::Entry,
                13,
                0,
                80,
                120,
                32,
            )],
            vec![],
        );
        let mut app = App::new(None, "repo".to_string(), vec![], None);
        app.architecture_map = Some(map);
        app.graph_scale = 1.0;
        app.graph_pan_x = 0.0;
        app.graph_pan_y = 0.0;
        let map_area = Rect {
            x: 0,
            y: 0,
            width: 80,
            height: 24,
        };
        let width = (map_area.width.saturating_sub(2) as f64) * 2.0;
        let height = (map_area.height.saturating_sub(2) as f64) * 4.0;
        let positions = architecture_positions(&app, width, height);
        let (px, py) = positions[0];
        let anchor_col = map_area.x + 1 + (px / 2.0) as u16;
        let anchor_row = map_area.y + 1 + ((height - py) / 4.0) as u16;
        let label_x = anchor_col + 2;
        let hit =
            overview_hit_test_terminal(&app, label_x + 1, anchor_row, map_area, width, height);
        assert_eq!(
            hit,
            Some(0),
            "hover should stay active over the rendered label"
        );
    }

    #[test]
    fn overview_map_rect_excludes_summary_header() {
        let area = Rect {
            x: 4,
            y: 2,
            width: 100,
            height: 30,
        };
        let map_rect = overview_map_rect(area);
        assert_eq!(map_rect.y, area.y + 6);
        assert_eq!(map_rect.height, area.height.saturating_sub(6));
    }

    #[test]
    fn default_graph_node_color_respects_color_mode() {
        let mut app = App::new(None, "repo".to_string(), vec![], None);
        assert_eq!(
            default_graph_node_color(&app, 0),
            graph_relation_color(GraphRelationSemantic::Neutral)
        );

        app.clustering_config.presentation = Some(crate::config::ClusteringPresentationConfig {
            color_mode: crate::config::ClusterColorMode::Semantic,
            ..Default::default()
        });

        assert_eq!(
            default_graph_node_color(&app, 0),
            crate::tui::graph_renderer::palette_node_color(0)
        );
    }

    #[test]
    fn build_trend_data_uses_scan_order_not_timestamps() {
        let snapshots = vec![
            SnapshotMetadata {
                commit_hash: "latest".to_string(),
                scan_order: 4,
                timestamp: 10,
                drift: Some(DriftScore {
                    total: 40,
                    fan_in_delta: 0,
                    fan_out_delta: 0,
                    new_cycles: 0,
                    boundary_violations: 0,
                    layering_violations: 0,
                    cognitive_complexity: 0.0,
                    timestamp: 10,
                    cycle_debt: 0.0,
                    layering_debt: 0.0,
                    hub_debt: 0.0,
                    coupling_debt: 0.0,
                    cognitive_debt: 0.0,
                    instability_debt: 0.0,
                }),
            },
            SnapshotMetadata {
                commit_hash: "mid".to_string(),
                scan_order: 3,
                timestamp: 20,
                drift: Some(DriftScore {
                    total: 30,
                    fan_in_delta: 0,
                    fan_out_delta: 0,
                    new_cycles: 0,
                    boundary_violations: 0,
                    layering_violations: 0,
                    cognitive_complexity: 0.0,
                    timestamp: 20,
                    cycle_debt: 0.0,
                    layering_debt: 0.0,
                    hub_debt: 0.0,
                    coupling_debt: 0.0,
                    cognitive_debt: 0.0,
                    instability_debt: 0.0,
                }),
            },
            SnapshotMetadata {
                commit_hash: "older".to_string(),
                scan_order: 2,
                timestamp: 30,
                drift: Some(DriftScore {
                    total: 20,
                    fan_in_delta: 0,
                    fan_out_delta: 0,
                    new_cycles: 0,
                    boundary_violations: 0,
                    layering_violations: 0,
                    cognitive_complexity: 0.0,
                    timestamp: 30,
                    cycle_debt: 0.0,
                    layering_debt: 0.0,
                    hub_debt: 0.0,
                    coupling_debt: 0.0,
                    cognitive_debt: 0.0,
                    instability_debt: 0.0,
                }),
            },
        ];

        assert_eq!(build_trend_data(&snapshots, 0), vec![20, 30, 40]);
        assert_eq!(build_trend_data(&snapshots, 1), vec![20, 30]);
    }
}
