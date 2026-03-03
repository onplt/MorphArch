// =============================================================================
// tui/insight_panel.rs — Drift insight right panel
// =============================================================================
//
// Information shown in the right panel:
//   1. Drift score (large number + color)
//   2. Sub-metrics: fan-in/out delta, cycles, violations
//   3. Score trend (last 5 commits → sparkline)
//   4. Short recommendation (most important issue)
//
// Coloring uses the drift_color function (green → red gradient).
// =============================================================================

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

use crate::models::{DriftScore, GraphSnapshot};

use super::graph_renderer::{
    ACCENT_BLUE, ACCENT_LAVENDER, BG_SURFACE, FG_OVERLAY, FG_TEXT, drift_color,
};

/// Renders the drift insight panel.
///
/// # Parameters
/// - `drift`: Current commit's drift score (None = no data)
/// - `snapshots`: All snapshots (for trend calculation)
/// - `current_index`: Current timeline position
pub fn render_insight_panel(
    frame: &mut Frame,
    area: Rect,
    drift: &Option<DriftScore>,
    snapshots: &[GraphSnapshot],
    current_index: usize,
) {
    let block = Block::default()
        .title(" Drift Insight ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(ACCENT_LAVENDER))
        .style(Style::default().bg(BG_SURFACE));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let mut lines: Vec<Line> = Vec::new();

    match drift {
        Some(d) => {
            // ── Drift score (large display) ──
            let color = drift_color(d.total);
            let severity = match d.total {
                0..=30 => "Healthy",
                31..=60 => "Warning",
                61..=80 => "Degraded",
                _ => "Critical",
            };
            let emoji = match d.total {
                0..=30 => "🟢",
                31..=60 => "🟡",
                61..=80 => "🟠",
                _ => "🔴",
            };

            lines.push(Line::from(""));
            lines.push(Line::from(vec![
                Span::styled(format!("  {emoji} Drift: "), Style::default().fg(FG_TEXT)),
                Span::styled(
                    format!("{}", d.total),
                    Style::default().fg(color).add_modifier(Modifier::BOLD),
                ),
                Span::styled("/100".to_string(), Style::default().fg(FG_OVERLAY)),
            ]));
            lines.push(Line::from(Span::styled(
                format!("  Status: {severity}"),
                Style::default().fg(color),
            )));
            lines.push(Line::from(""));

            // ── Sub-metrics ──
            lines.push(Line::from(Span::styled(
                "  -- Metrics --",
                Style::default()
                    .fg(ACCENT_BLUE)
                    .add_modifier(Modifier::BOLD),
            )));

            let fan_in_str = format_delta(d.fan_in_delta);
            let fan_out_str = format_delta(d.fan_out_delta);
            let cycles_str = format!("{}", d.new_cycles);
            let violations_str = format!("{}", d.boundary_violations);
            let complexity_str = format!("{:.1}", d.cognitive_complexity);

            lines.push(metric_line("Max Fan-in Δ", &fan_in_str));
            lines.push(metric_line("Max Fan-out Δ", &fan_out_str));
            lines.push(metric_line("Cycles", &cycles_str));
            lines.push(metric_line("Violations", &violations_str));
            lines.push(metric_line("Complexity", &complexity_str));

            // ── Trend (sparkline) ──
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "  -- Trend --",
                Style::default()
                    .fg(ACCENT_BLUE)
                    .add_modifier(Modifier::BOLD),
            )));

            let trend_str = build_sparkline(snapshots, current_index);
            lines.push(Line::from(Span::styled(
                format!("  {trend_str}"),
                Style::default().fg(ACCENT_LAVENDER),
            )));

            // ── Recommendation ──
            lines.push(Line::from(""));
            let recommendation = generate_recommendation(d);
            lines.push(Line::from(Span::styled(
                format!("  {recommendation}"),
                Style::default().fg(FG_OVERLAY),
            )));
        }
        None => {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "  No drift data available.",
                Style::default().fg(FG_OVERLAY),
            )));
            lines.push(Line::from(Span::styled(
                "  Run 'morpharch scan .'",
                Style::default().fg(FG_OVERLAY),
            )));
            lines.push(Line::from(Span::styled(
                "  to scan first.",
                Style::default().fg(FG_OVERLAY),
            )));
        }
    }

    let paragraph = Paragraph::new(lines).wrap(Wrap { trim: true });
    frame.render_widget(paragraph, inner);
}

/// Creates a single metric line (owned — can be stored in Vec<Line>).
fn metric_line(label: &str, value: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("  {label}: "), Style::default().fg(FG_OVERLAY)),
        Span::styled(value.to_string(), Style::default().fg(FG_TEXT)),
    ])
}

/// Formats a delta value as +/- string.
fn format_delta(delta: i32) -> String {
    if delta > 0 {
        format!("+{delta}")
    } else if delta < 0 {
        format!("{delta}")
    } else {
        "0".to_string()
    }
}

