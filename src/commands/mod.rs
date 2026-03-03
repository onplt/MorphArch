// =============================================================================
// commands/mod.rs — Subcommand modules
// =============================================================================
//
// Each CLI subcommand lives in a separate module:
//   scan    → Git repo scanning + dependency graph + drift score
//   analyze → Detailed drift report
//   watch   → Scan + animated TUI launch
//
// Future additions:
//   diff  → Graph comparison between two commits
// =============================================================================

pub mod analyze;
pub mod scan;
pub mod watch;
