// =============================================================================
// tui/mod.rs — Ratatui TUI module declarations
// =============================================================================
//
// interactive terminal interface:
//   app             → Main application state + event loop
//   graph_renderer  → Verlet physics + Canvas graph rendering
//   timeline        → Commit timeline slider widget
//   insight_panel   → Drift score + recommendation panel
//   widgets         → Shared widget helpers
// =============================================================================

pub mod app;
pub mod graph_renderer;
pub mod insight_panel;
pub mod timeline;
pub mod widgets;
