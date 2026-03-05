// =============================================================================
// tui/graph_renderer.rs — Verlet physics + Ratatui Canvas graph rendering
// =============================================================================
//
// Force-directed graph layout using Verlet integration:
//   1. Repulsion: Barnes-Hut — optimized O(V log V) repulsion using Quadtree
//   2. Attraction: Hooke spring — edge-connected nodes attract toward ideal length
//   3. Center gravity: Gentle pull toward canvas center prevents drift
//   4. Micro-jitter: Temperature-scaled random perturbation for organic feel
//   5. Damping: Velocity reduced each step for convergence
//   6. Bounds enforcement: Nodes stay within the drawing area
//   7. Temperature decay: Graph gradually settles; reheat for re-energize
//
// The ideal edge length adapts dynamically to node count and canvas area,
// ensuring good layouts for both small (3 nodes) and large (100+) graphs.
//
// Performance: O(V log V) repulsion + O(E) attraction
//
// Coloring: Green -> red gradient based on drift score (Catppuccin Mocha)
// =============================================================================

use rand::Rng;
use ratatui::style::Color;

/// 2D position and velocity for Verlet integration.
///
/// `prev_x/prev_y` encode velocity implicitly:
///   velocity = (pos - prev_pos) * damping
///   new_pos  = pos + velocity + acceleration
#[derive(Debug, Clone)]
pub struct NodePosition {
    pub x: f64,
    pub y: f64,
    pub prev_x: f64,
    pub prev_y: f64,
    /// Whether the node is pinned (for mouse drag)
    pub pinned: bool,
}

/// Barnes-Hut Quadtree for O(V log V) repulsion.
struct Quadtree {
    mass: f64,
    com_x: f64,
    com_y: f64,
    x: f64,
    y: f64,
    size: f64,
    children: Option<Box<[Quadtree; 4]>>,
    node_idx: Option<usize>,
    depth: usize,
}

const MAX_QUADTREE_DEPTH: usize = 20;

impl Quadtree {
    fn new(x: f64, y: f64, size: f64, depth: usize) -> Self {
        Self {
            mass: 0.0,
            com_x: 0.0,
            com_y: 0.0,
            x,
            y,
            size,
            children: None,
            node_idx: None,
            depth,
        }
    }

    fn insert(&mut self, idx: usize, px: f64, py: f64) {
        // If this node already has mass but no children, we need to split
        // or just add to the current node if we've reached max depth.
        if self.mass > 0.0 && self.children.is_none() {
            // Check depth to prevent infinite recursion if positions are identical
            if self.depth >= MAX_QUADTREE_DEPTH {
                // At max depth, just update center of mass/mass but don't split.
                // This essentially treats the nodes as a single mass at this point.
                let new_mass = self.mass + 1.0;
                self.com_x = (self.com_x * self.mass + px) / new_mass;
                self.com_y = (self.com_y * self.mass + py) / new_mass;
                self.mass = new_mass;
                return;
            }

            let half = self.size / 2.0;
            let d = self.depth + 1;
            let mut children = Box::new([
                Quadtree::new(self.x, self.y, half, d),
                Quadtree::new(self.x + half, self.y, half, d),
                Quadtree::new(self.x, self.y + half, half, d),
                Quadtree::new(self.x + half, self.y + half, half, d),
            ]);

            if let Some(old_idx) = self.node_idx.take() {
                let ox = self.com_x;
                let oy = self.com_y;
                let q = self.get_quadrant(ox, oy);
                children[q].insert(old_idx, ox, oy);
            }
            self.children = Some(children);
        }

        let q = self.get_quadrant(px, py);
        if let Some(children) = &mut self.children {
            children[q].insert(idx, px, py);
        } else {
            // First node in this leaf
            self.node_idx = Some(idx);
            self.com_x = px;
            self.com_y = py;
        }

        // Update center of mass
        let new_mass = self.mass + 1.0;
        self.com_x = (self.com_x * self.mass + px) / new_mass;
        self.com_y = (self.com_y * self.mass + py) / new_mass;
        self.mass = new_mass;
    }

