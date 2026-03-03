// =============================================================================
// tui/widgets.rs — Shared TUI widget helpers
// =============================================================================
//
// Left panel package list and other helper widgets:
//   - render_package_list: Module/package list from current graph
//   - truncate_str: Truncate long strings
//   - format_timestamp: Unix timestamp → readable date
//
// This module is imported by other tui modules.
// =============================================================================

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

use super::graph_renderer::{ACCENT_BLUE, ACCENT_LAVENDER, BG_SURFACE, FG_OVERLAY, FG_TEXT};

/// Renders the package/module list in the left panel.
///
/// Each module name is listed in order. If a search query is active,
/// matching modules are highlighted.
///
/// # Parameters
/// - `labels`: Module names in the current graph
/// - `search_query`: Active search query (empty = no filtering)
pub fn render_package_list(
    frame: &mut Frame,
    area: Rect,
    labels: &[String],
    search_query: &str,
    scroll_offset: usize,
) {
    let block = Block::default()
        .title(format!(" Packages ({}) ", labels.len()))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(ACCENT_BLUE))
        .style(Style::default().bg(BG_SURFACE));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if labels.is_empty() {
        let empty = Paragraph::new("  (empty)").style(Style::default().fg(FG_OVERLAY));
        frame.render_widget(empty, inner);
        return;
    }

    let max_visible = inner.height as usize;
    let query_lower = search_query.to_lowercase();

    // Sort labels alphabetically (case-insensitive) for consistent display
    let mut sorted_labels: Vec<String> = labels.to_vec();
    sorted_labels.sort_by_key(|a| a.to_lowercase());

    let mut lines: Vec<Line> = Vec::new();

    // Apply scroll offset (clamped to valid range)
    let effective_offset = scroll_offset.min(sorted_labels.len().saturating_sub(1));
    // Reserve 1 line for the scroll indicator at the bottom
    let list_height = if sorted_labels.len() > max_visible {
        max_visible.saturating_sub(1)
    } else {
        max_visible
    };

    for (i, label) in sorted_labels.iter().enumerate().skip(effective_offset) {
        if lines.len() >= list_height {
            break;
        }

        let short = truncate_str(label, inner.width.saturating_sub(4) as usize);

        // Does the search query match?
        let is_match = !query_lower.is_empty() && label.to_lowercase().contains(&query_lower);

        let style = if is_match {
            Style::default()
                .fg(ACCENT_LAVENDER)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(FG_TEXT)
        };

        let idx_str = format!(" {:>3}.", i + 1);

        lines.push(Line::from(vec![
            Span::styled(
                idx_str,
                if is_match {
                    Style::default().fg(ACCENT_LAVENDER)
                } else {
                    Style::default().fg(FG_OVERLAY)
                },
            ),
            Span::styled(short.to_string(), style),
        ]));
    }

    // Scroll indicator
    if sorted_labels.len() > max_visible {
        let visible_end = (effective_offset + list_height).min(sorted_labels.len());
        lines.push(Line::from(Span::styled(
            format!(
                "  [{}-{}/{}] [/] scroll",
                effective_offset + 1,
                visible_end,
                sorted_labels.len()
            ),
            Style::default().fg(FG_OVERLAY),
        )));
    }

    let paragraph = Paragraph::new(lines);
    frame.render_widget(paragraph, inner);
}

/// Truncates a long string to the specified width.
///
/// If the width is exceeded, "…" is appended.
pub fn truncate_str(s: &str, max_width: usize) -> String {
    if max_width == 0 {
        return String::new();
    }
    if s.len() <= max_width {
        s.to_string()
    } else if max_width <= 1 {
        "…".to_string()
    } else {
        format!("{}…", &s[..max_width - 1])
    }
}

/// Converts a Unix timestamp to a readable date string.
///
/// Format: "YYYY-MM-DD HH:MM"
/// Invalid timestamps return "?".
#[allow(dead_code)]
pub fn format_timestamp(timestamp: i64) -> String {
    chrono::DateTime::from_timestamp(timestamp, 0)
        .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_else(|| "?".to_string())
}

// =============================================================================
// Tests
// =============================================================================
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_truncate_str_short() {
        assert_eq!(truncate_str("hello", 10), "hello");
    }

    #[test]
    fn test_truncate_str_exact() {
        assert_eq!(truncate_str("hello", 5), "hello");
    }

    #[test]
    fn test_truncate_str_long() {
        let result = truncate_str("hello_world_module", 10);
        assert_eq!(result, "hello_wor…");
        assert!(result.len() <= 12); // UTF-8 … = 3 bytes
    }

    #[test]
    fn test_truncate_str_zero() {
        assert_eq!(truncate_str("hello", 0), "");
    }

    #[test]
    fn test_truncate_str_one() {
        assert_eq!(truncate_str("hello", 1), "…");
    }

    #[test]
    fn test_format_timestamp_valid() {
        // 2024-01-01 00:00:00 UTC
        let result = format_timestamp(1_704_067_200);
        assert!(result.contains("2024"), "Year should be 2024: {result}");
    }

    #[test]
    fn test_format_timestamp_zero() {
        let result = format_timestamp(0);
        assert!(result.contains("1970"), "Epoch start: {result}");
    }
}
