// =============================================================================
// commands/analyze.rs — Analyze command: detailed drift report
// =============================================================================
//
//  For the specified commit (or HEAD):
//   1. Fetches graph snapshot from DB
//   2. Displays drift score and sub-metrics
//   3. Computes temporal delta with the previous 3 commits
//   4. Lists top boundary violators
//   5. Reports cycle information
//   6. Offers improvement recommendations
//
// Usage:
//   morpharch analyze           → HEAD commit analysis
//   morpharch analyze main~5    → Specified commit analysis
// =============================================================================

use std::collections::HashSet;
use std::path::Path;

use anyhow::{Context, Result};
use tracing::info;

use crate::db::Database;
use crate::graph_builder;
use crate::models::DriftScore;
use crate::scoring;

/// Runs the analyze command: produces a detailed drift report.
pub fn run_analyze(repo_path: &Path, commit_ish: Option<&str>, db: &Database) -> Result<()> {
    // ── Resolve commit hash ──
    let commit_hash = resolve_commit(repo_path, commit_ish)?;
    let short_hash = if commit_hash.len() >= 7 {
        &commit_hash[..7]
    } else {
        &commit_hash
    };

    info!(hash = %commit_hash, "Analyzing commit");

    // ── Fetch graph snapshot ──
    let snapshot = db
        .get_graph_snapshot(&commit_hash)?
        .with_context(|| format!("No graph snapshot found for this commit: {short_hash}"))?;

    println!("  Commit Analysis: {short_hash}");
    println!();

    // ── Drift report ──
    if let Some(ref drift) = snapshot.drift {
        print_drift_report(drift, snapshot.node_count, snapshot.edge_count);
    } else {
        println!("  No drift score calculated for this commit.");
        println!("   Run 'morpharch scan <path>' to re-scan first.");
        return Ok(());
    }

    // ── Temporal analysis: compare with previous 3 commits ──
    println!();
    println!("  Temporal Analysis (comparison with previous commits):");
    println!();

    let trend = db.list_drift_trend(20)?;

    let current_pos = trend.iter().position(|(h, ..)| h == &commit_hash);

    if let Some(pos) = current_pos {
        let prev_commits: Vec<_> = trend.iter().skip(pos + 1).take(3).collect();

        if prev_commits.is_empty() {
            println!("  First commit — no previous commits to compare.");
        } else {
            let header = format!(
                "  {:<9} {:>6} {:>6} {:>7} {:>8}",
                "HASH", "NODES", "EDGES", "DRIFT", "DELTA"
            );
            println!("{header}");
            let separator = format!("  {}", "-".repeat(45));
            println!("{separator}");

            let current_drift = snapshot.drift.as_ref().map(|d| d.total).unwrap_or(0);

            for (prev_hash, _msg, prev_nodes, prev_edges, prev_drift, _ts) in &prev_commits {
                let prev_short = if prev_hash.len() >= 7 {
                    &prev_hash[..7]
                } else {
                    prev_hash
                };
                let drift_str = prev_drift
                    .map(|d| format!("{d}"))
                    .unwrap_or_else(|| "?".to_string());
                let delta = prev_drift
                    .map(|d| current_drift as i32 - d as i32)
                    .map(|d| {
                        if d > 0 {
                            format!("+{d}")
                        } else {
                            format!("{d}")
                        }
                    })
                    .unwrap_or_else(|| "?".to_string());

                println!(
                    "  {:<9} {:>6} {:>6} {:>7} {:>8}",
                    prev_short, prev_nodes, prev_edges, drift_str, delta
                );
            }
        }
    } else {
        println!("  This commit was not found in the trend data.");
    }

    // ── Boundary violation details ──
    println!();
    print_boundary_details(&snapshot.edges);

    // ── Cycle information ──
    println!();
    print_cycle_info(&snapshot.nodes, &snapshot.edges);

    // ── Recommendations ──
    println!();
    print_recommendations(&snapshot.drift);

    Ok(())
}

fn resolve_commit(repo_path: &Path, commit_ish: Option<&str>) -> Result<String> {
    let repo = gix::discover(repo_path)
        .with_context(|| format!("Git repository not found: {}", repo_path.display()))?;

    let reference = commit_ish.unwrap_or("HEAD");

    let object = repo
        .rev_parse_single(reference)
        .with_context(|| format!("Failed to resolve commit reference: '{reference}'"))?;

    Ok(object.detach().to_string())
}