    fn get_quadrant(&self, px: f64, py: f64) -> usize {
        let mid_x = self.x + self.size / 2.0;
        let mid_y = self.y + self.size / 2.0;
        match (px >= mid_x, py >= mid_y) {
            (false, false) => 0,
            (true, false) => 1,
            (false, true) => 2,
            (true, true) => 3,
        }
    }

    fn compute_repulsion(
        &self,
        idx: usize,
        pos: (f64, f64),
        theta: f64,
        repulsion_const: f64,
        force: &mut (f64, f64),
    ) {
        if self.mass == 0.0 || (self.node_idx == Some(idx)) {
            return;
        }

        let dx = self.com_x - pos.0;
        let dy = self.com_y - pos.1;
        let dist_sq = dx * dx + dy * dy;
        let dist = dist_sq.sqrt().max(0.5);

        // Barnes-Hut criterion: s / d < theta
        if self.children.is_none() || (self.size / dist < theta) {
            let f = (repulsion_const * self.mass) / dist_sq.max(1.0);
            force.0 -= f * (dx / dist);
            force.1 -= f * (dy / dist);
        } else if let Some(children) = &self.children {
            for child in children.iter() {
                child.compute_repulsion(idx, pos, theta, repulsion_const, force);
            }
        }
    }
}

/// Force-directed graph layout engine using Verlet integration.
pub struct GraphLayout {
    /// Node positions (index = graph order)
    pub positions: Vec<NodePosition>,
    /// Edge list: (from_index, to_index) pairs
    pub edges: Vec<(usize, usize)>,
    /// Edge weights: `weight[i]` corresponds to `edges[i]` (import count)
    pub edge_weights: Vec<u32>,
    /// Node labels (module names)
    pub labels: Vec<String>,
    /// Repulsion coefficient (Coulomb constant)
    pub repulsion: f64,
    /// Attraction coefficient (Hooke spring constant)
    pub attraction: f64,
    /// Velocity damping factor (0.0 = full stop, 1.0 = no damping)
    pub damping: f64,
    /// Dynamic ideal edge length based on node count and area
    pub ideal_length: f64,
    /// Drawing area width
    pub width: f64,
    /// Drawing area height
    pub height: f64,
    /// Simulation temperature (1.0 = hot/active, 0.0 = frozen).
    /// Scales jitter and force magnitude. Decays each step for natural settling.
    pub temperature: f64,
}

impl GraphLayout {
    /// Creates a new graph layout with circular initial placement.
    ///
    /// Nodes start on a circle around the center with small jitter
    /// and initial velocity for immediate visible motion.
    pub fn new(
        labels: Vec<String>,
        edges: Vec<(usize, usize)>,
        edge_weights: Vec<u32>,
        width: f64,
        height: f64,
    ) -> Self {
        let mut rng = rand::rng();
        let n = labels.len();

        // Place nodes in a circle with generous radius for spread
        let radius = width.min(height) * 0.40;
        let cx = width / 2.0;
        let cy = height / 2.0;

        let positions: Vec<NodePosition> = labels
            .iter()
            .enumerate()
            .map(|(i, _)| {
                // Distribute evenly on circle with angle jitter
                let angle = (i as f64 / n.max(1) as f64) * std::f64::consts::TAU
                    + rng.random_range(-0.15..0.15);
                let r = radius + rng.random_range(-5.0..5.0);
                let x = cx + angle.cos() * r;
                let y = cy + angle.sin() * r;
                // Small initial velocity for immediate motion
                NodePosition {
                    x,
                    y,
                    prev_x: x + rng.random_range(-1.5..1.5),
                    prev_y: y + rng.random_range(-1.5..1.5),
                    pinned: false,
                }
            })
            .collect();

        let ideal_length = compute_ideal_length(n, width, height);

        Self {
            positions,
            edges,
            edge_weights,
            labels,
            repulsion: 1500.0,
            attraction: 0.045,
            damping: 0.82,
            ideal_length,
            width,
            height,
            temperature: 1.0,
        }
    }

