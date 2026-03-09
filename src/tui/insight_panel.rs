// =============================================================================
// tui/insight_panel.rs — k9s-inspired Architecture Monitor
// =============================================================================

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, LineGauge, Paragraph, Wrap};

use super::app::App;
use super::graph_renderer::{
    ACCENT_BLUE, ACCENT_LAVENDER, BG_SURFACE, FG_OVERLAY, FG_TEXT, drift_color,
};
use crate::models::DriftScore;

pub fn render_insight_panel(
    frame: &mut Frame,
    area: Rect,
    drift: &Option<DriftScore>,
    advisory_lines: &[String],
) {
    if let Some(d) = drift {
        let health = 100u8.saturating_sub(d.total);
        let health_color = drift_color(d.total);

        // Adaptive layout based on available height
        if area.height < 4 {
            // Ultra-compact: single line summary
            let line = Line::from(vec![
                Span::styled(
                    format!(" {}% ", health),
                    Style::default()
                        .fg(health_color)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!(
                        "C:{} L:{} H:{:.0}",
                        d.new_cycles, d.boundary_violations, d.hub_debt
                    ),
                    Style::default().fg(FG_OVERLAY),
                ),
            ]);
            frame.render_widget(Paragraph::new(line), area);
            return;
        }

        if area.height < 8 {
            // Compact: health gauge + top 3 sub-scores inline
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Length(2), Constraint::Min(1)])
                .split(area);

            let health_gauge = LineGauge::default()
                .block(
                    Block::default().title(Span::styled(
                        format!(" HEALTH {}% ", health),
                        Style::default()
                            .fg(health_color)
                            .add_modifier(Modifier::BOLD),
                    )),
                )
                .filled_style(Style::default().fg(health_color))
                .unfilled_style(Style::default().fg(Color::Rgb(50, 50, 70)))
                .ratio(health as f64 / 100.0)
                .line_set(ratatui::symbols::line::THICK);
            frame.render_widget(health_gauge, chunks[0]);

            let metrics = Line::from(vec![
                metric_chip("CYC", &format!("{:.0}", d.cycle_debt), d.cycle_debt > 10.0),
                Span::raw(" "),
                metric_chip(
                    "LAY",
                    &format!("{:.0}", d.layering_debt),
                    d.layering_debt > 10.0,
                ),
                Span::raw(" "),
                metric_chip("HUB", &format!("{:.0}", d.hub_debt), d.hub_debt > 10.0),
            ]);
            frame.render_widget(Paragraph::new(metrics), chunks[1]);
            return;
        }

        // Full layout
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(2), // Health gauge
                Constraint::Length(7), // 6-component metrics grid
                Constraint::Min(2),    // Advisory
            ])
            .split(area);

        // ── SYSTEM HEALTH (Gauge) ──
        let health_label = format!(" {}% ", health);
        let health_gauge = LineGauge::default()
            .block(
                Block::default().title(Span::styled(
                    " SYSTEM HEALTH ",
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                )),
            )
            .filled_style(Style::default().fg(health_color))
            .unfilled_style(Style::default().fg(Color::Rgb(50, 50, 70)))
            .ratio(health as f64 / 100.0)
            .label(health_label)
            .line_set(ratatui::symbols::line::THICK);
        frame.render_widget(health_gauge, chunks[0]);

        // ── 6-COMPONENT METRICS GRID ──
        let metric_lines = vec![
            subscore_row("Cycles", "30%", d.cycle_debt, d.new_cycles as f64),
            subscore_row(
                "Layering",
                "25%",
                d.layering_debt,
                d.boundary_violations as f64,
            ),
            subscore_row("Hub/God", "15%", d.hub_debt, 0.0),
            subscore_row("Coupling", "12%", d.coupling_debt, 0.0),
            subscore_row("Cognitive", "10%", d.cognitive_debt, 0.0),
            subscore_row("Instability", " 8%", d.instability_debt, 0.0),
        ];
        frame.render_widget(
            Paragraph::new(metric_lines).block(
                Block::default().title(Span::styled(
                    " COMPONENTS ",
                    Style::default()
                        .fg(ACCENT_LAVENDER)
                        .add_modifier(Modifier::BOLD),
                )),
            ),
            chunks[1],
        );

        // ── ADVISORY (with diagnostics) ──
        let mut adv_lines: Vec<Line> = Vec::new();

        // Show component-specific diagnostics from scoring engine
        if !advisory_lines.is_empty() {
            let max_lines = (chunks[2].height.saturating_sub(1) as usize).max(2);
            for line in advisory_lines.iter().take(max_lines) {
                adv_lines.push(Line::from(vec![
                    Span::styled(" \u{25b8} ", Style::default().fg(ACCENT_LAVENDER)),
                    Span::styled(line.clone(), Style::default().fg(FG_TEXT)),
                ]));
            }
            if advisory_lines.len() > max_lines {
                adv_lines.push(Line::from(Span::styled(
                    format!("   +{} more insights", advisory_lines.len() - max_lines),
                    Style::default().fg(FG_OVERLAY),
                )));
            }
        } else {
            // Fallback: generic recommendation
            let rec = generate_recommendation(d);
            adv_lines.push(Line::from(Span::styled(
                format!("  {}", rec),
                Style::default().fg(FG_TEXT),
            )));
        }

        let advisory = Paragraph::new(adv_lines).wrap(Wrap { trim: true }).block(
            Block::default().title(Span::styled(
                " ADVISORY ",
                Style::default()
                    .fg(Color::Black)
                    .bg(ACCENT_LAVENDER)
                    .add_modifier(Modifier::BOLD),
            )),
        );
        frame.render_widget(advisory, chunks[2]);
    } else {
        frame.render_widget(
            Paragraph::new(" WAITING FOR DATA...").style(Style::default().fg(FG_OVERLAY)),
            area,
        );
    }
}

