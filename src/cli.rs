// =============================================================================
// cli.rs — MorphArch command-line interface definitions
// =============================================================================
//
// Ergonomic CLI structure using clap derive macros:
//   morpharch scan <path>        → Scan repo: commit metadata + graph + drift
//   morpharch watch <path>       → Scan + launch animated TUI
//   morpharch list-graphs        → List recent dependency graph snapshots
//   morpharch analyze [commit]   → Show drift report for a specific commit
//   morpharch list-drift         → Drift trend table for the last 20 commits
//   morpharch --help             → Help message
//
// Each subcommand takes an optional path; defaults to "." (current directory).
// list-graphs, list-drift, and analyze do not take a path — they read from DB.
// =============================================================================

use clap::{Parser, Subcommand};
use std::path::PathBuf;

/// MorphArch — Monorepo architecture drift visualizer
///
/// Scans large monorepo Git history, builds per-commit dependency graphs,
/// calculates architecture drift scores, and visualizes them with an
/// animated TUI.
#[derive(Parser, Debug)]
#[command(
    name = "morpharch",
    version,
    about = "Monorepo architecture drift visualizer with animated TUI",
    long_about = "MorphArch scans monorepo Git history, builds per-commit dependency graphs,\n\
                  calculates architecture drift scores, and visualizes them with an\n\
                  animated force-graph + timeline using ratatui.",
    after_help = "Examples:\n  morpharch scan .          Scan repo: commits + graphs + drift scores\n  morpharch scan ../myrepo  Scan a specific repository\n  morpharch watch .         Scan + activate watch mode\n  morpharch list-graphs     Show last 10 graph snapshots\n  morpharch list-drift      Show drift score trend (last 20 commits)\n  morpharch analyze         Analyze HEAD commit drift\n  morpharch analyze main~5  Analyze specific commit drift"
)]
pub struct Cli {
    /// Subcommand to execute
    #[command(subcommand)]
    pub command: Commands,

    /// Enable verbose logging (shows INFO level logs)
    #[arg(short, long, global = true)]
    pub verbose: bool,
}

/// Available subcommands
#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Scan a Git repository: commit metadata + dependency graph + drift score
    ///
    /// Reads commits from the Git repository at the given path, builds a
    /// dependency graph for each commit, calculates drift scores, and stores
    /// everything in the SQLite database.
    ///
    /// Use --max-commits 0 to scan ALL commits (no limit).
    Scan {
        /// Path to the Git repository to scan
        #[arg(default_value = ".")]
        path: PathBuf,

        /// Maximum number of commits to scan (0 = unlimited, default: unlimited)
        #[arg(short = 'n', long, default_value = "0")]
        max_commits: usize,
    },

    /// Scan the repository and launch the animated TUI
    ///
    /// First performs a scan, then loads graph snapshots from the database
    /// to present a Verlet physics force-directed graph visualization,
    /// timeline slider, and drift insight panel.
    ///
    /// Keys: j/k=navigate, p=play/pause, r=reset, /=search, q=quit
    Watch {
        /// Path to the Git repository to watch
        #[arg(default_value = ".")]
        path: PathBuf,

        /// Maximum number of commits to scan (0 = unlimited, default: unlimited)
        #[arg(short = 'n', long, default_value = "0")]
        max_commits: usize,

        /// Maximum snapshots to load in the TUI timeline (default: 200).
        /// When the DB has more, snapshots are sampled at even intervals
        /// so the timeline covers the full commit history.
        #[arg(short = 's', long, default_value = "200")]
        max_snapshots: usize,
    },

    /// List recent dependency graph snapshots
    ///
    /// Shows the last 10 graph snapshots from the database in table format
    /// with commit info.
    ListGraphs,

    /// Show detailed drift report for a specific commit
    ///
    /// Includes drift score, sub-metrics, boundary violations, circular
    /// dependencies, and improvement recommendations. Defaults to HEAD
    /// if no commit is specified.
    Analyze {
        /// Commit reference to analyze (e.g., HEAD, main~5, abc1234)
        #[arg(default_value = None)]
        commit: Option<String>,

        /// Path to the Git repository (needed for rev-parse)
        #[arg(short, long, default_value = ".")]
        path: PathBuf,
    },

    /// Show drift score trend for recent commits
    ///
    /// Displays drift scores, node/edge counts, and delta changes
    /// compared to the previous commit for the last 20 commits.
    ListDrift,
}
