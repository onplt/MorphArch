// =============================================================================
// tui/app.rs — Main TUI application state and event loop
// =============================================================================
//
// Responsibilities:
//   1. App struct: Holds all TUI state (graph, timeline, drift, flags, mouse)
//   2. Event handling: Keyboard (j/k/p/r/q/ESC) + Mouse drag interaction
//   3. Render loop: Draws panels with ratatui
//   4. Physics loop: Advances Verlet step + frame drawing (~30 fps)
//
// Layout:
//   +-----------------------------------------------------+
//   |  Packages  |      Graph Canvas       |  Drift Info   |
//   |  (left)    |      (center)           |  (right)      |
//   +------------+-------------------------+---------------+
//   |                 Timeline (bottom)                     |
//   +-----------------------------------------------------+
//
// Keyboard:
//   j/Down   -> Next commit (older)
//   k/Up     -> Previous commit (newer)
//   p/Space  -> Play/Pause auto-play
//   r        -> Reheat graph (re-energize temperature)
//   /        -> Enter search mode
//   q/ESC    -> Quit
//
// Mouse:
//   Click+Drag -> Move any node (pins during drag, unpins on release)
// =============================================================================

use std::collections::HashMap;
use std::time::{Duration, Instant};

use crossterm::event::{
    self, Event, KeyCode, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::canvas::Canvas;
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::db::Database;
use crate::models::{DriftScore, GraphSnapshot, SnapshotMetadata};

use super::graph_renderer::{
    ACCENT_BLUE, ACCENT_LAVENDER, ACCENT_MAUVE, BG_BASE, BG_SURFACE, FG_OVERLAY, FG_TEXT,
    GraphLayout, NODE_PALETTE, drift_color, weighted_edge_color,
};
use super::insight_panel::render_insight_panel;
use super::timeline::{TimelineState, render_timeline};
use super::widgets::render_package_list;

/// Returns the number of physics steps scaled inversely with node count.
///
/// Barnes-Hut O(N log N) repulsion is much cheaper than O(N²).
/// This helper keeps total work roughly constant regardless of graph size:
///
///   N ≤ 100 → full `base` steps  (small graphs settle instantly)
///   N > 100 → base × 100 / N     (budget scales down linearly)
///
/// The result is clamped to `[min, base]` so we always make *some*
/// progress but never exceed the original budget.
fn adaptive_steps(n_nodes: usize, base: usize, min: usize) -> usize {
    if n_nodes <= 100 {
        base
    } else {
        (base * 100 / n_nodes).clamp(min, base)
    }
}

/// Main TUI application state
pub struct App {
    /// SQLite database for lazy-loading snapshots
    pub db: Option<Database>,
    /// Verlet physics engine for graph layout
    pub graph_layout: GraphLayout,
    /// Timeline slider state
    pub timeline: TimelineState,
    /// Metadata for all sampled snapshots in the timeline
    pub snapshots_metadata: Vec<SnapshotMetadata>,
    /// Cache of loaded full graph snapshots
    pub snapshot_cache: HashMap<String, GraphSnapshot>,
    /// Current commit's drift score
    pub current_drift: Option<DriftScore>,
    /// Auto-play active
    pub is_playing: bool,
    /// Search mode active
    pub show_search: bool,
    /// Search query
    pub search_query: String,
    /// Quit flag
    pub should_quit: bool,
    /// Last auto-play advance time
    pub last_auto_advance: Instant,
    /// Auto-play speed (ms between commits)
    pub auto_play_interval: Duration,
    /// Physics tick rate (~30 fps)
    pub tick_rate: Duration,
    /// Last frame time (for fps calculation)
    pub last_tick: Instant,
    /// Frame counter (debug)
    pub frame_count: u64,
    /// Node being dragged by mouse (index into positions)
    pub dragging_node: Option<usize>,
    /// Last known graph canvas area in terminal coordinates (for mouse mapping)
    pub graph_area: Rect,
    /// Whether the initial warmup is pending (deferred until first render
    /// so the physics runs at the actual canvas dimensions, not hardcoded ones)
    pub needs_warmup: bool,
    /// Scroll offset for the left-panel package list
    pub pkg_scroll_offset: usize,
    /// Last known packages panel area (for mouse scroll hit-testing)
    pub pkg_area: Rect,
    /// Last known timeline panel area (for mouse click-to-seek)
    pub timeline_area: Rect,
    /// User-adjustable width of the packages panel (draggable border)
    pub pkg_panel_width: u16,
    /// Whether the user is currently dragging the packages panel border
    pub resizing_pkg: bool,
    /// Whether the user is currently dragging the timeline slider
    pub dragging_timeline: bool,
}

impl App {
    /// Creates a new TUI application.
    ///
    /// `snapshots` are initial full snapshots. If `db` is provided,
    /// further snapshots can be loaded lazily if they were only provided as metadata.
    pub fn new(db: Option<Database>, snapshots: Vec<GraphSnapshot>) -> Self {
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

        // Initial graph layout from first snapshot
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

        Self {
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
            tick_rate: Duration::from_millis(33), // ~30 fps
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
        }
    }

    /// Updates timeline commit info from external source.
    pub fn set_timeline_commits(&mut self, commits: Vec<(String, String, i64)>) {
        self.timeline = TimelineState::new(commits);
    }

    /// Updates the graph for the current timeline position's snapshot.
    /// Loads from DB if not in cache (lazy loading).
    fn update_graph_for_current_commit(&mut self) {
        let idx = self.timeline.current_index;
        let meta = match self.snapshots_metadata.get(idx) {
            Some(m) => m,
            None => return,
        };

        // Try cache first
        let hash = meta.commit_hash.clone();
        if !self.snapshot_cache.contains_key(&hash) {
            if let Some(db) = &self.db {
                if let Ok(Some(snapshot)) = db.get_graph_snapshot(&hash) {
                    self.snapshot_cache.insert(hash.clone(), snapshot);
                }
            }
        }

        if let Some(snapshot) = self.snapshot_cache.get(&hash) {
            let (labels, edges, weights) = snapshot_to_layout_data(snapshot);
            self.graph_layout.update_graph(labels, edges, weights);
            self.current_drift = snapshot.drift.clone();
            // Cancel any in-progress drag and reset scroll
            self.dragging_node = None;
            self.pkg_scroll_offset = 0;
            // Zero out velocity for all nodes
            for pos in &mut self.graph_layout.positions {
                pos.prev_x = pos.x;
                pos.prev_y = pos.y;
            }
            // Warmup
            let n = self.graph_layout.positions.len();
            let steps = adaptive_steps(n, 300, 3);
            self.graph_layout.multi_step(steps);
            self.graph_layout.center_layout();
            self.graph_layout.temperature = 0.01;
        }
    }

    /// Advances to the next commit (timeline forward).
    pub fn next_commit(&mut self) {
        let old_idx = self.timeline.current_index;
        self.timeline.next();
        if self.timeline.current_index != old_idx {
            self.update_graph_for_current_commit();
        }
    }

    /// Goes back to the previous commit (timeline backward).
    pub fn prev_commit(&mut self) {
        let old_idx = self.timeline.current_index;
        self.timeline.prev();
        if self.timeline.current_index != old_idx {
            self.update_graph_for_current_commit();
        }
    }

    /// Jumps timeline by `n` positions.
    pub fn jump_commit(&mut self, n: isize) {
        let old_idx = self.timeline.current_index;
        self.timeline.jump_by(n);
        if self.timeline.current_index != old_idx {
            self.update_graph_for_current_commit();
        }
    }

    /// Jumps to the first commit in the timeline.
    pub fn jump_to_first(&mut self) {
        let old_idx = self.timeline.current_index;
        self.timeline.jump_to_start();
        if self.timeline.current_index != old_idx {
            self.update_graph_for_current_commit();
        }
    }

    /// Jumps to the last commit in the timeline.
    pub fn jump_to_last(&mut self) {
        let old_idx = self.timeline.current_index;
        self.timeline.jump_to_end();
        if self.timeline.current_index != old_idx {
            self.update_graph_for_current_commit();
        }
    }

    /// Jumps to a specific timeline index (for mouse-click seeking).
    pub fn seek_to(&mut self, index: usize) {
        let old_idx = self.timeline.current_index;
        self.timeline.jump_to(index);
        if self.timeline.current_index != old_idx {
            self.update_graph_for_current_commit();
        }
    }

    /// Reheats the graph layout.
    pub fn reheat_layout(&mut self) {
        self.graph_layout.reheat();
    }

    /// Processes keyboard input.
    pub fn handle_key(&mut self, code: KeyCode, modifiers: KeyModifiers) {
        // Special behavior in search mode
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

        // Normal mode
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
            KeyCode::Char('J') => {
                self.jump_commit(10);
            }
            KeyCode::Char('K') => {
                self.jump_commit(-10);
            }
            KeyCode::Char('h') => {
                self.jump_commit(-10);
            }
            KeyCode::Char('l') => {
                self.jump_commit(10);
            }
            KeyCode::PageDown => {
                self.jump_commit(10);
            }
            KeyCode::PageUp => {
                self.jump_commit(-10);
            }
            KeyCode::Home | KeyCode::Char('g') => {
                self.jump_to_first();
            }
            KeyCode::End => {
                self.jump_to_last();
            }
            KeyCode::Char('G') => {
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

    /// Processes mouse events for node drag interaction.
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
                    let new_w = col.saturating_sub(self.pkg_area.x).clamp(14, 60);
                    self.pkg_panel_width = new_w;
                    return;
                }
                MouseEventKind::Up(MouseButton::Left) | MouseEventKind::Moved => {
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
            && tl.width > 4
            && !self.timeline.is_empty();

        match mouse.kind {
            // Timeline drag start
            MouseEventKind::Down(MouseButton::Left) if in_timeline => {
                self.dragging_timeline = true;
                self.is_playing = false; // pause when dragging
                let rel_x = col.saturating_sub(tl_inner_x) as f64;
                let bar_w = tl_inner_w.max(1) as f64;
                let ratio = (rel_x / bar_w).clamp(0.0, 1.0);
                let target = (ratio * (self.timeline.len() - 1) as f64).round() as usize;
                self.seek_to(target);
            }
            // Timeline dragging
            MouseEventKind::Drag(MouseButton::Left) if self.dragging_timeline => {
                let rel_x = col.saturating_sub(tl_inner_x) as f64;
                let bar_w = tl_inner_w.max(1) as f64;
                let ratio = (rel_x / bar_w).clamp(0.0, 1.0);
                let target = (ratio * (self.timeline.len() - 1) as f64).round() as usize;
                self.seek_to(target);
            }
            // Timeline drag end
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
                        let n = self.graph_layout.positions.len();
                        let steps = adaptive_steps(n, 80, 5);
                        self.graph_layout.temperature = 0.15;
                        self.graph_layout.multi_step(steps);
                        self.graph_layout.positions[idx].pinned = false;
                        self.graph_layout.temperature = 0.01;
                    }
                }
            }
            MouseEventKind::Moved => {
                if let Some(idx) = self.dragging_node.take() {
                    if idx < self.graph_layout.positions.len() {
                        self.graph_layout.positions[idx].pinned = false;
                        self.graph_layout.temperature = 0.01;
                    }
                }
            }
            MouseEventKind::ScrollUp => {
                let pkg = self.pkg_area;
                if col >= pkg.x
                    && col < pkg.x + pkg.width
                    && row >= pkg.y
                    && row < pkg.y + pkg.height
                {
                    self.pkg_scroll_offset = self.pkg_scroll_offset.saturating_sub(3);
                }
            }
            MouseEventKind::ScrollDown => {
                let pkg = self.pkg_area;
                if col >= pkg.x
                    && col < pkg.x + pkg.width
                    && row >= pkg.y
                    && row < pkg.y + pkg.height
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

    /// Converts terminal column/row to physics space coordinates.
    fn terminal_to_physics(
        &self,
        col: u16,
        row: u16,
        inner_x: u16,
        inner_y: u16,
        inner_w: u16,
        inner_h: u16,
    ) -> (f64, f64) {
        let norm_x = (col.saturating_sub(inner_x) as f64) / inner_w.max(1) as f64;
        let norm_y = (row.saturating_sub(inner_y) as f64) / inner_h.max(1) as f64;
        let px = norm_x * self.graph_layout.width;
        let py = (1.0 - norm_y) * self.graph_layout.height;
        (px, py)
    }

    /// Advances timeline in auto-play mode.
    pub fn tick_auto_play(&mut self) {
        if self.is_playing && self.last_auto_advance.elapsed() >= self.auto_play_interval {
            self.next_commit();
            self.last_auto_advance = Instant::now();

            if self.timeline.current_index + 1 >= self.timeline.len() {
                self.is_playing = false;
            }
        }
    }

    /// Advances physics.
    pub fn tick_physics(&mut self) {
        if self.graph_layout.temperature >= 0.02 || self.dragging_node.is_some() {
            let n = self.graph_layout.positions.len();
            let steps = adaptive_steps(n, 3, 1);
            self.graph_layout.multi_step(steps);
        }
        self.frame_count += 1;
    }
}

/// Converts a GraphSnapshot to layout data.
fn snapshot_to_layout_data(
    snapshot: &GraphSnapshot,
) -> (Vec<String>, Vec<(usize, usize)>, Vec<u32>) {
    let labels = snapshot.nodes.clone();

    let label_to_idx: std::collections::HashMap<&String, usize> =
        labels.iter().enumerate().map(|(i, l)| (l, i)).collect();

    let mut edges: Vec<(usize, usize)> = Vec::new();
    let mut weights: Vec<u32> = Vec::new();

    for e in &snapshot.edges {
        let from_idx = label_to_idx.get(&e.from_module).copied();
        let to_idx = label_to_idx.get(&e.to_module).copied();
        if let (Some(f), Some(t)) = (from_idx, to_idx) {
            edges.push((f, t));
            weights.push(e.weight);
        }
    }

    (labels, edges, weights)
}

/// Builds the TUI layout and renders all panels.
pub fn render_app(frame: &mut Frame, app: &mut App) {
    let size = frame.area();

    let background = Block::default().style(Style::default().bg(BG_BASE));
    frame.render_widget(background, size);

    let main_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(10), Constraint::Length(4)])
        .split(size);

    let top_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(app.pkg_panel_width),
            Constraint::Min(30),
            Constraint::Length(32),
        ])
        .split(main_chunks[0]);

    render_package_list(
        frame,
        top_chunks[0],
        &app.graph_layout.labels,
        &app.search_query,
        app.pkg_scroll_offset,
    );

    render_graph_canvas(frame, top_chunks[1], app);

    render_insight_panel(
        frame,
        top_chunks[2],
        &app.current_drift,
        &app.snapshots_metadata,
        app.timeline.current_index,
    );

    render_timeline(frame, main_chunks[1], &app.timeline);

    if app.show_search {
        render_search_overlay(frame, top_chunks[1], &app.search_query);
    }

    render_status_bar(frame, size, app);
}

