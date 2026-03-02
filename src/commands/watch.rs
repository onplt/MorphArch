// =============================================================================
// commands/watch.rs — Watch command: Scan + TUI launch
// =============================================================================
//
// Watch command flow:
//   1. Scan the repository (like the scan command — commit + graph + drift)
//   2. Load the last N graph snapshots from the database
//   3. Fetch commit info (hash + message + timestamp for the timeline)
//   4. Launch the TUI application (app::run_tui)
//
// This command runs inside a `tokio` async runtime (from main.rs).
// =============================================================================

use anyhow::{Context, Result};
use std::path::Path;
use tracing::info;

use crate::commands::scan::run_scan;
use crate::db::Database;
use crate::tui::app::{App, run_tui};

/// Number of graph snapshots to load (TUI timeline capacity).
const MAX_TIMELINE_SNAPSHOTS: usize = 50;

/// Runs the watch command: scan + TUI launch.
pub async fn run_watch(repo_path: &Path, db: &Database, max_commits: usize) -> Result<()> {
    // ── 1. Scan the repository ──
    info!(path = %repo_path.display(), "Watch: Scanning repository");
    let scan_result = run_scan(repo_path, db, max_commits)?;
    info!(
        commits = scan_result.commits_scanned,
        graphs = scan_result.graphs_created,
        drifts = scan_result.drifts_calculated,
        "Watch: Scan complete"
    );

    // ── 2. Load recent snapshots ──
    let snapshots = db
        .get_recent_snapshots(MAX_TIMELINE_SNAPSHOTS)
        .context("Failed to load graph snapshots")?;

    if snapshots.is_empty() {
        println!("No graph snapshots yet. Please scan a Git repository first.");
        return Ok(());
    }

    info!(count = snapshots.len(), "Watch: Snapshots loaded");

    // ── 3. Fetch commit info (for timeline) ──
    let timeline_commits: Vec<(String, String, i64)> = db
        .get_commit_messages_for_snapshots(&snapshots)
        .context("Failed to fetch commit info")?;

    // ── 4. Create and launch TUI ──
    let mut app = App::new(snapshots);
    app.set_timeline_commits(timeline_commits);

    info!("Watch: Launching TUI");
    run_tui(app).await?;

    info!("Watch: TUI closed");
    Ok(())
}

// =============================================================================
// Tests
// =============================================================================
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_max_timeline_snapshots_constant() {
        assert_eq!(MAX_TIMELINE_SNAPSHOTS, 50);
        assert!(MAX_TIMELINE_SNAPSHOTS > 0);
    }
}