    /// Advances one Verlet physics step.
    ///
    /// # Algorithm
    /// 1. Barnes-Hut repulsion using Quadtree (O(V log V))
    /// 2. Hooke spring attraction for every edge (O(E))
    /// 3. Center gravity pull (O(V))
    /// 4. Temperature-scaled micro-jitter (O(V))
    /// 5. Verlet integration with velocity/force clamping (O(V))
    /// 6. Bounds enforcement (O(V))
    /// 7. Temperature decay
    pub fn step(&mut self) {
        let n = self.positions.len();
        if n == 0 {
            return;
        }

        let mut fx = vec![0.0f64; n];
        let mut fy = vec![0.0f64; n];

        // Temperature-scaled force multiplier (hot = stronger forces for exploration)
        let temp_scale = 0.5 + self.temperature * 0.5; // range [0.5, 1.0]

        // 1. Barnes-Hut Repulsion: build quadtree then compute forces
        let q_size = self.width.max(self.height).max(1.0);
        let mut qt = Quadtree::new(0.0, 0.0, q_size, 0);
        for (i, pos) in self.positions.iter().enumerate() {
            qt.insert(i, pos.x, pos.y);
        }

        let theta = 0.7; // Barnes-Hut approximation threshold
        let repulsion_const = self.repulsion * temp_scale;

        for i in 0..n {
            let mut f = (0.0, 0.0);
            qt.compute_repulsion(
                i,
                (self.positions[i].x, self.positions[i].y),
                theta,
                repulsion_const,
                &mut f,
            );
            fx[i] += f.0;
            fy[i] += f.1;
        }

        // 2. Attraction (Hooke spring): connected nodes attract toward ideal length
        for &(from, to) in &self.edges {
            if from >= n || to >= n {
                continue;
            }
            let dx = self.positions[to].x - self.positions[from].x;
            let dy = self.positions[to].y - self.positions[from].y;
            let dist = (dx * dx + dy * dy).sqrt().max(0.5);

            let displacement = dist - self.ideal_length;
            let force = self.attraction * displacement;
            let ux = dx / dist;
            let uy = dy / dist;

            fx[from] += force * ux;
            fy[from] += force * uy;
            fx[to] -= force * ux;
            fy[to] -= force * uy;
        }

        // 3. Center gravity: gentle pull toward canvas center
        let cx = self.width / 2.0;
        let cy = self.height / 2.0;
        let gravity = 0.02;

        for i in 0..n {
            fx[i] += (cx - self.positions[i].x) * gravity;
            fy[i] += (cy - self.positions[i].y) * gravity;
        }

        // 4. Temperature-scaled micro-jitter: alive feel that fades as graph settles
        let mut rng = rand::rng();
        let jitter = 0.15 * self.temperature;
        if jitter > 0.001 {
            for i in 0..n {
                fx[i] += rng.random_range(-jitter..jitter);
                fy[i] += rng.random_range(-jitter..jitter);
            }
        }

        // 5. Verlet integration with clamping
        let max_disp = 5.0;
        for i in 0..n {
            if self.positions[i].pinned {
                continue;
            }

            let vx = (self.positions[i].x - self.positions[i].prev_x) * self.damping;
            let vy = (self.positions[i].y - self.positions[i].prev_y) * self.damping;

            // Clamp velocity and force to prevent explosions
            let vx = vx.clamp(-max_disp, max_disp);
            let vy = vy.clamp(-max_disp, max_disp);
            let fxi = fx[i].clamp(-max_disp, max_disp);
            let fyi = fy[i].clamp(-max_disp, max_disp);

            let new_x = self.positions[i].x + vx + fxi;
            let new_y = self.positions[i].y + vy + fyi;

            self.positions[i].prev_x = self.positions[i].x;
            self.positions[i].prev_y = self.positions[i].y;
            self.positions[i].x = new_x;
            self.positions[i].y = new_y;
        }

        // 6. Bounds enforcement (zero velocity at wall to prevent sticking)
        let margin = 8.0;
        for pos in &mut self.positions {
            if pos.x < margin {
                pos.x = margin;
                pos.prev_x = margin;
            } else if pos.x > self.width - margin {
                pos.x = self.width - margin;
                pos.prev_x = self.width - margin;
            }
            if pos.y < margin {
                pos.y = margin;
                pos.prev_y = margin;
            } else if pos.y > self.height - margin {
                pos.y = self.height - margin;
                pos.prev_y = self.height - margin;
            }
        }

        // 7. Temperature decay — graph settles quickly then freezes.
        // Rate 0.997 per step: with 90 steps/sec (~30fps * 3 substeps),
        // temperature drops to ~7% after 10s, ~2% after 15s → physics freezes.
        self.temperature = (self.temperature * 0.997).max(0.01);
    }

