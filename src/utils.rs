//! Utility functions for logging and error display.
//!
//! - [`init_logging`] — Initializes `tracing-subscriber` with `RUST_LOG` support.
//! - [`print_error`] — Prints an `anyhow` error chain in a user-friendly format.

use anyhow::Error;
use tracing_subscriber::{EnvFilter, fmt};

/// Initializes the structured logging infrastructure.
pub fn init_logging(verbose: bool) {
    let default_level = if verbose {
        "morpharch=info"
    } else {
        "morpharch=warn"
    };

    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(default_level));

    fmt()
        .with_env_filter(filter)
        .with_target(false)
        .without_time()
        .init();
}

/// Prints an anyhow error chain to stderr in a user-friendly format.
pub fn print_error(err: &Error) {
    eprintln!("\nError: {err}");
    for (i, cause) in err.chain().skip(1).enumerate() {
        eprintln!("   {}: {cause}", i + 1);
    }
    eprintln!();
}