pub fn render_module_inspector(frame: &mut Frame, area: Rect, app: &App) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(ACCENT_LAVENDER))
        .style(Style::default().bg(BG_SURFACE));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let module_name = match &app.active_view {
        crate::tui::app::ActiveView::Inspect(name) => name,
        _ => return,
    };

    // Find the module stats
    let (instability, fan_in, fan_out) = app
        .brittle_packages
        .iter()
        .find(|(n, ..)| n == module_name)
        .map(|(_, inst, i, o)| (*inst, *i, *o))
        .unwrap_or((0.0, 0, 0));

    // Get current snapshot
    let current_meta = match app.snapshots_metadata.get(app.timeline.current_index) {
        Some(m) => m,
        None => return,
    };

    let mut imported_by = Vec::new();
    let mut depends_on = Vec::new();

    if let Some(snapshot) = app.snapshot_cache.get(&current_meta.commit_hash) {
        for edge in &snapshot.edges {
            if edge.to_module == *module_name {
                imported_by.push((edge.from_module.clone(), edge.weight));
            }
            if edge.from_module == *module_name {
                depends_on.push((edge.to_module.clone(), edge.weight));
            }
        }
    }

    imported_by.sort_by(|a, b| b.1.cmp(&a.1));
    depends_on.sort_by(|a, b| b.1.cmp(&a.1));

    // Refactored Layout: Header -> Risk -> Split Lists (Left/Right)
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Header info
            Constraint::Length(5), // Risk Analysis
            Constraint::Min(4),    // Split Lists (Horizontal)
        ])
        .margin(1)
        .split(inner);

    // 1. Header
    let instab_color = if instability > 0.8 {
        Color::Red
    } else if instability > 0.5 {
        Color::Yellow
    } else {
        Color::Green
    };
    let header_lines = vec![
        Line::from(vec![
            Span::styled("MODULE: ", Style::default().fg(FG_OVERLAY)),
            Span::styled(
                module_name,
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::styled(
                format!("{:<15}", "Instability:"),
                Style::default().fg(FG_OVERLAY),
            ),
            Span::styled(
                format!("{:.2} ", instability),
                Style::default()
                    .fg(instab_color)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("{:<15}", "Total Edges:"),
                Style::default().fg(FG_OVERLAY),
            ),
            Span::styled(
                format!("{}", fan_in + fan_out),
                Style::default().fg(Color::White),
            ),
        ]),
    ];
    frame.render_widget(Paragraph::new(header_lines), chunks[0]);

    // 2. Specific Advisory (Moved to Top)
    let rec = if fan_in > 15 && fan_out > 15 {
        "This module connects to many others in both directions. \
         Changes here could ripple widely — consider splitting into smaller modules."
    } else if instability > 0.8 && fan_out > 10 {
        "This module depends on many others but few depend on it. \
         It may break when upstream modules change. Add abstractions to reduce exposure."
    } else if fan_in > 20 {
        "Many modules rely on this one — it's a foundational piece. \
         Make sure it has strong test coverage to protect everything that depends on it."
    } else {
        "This module has a healthy dependency profile."
    };

    let advisory = Paragraph::new(format!("  {}", rec))
        .wrap(Wrap { trim: true })
        .block(
            Block::default().title(Span::styled(
                " RISK ANALYSIS ",
                Style::default()
                    .fg(Color::Black)
                    .bg(if instability > 0.8 || (fan_in > 15 && fan_out > 15) {
                        Color::Red
                    } else {
                        ACCENT_LAVENDER
                    })
                    .add_modifier(Modifier::BOLD),
            )),
        )
        .style(Style::default().fg(FG_TEXT));
    frame.render_widget(advisory, chunks[1]);

    // 3. Horizontal Split for Lists
    let list_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(chunks[2]);

    // Left List: Imported By (Fan-In)
    let in_color = if fan_in > 15 {
        Color::Rgb(203, 166, 247)
    } else if fan_in > 5 {
        ACCENT_BLUE
    } else {
        FG_OVERLAY
    };
    let mut in_lines = Vec::new();
    if imported_by.is_empty() {
        in_lines.push(Line::from(Span::styled(
            "  (No inbound edges)",
            Style::default().fg(FG_OVERLAY),
        )));
    } else {
        // Calculate max items that fit in the allocated height dynamically
        let max_items = list_chunks[0].height.saturating_sub(2) as usize;
        let display_count = if imported_by.len() > max_items {
            max_items.saturating_sub(1)
        } else {
            imported_by.len()
        };

        for (i, (name, weight)) in imported_by.iter().take(display_count).enumerate() {
            in_lines.push(Line::from(vec![
                Span::styled(format!("{:>2}. ", i + 1), Style::default().fg(FG_OVERLAY)),
                Span::styled(
                    format!("{:<15}", if name.len() > 14 { &name[..13] } else { name }),
                    Style::default().fg(FG_TEXT),
                ),
                Span::styled(format!(" (w: {})", weight), Style::default().fg(FG_OVERLAY)),
            ]));
        }
        if imported_by.len() > display_count {
            in_lines.push(Line::from(Span::styled(
                format!("    ... and {} more", imported_by.len() - display_count),
                Style::default().fg(FG_OVERLAY),
            )));
        }
    }

    let in_block = Block::default()
        .title(Span::styled(
            format!(" IMPORTED BY (In: {}) ", fan_in),
            Style::default().fg(in_color),
        ))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Rgb(60, 60, 80)));
    frame.render_widget(Paragraph::new(in_lines).block(in_block), list_chunks[0]);

    // Right List: Depends On (Fan-Out)
    let out_color = if fan_out > 15 {
        Color::Rgb(203, 166, 247)
    } else if fan_out > 5 {
        ACCENT_BLUE
    } else {
        FG_OVERLAY
    };
    let mut out_lines = Vec::new();
    if depends_on.is_empty() {
        out_lines.push(Line::from(Span::styled(
            "  (No outbound edges)",
            Style::default().fg(FG_OVERLAY),
        )));
    } else {
        // Calculate max items that fit in the allocated height dynamically
        let max_items = list_chunks[1].height.saturating_sub(2) as usize;
        let display_count = if depends_on.len() > max_items {
            max_items.saturating_sub(1)
        } else {
            depends_on.len()
        };

        for (i, (name, weight)) in depends_on.iter().take(display_count).enumerate() {
            out_lines.push(Line::from(vec![
                Span::styled(format!("{:>2}. ", i + 1), Style::default().fg(FG_OVERLAY)),
                Span::styled(
                    format!("{:<15}", if name.len() > 14 { &name[..13] } else { name }),
                    Style::default().fg(FG_TEXT),
                ),
                Span::styled(format!(" (w: {})", weight), Style::default().fg(FG_OVERLAY)),
            ]));
        }
        if depends_on.len() > display_count {
            out_lines.push(Line::from(Span::styled(
                format!("    ... and {} more", depends_on.len() - display_count),
                Style::default().fg(FG_OVERLAY),
            )));
        }
    }

    let out_block = Block::default()
        .title(Span::styled(
            format!(" DEPENDS ON (Out: {}) ", fan_out),
            Style::default().fg(out_color),
        ))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Rgb(60, 60, 80)));
    frame.render_widget(Paragraph::new(out_lines).block(out_block), list_chunks[1]);
}