/// Builds a sparkline from the last 7 commits' drift scores.
///
/// Uses braille character set for a mini graph:
///   ▁ ▂ ▃ ▄ ▅ ▆ ▇ █
fn build_sparkline(snapshots: &[GraphSnapshot], current_index: usize) -> String {
    let spark_chars = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];

    // Get 7 snapshots around the current index
    let start = current_index.saturating_sub(3);
    let end = (start + 7).min(snapshots.len());

    let scores: Vec<u8> = snapshots[start..end]
        .iter()
        .map(|s| s.drift.as_ref().map(|d| d.total).unwrap_or(50))
        .collect();

    if scores.is_empty() {
        return "—".to_string();
    }

    scores
        .iter()
        .map(|&score| {
            // 0-100 → 0-7 index
            let idx = ((score as usize) * 7) / 100;
            spark_chars[idx.min(7)]
        })
        .collect()
}

/// Generates a short recommendation based on drift score and metrics.
///
/// Priority order: overall status first (healthy / critical), then specific
/// sub-metric alerts. This prevents contradictory messages like "Healthy"
/// status + "High complexity — refactor" recommendation.
fn generate_recommendation(drift: &DriftScore) -> String {
    // ── Overall status takes priority ──
    if drift.total <= 30 {
        return "Architecture is healthy ✓".to_string();
    }

    // ── Critical alerts ──
    if drift.new_cycles > 0 {
        return format!(
            "{} cycle{} detected — break circular deps",
            drift.new_cycles,
            if drift.new_cycles > 1 { "s" } else { "" }
        );
    }

    if drift.total > 80 {
        return "Critical drift — immediate review needed".to_string();
    }

    // ── Specific sub-metric alerts ──
    if drift.boundary_violations > 2 {
        return format!(
            "{} boundary violations — enforce module boundaries",
            drift.boundary_violations
        );
    }

    if drift.fan_out_delta > 5 {
        return "High fan-out growth — consider splitting modules".to_string();
    }

    if drift.total > 60 {
        return "Drift is high — review recent architectural changes".to_string();
    }

    // ── Moderate: complexity based advice ──
    if drift.cognitive_complexity > 40.0 {
        return "High edge density — consider splitting modules".to_string();
    }

    "Monitor drift trend".to_string()
}

// =============================================================================
// Tests
// =============================================================================
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_delta() {
        assert_eq!(format_delta(5), "+5");
        assert_eq!(format_delta(-3), "-3");
        assert_eq!(format_delta(0), "0");
    }

    #[test]
    fn test_generate_recommendation_cycles() {
        let drift = DriftScore {
            total: 70,
            fan_in_delta: 0,
            fan_out_delta: 0,
            new_cycles: 2,
            boundary_violations: 0,
            cognitive_complexity: 0.0,
            timestamp: 0,
        };
        let rec = generate_recommendation(&drift);
        assert!(rec.contains("cycle"), "Should recommend about cycles: {rec}");
    }

    #[test]
    fn test_generate_recommendation_healthy() {
        let drift = DriftScore {
            total: 20,
            fan_in_delta: 0,
            fan_out_delta: 0,
            new_cycles: 0,
            boundary_violations: 0,
            cognitive_complexity: 5.0,
            timestamp: 0,
        };
        let rec = generate_recommendation(&drift);
        assert!(rec.contains("healthy"), "Should recommend healthy: {rec}");
    }

    #[test]
    fn test_generate_recommendation_healthy_overrides_complexity() {
        // Drift ≤ 30 should show "healthy" even if complexity > 40
        let drift = DriftScore {
            total: 25,
            fan_in_delta: 0,
            fan_out_delta: 0,
            new_cycles: 0,
            boundary_violations: 0,
            cognitive_complexity: 45.0,
            timestamp: 0,
        };
        let rec = generate_recommendation(&drift);
        assert!(
            rec.contains("healthy"),
            "Healthy status should override complexity warning: {rec}"
        );
    }

    #[test]
    fn test_sparkline_generation() {
        let snapshots: Vec<GraphSnapshot> = (0..5)
            .map(|i| GraphSnapshot {
                commit_hash: format!("hash{i}"),
                nodes: vec![],
                edges: vec![],
                node_count: 0,
                edge_count: 0,
                timestamp: i as i64,
                drift: Some(DriftScore {
                    total: (i * 20) as u8,
                    fan_in_delta: 0,
                    fan_out_delta: 0,
                    new_cycles: 0,
                    boundary_violations: 0,
                    cognitive_complexity: 0.0,
                    timestamp: 0,
                }),
            })
            .collect();

        let sparkline = build_sparkline(&snapshots, 2);
        assert!(!sparkline.is_empty(), "Sparkline should not be empty");
        assert!(sparkline.len() >= 3, "Should have at least 3 characters");
    }
}
