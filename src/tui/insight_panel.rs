// =============================================================================
// tui/insight_panel.rs — k9s-inspired Architecture Monitor
// =============================================================================

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, LineGauge, Paragraph, Sparkline, Wrap};

use super::app::App;
use super::graph_renderer::{
    ACCENT_BLUE, ACCENT_LAVENDER, BG_SURFACE, FG_OVERLAY, FG_TEXT, blast_color, drift_color,
};
use super::widgets::truncate_str;
use crate::config::Weights;
use crate::models::{BlastRadiusReport, DriftScore};

#[allow(clippy::too_many_arguments)]
pub fn render_insight_panel(
    frame: &mut Frame,
    area: Rect,
    drift: &Option<DriftScore>,
    context_lines: &[String],
    advisory_lines: &[String],
    weights: &Weights,
    trend_data: &[u64],
    current_index: usize,
    total_commits: usize,
) {
    if let Some(d) = drift {
        let area = inset_rect(area, 1, 0);
        let health = 100u8.saturating_sub(d.total);
        let health_color = drift_color(d.total);
        let health_data: Vec<u64> = trend_data
            .iter()
            .map(|debt| 100u64.saturating_sub(*debt))
            .collect();
        let (trend_label, trend_color) = trend_direction(trend_data);

        if area.height < 4 {
            let line = Line::from(vec![
                Span::styled(
                    format!(" {}% ", health),
                    Style::default()
                        .fg(health_color)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("Debt {}  ", d.total),
                    Style::default().fg(FG_OVERLAY),
                ),
                Span::styled(
                    trend_label,
                    Style::default()
                        .fg(trend_color)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("  {} commits", trend_data.len()),
                    Style::default().fg(FG_OVERLAY),
                ),
            ]);
            frame.render_widget(Paragraph::new(line), area);
            return;
        }

        if area.height < 10 {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(2),
                    Constraint::Length(1),
                    Constraint::Length(3),
                    Constraint::Length(1),
                    Constraint::Min(1),
                ])
                .split(area);

            let health_gauge = LineGauge::default()
                .block(
                    Block::default().title(Span::styled(
                        format!(" Overview {}% ", health),
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

            let sparkline = Sparkline::default()
                .block(
                    Block::default()
                        .title(Span::styled(
                            format!(" Recent trend {} ", trend_label),
                            Style::default()
                                .fg(trend_color)
                                .add_modifier(Modifier::BOLD),
                        ))
                        .borders(Borders::TOP),
                )
                .data(&health_data)
                .max(100)
                .style(Style::default().fg(health_color));
            frame.render_widget(sparkline, chunks[2]);

            let metrics = Line::from(vec![
                Span::styled(
                    format!(" Debt {}  ", d.total),
                    Style::default().fg(FG_OVERLAY),
                ),
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
            frame.render_widget(Paragraph::new(metrics), chunks[4]);
            return;
        }

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(4), // Current state
                Constraint::Length(1), // Spacer
                Constraint::Length(5), // Trajectory chart
                Constraint::Length(2), // Trajectory stats
                Constraint::Length(1), // Spacer
                Constraint::Length(7), // 6-component metrics grid
                Constraint::Length(1), // Spacer
                Constraint::Min(2),    // Context + actions
            ])
            .split(area);

        let state_lines = vec![
            Line::from(vec![
                Span::styled(" Current", Style::default().fg(FG_OVERLAY)),
                Span::styled(
                    format!("  {}%", health),
                    Style::default()
                        .fg(health_color)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("  Debt {}", d.total),
                    Style::default().fg(FG_OVERLAY),
                ),
            ]),
            Line::from(vec![
                Span::styled(" Trajectory", Style::default().fg(FG_OVERLAY)),
                Span::styled(
                    format!(" {}", trend_label),
                    Style::default()
                        .fg(trend_color)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!(
                        "  Last {} commits  {}/{}",
                        trend_data.len(),
                        current_index + 1,
                        total_commits
                    ),
                    Style::default().fg(FG_OVERLAY),
                ),
            ]),
        ];
        frame.render_widget(
            Paragraph::new(state_lines).block(
                Block::default()
                    .title(Span::styled(
                        " Current state ",
                        Style::default().fg(FG_OVERLAY).add_modifier(Modifier::BOLD),
                    ))
                    .borders(Borders::TOP)
                    .border_style(Style::default().fg(Color::Rgb(50, 50, 70))),
            ),
            chunks[0],
        );

        let trend_block = Block::default()
            .title(Span::styled(
                " Recent trend ",
                Style::default()
                    .fg(ACCENT_BLUE)
                    .add_modifier(Modifier::BOLD),
            ))
            .borders(Borders::TOP);
        let sparkline = Sparkline::default()
            .block(trend_block)
            .data(&health_data)
            .max(100)
            .style(Style::default().fg(health_color));
        frame.render_widget(sparkline, chunks[2]);

        let (min_h, max_h, avg_h) = health_stats(&health_data);
        let stats_line = Line::from(vec![
            Span::styled(" Current:", Style::default().fg(FG_OVERLAY)),
            Span::styled(
                format!("{:>3}%", health),
                Style::default()
                    .fg(health_color)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("  Min:", Style::default().fg(FG_OVERLAY)),
            Span::styled(
                format!("{:>3}%", min_h),
                Style::default().fg(drift_color(100 - min_h as u8)),
            ),
            Span::styled("  Avg:", Style::default().fg(FG_OVERLAY)),
            Span::styled(
                format!("{:>3}%", avg_h),
                Style::default().fg(drift_color(100 - avg_h as u8)),
            ),
            Span::styled("  Max:", Style::default().fg(FG_OVERLAY)),
            Span::styled(
                format!("{:>3}%", max_h),
                Style::default().fg(drift_color(100 - max_h as u8)),
            ),
        ]);
        frame.render_widget(
            Paragraph::new(stats_line).block(
                Block::default()
                    .borders(Borders::TOP)
                    .border_style(Style::default().fg(Color::Rgb(50, 50, 70))),
            ),
            chunks[3],
        );

        let n = weights.normalized();
        let fmt_pct = |v: f64| -> String {
            let pct = (v * 100.0).round() as u32;
            if pct < 10 {
                format!(" {pct}%")
            } else {
                format!("{pct}%")
            }
        };
        let metric_lines = vec![
            padded_line(subscore_row(
                "Cycles",
                &fmt_pct(n.cycle),
                d.cycle_debt,
                d.new_cycles as f64,
            )),
            padded_line(subscore_row(
                "Layering",
                &fmt_pct(n.layering),
                d.layering_debt,
                d.layering_violations as f64,
            )),
            padded_line(subscore_row("Hub/God", &fmt_pct(n.hub), d.hub_debt, 0.0)),
            padded_line(subscore_row(
                "Coupling",
                &fmt_pct(n.coupling),
                d.coupling_debt,
                0.0,
            )),
            padded_line(subscore_row(
                "Cognitive",
                &fmt_pct(n.cognitive),
                d.cognitive_debt,
                0.0,
            )),
            padded_line(subscore_row(
                "Instability",
                &fmt_pct(n.instability),
                d.instability_debt,
                0.0,
            )),
        ];
        frame.render_widget(
            Paragraph::new(metric_lines).block(
                Block::default()
                    .title(Span::styled(
                        " Risk drivers ",
                        Style::default()
                            .fg(ACCENT_LAVENDER)
                            .add_modifier(Modifier::BOLD),
                    ))
                    .borders(Borders::TOP)
                    .border_style(Style::default().fg(Color::Rgb(50, 50, 70))),
            ),
            chunks[5],
        );

        let advisory_lines = advisory_lines
            .iter()
            .filter(|line| is_actionable_advisory_line(line))
            .cloned()
            .collect::<Vec<_>>();
        let mut adv_lines: Vec<Line> = Vec::new();

        let max_lines = (chunks[7].height.saturating_sub(1) as usize).max(2);
        let mut used_lines = 0usize;

        if !context_lines.is_empty() {
            adv_lines.push(Line::from(Span::styled(
                " Context",
                Style::default()
                    .fg(ACCENT_BLUE)
                    .add_modifier(Modifier::BOLD),
            )));
            for line in context_lines {
                if used_lines >= max_lines.saturating_sub(1) {
                    break;
                }
                adv_lines.push(Line::from(vec![
                    Span::styled(" \u{25b8} ", Style::default().fg(ACCENT_BLUE)),
                    Span::styled(line.clone(), Style::default().fg(FG_TEXT)),
                ]));
                used_lines += 1;
            }
        }

        if !advisory_lines.is_empty() && used_lines < max_lines {
            if !adv_lines.is_empty() {
                adv_lines.push(Line::from(""));
            }
            for line in &advisory_lines {
                if used_lines >= max_lines {
                    break;
                }
                adv_lines.push(Line::from(vec![
                    Span::styled(" \u{25b8} ", Style::default().fg(ACCENT_LAVENDER)),
                    Span::styled(line.clone(), Style::default().fg(FG_TEXT)),
                ]));
                used_lines += 1;
            }
            if advisory_lines.len() > used_lines.saturating_sub(context_lines.len()) {
                adv_lines.push(Line::from(Span::styled(
                    format!(
                        "   +{} more suggestions",
                        advisory_lines
                            .len()
                            .saturating_sub(used_lines.saturating_sub(context_lines.len()))
                    ),
                    Style::default().fg(FG_OVERLAY),
                )));
            }
        }

        if adv_lines.is_empty() {
            // Fallback: generic recommendation
            let rec = generate_recommendation(d);
            adv_lines.push(Line::from(Span::styled(
                format!("  {}", rec),
                Style::default().fg(FG_TEXT),
            )));
        }

        let advisory = Paragraph::new(adv_lines).wrap(Wrap { trim: true }).block(
            Block::default()
                .title(Span::styled(
                    " Suggested actions ",
                    Style::default()
                        .fg(Color::Black)
                        .bg(ACCENT_LAVENDER)
                        .add_modifier(Modifier::BOLD),
                ))
                .borders(Borders::TOP)
                .border_style(Style::default().fg(Color::Rgb(50, 50, 70))),
        );
        frame.render_widget(advisory, chunks[7]);
    } else {
        frame.render_widget(
            Paragraph::new(" WAITING FOR DATA...").style(Style::default().fg(FG_OVERLAY)),
            area,
        );
    }
}

fn trend_direction(trend_data: &[u64]) -> (&'static str, Color) {
    if trend_data.len() >= 2 {
        let prev = trend_data[trend_data.len().saturating_sub(2)];
        let curr = trend_data[trend_data.len() - 1];
        if curr < prev {
            ("improving", Color::Rgb(166, 227, 161))
        } else if curr > prev {
            ("degrading", Color::Rgb(243, 139, 168))
        } else {
            ("stable", ACCENT_BLUE)
        }
    } else {
        ("stable", FG_OVERLAY)
    }
}

fn health_stats(health_data: &[u64]) -> (u64, u64, u64) {
    if !health_data.is_empty() {
        let min = *health_data.iter().min().unwrap_or(&0);
        let max = *health_data.iter().max().unwrap_or(&100);
        let sum: u64 = health_data.iter().sum();
        let avg = sum / health_data.len() as u64;
        (min, max, avg)
    } else {
        (0, 100, 50)
    }
}

fn inset_rect(area: Rect, horizontal: u16, vertical: u16) -> Rect {
    let x = area.x.saturating_add(horizontal);
    let y = area.y.saturating_add(vertical);
    let width = area
        .width
        .saturating_sub(horizontal.saturating_mul(2))
        .max(1);
    let height = area
        .height
        .saturating_sub(vertical.saturating_mul(2))
        .max(1);
    Rect::new(x, y, width, height)
}

fn padded_line(line: Line<'static>) -> Line<'static> {
    let mut spans = Vec::with_capacity(line.spans.len() + 1);
    spans.push(Span::raw(" "));
    spans.extend(line.spans);
    Line::from(spans)
}

fn is_actionable_advisory_line(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    [
        "break ",
        "consider ",
        "simplify",
        "tighten",
        "review ",
        "split ",
        "introduce ",
        "stabilize ",
        "add abstractions",
        "reduce ",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
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

    if let Some(snapshot) = app.snapshot_cache.peek(&current_meta.commit_hash) {
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
            let display_name = truncate_str(name, 14);
            in_lines.push(Line::from(vec![
                Span::styled(format!("{:>2}. ", i + 1), Style::default().fg(FG_OVERLAY)),
                Span::styled(
                    format!("{:<15}", display_name),
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
            let display_name = truncate_str(name, 14);
            out_lines.push(Line::from(vec![
                Span::styled(format!("{:>2}. ", i + 1), Style::default().fg(FG_OVERLAY)),
                Span::styled(
                    format!("{:<15}", display_name),
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

// ── Blast Radius Panel ──

/// Renders the Blast Radius insight tab.
///
/// Layout:
///   - Summary: keystones count, max impact, chain depth
///   - Keystones: top articulation points
///   - Top Impact: sorted module list with blast score bars
pub fn render_blast_radius_panel(
    frame: &mut Frame,
    area: Rect,
    blast_radius: &Option<BlastRadiusReport>,
    scroll_offset: usize,
) {
    if let Some(br) = blast_radius {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3), // Summary stats
                Constraint::Length(5), // Articulation points
                Constraint::Min(3),    // Top impact modules
            ])
            .split(area);

        // ── Summary ──
        let summary_lines = vec![
            Line::from(vec![
                Span::styled(" Keystones: ", Style::default().fg(ACCENT_LAVENDER)),
                Span::styled(
                    format!("{}", br.summary.articulation_point_count),
                    Style::default().fg(FG_TEXT).add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("  Chain: {}", br.summary.longest_chain_depth),
                    Style::default().fg(FG_OVERLAY),
                ),
            ]),
            Line::from(vec![
                Span::styled(" Max Impact: ", Style::default().fg(ACCENT_LAVENDER)),
                Span::styled(
                    format!("{:.0}%", br.summary.max_blast_score * 100.0),
                    Style::default()
                        .fg(blast_color(br.summary.max_blast_score))
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!(" ({})", truncate_str(&br.summary.most_impactful_module, 16)),
                    Style::default().fg(FG_OVERLAY),
                ),
            ]),
        ];
        frame.render_widget(
            Paragraph::new(summary_lines).block(
                Block::default()
                    .title(Span::styled(
                        " REPO BLAST ",
                        Style::default()
                            .fg(Color::Rgb(30, 30, 46))
                            .bg(ACCENT_LAVENDER)
                            .add_modifier(Modifier::BOLD),
                    ))
                    .style(Style::default().bg(BG_SURFACE)),
            ),
            chunks[0],
        );

        // ── Articulation Points ──
        let ap_lines: Vec<Line> = if br.articulation_points.is_empty() {
            vec![Line::from(Span::styled(
                " No keystones — redundant graph",
                Style::default().fg(FG_OVERLAY),
            ))]
        } else {
            br.articulation_points
                .iter()
                .take(4)
                .map(|ap| {
                    let name_display = truncate_str(&ap.module_name, 16);
                    Line::from(vec![
                        Span::styled(" ◆ ", Style::default().fg(Color::Rgb(243, 139, 168))),
                        Span::styled(name_display, Style::default().fg(FG_TEXT)),
                        Span::styled(
                            format!(" ({}in/{}out)", ap.fan_in, ap.fan_out),
                            Style::default().fg(FG_OVERLAY),
                        ),
                    ])
                })
                .collect()
        };
        frame.render_widget(
            Paragraph::new(ap_lines).block(
                Block::default()
                    .title(Span::styled(
                        " REPO KEYSTONES ",
                        Style::default()
                            .fg(ACCENT_LAVENDER)
                            .add_modifier(Modifier::BOLD),
                    ))
                    .style(Style::default().bg(BG_SURFACE)),
            ),
            chunks[1],
        );

        // ── Top Impact Modules (scrollable) ──
        let total_impacts = br.impacts.len();
        // Reserve 1 row for the block title
        let avail_rows = chunks[2].height.saturating_sub(1) as usize;
        let has_more_above = scroll_offset > 0;
        // Two-pass: estimate data_rows accounting for indicators
        let above_row = if has_more_above { 1 } else { 0 };
        // Pessimistic: assume "more below" takes a row, compute data_rows
        let data_rows_pessimistic = avail_rows.saturating_sub(above_row + 1).max(1);
        let has_more_below = total_impacts > scroll_offset + data_rows_pessimistic;
        // Final calculation with actual indicator count
        let below_row = if has_more_below { 1 } else { 0 };
        let data_rows = avail_rows.saturating_sub(above_row + below_row).max(1);

        let mut impact_lines: Vec<Line> = Vec::new();

        // Scroll-up indicator
        if has_more_above {
            impact_lines.push(Line::from(Span::styled(
                format!(" ▲ {} more above", scroll_offset),
                Style::default().fg(FG_OVERLAY),
            )));
        }

        // Visible impact entries
        for m in br.impacts.iter().skip(scroll_offset).take(data_rows) {
            let score_pct = (m.blast_score * 100.0) as u32;
            let bar_width = (score_pct / 5).min(10) as usize;
            let bar = "█".repeat(bar_width);
            let name_display = truncate_str(&m.module_name, 14);
            impact_lines.push(Line::from(vec![
                Span::styled(
                    format!(" {:>3}% ", score_pct),
                    Style::default().fg(blast_color(m.blast_score)),
                ),
                Span::styled(bar, Style::default().fg(blast_color(m.blast_score))),
                Span::raw(" "),
                Span::styled(
                    name_display,
                    Style::default().fg(if m.is_articulation_point {
                        Color::Rgb(243, 139, 168) // Red for APs
                    } else {
                        FG_TEXT
                    }),
                ),
            ]));
        }

        // Scroll-down indicator
        if has_more_below {
            let remaining = total_impacts.saturating_sub(scroll_offset + data_rows);
            impact_lines.push(Line::from(Span::styled(
                format!(" ▼ {} more below", remaining),
                Style::default().fg(FG_OVERLAY),
            )));
        }

        frame.render_widget(
            Paragraph::new(impact_lines).block(
                Block::default()
                    .title(Span::styled(
                        " REPO TOP IMPACT ",
                        Style::default()
                            .fg(ACCENT_LAVENDER)
                            .add_modifier(Modifier::BOLD),
                    ))
                    .style(Style::default().bg(BG_SURFACE)),
            ),
            chunks[2],
        );
    } else {
        frame.render_widget(
            Paragraph::new(" No blast radius data. Run scan first.")
                .style(Style::default().fg(FG_OVERLAY)),
            area,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::is_actionable_advisory_line;

    #[test]
    fn actionable_advisory_filter_keeps_recommendations() {
        assert!(is_actionable_advisory_line(
            "Some modules share more dependencies than expected. Review coupling and consider interfaces."
        ));
        assert!(is_actionable_advisory_line(
            "2 circular dependency groups found. Consider dependency inversion."
        ));
    }

    #[test]
    fn actionable_advisory_filter_drops_plain_facts() {
        assert!(!is_actionable_advisory_line(
            "Strongest link: libs/core → std (399 imports). This tight binding makes both harder to change."
        ));
        assert!(!is_actionable_advisory_line(
            "The graph has 173% more connections than typical. Fewer links would make the architecture easier to reason about."
        ));
    }
}