    /// Runs multiple physics steps at once (for warmup or faster convergence).
    pub fn multi_step(&mut self, count: usize) {
        for _ in 0..count {
            self.step();
        }
    }

    /// Sets temperature to a high value, re-energizing the simulation.
    ///
    /// Call this when the user presses 'r' or when the graph changes
    /// to make nodes spread out and find a new equilibrium.
    pub fn reheat(&mut self) {
        self.temperature = 1.5;
    }

    /// Re-places all nodes on a circle at the current width/height.
    ///
    /// Call this after `resize()` but before warmup so the initial
    /// placement matches the actual canvas dimensions exactly.
    pub fn reinitialize_positions(&mut self) {
        let mut rng = rand::rng();
        let n = self.labels.len();
        let radius = self.width.min(self.height) * 0.35;
        let cx = self.width / 2.0;
        let cy = self.height / 2.0;

        self.positions = (0..n)
            .map(|i| {
                let angle = (i as f64 / n.max(1) as f64) * std::f64::consts::TAU
                    + rng.random_range(-0.15..0.15);
                let r = radius + rng.random_range(-5.0..5.0);
                let x = cx + angle.cos() * r;
                let y = cy + angle.sin() * r;
                NodePosition {
                    x,
                    y,
                    prev_x: x + rng.random_range(-1.5..1.5),
                    prev_y: y + rng.random_range(-1.5..1.5),
                    pinned: false,
                }
            })
            .collect();

        self.temperature = 1.0;
        self.ideal_length = compute_ideal_length(n, self.width, self.height);
    }

    /// Shifts all node positions so the centroid sits at the canvas center.
    ///
    /// Force-directed layouts can settle slightly off-center due to
    /// asymmetric forces. This post-warmup correction eliminates visible drift.
    pub fn center_layout(&mut self) {
        let n = self.positions.len();
        if n == 0 {
            return;
        }
        let avg_x: f64 = self.positions.iter().map(|p| p.x).sum::<f64>() / n as f64;
        let avg_y: f64 = self.positions.iter().map(|p| p.y).sum::<f64>() / n as f64;
        let dx = self.width / 2.0 - avg_x;
        let dy = self.height / 2.0 - avg_y;

        let margin = 8.0;
        for pos in &mut self.positions {
            pos.x = (pos.x + dx).clamp(margin, self.width - margin);
            pos.y = (pos.y + dy).clamp(margin, self.height - margin);
            pos.prev_x = (pos.prev_x + dx).clamp(margin, self.width - margin);
            pos.prev_y = (pos.prev_y + dy).clamp(margin, self.height - margin);
        }
    }

    /// Updates drawing area dimensions, rescaling positions proportionally.
    pub fn resize(&mut self, width: f64, height: f64) {
        if (self.width - width).abs() < 1.0 && (self.height - height).abs() < 1.0 {
            return;
        }
        // Skip if dimensions haven't meaningfully changed
        if (self.width - width).abs() < 0.5 && (self.height - height).abs() < 0.5 {
            return;
        }
        // Rescale existing positions to fit new dimensions
        if self.width > 0.0 && self.height > 0.0 {
            let sx = width / self.width;
            let sy = height / self.height;
            for pos in &mut self.positions {
                pos.x *= sx;
                pos.y *= sy;
                pos.prev_x *= sx;
                pos.prev_y *= sy;
            }
        }
        self.width = width;
        self.height = height;
        self.ideal_length = compute_ideal_length(self.labels.len(), width, height);
    }