/// Renders the Graph Canvas.
pub fn render_graph_canvas(frame: &mut Frame, area: Rect, app: &mut App) {
    let canvas_w = (area.width.saturating_sub(2) as f64) * 2.0;
    let canvas_h = (area.height.saturating_sub(2) as f64) * 4.0;

    if canvas_w > 80.0 && canvas_h > 50.0 {
        app.graph_layout.resize(canvas_w, canvas_h);
    }

    if app.needs_warmup {
        let n = app.graph_layout.positions.len();
        let steps = adaptive_steps(n, 300, 3);
        app.graph_layout.reinitialize_positions();
        app.graph_layout.multi_step(steps);
        app.graph_layout.center_layout();
        app.graph_layout.temperature = 0.01;
        app.needs_warmup = false;
    }

    let search_active = !app.search_query.is_empty();
    let (search_matched, search_visible) = if search_active {
        let query_lower = app.search_query.to_lowercase();
        let mut matched: std::collections::HashSet<usize> = std::collections::HashSet::new();
        for (i, label) in app.graph_layout.labels.iter().enumerate() {
            if label.to_lowercase().contains(&query_lower) {
                matched.insert(i);
            }
        }
        let mut visible = matched.clone();
        for &(from, to) in &app.graph_layout.edges {
            if matched.contains(&from) {
                visible.insert(to);
            }
            if matched.contains(&to) {
                visible.insert(from);
            }
        }
        (matched, visible)
    } else {
        (
            std::collections::HashSet::new(),
            std::collections::HashSet::new(),
        )
    };

    let drift_score = app.current_drift.as_ref().map(|d| d.total).unwrap_or(50);

    let temp_pct = (app.graph_layout.temperature * 100.0).round() as u32;
    let block = Block::default()
        .title(if search_active {
            format!(
                " Graph — /{} ({} found, {} visible) ",
                app.search_query,
                search_matched.len(),
                search_visible.len()
            )
        } else {
            format!(
                " Graph [{} nodes, {} edges] T:{}% ",
                app.graph_layout.labels.len(),
                app.graph_layout.edges.len(),
                temp_pct,
            )
        })
        .borders(Borders::ALL)
        .border_style(Style::default().fg(drift_color(drift_score)))
        .style(Style::default().bg(BG_SURFACE));

    let canvas_width = area.width.saturating_sub(2) as f64 * 2.0;
    let canvas_height = area.height.saturating_sub(2) as f64 * 4.0;

    let layout = &app.graph_layout;

    let snapped: Vec<(f64, f64)> = layout
        .positions
        .iter()
        .map(|pos| {
            let raw_x = (pos.x / layout.width) * canvas_width;
            let raw_y = (pos.y / layout.height) * canvas_height;
            let cell_w = 2.0_f64;
            let cell_h = 4.0_f64;
            let sx = (raw_x / cell_w).floor() * cell_w + cell_w / 2.0;
            let sy = (raw_y / cell_h).floor() * cell_h + cell_h / 2.0;
            (sx, sy)
        })
        .collect();

    let n_nodes = layout.positions.len();
    let max_edges = if n_nodes > 200 {
        400
    } else if n_nodes > 100 {
        600
    } else {
        usize::MAX
    };

    let mut edge_data: Vec<(f64, f64, f64, f64, Color, u32)> = layout
        .edges
        .iter()
        .enumerate()
        .filter_map(|(idx, &(from, to))| {
            if search_active && (!search_visible.contains(&from) || !search_visible.contains(&to)) {
                return None;
            }
            if from < snapped.len() && to < snapped.len() {
                let (x1, y1) = snapped[from];
                let (x2, y2) = snapped[to];
                let weight = layout.edge_weights.get(idx).copied().unwrap_or(1);
                let color = weighted_edge_color(weight);
                Some((x1, y1, x2, y2, color, weight))
            } else {
                None
            }
        })
        .collect();

    edge_data.sort_by_key(|e| e.5);
    if edge_data.len() > max_edges {
        let skip = edge_data.len() - max_edges;
        edge_data = edge_data.into_iter().skip(skip).collect();
    }

    let max_labels = if n_nodes > 200 {
        (n_nodes / 7).clamp(20, 60)
    } else if n_nodes > 80 {
        (n_nodes / 3).max(20)
    } else {
        n_nodes
    };

    let mut degree: Vec<usize> = vec![0; n_nodes];
    for &(from, to) in &layout.edges {
        if from < n_nodes {
            degree[from] += 1;
        }
        if to < n_nodes {
            degree[to] += 1;
        }
    }
    let label_visible: std::collections::HashSet<usize> = {
        let mut ranked: Vec<(usize, usize)> = degree.iter().copied().enumerate().collect();
        ranked.sort_by(|a, b| b.1.cmp(&a.1));
        ranked
            .into_iter()
            .take(max_labels)
            .map(|(i, _)| i)
            .collect()
    };

    let label_max_len: usize = if n_nodes > 200 {
        10
    } else if n_nodes > 80 {
        12
    } else {
        14
    };
    let node_points: Vec<(f64, f64, String, Color, bool)> = snapped
        .iter()
        .enumerate()
        .filter_map(|(i, &(x, y))| {
            if search_active && !search_visible.contains(&i) {
                return None;
            }
            let is_matched = search_active && search_matched.contains(&i);
            let show_label = if search_active {
                true
            } else {
                label_visible.contains(&i)
            };
            let label = if show_label && i < layout.labels.len() {
                let l = &layout.labels[i];
                if l.len() > label_max_len {
                    l[..label_max_len].to_string()
                } else {
                    l.clone()
                }
            } else {
                String::new()
            };
            let color = if is_matched {
                Color::Rgb(255, 232, 115)
            } else {
                NODE_PALETTE[i % NODE_PALETTE.len()]
            };
            Some((x, y, label, color, is_matched))
        })
        .collect();

    fn scale_color(c: Color, factor: f64) -> Color {
        match c {
            Color::Rgb(r, g, b) => Color::Rgb(
                (r as f64 * factor).min(255.0) as u8,
                (g as f64 * factor).min(255.0) as u8,
                (b as f64 * factor).min(255.0) as u8,
            ),
            _ => c,
        }
    }

    let canvas = Canvas::default()
        .block(block)
        .marker(ratatui::symbols::Marker::Braille)
        .x_bounds([0.0, canvas_width.max(1.0)])
        .y_bounds([0.0, canvas_height.max(1.0)])
        .paint(move |ctx: &mut ratatui::widgets::canvas::Context<'_>| {
            use ratatui::widgets::canvas::Line as CLine;

            for &(x1, y1, x2, y2, color, _weight) in &edge_data {
                let dx = x2 - x1;
                let dy = y2 - y1;
                let len = (dx * dx + dy * dy).sqrt();
                if len < 0.001 {
                    continue;
                }

                let segs = (len / 18.0).clamp(4.0, 14.0) as usize;
                for s in 0..segs {
                    let t0 = s as f64 / segs as f64;
                    let t1 = (s + 1) as f64 / segs as f64;
                    let mid_t = (t0 + t1) / 2.0;
                    let brightness = 0.35 + 0.65 * mid_t;
                    let sc = scale_color(color, brightness);
                    ctx.draw(&CLine {
                        x1: x1 + dx * t0,
                        y1: y1 + dy * t0,
                        x2: x1 + dx * t1,
                        y2: y1 + dy * t1,
                        color: sc,
                    });
                }
            }
        });

    frame.render_widget(canvas, area);

    let inner_w_cells = area.width.saturating_sub(2);
    let inner_h_cells = area.height.saturating_sub(2);
    if inner_w_cells > 0 && inner_h_cells > 0 && canvas_width > 0.0 && canvas_height > 0.0 {
        let res_x = inner_w_cells as f64 * 2.0;
        let res_y = inner_h_cells as f64 * 4.0;

        let inner_left = area.x + 1;
        let inner_top = area.y + 1;
        let inner_right = area.x + area.width.saturating_sub(1);
        let inner_bottom = area.y + area.height.saturating_sub(1);

        let buf = frame.buffer_mut();

        for (sx, sy, label, color, is_matched) in &node_points {
            let grid_x = (*sx * (res_x - 1.0) / canvas_width) as u16;
            let grid_y = ((canvas_height - *sy) * (res_y - 1.0) / canvas_height) as u16;

            let col = inner_left + grid_x / 2;
            let row = inner_top + grid_y / 4;

            if col >= inner_left && col < inner_right && row >= inner_top && row < inner_bottom {
                let marker = if *is_matched { "◆" } else { "●" };
                buf.set_string(col, row, marker, Style::default().fg(*color).bg(BG_SURFACE));
                if !label.is_empty() {
                    let label_col = col + 2;
                    if label_col < inner_right {
                        let max_chars = (inner_right - label_col) as usize;
                        let text: &str = if label.len() > max_chars {
                            &label[..max_chars]
                        } else {
                            label.as_str()
                        };
                        let label_fg = if *is_matched { *color } else { FG_TEXT };
                        buf.set_string(
                            label_col,
                            row,
                            text,
                            Style::default().fg(label_fg).bg(BG_SURFACE),
                        );
                    }
                }
            }
        }
    }
}

