// =============================================================================
// tui/timeline.rs — Commit timeline slider widget
// =============================================================================
//
// Timeline slider shown in the bottom panel:
//   - One position per commit
//   - Navigate with j/k
//   - Current commit highlighted
//   - Shows commit hash + message summary
//
// Does not implement ratatui::widgets::Widget trait (uses direct render
// functions instead) because state updates are needed.
// =============================================================================

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

use super::graph_renderer::{ACCENT_BLUE, ACCENT_LAVENDER, BG_SURFACE, FG_OVERLAY, FG_TEXT};

/// Timeline state — current position and commit list.
pub struct TimelineState {
    /// Commit info: (hash, first line of message, timestamp)
    pub commits: Vec<(String, String, i64)>,
    /// Current selected commit index (0 = newest)
    pub current_index: usize,
}

impl TimelineState {
    /// Creates a new timeline.
    pub fn new(commits: Vec<(String, String, i64)>) -> Self {
        Self {
            current_index: 0,
            commits,
        }
    }

    /// Advance to the next commit (older).
    pub fn next(&mut self) {
        if self.current_index + 1 < self.commits.len() {
            self.current_index += 1;
        }
    }

    /// Go back to the previous commit (newer).
    pub fn prev(&mut self) {
        if self.current_index > 0 {
            self.current_index -= 1;
        }
    }

    /// Returns the current commit's hash (None = empty timeline).
    pub fn current_commit_hash(&self) -> Option<&str> {
        self.commits
            .get(self.current_index)
            .map(|(h, _, _)| h.as_str())
    }

    /// Returns the current commit's message.
    pub fn current_commit_message(&self) -> Option<&str> {
        self.commits
            .get(self.current_index)
            .map(|(_, m, _)| m.as_str())
    }

    /// Total commit count.
    pub fn len(&self) -> usize {
        self.commits.len()
    }

    /// Is the timeline empty?
    pub fn is_empty(&self) -> bool {
        self.commits.is_empty()
    }
}

/// Renders the timeline widget.
///
/// Top line: slider bar (█ = current position)
/// Bottom line: commit hash + message
pub fn render_timeline(frame: &mut Frame, area: Rect, state: &TimelineState) {
    let block = Block::default()
        .title(" Timeline (j/k) ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(FG_OVERLAY))
        .style(Style::default().bg(BG_SURFACE));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if state.is_empty() {
        let empty = Paragraph::new("  No commits yet. Run 'morpharch scan .' first.")
            .style(Style::default().fg(FG_OVERLAY));
        frame.render_widget(empty, inner);
        return;
    }

    // ── Build slider bar ──
    let total = state.len();
    let bar_width = inner.width.saturating_sub(2) as usize;

    let slider_chars: String = if bar_width > 0 && total > 0 {
        let pos = if total <= 1 {
            0
        } else {
            (state.current_index * (bar_width.saturating_sub(1))) / (total.saturating_sub(1))
        };
        (0..bar_width)
            .map(|i| if i == pos { '█' } else { '─' })
            .collect()
    } else {
        String::new()
    };

    // ── Commit info ──
    let hash = state.current_commit_hash().unwrap_or("?");
    let short_hash = if hash.len() >= 7 { &hash[..7] } else { hash };
    let message = state.current_commit_message().unwrap_or("");
    let truncated_msg = if message.len() > 50 {
        format!("{}…", &message[..49])
    } else {
        message.to_string()
    };

    let lines = vec![
        Line::from(vec![
            Span::styled(" ", Style::default().fg(FG_TEXT)),
            Span::styled(slider_chars, Style::default().fg(ACCENT_BLUE)),
        ]),
        Line::from(vec![
            Span::styled(
                format!(" [{}/{}] ", state.current_index + 1, total),
                Style::default()
                    .fg(ACCENT_LAVENDER)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(format!("{short_hash} "), Style::default().fg(ACCENT_BLUE)),
            Span::styled(truncated_msg, Style::default().fg(FG_TEXT)),
        ]),
    ];

    let paragraph = Paragraph::new(lines);
    frame.render_widget(paragraph, inner);
}

// =============================================================================
// Tests
// =============================================================================
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_timeline_navigation() {
        let commits = vec![
            ("aaa".to_string(), "First".to_string(), 3),
            ("bbb".to_string(), "Second".to_string(), 2),
            ("ccc".to_string(), "Third".to_string(), 1),
        ];
        let mut state = TimelineState::new(commits);

        assert_eq!(state.current_index, 0);
        assert_eq!(state.current_commit_hash(), Some("aaa"));

        state.next();
        assert_eq!(state.current_index, 1);
        assert_eq!(state.current_commit_hash(), Some("bbb"));

        state.next();
        assert_eq!(state.current_index, 2);
        assert_eq!(state.current_commit_hash(), Some("ccc"));

        // next at last element should not change
        state.next();
        assert_eq!(state.current_index, 2);

        state.prev();
        assert_eq!(state.current_index, 1);

        state.prev();
        state.prev();
        // prev at first element should not change
        assert_eq!(state.current_index, 0);
    }

    #[test]
    fn test_timeline_empty() {
        let state = TimelineState::new(vec![]);
        assert!(state.is_empty());
        assert_eq!(state.current_commit_hash(), None);
        assert_eq!(state.len(), 0);
    }
}