    /// Updates the layout with a new graph snapshot.
    ///
    /// Preserves existing positions where possible (name matching).
    /// New nodes are placed at random positions near center.
    /// Reheats temperature to 0.8 so the new layout settles organically.
    pub fn update_graph(
        &mut self,
        labels: Vec<String>,
        edges: Vec<(usize, usize)>,
        edge_weights: Vec<u32>,
    ) {
        use std::collections::HashMap;

        // Save old positions by name
        let old_positions: HashMap<&str, &NodePosition> = self
            .labels
            .iter()
            .zip(self.positions.iter())
            .map(|(l, p)| (l.as_str(), p))
            .collect();

        let mut rng = rand::rng();
        let cx = self.width / 2.0;
        let cy = self.height / 2.0;

        // Create new positions, preserving known nodes
        let new_positions: Vec<NodePosition> = labels
            .iter()
            .map(|label| {
                if let Some(old) = old_positions.get(label.as_str()) {
                    NodePosition {
                        x: old.x,
                        y: old.y,
                        prev_x: old.prev_x,
                        prev_y: old.prev_y,
                        pinned: false,
                    }
                } else {
                    // New node: random position near center with initial velocity
                    let x = cx + rng.random_range(-30.0..30.0);
                    let y = cy + rng.random_range(-30.0..30.0);
                    NodePosition {
                        x,
                        y,
                        prev_x: x + rng.random_range(-1.0..1.0),
                        prev_y: y + rng.random_range(-1.0..1.0),
                        pinned: false,
                    }
                }
            })
            .collect();

        self.positions = new_positions;
        self.labels = labels;
        self.edges = edges;
        self.edge_weights = edge_weights;
        self.ideal_length = compute_ideal_length(self.labels.len(), self.width, self.height);
        // Warm reheat so the new graph spreads out
        self.temperature = 0.8;
    }
}

/// Computes ideal edge length based on node count and canvas area.
///
/// Uses the Fruchterman-Reingold heuristic: k = C * sqrt(area / n),
/// clamped to a reasonable range for the canvas dimensions.
fn compute_ideal_length(node_count: usize, width: f64, height: f64) -> f64 {
    if node_count <= 1 {
        return width.min(height) * 0.3;
    }
    let area = width * height;
    let k = 0.7 * (area / node_count as f64).sqrt();
    k.clamp(25.0, width.min(height) * 0.45)
}

/// Returns the node color based on drift score.
///
/// Catppuccin Mocha color theme:
/// - 0-30:  Green (#a6e3a1)
/// - 31-60: Yellow (#f9e2af)
/// - 61-80: Peach  (#fab387)
/// - 81+:   Red    (#f38ba8)
pub fn drift_color(drift_score: u8) -> Color {
    match drift_score {
        0..=30 => Color::Rgb(166, 227, 161),  // Catppuccin Green
        31..=60 => Color::Rgb(249, 226, 175), // Catppuccin Yellow
        61..=80 => Color::Rgb(250, 179, 135), // Catppuccin Peach
        _ => Color::Rgb(243, 139, 168),       // Catppuccin Red
    }
}

/// Returns an edge color scaled by weight (import count).
///
/// - weight 1:   dim overlay (low-traffic dependency)
/// - weight 2-3: medium Sapphire
/// - weight 4+:  bright Sapphire (high-traffic dependency)
pub fn weighted_edge_color(weight: u32) -> Color {
    match weight {
        1 => Color::Rgb(88, 91, 112),       // Catppuccin Overlay0 — dim
        2..=3 => Color::Rgb(116, 199, 236), // Catppuccin Sapphire — normal
        _ => Color::Rgb(137, 220, 255),     // Bright Sapphire — heavy
    }
}

