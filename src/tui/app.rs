// =============================================================================
// tui/app.rs — Main TUI application state and event loop
// =============================================================================

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use crossterm::event::{
    self, Event, KeyCode, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use petgraph::graph::DiGraph;
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::canvas::Canvas;
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use crate::db::Database;
use crate::models::{DriftScore, GraphSnapshot, SnapshotMetadata};
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
    Health,
    Hotspots,
    Trends,
}

// Keep ActiveView for backward compat during transition
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActiveView {
    Dashboard,
    Inspect(String),
    MacroGraph,
}

use super::graph_renderer::{
    ACCENT_BLUE, ACCENT_LAVENDER, ACCENT_MAUVE, BG_BASE, BG_SURFACE, FG_OVERLAY, FG_TEXT,
    GraphLayout, NODE_PALETTE, drift_color, weighted_edge_color,
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
    pub graph_layout: GraphLayout,
    pub timeline: TimelineState,
    pub snapshots_metadata: Vec<SnapshotMetadata>,
    pub snapshot_cache: HashMap<String, GraphSnapshot>,
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
    render_cache: Option<GraphRenderCache>,
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
    /// Component diagnostic advisory lines (computed from current graph)
    pub advisory_lines: Vec<String>,
}

struct GraphRenderCache {
    label_visible: HashSet<usize>,
    sorted_edge_indices: Vec<usize>,
}

impl App {
    pub fn new(db: Option<Database>, snapshots: Vec<GraphSnapshot>) -> Self {
        // ... (existing constructor logic) ...
        let timeline_commits: Vec<(String, String, i64)> = snapshots
            .iter()
            .map(|s| (s.commit_hash.clone(), String::new(), s.timestamp))
            .collect();

        let timeline = TimelineState::new(timeline_commits);

        let snapshots_metadata: Vec<SnapshotMetadata> = snapshots
            .iter()
            .map(|s| SnapshotMetadata {
                commit_hash: s.commit_hash.clone(),
                timestamp: s.timestamp,
                drift: s.drift.clone(),
            })
            .collect();

        let mut snapshot_cache = HashMap::new();
        for s in snapshots {
            snapshot_cache.insert(s.commit_hash.clone(), s);
        }

        let (labels, edges, weights) = if let Some(first_meta) = snapshots_metadata.first() {
            if let Some(first) = snapshot_cache.get(&first_meta.commit_hash) {
                snapshot_to_layout_data(first)
            } else {
                (vec![], vec![], vec![])
            }
        } else {
            (vec![], vec![], vec![])
        };

        let graph_layout = GraphLayout::new(labels, edges, weights, 500.0, 500.0);
        let current_drift = snapshots_metadata.first().and_then(|m| m.drift.clone());

        let now = Instant::now();

        let mut app = Self {
            db,
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
            render_cache: None,
            loading_hash: None,
            brittle_packages: Vec::new(),
            hotspots_state: ratatui::widgets::TableState::default(),
            hotspots_sort: HotspotsSort::Instability,
            active_view: ActiveView::Dashboard,
            // New TUI redesign fields
            focused_panel: FocusedPanel::Graph,
            nav_stack: vec![ViewContext::Overview],
            insight_tab: InsightTab::Health,
            show_help: false,
            sidebar_visible: true,
            insights_visible: true,
            filter_active: false,
            filter_text: String::new(),
            selected_pkg_index: None,
            repo_name: String::new(),
            insights_panel_width: 34,
            insights_area: Rect::default(),
            advisory_lines: Vec::new(),
        };

        if let Some(first_meta) = app.snapshots_metadata.first() {
            let hash = first_meta.commit_hash.clone();
            app.refresh_render_cache(&hash);
            app.compute_insights();
        }

        app
    }

