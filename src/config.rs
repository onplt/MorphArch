// =============================================================================
// config.rs — MorphArch configuration management
// =============================================================================
//
// Manages default configuration values and the database path.
//
// Database location: ~/.morpharch/morpharch.db
//   - Directory is auto-created if it doesn't exist
//   - Platform-independent: home directory detected via dirs crate
//
// max_commits: Maximum commits to scan per run (default: 500)
// =============================================================================

use anyhow::{Context, Result};
use std::path::PathBuf;
use tracing::info;

/// Runtime configuration for the MorphArch application.
#[derive(Debug)]
pub struct MorphArchConfig {
    /// Full path to the SQLite database file
    pub db_path: PathBuf,
}

impl MorphArchConfig {
    /// Loads the default configuration.
    ///
    /// Creates ~/.morpharch/ if needed and sets the database path.
    pub fn load() -> Result<Self> {
        let home = dirs::home_dir().context(
            "Home directory not found. \
             Check your HOME (Linux/macOS) or USERPROFILE (Windows) environment variable.",
        )?;

        let morpharch_dir = home.join(".morpharch");
        std::fs::create_dir_all(&morpharch_dir).with_context(|| {
            format!(
                "Failed to create MorphArch data directory: {}",
                morpharch_dir.display()
            )
        })?;

        let db_path = morpharch_dir.join("morpharch.db");
        info!(path = %db_path.display(), "Configuration loaded");

        Ok(Self {
            db_path,
        })
    }
}