/// Renders the search overlay.
fn render_search_overlay(frame: &mut Frame, graph_area: Rect, query: &str) {
    let bar_width = (query.len() as u16 + 4)
        .max(20)
        .min(graph_area.width.saturating_sub(4));
    let bar_y = graph_area.y + graph_area.height.saturating_sub(2);
    let bar_area = Rect::new(graph_area.x + 2, bar_y, bar_width, 1);

    let line = Line::from(vec![
        Span::styled(
            "/",
            Style::default()
                .fg(ACCENT_MAUVE)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(query, Style::default().fg(FG_TEXT)),
        Span::styled("█", Style::default().fg(ACCENT_MAUVE)),
    ]);

    let bar = Paragraph::new(line).style(Style::default().bg(BG_SURFACE));
    frame.render_widget(bar, bar_area);
}

/// Renders the status bar.
fn render_status_bar(frame: &mut Frame, area: Rect, app: &App) {
    if area.height < 2 {
        return;
    }

    let status_area = Rect::new(area.x, area.height.saturating_sub(1), area.width, 1);

    let play_status = if app.is_playing { "> PLAY" } else { "|| PAUSE" };
    let commit_count = app.snapshots_metadata.len();
    let fps_info = format!("frame #{}", app.frame_count);

    let mut spans = vec![
        Span::styled(
            format!(" {play_status} "),
            Style::default()
                .fg(if app.is_playing {
                    ACCENT_BLUE
                } else {
                    FG_OVERLAY
                })
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" | ", Style::default().fg(FG_OVERLAY)),
        Span::styled(
            format!("{commit_count} commits"),
            Style::default().fg(ACCENT_LAVENDER),
        ),
        Span::styled(" | ", Style::default().fg(FG_OVERLAY)),
        Span::styled(fps_info, Style::default().fg(FG_OVERLAY)),
    ];

    if !app.search_query.is_empty() {
        spans.push(Span::styled(" | ", Style::default().fg(FG_OVERLAY)));
        spans.push(Span::styled(
            format!("/{}", app.search_query),
            Style::default()
                .fg(ACCENT_MAUVE)
                .add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::styled(
            " (Esc:clear)",
            Style::default().fg(FG_OVERLAY),
        ));
    }

    if app.is_playing {
        let speed_ms = app.auto_play_interval.as_millis();
        spans.push(Span::styled(" | ", Style::default().fg(FG_OVERLAY)));
        spans.push(Span::styled(
            format!("speed:{speed_ms}ms"),
            Style::default().fg(ACCENT_BLUE),
        ));
    }

    spans.push(Span::styled(" | ", Style::default().fg(FG_OVERLAY)));
    spans.push(Span::styled(
        "j/k:±1  h/l:±10  g/G:start/end  p:play  +/-:speed  /:search  q:quit",
        Style::default().fg(FG_OVERLAY),
    ));

    let status = Line::from(spans);
    let status_widget = Paragraph::new(status).style(Style::default().bg(BG_BASE));
    frame.render_widget(status_widget, status_area);
}

/// Main TUI event loop.
pub async fn run_tui(mut app: App) -> anyhow::Result<()> {
    use crossterm::ExecutableCommand;
    use crossterm::event::{DisableMouseCapture, EnableMouseCapture};
    use crossterm::terminal::{
        EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
    };
    use ratatui::Terminal;
    use ratatui::backend::CrosstermBackend;
    use std::io;

    enable_raw_mode()?;
    io::stdout().execute(EnterAlternateScreen)?;
    io::stdout().execute(EnableMouseCapture)?;
    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;

    let tick_rate = app.tick_rate;

    loop {
        terminal.draw(|frame| {
            let size = frame.area();

            let main_chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Min(10), Constraint::Length(4)])
                .split(size);

            let top_chunks = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([
                    Constraint::Length(app.pkg_panel_width),
                    Constraint::Min(30),
                    Constraint::Length(32),
                ])
                .split(main_chunks[0]);

            app.pkg_area = top_chunks[0];
            app.graph_area = top_chunks[1];
            app.timeline_area = main_chunks[1];

            render_app(frame, &mut app);
        })?;

        let timeout = tick_rate
            .checked_sub(app.last_tick.elapsed())
            .unwrap_or(Duration::ZERO);

        if event::poll(timeout)? {
            match event::read()? {
                Event::Key(key) if key.kind == event::KeyEventKind::Press => {
                    app.handle_key(key.code, key.modifiers);
                }
                Event::Mouse(mouse) => {
                    app.handle_mouse(mouse);
                }
                _ => {}
            }
        }

        if app.last_tick.elapsed() >= tick_rate {
            app.tick_physics();
            app.tick_auto_play();
            app.last_tick = Instant::now();
        }

        if app.should_quit {
            break;
        }
    }

    disable_raw_mode()?;
    io::stdout().execute(DisableMouseCapture)?;
    io::stdout().execute(LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    Ok(())
}

// =============================================================================
// Tests
// =============================================================================
#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{DependencyEdge, DriftScore, GraphSnapshot};

    fn make_test_snapshot(hash: &str, nodes: Vec<&str>, drift_total: u8) -> GraphSnapshot {
        GraphSnapshot {
            commit_hash: hash.to_string(),
            nodes: nodes.iter().map(|s| s.to_string()).collect(),
            edges: if nodes.len() >= 2 {
                vec![DependencyEdge {
                    from_module: nodes[0].to_string(),
                    to_module: nodes[1].to_string(),
                    file_path: "test.rs".to_string(),
                    line: 1,
                    weight: 1,
                }]
            } else {
                vec![]
            },
            node_count: nodes.len(),
            edge_count: if nodes.len() >= 2 { 1 } else { 0 },
            timestamp: 1_000_000,
            drift: Some(DriftScore {
                total: drift_total,
                fan_in_delta: 0,
                fan_out_delta: 0,
                new_cycles: 0,
                boundary_violations: 0,
                cognitive_complexity: 0.0,
                timestamp: 1_000_000,
            }),
        }
    }

    #[test]
    fn test_app_creation_empty() {
        let app = App::new(None, vec![]);
        assert!(app.snapshots_metadata.is_empty());
        assert!(app.timeline.is_empty());
        assert!(!app.is_playing);
        assert!(!app.should_quit);
        assert!(app.dragging_node.is_none());
    }

    #[test]
    fn test_app_creation_with_snapshots() {
        let snapshots = vec![
            make_test_snapshot("aaa", vec!["A", "B"], 30),
            make_test_snapshot("bbb", vec!["A", "B", "C"], 45),
        ];
        let app = App::new(None, snapshots);

        assert_eq!(app.snapshots_metadata.len(), 2);
        assert_eq!(app.graph_layout.labels.len(), 2);
        assert_eq!(app.timeline.len(), 2);
        assert!(app.snapshot_cache.contains_key("aaa"));
        assert!(app.snapshot_cache.contains_key("bbb"));
    }

    #[test]
    fn test_app_navigation() {
        let snapshots = vec![
            make_test_snapshot("aaa", vec!["A", "B"], 30),
            make_test_snapshot("bbb", vec!["A", "B", "C"], 45),
            make_test_snapshot("ccc", vec!["X", "Y"], 60),
        ];
        let mut app = App::new(None, snapshots);

        assert_eq!(app.timeline.current_index, 0);

        app.next_commit();
        assert_eq!(app.timeline.current_index, 1);
        assert_eq!(app.graph_layout.labels.len(), 3);

        app.next_commit();
        assert_eq!(app.timeline.current_index, 2);
        assert_eq!(app.graph_layout.labels.len(), 2);

        app.prev_commit();
        assert_eq!(app.timeline.current_index, 1);
    }

    #[test]
    fn test_app_key_handling_quit() {
        let mut app = App::new(None, vec![]);

        assert!(!app.should_quit);
        app.handle_key(KeyCode::Char('q'), KeyModifiers::NONE);
        assert!(app.should_quit);
    }

    #[test]
    fn test_app_key_handling_play_toggle() {
        let mut app = App::new(None, vec![]);

        assert!(!app.is_playing);
        app.handle_key(KeyCode::Char('p'), KeyModifiers::NONE);
        assert!(app.is_playing);
        app.handle_key(KeyCode::Char('p'), KeyModifiers::NONE);
        assert!(!app.is_playing);
    }

    #[test]
    fn test_app_reheat() {
        let snapshots = vec![make_test_snapshot("aaa", vec!["A", "B"], 30)];
        let mut app = App::new(None, snapshots);

        for _ in 0..200 {
            app.tick_physics();
        }
        let cold_temp = app.graph_layout.temperature;

        app.handle_key(KeyCode::Char('r'), KeyModifiers::NONE);
        assert!(app.graph_layout.temperature > cold_temp);
    }

    #[test]
    fn test_app_search_mode() {
        let mut app = App::new(None, vec![]);

        app.handle_key(KeyCode::Char('/'), KeyModifiers::NONE);
        assert!(app.show_search);

        app.handle_key(KeyCode::Char('t'), KeyModifiers::NONE);
        app.handle_key(KeyCode::Char('e'), KeyModifiers::NONE);
        assert_eq!(app.search_query, "te");

        app.handle_key(KeyCode::Backspace, KeyModifiers::NONE);
        assert_eq!(app.search_query, "t");

        app.handle_key(KeyCode::Esc, KeyModifiers::NONE);
        assert!(!app.show_search);
        assert!(app.search_query.is_empty());
    }

    #[test]
    fn test_snapshot_to_layout_data() {
        let snapshot = make_test_snapshot("abc", vec!["main", "serde"], 40);
        let (labels, edges, weights) = snapshot_to_layout_data(&snapshot);

        assert_eq!(labels, vec!["main", "serde"]);
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0], (0, 1));
        assert_eq!(weights.len(), 1);
        assert_eq!(weights[0], 1);
    }

    #[test]
    fn test_terminal_to_physics() {
        let app = App::new(None, vec![]);
        let (px, py) = app.terminal_to_physics(50, 25, 0, 0, 100, 50);
        assert!((px - 250.0).abs() < 0.1);
        assert!((py - 250.0).abs() < 0.1);
    }
}
