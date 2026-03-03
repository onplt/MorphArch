//! # MorphArch
//!
//! Monorepo architecture drift visualizer with animated TUI.
//!
//! MorphArch scans Git history, builds per-commit dependency graphs using
//! tree-sitter AST parsing, calculates architecture drift scores, and renders
//! the results as an animated force-directed graph in your terminal.
//!
//! ## Supported Languages
//!
//! - **Rust** — `use` / `extern crate` statements
//! - **TypeScript** — `import ... from` statements
//! - **Python** — `import` / `from ... import` statements
//! - **Go** — `import` declarations

mod cli;
mod commands;
mod config;
mod db;
mod git_scanner;
mod graph_builder;
mod models;
mod parser;
mod scoring;
mod tui;
mod utils;

use std::process;
use std::time::Instant;

use anyhow::Result;
use clap::Parser;
use tracing::info;

use cli::{Cli, Commands};
use config::MorphArchConfig;
use db::Database;

fn main() {
    // Initialize logging — before anything else
    utils::init_logging();

    // Run business logic; on error, print and exit
    if let Err(err) = run() {
        utils::print_error(&err);
        process::exit(1);
    }
}

/// Main business logic — returns anyhow::Result; main() catches errors.
///
/// This separation ensures all errors are caught at a single point (main)
/// and displayed in a user-friendly format.
fn run() -> Result<()> {
    // Parse CLI arguments
    let cli = Cli::parse();

    // Load configuration (~/.morpharch/ directory created automatically)
    let config = MorphArchConfig::load()?;
    info!(db_path = %config.db_path.display(), "Configuration ready");

    // Open SQLite database (table migrations run automatically)
    let db = Database::open(&config.db_path)?;

    // Dispatch to subcommand
    match cli.command {
        Commands::Scan { path, max_commits } => {
            let limit = if max_commits == 0 {
                usize::MAX
            } else {
                max_commits
            };
            execute_scan(&path, &db, limit)?;
        }
        Commands::Watch {
            path,
            max_commits,
            max_snapshots,
        } => {
            let limit = if max_commits == 0 {
                usize::MAX
            } else {
                max_commits
            };
            // Scan + launch animated TUI
            let rt = tokio::runtime::Runtime::new()?;
            rt.block_on(commands::watch::run_watch(&path, &db, limit, max_snapshots))?;
        }
        Commands::ListGraphs => {
            execute_list_graphs(&db)?;
        }
        Commands::Analyze { commit, path } => {
            commands::analyze::run_analyze(&path, commit.as_deref(), &db)?;
        }
        Commands::ListDrift => {
            execute_list_drift(&db)?;
        }
    }

    Ok(())
}

/// Executes the scan and prints a result summary.
///
/// commit scanning + dependency graph + drift scoring
/// all run in a single command. `commands::scan::run_scan` orchestrates
/// all three steps.
fn execute_scan(path: &std::path::Path, db: &Database, max_commits: usize) -> Result<()> {
    println!("Scanning repository: {}", path.display());
    println!();

    // Start timer
    let start = Instant::now();

    // commit scanning + dependency graph + drift scoring
    let result = commands::scan::run_scan(path, db, max_commits)?;

    // Calculate elapsed time
    let elapsed = start.elapsed();

    // Total record counts in database
    let total_commits = db.commit_count()?;
    let total_graphs = db.graph_snapshot_count()?;

    // Result summary
    println!(
        "Done: {} commits scanned, {} graphs + {} drift scores calculated in {:.1}s",
        result.commits_scanned,
        result.graphs_created,
        result.drifts_calculated,
        elapsed.as_secs_f64()
    );

    if total_commits > result.commits_scanned {
        println!(
            "Database totals: {} commits, {} graph snapshots stored",
            total_commits, total_graphs
        );
    }

    Ok(())
}