fn print_drift_report(drift: &DriftScore, node_count: usize, edge_count: usize) {
    let (emoji, level) = match drift.total {
        0..=15 => ("  ", "Excellent"),
        16..=30 => ("  ", "Healthy"),
        31..=55 => ("  ", "Warning"),
        56..=80 => ("  ", "Degraded"),
        _ => ("  ", "Critical"),
    };

    println!("{emoji} Drift Score: {}/100 ({level})", drift.total);
    println!("     Health: {}%", 100u8.saturating_sub(drift.total));
    println!();
    println!("  Graph Statistics:");
    println!("     Node (module) count:      {node_count}");
    println!("     Edge (dependency) count:   {edge_count}");
    println!();
    println!("  Component Breakdown (6-factor analysis):");
    println!(
        "     Cycles       (30%):  {:5.1}/100  {} SCCs",
        drift.cycle_debt, drift.new_cycles
    );
    println!(
        "     Layering     (25%):  {:5.1}/100  {} back-edges",
        drift.layering_debt, drift.boundary_violations
    );
    println!("     Hub/God      (15%):  {:5.1}/100", drift.hub_debt);
    println!("     Coupling     (12%):  {:5.1}/100", drift.coupling_debt);
    println!("     Cognitive    (10%):  {:5.1}/100", drift.cognitive_debt);
    println!(
        "     Instability   (8%):  {:5.1}/100",
        drift.instability_debt
    );
    println!();
    println!("  Delta Metrics:");
    println!("     Fan-in change (median):   {:+}", drift.fan_in_delta);
    println!("     Fan-out change (median):  {:+}", drift.fan_out_delta);
}

fn print_boundary_details(edges: &[crate::models::DependencyEdge]) {
    let pairs = scoring::edges_to_pairs(edges);
    let violations: Vec<_> = pairs
        .iter()
        .filter(|(from, to)| {
            scoring::BOUNDARY_RULES
                .iter()
                .any(|(fp, tp)| from.starts_with(fp) && to.starts_with(tp))
        })
        .collect();

    if violations.is_empty() {
        println!("  Boundary Violations: None — package boundaries are clean.");
    } else {
        println!("  Boundary Violations ({} found):", violations.len());
        for (i, (from, to)) in violations.iter().enumerate().take(10) {
            println!("     {}. {} -> {}", i + 1, from, to);
        }
        if violations.len() > 10 {
            println!("     ... and {} more", violations.len() - 10);
        }
    }
}

fn print_cycle_info(nodes: &[String], edges: &[crate::models::DependencyEdge]) {
    let node_set: HashSet<String> = nodes.iter().cloned().collect();
    let graph = graph_builder::build_graph(&node_set, edges);
    let cycle_count = scoring::count_cycles_public(&graph);

    if cycle_count == 0 {
        println!("  Cyclic Dependencies: None — DAG structure is maintained.");
    } else {
        println!("  Cyclic Dependencies: {cycle_count} cycle(s) detected.");
        println!("     Cycles increase architectural complexity and make refactoring harder.");
    }
}

fn print_recommendations(drift: &Option<DriftScore>) {
    println!("  Recommendations:");

    let Some(d) = drift else {
        println!("   No drift score calculated — run 'morpharch scan' first.");
        return;
    };

    let mut suggestions = Vec::new();

    if d.cycle_debt > 20.0 {
        suggestions.push(format!(
            "   {} circular dependency group(s) detected (score: {:.0}/100). \
             Some modules depend on each other in loops — breaking these cycles \
             with interfaces or traits will make the code easier to maintain.",
            d.new_cycles, d.cycle_debt
        ));
    }

    if d.layering_debt > 20.0 {
        suggestions.push(format!(
            "   {} extra edge(s) inside dependency cycles (score: {:.0}/100). \
             The dependency flow isn't clean — organizing layers to depend \
             only in one direction would improve clarity.",
            d.boundary_violations, d.layering_debt
        ));
    }

    if d.hub_debt > 20.0 {
        suggestions.push(format!(
            "   Some modules are doing too much (score: {:.0}/100). They connect to \
             many others in both directions. Splitting them into smaller, \
             focused modules would reduce the blast radius of changes.",
            d.hub_debt
        ));
    }

    if d.coupling_debt > 20.0 {
        suggestions.push(format!(
            "   Modules are more tightly connected than expected (score: {:.0}/100). \
             Adding abstractions between heavily coupled modules would \
             improve flexibility and make changes safer.",
            d.coupling_debt
        ));
    }

    if d.cognitive_debt > 20.0 {
        suggestions.push(format!(
            "   The dependency structure is complex (score: {:.0}/100). \
             There are more connections than needed. Simplifying the wiring \
             would make the architecture easier to understand and navigate.",
            d.cognitive_debt
        ));
    }

    if d.instability_debt > 20.0 {
        suggestions.push(format!(
            "   Some core modules are fragile (score: {:.0}/100). They depend \
             heavily on others, so upstream changes may cascade through them. \
             Adding abstractions would help stabilize them.",
            d.instability_debt
        ));
    }

    if d.total <= 15 {
        suggestions.push(
            "   Architecture looks great — clean structure with minimal coupling.".to_string(),
        );
    } else if d.total <= 30 {
        suggestions
            .push("   Overall healthy architecture with minor areas for improvement.".to_string());
    }

    if suggestions.is_empty() {
        suggestions.push("   Architecture is in an acceptable state.".to_string());
    }

    for suggestion in &suggestions {
        println!("{suggestion}");
    }
}