/// Creates a sub-score row with component name, weight, score bar, and optional count.
fn subscore_row(name: &str, weight: &str, score: f64, count: f64) -> Line<'static> {
    let color = subscore_color(score);
    // Mini bar: 5-char visual indicator
    let filled = ((score / 100.0) * 5.0).round() as usize;
    let bar: String = "\u{2588}".repeat(filled) + &"\u{2591}".repeat(5 - filled);

    let count_str = if count > 0.0 {
        format!(" ({})", count as usize)
    } else {
        String::new()
    };

    Line::from(vec![
        Span::styled(format!(" {:<11}", name), Style::default().fg(FG_OVERLAY)),
        Span::styled(
            format!("{} ", weight),
            Style::default().fg(Color::Rgb(88, 91, 112)),
        ),
        Span::styled(bar, Style::default().fg(color)),
        Span::styled(
            format!(" {:>5.1}", score),
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ),
        Span::styled(count_str, Style::default().fg(FG_OVERLAY)),
    ])
}

/// Color for a sub-score value (0-100 scale)
fn subscore_color(score: f64) -> Color {
    if score <= 10.0 {
        Color::Green
    } else if score <= 30.0 {
        Color::Rgb(166, 218, 149) // light green
    } else if score <= 50.0 {
        Color::Yellow
    } else if score <= 70.0 {
        Color::Rgb(250, 179, 135) // Peach
    } else {
        Color::Red
    }
}

