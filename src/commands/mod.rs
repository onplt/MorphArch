// =============================================================================
// commands/mod.rs — Subcommand modules
// =============================================================================
//
// Each CLI subcommand lives in a separate module:
//   scan    → Git repo scanning + dependency graph + drift score (Sprint 2-3)
//   analyze → Detailed drift report (Sprint 3)
//   watch   → Scan + animated TUI launch (Sprint 4)
//
// Future additions:
//   diff  → Graph comparison between two commits (Sprint 5)
// =============================================================================

pub mod analyze;
pub mod scan;
pub mod watch;