// =============================================================================
// Catppuccin Mocha color palette — used throughout the TUI
// =============================================================================

/// Catppuccin Mocha background color
pub const BG_BASE: Color = Color::Rgb(30, 30, 46);
/// Catppuccin Mocha text color
pub const FG_TEXT: Color = Color::Rgb(205, 214, 244);
/// Catppuccin Mocha surface color (panel backgrounds)
pub const BG_SURFACE: Color = Color::Rgb(36, 39, 58);
/// Catppuccin Mocha blue accent
pub const ACCENT_BLUE: Color = Color::Rgb(137, 180, 250);
/// Catppuccin Mocha lavender
pub const ACCENT_LAVENDER: Color = Color::Rgb(180, 190, 254);
/// Catppuccin Mocha mauve
pub const ACCENT_MAUVE: Color = Color::Rgb(203, 166, 247);
/// Catppuccin Mocha overlay — secondary text
pub const FG_OVERLAY: Color = Color::Rgb(108, 112, 134);

/// Catppuccin node color palette for individual node coloring
pub const NODE_PALETTE: [Color; 8] = [
    Color::Rgb(148, 226, 213), // Teal
    Color::Rgb(137, 180, 250), // Blue
    Color::Rgb(203, 166, 247), // Mauve
    Color::Rgb(166, 227, 161), // Green
    Color::Rgb(249, 226, 175), // Yellow
    Color::Rgb(250, 179, 135), // Peach
    Color::Rgb(243, 139, 168), // Pink
    Color::Rgb(180, 190, 254), // Lavender
];

