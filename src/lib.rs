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

pub mod analysis;
pub mod blast_radius;
pub mod cli;
pub mod commands;
pub mod config;
pub mod db;
pub mod git_scanner;
pub mod graph_builder;
pub mod models;
pub mod parser;
pub mod scoring;
pub mod tui;
pub mod utils;
