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

    /// Jump forward/backward by `n` commits.
    ///
    /// Positive `n` jumps forward (older), negative jumps backward (newer).
    /// Clamps to valid range.
    pub fn jump_by(&mut self, n: isize) {
        if self.commits.is_empty() {
            return;
        }
        let max = self.commits.len() - 1;
        let new_idx = (self.current_index as isize + n).clamp(0, max as isize) as usize;
        self.current_index = new_idx;
    }

    /// Jump to the first commit (index 0).
    pub fn jump_to_start(&mut self) {
        self.current_index = 0;
    }

    /// Jump to the last commit.
    pub fn jump_to_end(&mut self) {
        if !self.commits.is_empty() {
            self.current_index = self.commits.len() - 1;
        }
    }

    /// Jump to a specific index (clamped to valid range).
    pub fn jump_to(&mut self, index: usize) {
        if self.commits.is_empty() {
            return;
        }
        self.current_index = index.min(self.commits.len() - 1);
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

    /// Returns the current commit's timestamp.
    pub fn current_commit_timestamp(&self) -> i64 {
        self.commits
            .get(self.current_index)
            .map(|(_, _, ts)| *ts)
            .unwrap_or(0)
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
        .title(" Timeline (j/k H/L ±10  Home/End  +/-:speed  click:seek) ")
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
    let truncated_msg = if message.chars().count() > 50 {
        format!("{}…", message.chars().take(49).collect::<String>())
    } else {
        message.to_string()
    };

    let timestamp = state.current_commit_timestamp();
    let date_str = if timestamp > 0 {
        chrono::DateTime::from_timestamp(timestamp, 0)
            .map(|dt| dt.format("%Y-%m-%d").to_string())
            .unwrap_or_else(|| "-----".to_string())
    } else {
        "-----".to_string()
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
            Span::styled(format!("{date_str} "), Style::default().fg(FG_OVERLAY)),
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

    #[test]
    fn test_timeline_jump_by() {
        let commits: Vec<(String, String, i64)> = (0..20)
            .map(|i| (format!("hash{i}"), format!("Msg {i}"), i as i64))
            .collect();
        let mut state = TimelineState::new(commits);
        assert_eq!(state.current_index, 0);

        // Jump forward by 10
        state.jump_by(10);
        assert_eq!(state.current_index, 10);

        // Jump forward by 10 again — should clamp to 19
        state.jump_by(10);
        assert_eq!(state.current_index, 19);

        // Jump backward by 5
        state.jump_by(-5);
        assert_eq!(state.current_index, 14);

        // Jump backward past 0 — should clamp to 0
        state.jump_by(-100);
        assert_eq!(state.current_index, 0);
    }

    #[test]
    fn test_timeline_jump_to_start_end() {
        let commits: Vec<(String, String, i64)> = (0..50)
            .map(|i| (format!("hash{i}"), format!("Msg {i}"), i as i64))
            .collect();
        let mut state = TimelineState::new(commits);

        state.jump_to_end();
        assert_eq!(state.current_index, 49);

        state.jump_to_start();
        assert_eq!(state.current_index, 0);
    }

    #[test]
    fn test_timeline_jump_to() {
        let commits: Vec<(String, String, i64)> = (0..10)
            .map(|i| (format!("hash{i}"), format!("Msg {i}"), i as i64))
            .collect();
        let mut state = TimelineState::new(commits);

        state.jump_to(5);
        assert_eq!(state.current_index, 5);

        // Out of bounds — should clamp
        state.jump_to(100);
        assert_eq!(state.current_index, 9);
    }
}
