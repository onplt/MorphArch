// =============================================================================
// utils.rs — MorphArch utilities
// =============================================================================
//
// Logging infrastructure and error formatting:
//
//   init_logging()  → Initializes structured logging via tracing-subscriber
//     - Log level configurable via RUST_LOG env var
//     - Default level: INFO
//     - Target info hidden (cleaner output)
//     - No timestamps (unnecessary for a CLI tool)
//
//   print_error()   → Prints anyhow error chain in a user-friendly format
//     - Each context layer shown on a separate line
//     - Root cause highlighted
// =============================================================================

use anyhow::Error;
use tracing_subscriber::{EnvFilter, fmt};

/// Initializes the structured logging infrastructure.
pub fn init_logging() {
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("morpharch=info"));

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