    /// Computes architectural insights like instability for the current graph.
    pub fn compute_insights(&mut self) {
        // Build a temporary petgraph from current layout to run metrics
        let mut g: DiGraph<String, u32> = DiGraph::new();
        let mut node_map = HashMap::new();
        for label in &self.graph_layout.labels {
            node_map.insert(label.clone(), g.add_node(label.clone()));
        }
        for (idx, &(from, to)) in self.graph_layout.edges.iter().enumerate() {
            if from < self.graph_layout.labels.len() && to < self.graph_layout.labels.len() {
                let from_n = &self.graph_layout.labels[from];
                let to_n = &self.graph_layout.labels[to];
                let weight = self
                    .graph_layout
                    .edge_weights
                    .get(idx)
                    .copied()
                    .unwrap_or(1);
                g.add_edge(node_map[from_n], node_map[to_n], weight);
            }
        }
        let metrics = scoring::compute_instability_metrics(&g);
        self.brittle_packages = metrics;
        self.apply_hotspots_sort();

        // Compute component diagnostics for advisory display
        if let Some(ref drift) = self.current_drift {
            self.advisory_lines = scoring::generate_diagnostics(&g, drift);
        } else {
            self.advisory_lines.clear();
        }
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
        if let Some(i) = self.hotspots_state.selected() {
            if let Some(pkg) = self.brittle_packages.get(i) {
                // Focus the graph on the selected node if it exists
                if let Some(idx) = self.graph_layout.labels.iter().position(|l| l == &pkg.0) {
                    self.hovered_node = Some(idx);
                }
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
        let cached_snapshot = self.snapshot_cache.get(&hash).cloned();

        if let Some(snapshot) = cached_snapshot {
            self.apply_snapshot(&snapshot);
            self.loading_hash = None;
        } else {
            self.loading_hash = Some(hash);
        }
    }

    fn apply_snapshot(&mut self, snapshot: &GraphSnapshot) {
        let (labels, edges, weights) = snapshot_to_layout_data(snapshot);
        self.graph_layout.update_graph(labels, edges, weights);
        self.current_drift = snapshot.drift.clone();
        self.refresh_render_cache(&snapshot.commit_hash);
        self.compute_insights();
        self.dragging_node = None;
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
        if let Some(last) = self.nav_stack.last() {
            if matches!(last, ViewContext::ModuleInspect(_))
                && matches!(ctx, ViewContext::ModuleInspect(_))
            {
                self.nav_stack.pop();
            }
        }
        self.nav_stack.push(ctx);
    }

    /// Pop the navigation stack (go back). Returns false if already at root.
    pub fn pop_view(&mut self) -> bool {
        if self.nav_stack.len() > 1 {
            self.nav_stack.pop();
            // Sync active_view for graph rendering compatibility
            match self.current_view() {
                ViewContext::Overview | ViewContext::PackageDetail(_) => {
                    self.active_view = ActiveView::Dashboard;
                }
                ViewContext::ModuleInspect(name) => {
                    self.active_view = ActiveView::Inspect(name.clone());
                }
            }
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
                if self.insights_visible {
                    FocusedPanel::Insights
                } else {
                    FocusedPanel::Timeline
                }
            }
            FocusedPanel::Insights => FocusedPanel::Timeline,
            FocusedPanel::Timeline => {
                if self.sidebar_visible {
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
                if self.sidebar_visible {
                    FocusedPanel::Packages
                } else {
                    FocusedPanel::Timeline
                }
            }
            FocusedPanel::Insights => FocusedPanel::Graph,
            FocusedPanel::Timeline => {
                if self.insights_visible {
                    FocusedPanel::Insights
                } else {
                    FocusedPanel::Graph
                }
            }
        };
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

    /// Set repo name for breadcrumb display
    pub fn set_repo_name(&mut self, name: String) {
        self.repo_name = name;
    }

    /// Get sorted and filtered package list for sidebar
    pub fn get_sorted_packages(&self) -> Vec<String> {
        let mut sorted: Vec<String> = self.graph_layout.labels.clone();
        sorted.sort_by_key(|a| a.to_lowercase());
        if !self.filter_text.is_empty() {
            let q = self.filter_text.to_lowercase();
            sorted.retain(|s| s.to_lowercase().contains(&q));
        }
        sorted
    }

    pub fn handle_key(&mut self, code: KeyCode, modifiers: KeyModifiers) {
        // ── Filter bar input mode ──
        if self.filter_active {
            match code {
                KeyCode::Esc => {
                    self.filter_active = false;
                    self.filter_text.clear();
                }
                KeyCode::Enter => {
                    self.filter_active = false;
                    // Keep filter_text active (shown as badge)
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
                if self.sidebar_visible {
                    self.focused_panel = FocusedPanel::Packages;
                }
                return;
            }
            KeyCode::Char('2') => {
                self.focused_panel = FocusedPanel::Graph;
                return;
            }
            KeyCode::Char('3') => {
                if self.insights_visible {
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
            KeyCode::Char('p') | KeyCode::Char(' ') => {
                self.is_playing = !self.is_playing;
                self.last_auto_advance = Instant::now();
                return;
            }
            KeyCode::Esc => {
                // Cascading escape: filter → nav stack → deselect → quit
                if !self.filter_text.is_empty() {
                    self.filter_text.clear();
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
        let pkgs = self.get_sorted_packages();
        match code {
            KeyCode::Char('j') | KeyCode::Down => {
                if pkgs.is_empty() {
                    return;
                }
                self.selected_pkg_index = Some(
                    self.selected_pkg_index
                        .map(|i| (i + 1).min(pkgs.len() - 1))
                        .unwrap_or(0),
                );
                // Scroll to keep selection visible
                if let Some(idx) = self.selected_pkg_index {
                    if idx >= self.pkg_scroll_offset + 20 {
                        self.pkg_scroll_offset = idx.saturating_sub(19);
                    }
                }
            }
            KeyCode::Char('k') | KeyCode::Up => {
                if pkgs.is_empty() {
                    return;
                }
                self.selected_pkg_index = Some(
                    self.selected_pkg_index
                        .map(|i| i.saturating_sub(1))
                        .unwrap_or(0),
                );
                if let Some(idx) = self.selected_pkg_index {
                    if idx < self.pkg_scroll_offset {
                        self.pkg_scroll_offset = idx;
                    }
                }
            }
            KeyCode::Char('g') => {
                self.selected_pkg_index = Some(0);
                self.pkg_scroll_offset = 0;
            }
            KeyCode::Char('G') => {
                if !pkgs.is_empty() {
                    self.selected_pkg_index = Some(pkgs.len() - 1);
                    self.pkg_scroll_offset = pkgs.len().saturating_sub(20);
                }
            }
            KeyCode::Enter => {
                if let Some(idx) = self.selected_pkg_index {
                    if let Some(name) = pkgs.get(idx) {
                        let name = name.clone();
                        self.active_view = ActiveView::Inspect(name.clone());
                        self.push_view(ViewContext::ModuleInspect(name));
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
            KeyCode::Enter => {
                // Inspect hovered node
                if let Some(idx) = self.hovered_node {
                    if let Some(label) = self.graph_layout.labels.get(idx) {
                        let name = label.clone();
                        self.active_view = ActiveView::Inspect(name.clone());
                        self.push_view(ViewContext::ModuleInspect(name));
                    }
                }
            }
            KeyCode::Char('c') => {
                self.graph_layout.center_layout();
            }
            _ => {}
        }
    }

    fn handle_insights_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Char('j') | KeyCode::Down => {
                self.select_next_hotspot();
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.select_prev_hotspot();
            }
            KeyCode::Char('h') | KeyCode::Left => {
                self.insight_tab = match self.insight_tab {
                    InsightTab::Health => InsightTab::Trends,
                    InsightTab::Hotspots => InsightTab::Health,
                    InsightTab::Trends => InsightTab::Hotspots,
                };
            }
            KeyCode::Char('l') | KeyCode::Right => {
                self.insight_tab = match self.insight_tab {
                    InsightTab::Health => InsightTab::Hotspots,
                    InsightTab::Hotspots => InsightTab::Trends,
                    InsightTab::Trends => InsightTab::Health,
                };
            }
            KeyCode::Enter => {
                if let Some(i) = self.hotspots_state.selected() {
                    if let Some(pkg) = self.brittle_packages.get(i) {
                        let name = pkg.0.clone();
                        self.active_view = ActiveView::Inspect(name.clone());
                        self.push_view(ViewContext::ModuleInspect(name));
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

        // Click on a package in the sidebar
        let in_pkg = self
            .pkg_area
            .contains(ratatui::layout::Position::new(col, row))
            && self.pkg_area.width > 0;
        if matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left)) && in_pkg {
            self.focused_panel = FocusedPanel::Packages;
            // 2 rows offset for border + title
            let row_offset = row.saturating_sub(self.pkg_area.y + 2) as usize;
            let clicked_idx = self.pkg_scroll_offset + row_offset;
            let pkgs = self.get_sorted_packages();
            if clicked_idx < pkgs.len() {
                self.selected_pkg_index = Some(clicked_idx);
            }
            return;
        }

        let area = self.graph_area;
        let inner_x = area.x + 1;
        let inner_y = area.y + 1;
        let inner_w = area.width.saturating_sub(2);
        let inner_h = area.height.saturating_sub(2);
        if inner_w == 0 || inner_h == 0 {
            return;
        }

        let in_canvas =
            col >= inner_x && col < inner_x + inner_w && row >= inner_y && row < inner_y + inner_h;

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
                if let Some(old_idx) = self.dragging_node.take() {
                    if old_idx < self.graph_layout.positions.len() {
                        self.graph_layout.positions[old_idx].pinned = false;
                    }
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
                    if let Some(ref v) = vis {
                        if !v.contains(&i) {
                            continue;
                        }
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
                }
            }
            MouseEventKind::Drag(MouseButton::Left) => {
                if let Some(idx) = self.dragging_node {
                    if idx < self.graph_layout.positions.len() {
                        let (px, py) =
                            self.terminal_to_physics(col, row, inner_x, inner_y, inner_w, inner_h);
                        self.graph_layout.positions[idx].x = px;
                        self.graph_layout.positions[idx].y = py;
                        self.graph_layout.positions[idx].prev_x = px;
                        self.graph_layout.positions[idx].prev_y = py;
                    }
                }
            }
            MouseEventKind::Up(MouseButton::Left) => {
                if let Some(idx) = self.dragging_node.take() {
                    if idx < self.graph_layout.positions.len() {
                        self.graph_layout.positions[idx].pinned = false;
                        self.graph_layout.temperature = 0.01;
                    }
                }
            }
            MouseEventKind::Moved => {
                if in_canvas {
                    let (px, py) =
                        self.terminal_to_physics(col, row, inner_x, inner_y, inner_w, inner_h);
                    let vis = self.visible_node_set();
                    let mut closest: Option<(usize, f64)> = None;
                    for (i, pos) in self.graph_layout.positions.iter().enumerate() {
                        // Skip nodes hidden by inspect/filter
                        if let Some(ref v) = vis {
                            if !v.contains(&i) {
                                continue;
                            }
                        }
                        let dist = ((pos.x - px).powi(2) + (pos.y - py).powi(2)).sqrt();
                        if dist < 10.0 && (closest.is_none() || dist < closest.unwrap().1) {
                            closest = Some((i, dist));
                        }
                    }
                    self.hovered_node = closest.map(|(idx, _)| idx);
                } else {
                    self.hovered_node = None;
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
                    // Tabs: " Health " (8) " " (1) " Hotspots " (10) " " (1) " Trends " (8)
                    if rel_x < 8 {
                        self.insight_tab = InsightTab::Health;
                    } else if rel_x < 19 {
                        self.insight_tab = InsightTab::Hotspots;
                    } else {
                        self.insight_tab = InsightTab::Trends;
                    }
                } else if self.insight_tab == InsightTab::Hotspots {
                    // Click on hotspots table row
                    // Header row + tab bar + block border = offset ~3 rows
                    let row_offset = row.saturating_sub(self.insights_area.y + 3);
                    let clicked_idx = row_offset as usize;
                    if clicked_idx < self.brittle_packages.len() {
                        self.hotspots_state.select(Some(clicked_idx));
                        self.focus_selected_hotspot();
                    }
                }
            }
            MouseEventKind::ScrollUp => {
                let pos = ratatui::layout::Position::new(col, row);
                if self.pkg_area.contains(pos) {
                    self.pkg_scroll_offset = self.pkg_scroll_offset.saturating_sub(3);
                } else if self.insights_area.contains(pos) {
                    // Scroll insights: navigate hotspots up or switch tabs
                    if self.insight_tab == InsightTab::Hotspots {
                        self.select_prev_hotspot();
                    } else {
                        self.insight_tab = match self.insight_tab {
                            InsightTab::Hotspots => InsightTab::Health,
                            InsightTab::Trends => InsightTab::Hotspots,
                            InsightTab::Health => InsightTab::Trends,
                        };
                    }
                }
            }
            MouseEventKind::ScrollDown => {
                let pos = ratatui::layout::Position::new(col, row);
                if self.pkg_area.contains(pos) {
                    self.pkg_scroll_offset = self
                        .pkg_scroll_offset
                        .saturating_add(3)
                        .min(self.graph_layout.labels.len().saturating_sub(1));
                } else if self.insights_area.contains(pos) {
                    // Scroll insights: navigate hotspots down or switch tabs
                    if self.insight_tab == InsightTab::Hotspots {
                        self.select_next_hotspot();
                    } else {
                        self.insight_tab = match self.insight_tab {
                            InsightTab::Health => InsightTab::Hotspots,
                            InsightTab::Hotspots => InsightTab::Trends,
                            InsightTab::Trends => InsightTab::Health,
                        };
                    }
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
        (
            nx * self.graph_layout.width,
            (1.0 - ny) * self.graph_layout.height,
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
            let sx = (p.x / layout.width) * canvas_w;
            let sy = (p.y / layout.height) * canvas_h;
            (
                (sx / 2.0).floor() * 2.0 + 1.0,
                (sy / 4.0).floor() * 4.0 + 2.0,
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
    let canvas = Canvas::default()
        .block(block)
        .marker(ratatui::symbols::Marker::Braille)
        .x_bounds([0.0, canvas_w.max(1.0)])
        .y_bounds([0.0, canvas_h.max(1.0)])
        .paint(move |ctx| {
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

                    let color = if is_filtered {
                        if let Some(center) = inspect_center_idx {
                            if f == center {
                                // Outbound: Peach — this module depends on target
                                Color::Rgb(250, 179, 135)
                            } else if t == center {
                                // Inbound: Teal — source depends on this module
                                Color::Rgb(148, 226, 213)
                            } else {
                                // Neighbor-to-neighbor: muted Sapphire (visible but secondary)
                                Color::Rgb(88, 113, 150)
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

    let buf = frame.buffer_mut();
    let label_max_len = if n_nodes > 80 { 12 } else { 14 };
    for (i, &(sx, sy)) in snapped.iter().enumerate() {
        let is_m = is_filtered && search_matched.contains(&i);
        let is_v = is_filtered && search_visible.contains(&i);
        let is_h = app.hovered_node == Some(i);

        // In inspect/filter mode: completely hide nodes not in the subgraph
        if is_filtered && !is_v {
            continue;
        }

        // Show labels for visible nodes in filtered view, hovered nodes, or top-degree nodes
        let show_l = is_h
            || (is_filtered && is_v)
            || (!is_filtered && cache.is_some_and(|c| c.label_visible.contains(&i)));

        let col = area.x + 1 + (sx / 2.0) as u16;
        let row = area.y + 1 + ((canvas_h - sy) / 4.0) as u16;

        if col < area.x + area.width - 1 && row < area.y + area.height - 1 {
            let color = if is_m {
                Color::Rgb(255, 232, 115) // bright yellow for center
            } else if is_h {
                Color::White // white for hover
            } else if is_filtered && is_v {
                // Differentiate inbound vs outbound neighbors
                if let Some(center) = inspect_center_idx {
                    let is_inbound = layout.edges.iter().any(|&(f, t)| f == i && t == center);
                    let is_outbound = layout.edges.iter().any(|&(f, t)| f == center && t == i);
                    if is_inbound && is_outbound {
                        // Bidirectional: Mauve (special)
                        Color::Rgb(203, 166, 247)
                    } else if is_inbound {
                        // This node depends on center: Teal
                        Color::Rgb(148, 226, 213)
                    } else if is_outbound {
                        // Center depends on this node: Peach
                        Color::Rgb(250, 179, 135)
                    } else {
                        // Indirect neighbor (neighbor-of-neighbor): muted blue
                        Color::Rgb(116, 150, 200)
                    }
                } else {
                    Color::Rgb(137, 220, 255) // filter mode: bright blue
                }
            } else {
                NODE_PALETTE[i % NODE_PALETTE.len()]
            };

            let cell = &mut buf[(col, row)];
            cell.set_symbol(if is_m { "◆" } else { "●" }).set_fg(color);

            if show_l {
                let label = &layout.labels[i];
                let text = if is_h {
                    label.as_str()
                } else if label.len() > label_max_len {
                    &label[..label_max_len]
                } else {
                    label.as_str()
                };
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
                    let label_color = if is_m || (is_filtered && is_v) {
                        color
                    } else {
                        FG_TEXT
                    };
                    buf.set_string(label_x, row, text, Style::default().fg(label_color));
                }
            }
        }
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
        if let Some(msg) = app.timeline.current_commit_message() {
            if !msg.is_empty() {
                let truncated = super::widgets::truncate_str(msg, 40);
                spans.push(Span::styled(
                    format!(" \"{}\"", truncated),
                    Style::default().fg(FG_SUBTEXT),
                ));
            }
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

    let pkgs = app.get_sorted_packages();
    let total = app.graph_layout.labels.len();
    let shown = pkgs.len();

    let title_str = if shown < total {
        format!(" Packages ({}/{}) ", shown, total)
    } else {
        format!(" Packages ({}) ", total)
    };

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

    if pkgs.is_empty() {
        let empty = Paragraph::new("  (empty)").style(Style::default().fg(FG_OVERLAY));
        frame.render_widget(empty, inner);
        return;
    }

    let max_visible = inner.height as usize;
    let effective_offset = app.pkg_scroll_offset.min(pkgs.len().saturating_sub(1));
    let list_height = if pkgs.len() > max_visible {
        max_visible.saturating_sub(1)
    } else {
        max_visible
    };

    let filter_lower = app.filter_text.to_lowercase();
    let mut lines: Vec<Line> = Vec::new();

    for (rel_i, label) in pkgs.iter().enumerate().skip(effective_offset) {
        if lines.len() >= list_height {
            break;
        }

        let short = super::widgets::truncate_str(label, inner.width.saturating_sub(5) as usize);

        let is_selected = app.selected_pkg_index == Some(rel_i);
        let is_filter_match =
            !filter_lower.is_empty() && label.to_lowercase().contains(&filter_lower);

        let (fg, bg, modifier) = if is_selected && is_focused {
            (Color::White, BG_SURFACE1, Modifier::BOLD)
        } else if is_filter_match {
            (ACCENT_LAVENDER, BG_SURFACE, Modifier::BOLD)
        } else {
            (FG_TEXT, BG_SURFACE, Modifier::empty())
        };

        let prefix = if is_selected && is_focused {
            " ❯ "
        } else {
            "   "
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
            Span::styled(short, Style::default().fg(fg).bg(bg).add_modifier(modifier)),
        ]));
    }

    // Scroll indicator
    if pkgs.len() > max_visible {
        let visible_end = (effective_offset + list_height).min(pkgs.len());
        lines.push(Line::from(Span::styled(
            format!(" [{}-{}/{}]", effective_offset + 1, visible_end, pkgs.len()),
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

    let tabs = vec![
        ("Health", InsightTab::Health),
        ("Hotspots", InsightTab::Hotspots),
        ("Trends", InsightTab::Trends),
    ];

    let mut tab_spans = Vec::new();
    for (label, tab) in &tabs {
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
        tab_spans.push(Span::styled(" ", Style::default()));
    }
    frame.render_widget(Paragraph::new(Line::from(tab_spans)), tab_area);

    // ── Render active tab content ──
    match app.insight_tab {
        InsightTab::Health => {
            render_insight_panel(frame, content_area, &app.current_drift, &app.advisory_lines);
        }
        InsightTab::Hotspots => {
            render_hotspots_tab(frame, content_area, app);
        }
        InsightTab::Trends => {
            render_trends_tab(frame, content_area, app);
        }
    }
}

/// Renders the hotspots tab content
fn render_hotspots_tab(frame: &mut Frame, area: Rect, app: &mut App) {
    if app.brittle_packages.is_empty() {
        frame.render_widget(
            Paragraph::new("  Analyzing...").style(Style::default().fg(FG_OVERLAY)),
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
            format!(" HOTSPOTS (s:sort by {}) ", sort_label),
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

/// Renders the trends tab content
fn render_trends_tab(frame: &mut Frame, area: Rect, app: &App) {
    use ratatui::widgets::Sparkline;

    let trend_data = build_trend_data(&app.snapshots_metadata, app.timeline.current_index);
    let health_data: Vec<u64> = trend_data
        .iter()
        .map(|d| 100u64.saturating_sub(*d))
        .collect();

    let drift_val = app.current_drift.as_ref().map(|d| d.total).unwrap_or(0);
    let health_val = 100u8.saturating_sub(drift_val);
    let health_color = drift_color(drift_val);

    // Adaptive: if very little height, just show the sparkline
    if area.height < 6 {
        let sparkline = Sparkline::default()
            .block(
                Block::default().title(Span::styled(
                    format!(" HEALTH {}% ", health_val),
                    Style::default()
                        .fg(health_color)
                        .add_modifier(Modifier::BOLD),
                )),
            )
            .data(&health_data)
            .max(100)
            .style(Style::default().fg(health_color));
        frame.render_widget(sparkline, area);
        return;
    }

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2), // Current value header
            Constraint::Length(8), // Health sparkline (fixed height, optimal for TUI)
            Constraint::Length(3), // Summary stats
            Constraint::Min(0),    // Pad remaining space to prevent stretching
        ])
        .split(area);

    // ── Current Health Value ──
    let health_line = Line::from(vec![
        Span::styled(" HEALTH ", Style::default().fg(FG_OVERLAY)),
        Span::styled(
            format!("{}%", health_val),
            Style::default()
                .fg(health_color)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("  DEBT {}", drift_val),
            Style::default().fg(FG_OVERLAY),
        ),
    ]);

    // Trend direction indicator
    let trend_dir = if trend_data.len() >= 2 {
        let prev = trend_data[trend_data.len().saturating_sub(2)];
        let curr = trend_data[trend_data.len() - 1];
        if curr < prev {
            ("  improving", COLOR_HEALTHY)
        } else if curr > prev {
            ("  degrading", COLOR_DANGER)
        } else {
            ("  stable", ACCENT_BLUE)
        }
    } else {
        ("", FG_OVERLAY)
    };
    let dir_line = Line::from(vec![
        Span::styled(
            format!(" Last {} commits", trend_data.len()),
            Style::default().fg(FG_OVERLAY),
        ),
        Span::styled(trend_dir.0, Style::default().fg(trend_dir.1)),
    ]);

    frame.render_widget(Paragraph::new(vec![health_line, dir_line]), chunks[0]);

    // ── Health Sparkline ──
    let sparkline = Sparkline::default()
        .block(
            Block::default()
                .borders(Borders::TOP)
                .border_style(Style::default().fg(Color::Rgb(50, 50, 70))),
        )
        .data(&health_data)
        .max(100)
        .style(Style::default().fg(health_color));
    frame.render_widget(sparkline, chunks[1]);

    // ── Summary Stats ──
    let (min_h, max_h, avg_h) = if !health_data.is_empty() {
        let min = *health_data.iter().min().unwrap_or(&0);
        let max = *health_data.iter().max().unwrap_or(&100);
        let sum: u64 = health_data.iter().sum();
        let avg = sum / health_data.len() as u64;
        (min, max, avg)
    } else {
        (0, 100, 50)
    };

    let stats_line1 = Line::from(vec![
        Span::styled(" Min:", Style::default().fg(FG_OVERLAY)),
        Span::styled(
            format!("{:>3}%", min_h),
            Style::default().fg(drift_color(100 - min_h as u8)),
        ),
        Span::styled("  Avg:", Style::default().fg(FG_OVERLAY)),
        Span::styled(
            format!("{:>3}%", avg_h),
            Style::default().fg(drift_color(100 - avg_h as u8)),
        ),
        Span::styled("  Max:", Style::default().fg(FG_OVERLAY)),
        Span::styled(
            format!("{:>3}%", max_h),
            Style::default().fg(drift_color(100 - max_h as u8)),
        ),
    ]);

    let stats_line2 = Line::from(vec![Span::styled(
        format!(
            " {}/{} commits",
            app.timeline.current_index + 1,
            app.timeline.len()
        ),
        Style::default().fg(FG_OVERLAY),
    )]);

    frame.render_widget(
        Paragraph::new(vec![stats_line1, stats_line2]).block(
            Block::default()
                .borders(Borders::TOP)
                .border_style(Style::default().fg(Color::Rgb(50, 50, 70))),
        ),
        chunks[2],
    );
}

/// Build trend data from snapshot metadata
fn build_trend_data(snapshots: &[SnapshotMetadata], current_index: usize) -> Vec<u64> {
    if snapshots.is_empty() {
        return vec![];
    }
    let end = current_index.min(snapshots.len() - 1);
    let start = end.saturating_sub(49);
    let slice = &snapshots[start..=end];

    let mut data: Vec<u64> = slice
        .iter()
        .map(|s| s.drift.as_ref().map(|d| d.total as u64).unwrap_or(50))
        .collect();

    if data.len() > 1
        && snapshots.first().map(|s| s.timestamp).unwrap_or(0)
            > snapshots.last().map(|s| s.timestamp).unwrap_or(0)
    {
        data.reverse();
    }
    data
}

/// Renders the filter bar
fn render_filter_bar(frame: &mut Frame, area: Rect, app: &App) {
    let total = app.graph_layout.labels.len();
    let matched = app.get_sorted_packages().len();

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

    // Row 1: Contextual keybinding hints
    let hints = match app.focused_panel {
        FocusedPanel::Packages => {
            "j/k:Navigate  enter:Inspect  s:Sort  ←/→:Timeline  /:Filter  ?:Help  q:Quit"
        }
        FocusedPanel::Graph => {
            "r:Reheat  c:Center  enter:Inspect  ←/→:Timeline  /:Filter  ?:Help  q:Quit"
        }
        FocusedPanel::Insights => "j/k:Navigate  h/l:Tab  enter:Inspect  s:Sort  ?:Help  q:Quit",
        FocusedPanel::Timeline => "j/k:±1  h/l:±10  g/G:Start/End  Space:Play  +/-:Speed  ?:Help",
    };
    frame.render_widget(
        Paragraph::new(Span::styled(
            format!(" {}", hints),
            Style::default().fg(FG_OVERLAY),
        ))
        .style(Style::default().bg(BG_BASE)),
        rows[0],
    );

    // Row 2: Status info
    let view_label = match app.current_view() {
        ViewContext::Overview => "OVERVIEW".to_string(),
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

    let status = Line::from(vec![
        Span::styled(
            format!(" [{}]", view_label),
            Style::default()
                .fg(ACCENT_MAUVE)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" │ ", Style::default().fg(BORDER_UNFOCUSED)),
        Span::styled(
            format!("{}/{}", app.timeline.current_index + 1, app.timeline.len()),
            Style::default().fg(ACCENT_LAVENDER),
        ),
        Span::styled(" │ ", Style::default().fg(BORDER_UNFOCUSED)),
        Span::styled(
            if app.is_playing { "▶" } else { "⏸" },
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
        Span::styled(" │ ", Style::default().fg(BORDER_UNFOCUSED)),
        Span::styled(
            format!("Score: {}%", health),
            Style::default()
                .fg(health_color)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" │ ", Style::default().fg(BORDER_UNFOCUSED)),
        Span::styled(
            format!("{} nodes", app.graph_layout.labels.len()),
            Style::default().fg(FG_OVERLAY),
        ),
        Span::styled(" │ ", Style::default().fg(BORDER_UNFOCUSED)),
        Span::styled(
            format!("[{}]", panel_name),
            Style::default().fg(ACCENT_BLUE),
        ),
        Span::styled(" │ ", Style::default().fg(BORDER_UNFOCUSED)),
        Span::styled(
            "tab:Panel  b:Sidebar  i:Detail",
            Style::default().fg(FG_OVERLAY),
        ),
    ]);
    frame.render_widget(
        Paragraph::new(status).style(Style::default().bg(BG_BASE)),
        rows[1],
    );
}

/// Renders the help overlay
fn render_help_overlay(frame: &mut Frame, area: Rect) {
    // Clear everything behind the overlay so graph nodes don't bleed through
    frame.render_widget(Clear, area);
    let overlay = Block::default().style(Style::default().bg(Color::Rgb(20, 20, 35)));
    frame.render_widget(overlay, area);

    let w = 62.min(area.width.saturating_sub(4));
    let h = 30.min(area.height.saturating_sub(4));
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
        Line::from(vec![
            Span::styled(
                " NAVIGATION",
                Style::default()
                    .fg(ACCENT_LAVENDER)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "            GRAPH",
                Style::default()
                    .fg(ACCENT_LAVENDER)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::styled(" tab    ", Style::default().fg(ACCENT_BLUE)),
            Span::styled("Next panel      ", Style::default().fg(FG_TEXT)),
            Span::styled(" r    ", Style::default().fg(ACCENT_BLUE)),
            Span::styled("Reheat", Style::default().fg(FG_TEXT)),
        ]),
        Line::from(vec![
            Span::styled(" S-tab  ", Style::default().fg(ACCENT_BLUE)),
            Span::styled("Prev panel      ", Style::default().fg(FG_TEXT)),
            Span::styled(" c    ", Style::default().fg(ACCENT_BLUE)),
            Span::styled("Center", Style::default().fg(FG_TEXT)),
        ]),
        Line::from(vec![
            Span::styled(" 1-4   ", Style::default().fg(ACCENT_BLUE)),
            Span::styled("Jump to panel   ", Style::default().fg(FG_TEXT)),
            Span::styled(" enter ", Style::default().fg(ACCENT_BLUE)),
            Span::styled("Inspect node", Style::default().fg(FG_TEXT)),
        ]),
        Line::from(vec![
            Span::styled(" enter  ", Style::default().fg(ACCENT_BLUE)),
            Span::styled("Drill in", Style::default().fg(FG_TEXT)),
        ]),
        Line::from(vec![
            Span::styled(" esc    ", Style::default().fg(ACCENT_BLUE)),
            Span::styled("Go back         ", Style::default().fg(FG_TEXT)),
            Span::styled(
                " TIMELINE",
                Style::default()
                    .fg(ACCENT_LAVENDER)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::styled(" /      ", Style::default().fg(ACCENT_BLUE)),
            Span::styled("Filter          ", Style::default().fg(FG_TEXT)),
            Span::styled(" j/k  ", Style::default().fg(ACCENT_BLUE)),
            Span::styled("±1 commit", Style::default().fg(FG_TEXT)),
        ]),
        Line::from(vec![
            Span::styled(" ?      ", Style::default().fg(ACCENT_BLUE)),
            Span::styled("This help       ", Style::default().fg(FG_TEXT)),
            Span::styled(" h/l  ", Style::default().fg(ACCENT_BLUE)),
            Span::styled("±10 commits", Style::default().fg(FG_TEXT)),
        ]),
        Line::from(vec![
            Span::styled("                        ", Style::default()),
            Span::styled(" g/G  ", Style::default().fg(ACCENT_BLUE)),
            Span::styled("Start/End", Style::default().fg(FG_TEXT)),
        ]),
        Line::from(vec![
            Span::styled(
                " APP",
                Style::default()
                    .fg(ACCENT_LAVENDER)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("                    ", Style::default()),
            Span::styled(" Space ", Style::default().fg(ACCENT_BLUE)),
            Span::styled("Play/Pause", Style::default().fg(FG_TEXT)),
        ]),
        Line::from(vec![
            Span::styled(" q      ", Style::default().fg(ACCENT_BLUE)),
            Span::styled("Quit            ", Style::default().fg(FG_TEXT)),
            Span::styled(" +/-  ", Style::default().fg(ACCENT_BLUE)),
            Span::styled("Speed ±", Style::default().fg(FG_TEXT)),
        ]),
        Line::from(vec![
            Span::styled(" ctrl+c ", Style::default().fg(ACCENT_BLUE)),
            Span::styled("Force quit", Style::default().fg(FG_TEXT)),
        ]),
        Line::from(vec![Span::styled(
            "                        LAYOUT",
            Style::default()
                .fg(ACCENT_LAVENDER)
                .add_modifier(Modifier::BOLD),
        )]),
        Line::from(vec![
            Span::styled(
                " INSIGHTS",
                Style::default()
                    .fg(ACCENT_LAVENDER)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("               ", Style::default()),
            Span::styled(" b    ", Style::default().fg(ACCENT_BLUE)),
            Span::styled("Toggle sidebar", Style::default().fg(FG_TEXT)),
        ]),
        Line::from(vec![
            Span::styled(" j/k    ", Style::default().fg(ACCENT_BLUE)),
            Span::styled("Navigate        ", Style::default().fg(FG_TEXT)),
            Span::styled(" i    ", Style::default().fg(ACCENT_BLUE)),
            Span::styled("Toggle detail", Style::default().fg(FG_TEXT)),
        ]),
        Line::from(vec![
            Span::styled(" h/l    ", Style::default().fg(ACCENT_BLUE)),
            Span::styled("Switch tab", Style::default().fg(FG_TEXT)),
        ]),
        Line::from(vec![
            Span::styled(" s      ", Style::default().fg(ACCENT_BLUE)),
            Span::styled("Sort hotspots", Style::default().fg(FG_TEXT)),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            " EDGE COLORS",
            Style::default()
                .fg(ACCENT_LAVENDER)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(vec![
            Span::styled(
                " \u{2500}\u{2500}",
                Style::default().fg(Color::Rgb(69, 71, 90)),
            ),
            Span::styled(" w:1       ", Style::default().fg(FG_OVERLAY)),
            Span::styled(
                " \u{2500}\u{2500}",
                Style::default().fg(Color::Rgb(88, 91, 112)),
            ),
            Span::styled(" w:2-3     ", Style::default().fg(FG_OVERLAY)),
            Span::styled(
                " \u{2500}\u{2500}",
                Style::default().fg(Color::Rgb(116, 199, 236)),
            ),
            Span::styled(" w:4-7", Style::default().fg(FG_OVERLAY)),
        ]),
        Line::from(vec![
            Span::styled(
                " \u{2500}\u{2500}",
                Style::default().fg(Color::Rgb(250, 179, 135)),
            ),
            Span::styled(" w:8-15    ", Style::default().fg(FG_OVERLAY)),
            Span::styled(
                " \u{2500}\u{2500}",
                Style::default().fg(Color::Rgb(243, 139, 168)),
            ),
            Span::styled(" w:16+     ", Style::default().fg(FG_OVERLAY)),
            Span::styled("(w=imports)", Style::default().fg(FG_OVERLAY)),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "        Press ? or Esc to close",
            Style::default().fg(FG_OVERLAY),
        )),
    ];

    frame.render_widget(Paragraph::new(help_text), inner);
}

pub async fn run_tui(mut app: App) -> anyhow::Result<()> {
    use crossterm::ExecutableCommand;
    use crossterm::terminal::{
        EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
    };
    use ratatui::backend::CrosstermBackend;

    enable_raw_mode()?;
    std::io::stdout().execute(EnterAlternateScreen)?;
    std::io::stdout().execute(crossterm::event::EnableMouseCapture)?;
    let mut terminal = ratatui::Terminal::new(CrosstermBackend::new(std::io::stdout()))?;
    terminal.clear()?;

    loop {
        if let Some(hash) = app.loading_hash.take() {
            if let Some(db) = &app.db {
                if let Ok(Some(snapshot)) = db.get_graph_snapshot(&hash) {
                    app.snapshot_cache.insert(hash.clone(), snapshot.clone());
                    app.apply_snapshot(&snapshot);
                }
            }
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

    disable_raw_mode()?;
    std::io::stdout().execute(crossterm::event::DisableMouseCapture)?;
    std::io::stdout().execute(LeaveAlternateScreen)?;
    Ok(())
}