// =============================================================================
// Tests
// =============================================================================
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_graph_layout_creation() {
        let labels = vec!["A".to_string(), "B".to_string(), "C".to_string()];
        let edges = vec![(0, 1), (1, 2)];
        let weights = vec![1, 2];
        let layout = GraphLayout::new(labels.clone(), edges, weights, 200.0, 100.0);

        assert_eq!(layout.positions.len(), 3, "Should have 3 node positions");
        assert_eq!(layout.labels.len(), 3);
        assert_eq!(layout.edges.len(), 2);
        assert_eq!(layout.edge_weights.len(), 2);
        assert!(layout.ideal_length > 0.0, "Ideal length should be positive");
        assert!(
            (layout.temperature - 1.0).abs() < 0.001,
            "Initial temperature should be 1.0"
        );
    }

    #[test]
    fn test_verlet_step_convergence() {
        let labels = vec!["A".to_string(), "B".to_string()];
        let edges = vec![(0, 1)];
        let weights = vec![1];
        let mut layout = GraphLayout::new(labels, edges, weights, 200.0, 100.0);

        let initial_ax = layout.positions[0].x;

        // Advance 100 steps — positions should change
        for _ in 0..100 {
            layout.step();
        }

        let final_ax = layout.positions[0].x;
        assert!(
            layout.positions[0].x >= 0.0 && layout.positions[0].x <= 200.0,
            "X should be within bounds"
        );
        assert!(
            layout.positions[0].y >= 0.0 && layout.positions[0].y <= 100.0,
            "Y should be within bounds"
        );

        let moved = (final_ax - initial_ax).abs() > 0.001;
        assert!(moved, "Nodes should move");
    }

    #[test]
    fn test_multi_step() {
        let labels = vec!["A".to_string(), "B".to_string()];
        let edges = vec![(0, 1)];
        let weights = vec![1];
        let mut layout = GraphLayout::new(labels, edges, weights, 200.0, 100.0);

        let initial_ax = layout.positions[0].x;
        layout.multi_step(50);

        let moved = (layout.positions[0].x - initial_ax).abs() > 0.001;
        assert!(moved, "Nodes should move after multi_step");
    }

    #[test]
    fn test_bounds_enforcement() {
        let labels = vec!["A".to_string()];
        let layout = GraphLayout::new(labels, vec![], vec![], 100.0, 50.0);

        let pos = &layout.positions[0];
        assert!(pos.x >= 0.0 && pos.x <= 100.0);
        assert!(pos.y >= 0.0 && pos.y <= 50.0);
    }

    #[test]
    fn test_drift_color_ranges() {
        assert_eq!(drift_color(0), Color::Rgb(166, 227, 161));
        assert_eq!(drift_color(30), Color::Rgb(166, 227, 161));
        assert_eq!(drift_color(31), Color::Rgb(249, 226, 175));
        assert_eq!(drift_color(60), Color::Rgb(249, 226, 175));
        assert_eq!(drift_color(61), Color::Rgb(250, 179, 135));
        assert_eq!(drift_color(81), Color::Rgb(243, 139, 168));
        assert_eq!(drift_color(100), Color::Rgb(243, 139, 168));
    }

    #[test]
    fn test_empty_graph_step() {
        let mut layout = GraphLayout::new(vec![], vec![], vec![], 100.0, 100.0);
        layout.step(); // Should not crash on empty graph
        assert_eq!(layout.positions.len(), 0);
    }

    #[test]
    fn test_update_graph_preserves_positions() {
        let labels = vec!["A".to_string(), "B".to_string()];
        let edges = vec![(0, 1)];
        let weights = vec![1];
        let mut layout = GraphLayout::new(labels, edges, weights, 200.0, 100.0);

        // Advance a few steps
        for _ in 0..10 {
            layout.step();
        }
        let a_pos = layout.positions[0].x;

        // Update graph — A still exists, C is new
        let new_labels = vec!["A".to_string(), "C".to_string()];
        let new_edges = vec![(0, 1)];
        let new_weights = vec![1];
        layout.update_graph(new_labels, new_edges, new_weights);

        assert_eq!(layout.positions.len(), 2);
        // A's position should be preserved
        assert!(
            (layout.positions[0].x - a_pos).abs() < 0.01,
            "A position should be preserved"
        );
    }

    #[test]
    fn test_compute_ideal_length() {
        let k = compute_ideal_length(3, 100.0, 100.0);
        assert!(k > 15.0, "Ideal length should be reasonable: {k}");
        assert!(k < 50.0, "Ideal length should not be too large: {k}");

        let k1 = compute_ideal_length(1, 100.0, 100.0);
        assert!(k1 > 0.0, "Single node should have positive ideal length");
    }

    #[test]
    fn test_resize_rescales() {
        let labels = vec!["A".to_string(), "B".to_string()];
        let mut layout = GraphLayout::new(labels, vec![], vec![], 100.0, 100.0);

        // Set a known position
        layout.positions[0].x = 50.0;
        layout.positions[0].y = 50.0;
        layout.positions[0].prev_x = 50.0;
        layout.positions[0].prev_y = 50.0;

        layout.resize(200.0, 100.0);
        assert!(
            (layout.positions[0].x - 100.0).abs() < 0.01,
            "X should double when width doubles"
        );
        assert!(
            (layout.positions[0].y - 50.0).abs() < 0.01,
            "Y should stay same when height unchanged"
        );
    }

    #[test]
    fn test_temperature_decay() {
        let labels = vec!["A".to_string(), "B".to_string()];
        let mut layout = GraphLayout::new(labels, vec![(0, 1)], vec![1], 200.0, 100.0);

        let initial_temp = layout.temperature;
        layout.multi_step(100);

        assert!(
            layout.temperature < initial_temp,
            "Temperature should decay over time"
        );
        assert!(
            layout.temperature >= 0.01,
            "Temperature should not drop below minimum"
        );
    }

    #[test]
    fn test_reheat() {
        let labels = vec!["A".to_string(), "B".to_string()];
        let mut layout = GraphLayout::new(labels, vec![(0, 1)], vec![1], 200.0, 100.0);

        // Cool down
        layout.multi_step(200);
        let cold_temp = layout.temperature;

        // Reheat
        layout.reheat();
        assert!(
            layout.temperature > cold_temp,
            "Temperature should increase after reheat"
        );
        assert!(
            (layout.temperature - 1.5).abs() < 0.001,
            "Reheat should set temperature to 1.5"
        );
    }
}
