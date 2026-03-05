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
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::db::Database;
use crate::models::{DriftScore, GraphSnapshot, SnapshotMetadata};
use crate::scoring;

use super::graph_renderer::{
    ACCENT_BLUE, ACCENT_LAVENDER, ACCENT_MAUVE, BG_BASE, BG_SURFACE, FG_OVERLAY, FG_TEXT,
    GraphLayout, NODE_PALETTE, drift_color, weighted_edge_color,
};
use super::insight_panel::render_insight_panel;
use super::timeline::{TimelineState, render_timeline};
use super::widgets::render_package_list;

fn adaptive_steps(n_nodes: usize, base: usize, min: usize) -> usize {
    if n_nodes <= 100 {
        base
    } else {
        (base * 100 / n_nodes).clamp(min, base)
    }
}

pub struct App {
    pub db: Option<Database>,
    pub graph_layout: GraphLayout,
    pub timeline: TimelineState,
    pub snapshots_metadata: Vec<SnapshotMetadata>,
    pub snapshot_cache: HashMap<String, GraphSnapshot>,
    pub current_drift: Option<DriftScore>,
    pub is_playing: bool,
    pub show_search: bool,
    pub search_query: String,
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
    pub brittle_packages: Vec<(String, f64)>,
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
            show_search: false,
            search_query: String::new(),
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
            pkg_panel_width: 22,
            resizing_pkg: false,
            dragging_timeline: false,
            hovered_node: None,
            render_cache: None,
            loading_hash: None,
            brittle_packages: Vec::new(),
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
        let mut g = DiGraph::new();
        let mut node_map = HashMap::new();
        for label in &self.graph_layout.labels {
            node_map.insert(label.clone(), g.add_node(label.clone()));
        }
        for &(from, to) in &self.graph_layout.edges {
            if from < self.graph_layout.labels.len() && to < self.graph_layout.labels.len() {
                let from_n = &self.graph_layout.labels[from];
                let to_n = &self.graph_layout.labels[to];
                g.add_edge(node_map[from_n], node_map[to_n], ());
            }
        }
        let metrics = scoring::compute_instability_metrics(&g);
        self.brittle_packages = metrics.into_iter().take(5).collect();
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
        self.compute_insights(); // <--- Update insights here!
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