/// Lists recent graph snapshots in table format.
///
/// Fetches the last 10 graph snapshots from the database and displays:
/// - First 7 characters of commit hash
/// - First line of commit message (max 50 characters)
/// - Node count
/// - Edge count
/// - Date (Unix timestamp → readable format)
fn execute_list_graphs(db: &Database) -> Result<()> {
    let total = db.graph_snapshot_count()?;

    if total == 0 {
        println!("No graph snapshots yet. Run 'morpharch scan <path>' first.");
        return Ok(());
    }

    let graphs = db.list_recent_graphs(10)?;

    println!("Recent graph snapshots ({total} total):");
    println!();
    let header = format!(
        "{:<9} {:<50} {:>6} {:>6}   {}",
        "HASH", "MESSAGE", "NODES", "EDGES", "DATE"
    );
    println!("{header}");
    let separator = "─".repeat(95);
    println!("{separator}");

    for (hash, message, timestamp, nodes, edges) in &graphs {
        // Hash: first 7 characters
        let short_hash = if hash.len() >= 7 { &hash[..7] } else { hash };

        // Message: first line, max 50 characters
        let first_line = message.lines().next().unwrap_or("");
        let truncated = if first_line.len() > 50 {
            format!("{}…", &first_line[..49])
        } else {
            first_line.to_string()
        };

        // Timestamp → readable date
        let date = chrono::DateTime::from_timestamp(*timestamp, 0)
            .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
            .unwrap_or_else(|| "?".to_string());

        println!(
            "{:<9} {:<50} {:>6} {:>6}   {}",
            short_hash, truncated, nodes, edges, date
        );
    }

    println!();
    println!("Total: {total} graph snapshots");

    Ok(())
}

/// Displays the drift score trend for the last 20 commits in table format.
///
/// Each row: commit hash, message, node count, edge count,
/// drift score, and delta compared to the previous commit.
fn execute_list_drift(db: &Database) -> Result<()> {
    let trend = db.list_drift_trend(20)?;

    if trend.is_empty() {
        println!("No drift data yet. Run 'morpharch scan <path>' first.");
        return Ok(());
    }

    println!("Drift Score Trend (last {} commits):", trend.len());
    println!();
    let header = format!(
        "{:<9} {:<35} {:>6} {:>6} {:>7} {:>7}   {}",
        "HASH", "MESSAGE", "NODES", "EDGES", "DRIFT", "DELTA", "DATE"
    );
    println!("{header}");
    let separator = "─".repeat(100);
    println!("{separator}");

    let mut prev_drift: Option<u8> = None;

    // Trend is in descending timestamp order — iterate in reverse
    // for chronological delta calculation
    let reversed: Vec<_> = trend.iter().rev().collect();

    for (hash, message, nodes, edges, drift_total, timestamp) in &reversed {
        let short_hash = if hash.len() >= 7 { &hash[..7] } else { hash };

        let first_line = message.lines().next().unwrap_or("");
        let truncated = if first_line.len() > 35 {
            format!("{}…", &first_line[..34])
        } else {
            first_line.to_string()
        };

        let drift_str = drift_total
            .map(|d| format!("{d}"))
            .unwrap_or_else(|| "—".to_string());

        let delta_str = match (*drift_total, prev_drift) {
            (Some(curr), Some(prev)) => {
                let d = curr as i32 - prev as i32;
                if d > 0 {
                    format!("+{d}")
                } else if d < 0 {
                    format!("{d}")
                } else {
                    "0".to_string()
                }
            }
            _ => "—".to_string(),
        };

        let date = chrono::DateTime::from_timestamp(*timestamp, 0)
            .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
            .unwrap_or_else(|| "?".to_string());

        println!(
            "{:<9} {:<35} {:>6} {:>6} {:>7} {:>7}   {}",
            short_hash, truncated, nodes, edges, drift_str, delta_str, date
        );

        prev_drift = *drift_total;
    }

    println!();
    println!("Total: {} commits analyzed", trend.len());

    Ok(())
}