/// Creates a compact metric chip for inline display: "LABEL:VALUE"
fn metric_chip(label: &str, value: &str, is_warn: bool) -> Span<'static> {
    let color = if is_warn { Color::Yellow } else { Color::Green };
    Span::styled(format!(" {}:{} ", label, value), Style::default().fg(color))
}

fn generate_recommendation(drift: &DriftScore) -> String {
    // Find the worst component to give targeted advice
    let components = [
        (drift.cycle_debt, "Cycles"),
        (drift.layering_debt, "Layering"),
        (drift.hub_debt, "Hub"),
        (drift.coupling_debt, "Coupling"),
        (drift.cognitive_debt, "Cognitive"),
        (drift.instability_debt, "Instability"),
    ];
    let worst = components
        .iter()
        .max_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(score, name)| (*score, *name))
        .unwrap_or((0.0, "None"));

    if drift.total <= 15 {
        "Architecture looks great — clean structure with minimal coupling.".to_string()
    } else if drift.total <= 30 {
        format!(
            "Overall healthy. The {} area has the most room for improvement.",
            worst.1.to_lowercase()
        )
    } else if drift.total <= 55 {
        match worst.1 {
            "Cycles" => "Some modules depend on each other in circles. \
                         Breaking these loops will make the code easier to maintain."
                .to_string(),
            "Layering" => "Dependencies don't flow in a clean direction. \
                           Organizing layers to depend only downward would help."
                .to_string(),
            "Hub" => "Some modules are doing too much — they connect to \
                      everything. Splitting them would reduce risk."
                .to_string(),
            "Coupling" => "Modules are too tightly connected. Adding \
                           abstractions between them would improve flexibility."
                .to_string(),
            _ => format!(
                "The {} area needs attention — review affected modules.",
                worst.1.to_lowercase()
            ),
        }
    } else {
        match worst.1 {
            "Cycles" => "Circular dependencies are a significant concern. \
                         Prioritize breaking dependency loops."
                .to_string(),
            "Hub" => "Oversized modules pose high risk. Consider splitting \
                      them before they become harder to change."
                .to_string(),
            "Coupling" => "Very tight coupling across the codebase. \
                           Introducing boundaries would reduce change impact."
                .to_string(),
            _ => format!(
                "The {} area needs immediate attention to prevent further drift.",
                worst.1.to_lowercase()
            ),
        }
    }
}