    pub fn handle_key(&mut self, code: KeyCode, modifiers: KeyModifiers) {
        if self.show_search {
            match code {
                KeyCode::Esc => {
                    self.show_search = false;
                    self.search_query.clear();
                }
                KeyCode::Enter => {
                    self.show_search = false;
                }
                KeyCode::Backspace => {
                    self.search_query.pop();
                }
                KeyCode::Char(c) => {
                    self.search_query.push(c);
                }
                _ => {}
            }
            return;
        }

        match code {
            KeyCode::Char('q') => {
                self.should_quit = true;
            }
            KeyCode::Esc => {
                if !self.search_query.is_empty() {
                    self.search_query.clear();
                } else {
                    self.should_quit = true;
                }
            }
            KeyCode::Char('j') | KeyCode::Down => {
                self.next_commit();
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.prev_commit();
            }
            KeyCode::Char('J') | KeyCode::PageDown | KeyCode::Char('l') => {
                self.jump_commit(10);
            }
            KeyCode::Char('K') | KeyCode::PageUp | KeyCode::Char('h') => {
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
                    (ms - 100).max(200)
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
            KeyCode::Char('p') | KeyCode::Char(' ') => {
                self.is_playing = !self.is_playing;
                self.last_auto_advance = Instant::now();
            }
            KeyCode::Char('r') => {
                self.reheat_layout();
            }
            KeyCode::Char('[') => {
                self.pkg_scroll_offset = self.pkg_scroll_offset.saturating_sub(5);
            }
            KeyCode::Char(']') => {
                self.pkg_scroll_offset = self
                    .pkg_scroll_offset
                    .saturating_add(5)
                    .min(self.graph_layout.labels.len().saturating_sub(1));
            }
            KeyCode::Char('/') => {
                self.show_search = true;
                self.search_query.clear();
            }
            KeyCode::Char('c') if modifiers.contains(KeyModifiers::CONTROL) => {
                self.should_quit = true;
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
                let mut closest: Option<(usize, f64)> = None;
                for (i, pos) in self.graph_layout.positions.iter().enumerate() {
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
                    if self.graph_layout.temperature < 0.05 {
                        self.graph_layout.temperature = 0.05;
                    }
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
                    let mut closest: Option<(usize, f64)> = None;
                    for (i, pos) in self.graph_layout.positions.iter().enumerate() {
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
            MouseEventKind::ScrollUp => {
                if self
                    .pkg_area
                    .contains(ratatui::layout::Position::new(col, row))
                {
                    self.pkg_scroll_offset = self.pkg_scroll_offset.saturating_sub(3);
                }
            }
            MouseEventKind::ScrollDown => {
                if self
                    .pkg_area
                    .contains(ratatui::layout::Position::new(col, row))
                {
                    self.pkg_scroll_offset = self
                        .pkg_scroll_offset
                        .saturating_add(3)
                        .min(self.graph_layout.labels.len().saturating_sub(1));
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

pub fn render_app(frame: &mut Frame, app: &mut App) {
    let size = frame.area();
    frame.render_widget(Block::default().style(Style::default().bg(BG_BASE)), size);

    let main = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(10), Constraint::Length(4)])
        .split(size);
    let top = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(app.pkg_panel_width),
            Constraint::Min(30),
            Constraint::Length(32),
        ])
        .split(main[0]);

    render_package_list(
        frame,
        top[0],
        &app.graph_layout.labels,
        &app.search_query,
        app.pkg_scroll_offset,
    );
    render_graph_canvas(frame, top[1], app);
    render_insight_panel(
        frame,
        top[2],
        &app.current_drift,
        &app.snapshots_metadata,
        &app.brittle_packages,
    );
    render_timeline(frame, main[1], &app.timeline);
    if app.show_search {
        render_search_overlay(frame, top[1], &app.search_query);
    }
    render_status_bar(frame, size, app);
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

    let search_active = !app.search_query.is_empty();
    let (search_matched, search_visible) = if search_active {
        let q = app.search_query.to_lowercase();
        let mut m = HashSet::new();
        for (i, l) in app.graph_layout.labels.iter().enumerate() {
            if l.to_lowercase().contains(&q) {
                m.insert(i);
            }
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
    } else {
        (HashSet::new(), HashSet::new())
    };

    let drift_score = app.current_drift.as_ref().map(|d| d.total).unwrap_or(50);
    let block = Block::default()
        .title(format!(
            " Graph [{} nodes, {} edges] T:{}% ",
            app.graph_layout.labels.len(),
            app.graph_layout.edges.len(),
            (app.graph_layout.temperature * 100.0).round()
        ))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(drift_color(drift_score)))
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
                let mut count = 0;
                for &idx in c.sorted_edge_indices.iter().rev() {
                    let &(f, t) = &layout.edges[idx];
                    if search_active
                        && (!search_visible_cloned.contains(&f)
                            || !search_visible_cloned.contains(&t))
                    {
                        continue;
                    }
                    if count >= max_edges {
                        break;
                    }
                    let (x1, y1) = snapped_cloned[f];
                    let (x2, y2) = snapped_cloned[t];
                    let color = weighted_edge_color(layout.edge_weights[idx]);
                    ctx.draw(&ratatui::widgets::canvas::Line {
                        x1,
                        y1,
                        x2,
                        y2,
                        color,
                    });
                    count += 1;
                }
            }
        });
    frame.render_widget(canvas, area);

    let buf = frame.buffer_mut();
    let label_max_len = if n_nodes > 80 { 12 } else { 14 };
    for (i, &(sx, sy)) in snapped.iter().enumerate() {
        if search_active && !search_visible.contains(&i) {
            continue;
        }
        let is_m = search_active && search_matched.contains(&i);
        let is_h = app.hovered_node == Some(i);
        let show_l = search_active || is_h || cache.is_some_and(|c| c.label_visible.contains(&i));

        let col = area.x + 1 + (sx / 2.0) as u16;
        let row = area.y + 1 + ((canvas_h - sy) / 4.0) as u16;

        if col < area.x + area.width - 1 && row < area.y + area.height - 1 {
            let color = if is_m {
                Color::Rgb(255, 232, 115)
            } else if is_h {
                Color::White
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
                buf.set_string(
                    col + 2,
                    row,
                    text,
                    Style::default().fg(if is_m { color } else { FG_TEXT }),
                );
            }
        }
    }
}

fn render_search_overlay(frame: &mut Frame, area: Rect, query: &str) {
    let bar = Paragraph::new(Line::from(vec![
        Span::styled(
            "/",
            Style::default()
                .fg(ACCENT_MAUVE)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(query),
        Span::styled("█", Style::default().fg(ACCENT_MAUVE)),
    ]))
    .style(Style::default().bg(BG_SURFACE));
    frame.render_widget(
        bar,
        Rect::new(
            area.x + 2,
            area.y + area.height - 2,
            (query.len() as u16 + 4).max(20).min(area.width - 4),
            1,
        ),
    );
}

fn render_status_bar(frame: &mut Frame, area: Rect, app: &App) {
    let status = Line::from(vec![
        Span::styled(
            if app.is_playing {
                " > PLAY "
            } else {
                " || PAUSE "
            },
            Style::default()
                .fg(if app.is_playing {
                    ACCENT_BLUE
                } else {
                    FG_OVERLAY
                })
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" | "),
        Span::styled(
            format!("{} commits", app.snapshots_metadata.len()),
            Style::default().fg(ACCENT_LAVENDER),
        ),
        Span::raw(" | "),
        Span::styled(
            format!("frame #{}", app.frame_count),
            Style::default().fg(FG_OVERLAY),
        ),
        Span::raw(" | j/k:±1 h/l:±10 g/G:start/end p:play /:search q:quit"),
    ]);
    frame.render_widget(
        Paragraph::new(status).style(Style::default().bg(BG_BASE)),
        Rect::new(area.x, area.height - 1, area.width, 1),
    );
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
                    // Update areas before handling mouse
                    let size = terminal.size()?;
                    let area = Rect::new(0, 0, size.width, size.height);
                    let main = Layout::default()
                        .direction(Direction::Vertical)
                        .constraints([Constraint::Min(10), Constraint::Length(4)])
                        .split(area);
                    let top = Layout::default()
                        .direction(Direction::Horizontal)
                        .constraints([
                            Constraint::Length(app.pkg_panel_width),
                            Constraint::Min(30),
                            Constraint::Length(32),
                        ])
                        .split(main[0]);
                    app.pkg_area = top[0];
                    app.graph_area = top[1];
                    app.timeline_area = main[1];
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
