// =============================================================================
// tui/insight_panel.rs — k9s-inspired Architecture Monitor
// =============================================================================

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, LineGauge, Paragraph, Sparkline, Wrap};

use super::graph_renderer::{
    ACCENT_BLUE, ACCENT_LAVENDER, ACCENT_MAUVE, BG_SURFACE, FG_OVERLAY, FG_TEXT, drift_color,
};
use crate::models::{DriftScore, SnapshotMetadata};

pub fn render_insight_panel(
    frame: &mut Frame,
    area: Rect,
    drift: &Option<DriftScore>,
    snapshots: &[SnapshotMetadata],
    brittle_packages: &[(String, f64)],
) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(ACCENT_MAUVE))
        .style(Style::default().bg(BG_SURFACE));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if let Some(d) = drift {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(2), // HEADER: ARCHITECTURE HEALTH
                Constraint::Length(5), // SECTION: DEBT METRICS
                Constraint::Length(3), // SECTION: DRIFT HISTORY
                Constraint::Min(6),    // SECTION: HOTSPOTS (Brittle)
                Constraint::Length(4), // FOOTER: ADVISORY
            ])
            .margin(0)
            .split(inner);

        // ── 1. ARCHITECTURE HEALTH (Gauge) ──────────────────────────────────
        let health = 100u8.saturating_sub(d.total);
        let health_color = drift_color(d.total);
        let health_gauge = LineGauge::default()
            .block(
                Block::default().title(Span::styled(
                    " HEALTH ",
                    Style::default()
                        .bg(health_color)
                        .fg(Color::Black)
                        .add_modifier(Modifier::BOLD),
                )),
            )
            .filled_style(Style::default().fg(health_color))
            .ratio(health as f64 / 100.0)
            .label(format!("{}%", health))
            .line_set(ratatui::symbols::line::THICK);
        frame.render_widget(health_gauge, chunks[0]);

        // ── 2. DEBT METRICS (Key-Value) ──────────────────────────────────────
        let mut debt_lines = Vec::new();
        debt_lines.push(kv_line(
            "New Cycles ",
            &format!("{}", d.new_cycles),
            if d.new_cycles > 0 {
                Color::Red
            } else {
                Color::Green
            },
        ));
        debt_lines.push(kv_line(
            "Violations ",
            &format!("{}", d.boundary_violations),
            if d.boundary_violations > 0 {
                Color::Yellow
            } else {
                Color::Green
            },
        ));
        debt_lines.push(kv_line(
            "Complexity ",
            &format!("{:.1}", d.cognitive_complexity),
            ACCENT_BLUE,
        ));

        let debt_para = Paragraph::new(debt_lines).block(
            Block::default().title(Span::styled(
                " DEBT BREAKDOWN ",
                Style::default()
                    .fg(ACCENT_LAVENDER)
                    .add_modifier(Modifier::BOLD),
            )),
        );
        frame.render_widget(debt_para, chunks[1]);

        // ── 3. DRIFT HISTORY (Sparkline) ─────────────────────────────────────
        let trend_data = build_trend_data(snapshots);
        let sparkline = Sparkline::default()
            .block(
                Block::default().title(Span::styled(
                    format!(" TREND ({}) ", trend_data.len()),
                    Style::default()
                        .fg(ACCENT_LAVENDER)
                        .add_modifier(Modifier::BOLD),
                )),
            )
            .data(&trend_data)
            .max(100)
            .style(Style::default().fg(ACCENT_BLUE));
        frame.render_widget(sparkline, chunks[2]);

        // ── 4. HOTSPOTS (Vulnerable Components) ──────────────────────────────
        let mut brittle_lines = Vec::new();
        if brittle_packages.is_empty() {
            brittle_lines.push(Line::from(Span::styled(
                "  Analyzing...",
                Style::default().fg(FG_OVERLAY),
            )));
        } else {
            for (name, instability) in brittle_packages {
                let color = if *instability > 0.8 {
                    Color::Red
                } else if *instability > 0.5 {
                    Color::Yellow
                } else {
                    Color::Green
                };
                let display_name = if name.len() > 16 {
                    format!("{}…", &name[..13])
                } else {
                    name.clone()
                };
                brittle_lines.push(Line::from(vec![
                    Span::styled(
                        format!(" {:<16} ", display_name),
                        Style::default().fg(FG_TEXT),
                    ),
                    Span::styled(
                        format!("{:>6.2} I", instability),
                        Style::default().fg(color).add_modifier(Modifier::BOLD),
                    ),
                ]));
            }
        }
        let hotspots_para = Paragraph::new(brittle_lines).block(
            Block::default().title(Span::styled(
                " HOTSPOTS ",
                Style::default()
                    .fg(ACCENT_LAVENDER)
                    .add_modifier(Modifier::BOLD),
            )),
        );
        frame.render_widget(hotspots_para, chunks[3]);

        // ── 5. ADVISORY ─────────────────────────────────────────────────────
        let rec = generate_recommendation(d);
        let advisory = Paragraph::new(rec)
            .wrap(Wrap { trim: true })
            .block(
                Block::default().title(Span::styled(
                    " ADVISORY ",
                    Style::default()
                        .fg(Color::Black)
                        .bg(ACCENT_LAVENDER)
                        .add_modifier(Modifier::BOLD),
                )),
            )
            .style(Style::default().fg(FG_TEXT).add_modifier(Modifier::ITALIC));
        frame.render_widget(advisory, chunks[4]);
    } else {
        frame.render_widget(
            Paragraph::new("WAITING FOR DATA...").style(Style::default().fg(FG_OVERLAY)),
            inner,
        );
    }
}

/// Creates a k9s-style key-value line with dots filling the gap.
fn kv_line(key: &str, value: &str, val_color: Color) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!(" {:<12}", key), Style::default().fg(FG_OVERLAY)),
        Span::styled(" ┈ ", Style::default().fg(Color::Rgb(60, 60, 80))),
        Span::styled(
            value.to_string(),
            Style::default().fg(val_color).add_modifier(Modifier::BOLD),
        ),
    ])
}

fn build_trend_data(snapshots: &[SnapshotMetadata]) -> Vec<u64> {
    let mut data: Vec<u64> = snapshots
        .iter()
        .take(50)
        .map(|s| s.drift.as_ref().map(|d| d.total as u64).unwrap_or(50))
        .collect();
    data.reverse();
    data
}

fn generate_recommendation(drift: &DriftScore) -> String {
    if drift.total <= 30 {
        "Stable. No immediate action required."
    } else if drift.new_cycles > 0 {
        "Cycles detected! Decouple circular dependencies."
    } else if drift.boundary_violations > 0 {
        "Layer violation! Check package boundaries."
    } else {
        "Complexity rising. Consider refactoring core."
    }
    .to_string()
}
