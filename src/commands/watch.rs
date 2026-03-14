// =============================================================================
// commands/watch.rs - Watch command: scan + TUI launch
// =============================================================================

use std::path::Path;

use anyhow::{Context, Result};
use tracing::info;

use crate::commands::scan::run_scan;
use crate::config::ProjectConfig;
use crate::db::Database;
use crate::tui::app::{App, run_tui};

/// Runs the watch command: scan + TUI launch.
pub async fn run_watch(
    repo_path: &Path,
    repo_id: &str,
    cache_dir: &Path,
    db: Database,
    max_commits: usize,
    max_snapshots: usize,
    project_config: &ProjectConfig,
) -> Result<()> {
    info!(path = %repo_path.display(), "Watch: scanning repository");
    let scan_result = run_scan(
        repo_path,
        repo_id,
        cache_dir,
        &db,
        max_commits,
        project_config,
    )?;
    info!(
        commits = scan_result.commits_scanned,
        graphs = scan_result.graphs_created,
        drifts = scan_result.drifts_calculated,
        "Watch: scan complete"
    );

    let metadata = db
        .get_sampled_snapshot_metadata(repo_id, max_snapshots)
        .context("Failed to load graph snapshot metadata")?;
    if metadata.is_empty() {
        println!("No graph snapshots yet. Please scan a Git repository first.");
        return Ok(());
    }
    info!(count = metadata.len(), "Watch: timeline metadata loaded");

    let initial_snapshot = db
        .get_graph_snapshot(repo_id, &metadata[0].commit_hash)
        .context("Failed to load initial graph snapshot")?;
    let timeline_commits = db
        .get_commit_messages_for_metadata(repo_id, &metadata)
        .context("Failed to fetch commit info")?;

    let mut app = App::new(Some(db), repo_id.to_string(), metadata, initial_snapshot);
    app.set_skipped_snapshot_count(0);
    app.set_timeline_commits(timeline_commits);
    app.set_scoring_config(project_config.scoring.clone());
    app.set_clustering_config(project_config.clustering.clone());

    let repo_name = repo_path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "repo".to_string());
    app.set_repo_name(repo_name);

    info!("Watch: launching TUI");
    run_tui(app).await?;
    info!("Watch: TUI closed");
    Ok(())
}
