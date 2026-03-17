// =============================================================================
// tui/ai_panel.rs — AI Assistant inline bottom drawer panel
// =============================================================================
//
// Replaces the old modal "Sor" overlay with a k9s-style inline panel that
// supports streaming responses, cursor-based input, conversation history,
// and context-aware suggested questions.
// =============================================================================

use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, Borders, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState, Wrap,
};
use tui_markdown;

use super::graph_renderer::{ACCENT_LAVENDER, BG_BASE, FG_OVERLAY, FG_TEXT};

// Re-use app.rs color constants (duplicated here to avoid circular deps)
const FG_SUBTEXT: Color = Color::Rgb(166, 173, 200);
const COLOR_HEALTHY: Color = Color::Rgb(166, 227, 161);
const BORDER_FOCUSED: Color = Color::Rgb(180, 190, 254);

const SPINNER_FRAMES: &[char] = &[
    '\u{28FE}', '\u{28FD}', '\u{28FB}', '\u{28BF}', '\u{287F}', '\u{28DF}', '\u{28EF}', '\u{28F7}',
];

const DEFAULT_PANEL_HEIGHT: u16 = 12;
pub const MIN_PANEL_HEIGHT: u16 = 6;
pub const MAX_PANEL_HEIGHT: u16 = 30;

// ─── Data types ─────────────────────────────────────────────────────────────

/// A chunk received from the streaming LLM response.
pub enum StreamChunk {
    /// Incremental text delta.
    Delta(String),
    /// Stream completed successfully.
    Done,
    /// Stream encountered an error.
    Error(String),
}

/// Short label of the user's current view context when a question was asked.
#[derive(Debug, Clone)]
pub struct ContextLabel {
    /// e.g. "Overview", "Cluster: libs/core", "Module: parser.rs"
    pub view: String,
    /// e.g. "Graph", "Insights/Hotspots"
    pub focused: String,
}

/// A single question-answer pair in the conversation.
#[derive(Debug, Clone)]
pub struct ConversationEntry {
    pub question: String,
    pub answer: String,
    pub context_label: ContextLabel,
    pub is_complete: bool,
}

// ─── InputBuffer ────────────────────────────────────────────────────────────

/// Text input buffer with cursor position and query history recall.
#[derive(Debug, Clone, Default)]
pub struct InputBuffer {
    pub text: String,
    pub cursor: usize,
    history: Vec<String>,
    history_index: Option<usize>,
    /// Stashed current input when browsing history.
    stash: Option<String>,
}

impl InputBuffer {
    pub fn insert_char(&mut self, c: char) {
        let byte_pos = self.byte_offset();
        self.text.insert(byte_pos, c);
        self.cursor += 1;
        self.reset_history_nav();
    }

    pub fn delete_char_before(&mut self) {
        if self.cursor == 0 {
            return;
        }
        self.cursor -= 1;
        let byte_pos = self.byte_offset();
        let next = self.next_char_boundary(byte_pos);
        self.text.drain(byte_pos..next);
        self.reset_history_nav();
    }

    pub fn move_left(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    pub fn move_right(&mut self) {
        let char_count = self.text.chars().count();
        if self.cursor < char_count {
            self.cursor += 1;
        }
    }

    pub fn move_to_start(&mut self) {
        self.cursor = 0;
    }

    pub fn move_to_end(&mut self) {
        self.cursor = self.text.chars().count();
    }

    pub fn delete_word_before(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let chars: Vec<char> = self.text.chars().collect();
        let mut new_cursor = self.cursor;
        // Skip trailing whitespace
        while new_cursor > 0 && chars[new_cursor - 1].is_whitespace() {
            new_cursor -= 1;
        }
        // Skip word characters
        while new_cursor > 0 && !chars[new_cursor - 1].is_whitespace() {
            new_cursor -= 1;
        }
        let start_byte = self.char_to_byte(new_cursor);
        let end_byte = self.byte_offset();
        self.text.drain(start_byte..end_byte);
        self.cursor = new_cursor;
        self.reset_history_nav();
    }

    pub fn clear(&mut self) {
        self.text.clear();
        self.cursor = 0;
        self.reset_history_nav();
    }

    pub fn push_history(&mut self, query: String) {
        if !query.is_empty() {
            // Avoid consecutive duplicates
            if self.history.last() != Some(&query) {
                self.history.push(query);
            }
        }
        self.reset_history_nav();
    }

    pub fn recall_prev(&mut self) {
        if self.history.is_empty() {
            return;
        }
        match self.history_index {
            None => {
                self.stash = Some(self.text.clone());
                let idx = self.history.len().saturating_sub(1);
                self.history_index = Some(idx);
                self.text = self.history[idx].clone();
            }
            Some(idx) if idx > 0 => {
                let new_idx = idx - 1;
                self.history_index = Some(new_idx);
                self.text = self.history[new_idx].clone();
            }
            _ => return,
        }
        self.cursor = self.text.chars().count();
    }

    pub fn recall_next(&mut self) {
        if let Some(idx) = self.history_index {
            if idx + 1 < self.history.len() {
                let new_idx = idx + 1;
                self.history_index = Some(new_idx);
                self.text = self.history[new_idx].clone();
            } else {
                // Restore stashed input
                self.text = self.stash.take().unwrap_or_default();
                self.history_index = None;
            }
            self.cursor = self.text.chars().count();
        }
    }

    pub fn is_empty(&self) -> bool {
        self.text.trim().is_empty()
    }

    pub fn take_text(&mut self) -> String {
        let text = self.text.trim().to_string();
        self.text.clear();
        self.cursor = 0;
        self.reset_history_nav();
        text
    }

    // ── Private helpers ──

    fn byte_offset(&self) -> usize {
        self.char_to_byte(self.cursor)
    }

    fn char_to_byte(&self, char_idx: usize) -> usize {
        self.text
            .char_indices()
            .nth(char_idx)
            .map(|(i, _)| i)
            .unwrap_or(self.text.len())
    }

    fn next_char_boundary(&self, byte_pos: usize) -> usize {
        let mut end = byte_pos + 1;
        while end < self.text.len() && !self.text.is_char_boundary(end) {
            end += 1;
        }
        end.min(self.text.len())
    }

    fn reset_history_nav(&mut self) {
        self.history_index = None;
        self.stash = None;
    }
}

// ─── AiPanelState ───────────────────────────────────────────────────────────

/// Full state for the AI assistant inline panel.
pub struct AiPanelState {
    /// Whether the AI panel is visible.
    pub visible: bool,
    /// Current input buffer with cursor and history.
    pub input: InputBuffer,
    /// Conversation history for this session.
    pub conversation: Vec<ConversationEntry>,
    /// Streaming response receiver channel.
    pub stream_rx: Option<tokio::sync::mpsc::UnboundedReceiver<StreamChunk>>,
    /// Whether a query is currently in-flight.
    pub is_streaming: bool,
    /// Scroll offset for conversation history area.
    pub scroll_offset: usize,
    /// Panel height in rows (user-adjustable via drag).
    pub panel_height: u16,
    /// Whether user is dragging the top border to resize.
    pub resizing: bool,
    /// Currently highlighted suggestion index (None = input focused).
    pub suggestion_index: Option<usize>,
    /// Cached suggested questions.
    pub suggestions: Vec<String>,
    /// Whether to auto-scroll to bottom on new content.
    pub auto_scroll: bool,
    /// Spinner frame counter for streaming animation.
    pub spinner_tick: usize,
    /// Last submitted query (for Ctrl+R retry).
    pub last_query: Option<String>,
    /// Total wrapped-line count from the last render (for scroll clamping).
    pub total_lines: usize,
    /// AI configuration.
    pub ai_config: crate::config::AiConfig,
    /// Area rect stored during render for mouse hit testing.
    pub panel_area: Rect,
    /// Known module names from the current graph, sorted by length descending
    /// for longest-match-first highlighting in AI responses.
    pub known_modules: Vec<String>,
    /// Pending /diff command data (resolved by App, then submitted as AI query).
    pub pending_diff_query: Option<usize>,
}

impl Default for AiPanelState {
    fn default() -> Self {
        Self {
            visible: false,
            input: InputBuffer::default(),
            conversation: Vec::new(),
            stream_rx: None,
            is_streaming: false,
            scroll_offset: 0,
            panel_height: DEFAULT_PANEL_HEIGHT,
            resizing: false,
            suggestion_index: None,
            suggestions: Vec::new(),
            auto_scroll: true,
            last_query: None,
            spinner_tick: 0,
            total_lines: 0,
            ai_config: crate::config::AiConfig::default(),
            panel_area: Rect::default(),
            known_modules: Vec::new(),
            pending_diff_query: None,
        }
    }
}

impl AiPanelState {
    /// Toggle panel visibility. Resets input focus when opening.
    pub fn toggle(&mut self) {
        self.visible = !self.visible;
        if self.visible {
            self.suggestion_index = None;
        }
    }

    /// Scroll the conversation up by `amount` lines.
    pub fn scroll_up(&mut self, amount: usize) {
        self.scroll_offset = self.scroll_offset.saturating_sub(amount);
        self.auto_scroll = false;
    }

    /// Scroll the conversation down by `amount` lines.
    pub fn scroll_down(&mut self, amount: usize) {
        self.scroll_offset = self.scroll_offset.saturating_add(amount);
        // Re-enable auto_scroll if we've scrolled to the bottom
        // (will be clamped to max_scroll in render)
    }

    /// Handle a key event when the AI panel is visible.
    /// Returns `true` if the key was consumed by the panel.
    pub fn handle_key(&mut self, code: KeyCode, modifiers: KeyModifiers) -> bool {
        if !self.visible {
            return false;
        }

        let ctrl = modifiers.contains(KeyModifiers::CONTROL);
        let shift = modifiers.contains(KeyModifiers::SHIFT);

        match code {
            KeyCode::Esc => {
                if self.is_streaming {
                    // Cancel in-flight stream
                    self.is_streaming = false;
                    self.stream_rx = None;
                    if let Some(last) = self.conversation.last_mut()
                        && !last.is_complete
                    {
                        last.answer.push_str(" [cancelled]");
                        last.is_complete = true;
                    }
                } else if !self.input.text.is_empty() {
                    self.input.clear();
                    self.suggestion_index = None;
                } else {
                    self.visible = false;
                }
            }

            KeyCode::Enter => {
                // If a suggestion is highlighted, accept it into input
                if let Some(idx) = self.suggestion_index {
                    if let Some(suggestion) = self.suggestions.get(idx).cloned() {
                        self.input.text = suggestion;
                        self.input.cursor = self.input.text.chars().count();
                        self.suggestion_index = None;
                    }
                    return true;
                }
                // Submit is handled by the caller (needs app state for context)
                // Return false to signal "Enter pressed with text ready"
                if !self.input.is_empty() && !self.is_streaming {
                    return false;
                }
            }

            KeyCode::Tab if !ctrl => {
                if self.suggestions.is_empty() {
                    return true;
                }
                if shift {
                    // Cycle backward
                    self.suggestion_index = Some(match self.suggestion_index {
                        Some(0) | None => self.suggestions.len().saturating_sub(1),
                        Some(idx) => idx - 1,
                    });
                } else {
                    // Cycle forward
                    self.suggestion_index = Some(match self.suggestion_index {
                        None => 0,
                        Some(idx) => (idx + 1) % self.suggestions.len(),
                    });
                }
            }

            KeyCode::Up if !ctrl => {
                if self.input.text.is_empty()
                    && self.suggestion_index.is_none()
                    && self.scroll_offset == 0
                    && self.input.history_index.is_none()
                {
                    self.input.recall_prev();
                } else if self.suggestion_index.is_none() {
                    self.scroll_up(1);
                }
            }
            KeyCode::Down if !ctrl => {
                if self.input.history_index.is_some() {
                    self.input.recall_next();
                } else if self.suggestion_index.is_none() {
                    self.scroll_down(1);
                }
            }

            KeyCode::Left if !ctrl => self.input.move_left(),
            KeyCode::Right if !ctrl => self.input.move_right(),
            KeyCode::Home => self.input.move_to_start(),
            KeyCode::End => self.input.move_to_end(),

            KeyCode::Char('a') if ctrl => self.input.move_to_start(),
            KeyCode::Char('e') if ctrl => self.input.move_to_end(),
            KeyCode::Char('u') if ctrl => self.input.clear(),
            KeyCode::Char('w') if ctrl => self.input.delete_word_before(),
            KeyCode::Char('r') if ctrl => {
                // Retry last query — put it in input for submission
                if let Some(ref last) = self.last_query
                    && !self.is_streaming
                {
                    self.input.text = last.clone();
                    self.input.cursor = self.input.text.chars().count();
                    return false;
                }
            }

            KeyCode::Backspace => self.input.delete_char_before(),

            KeyCode::Char(c) if !ctrl => {
                self.input.insert_char(c);
                self.suggestion_index = None;
            }

            _ => {}
        }

        true
    }

    /// Drain all available stream chunks from the receiver.
    /// Returns `true` if any chunks were processed.
    pub fn drain_stream(&mut self) -> bool {
        let Some(rx) = &mut self.stream_rx else {
            return false;
        };

        let mut any = false;
        loop {
            match rx.try_recv() {
                Ok(StreamChunk::Delta(text)) => {
                    if let Some(last) = self.conversation.last_mut() {
                        last.answer.push_str(&text);
                    }
                    if self.auto_scroll {
                        self.scroll_offset = usize::MAX; // clamped in render
                    }
                    any = true;
                }
                Ok(StreamChunk::Done) => {
                    if let Some(last) = self.conversation.last_mut() {
                        last.is_complete = true;
                    }
                    self.is_streaming = false;
                    self.stream_rx = None;
                    self.update_post_response_suggestions();
                    return true;
                }
                Ok(StreamChunk::Error(e)) => {
                    if let Some(last) = self.conversation.last_mut() {
                        last.answer = format!("Error: {}", e);
                        last.is_complete = true;
                    }
                    self.is_streaming = false;
                    self.stream_rx = None;
                    return true;
                }
                Err(_) => break, // No more chunks this tick
            }
        }
        any
    }

    /// Advance the spinner animation frame.
    pub fn tick_spinner(&mut self) {
        if self.is_streaming {
            self.spinner_tick = self.spinner_tick.wrapping_add(1);
        }
    }

    /// Get the current spinner character.
    fn spinner_char(&self) -> char {
        SPINNER_FRAMES[self.spinner_tick % SPINNER_FRAMES.len()]
    }

    /// Try to execute a slash command. Returns true if the text was a command.
    pub fn try_execute_command(&mut self, text: &str) -> bool {
        let trimmed = text.trim();
        if !trimmed.starts_with('/') {
            return false;
        }

        let cmd = trimmed.split_whitespace().next().unwrap_or("");
        match cmd {
            "/clear" => {
                self.conversation.clear();
                self.scroll_offset = 0;
                self.auto_scroll = true;
                self.input.clear();
            }
            "/help" => {
                self.conversation.push(ConversationEntry {
                    question: "/help".to_string(),
                    answer: "Available commands:\n\
                        /clear    — Clear conversation history\n\
                        /diff N   — Compare architecture with N commits ago\n\
                        /help     — Show this help message\n\
                        /model    — Show current AI model and endpoint\n\
                        /history  — Show conversation statistics\n\
                        /export   — Export conversation to file"
                        .to_string(),
                    context_label: ContextLabel {
                        view: "System".to_string(),
                        focused: "Command".to_string(),
                    },
                    is_complete: true,
                });
                self.input.clear();
                self.auto_scroll = true;
                self.scroll_offset = usize::MAX;
            }
            "/model" => {
                let info = format!(
                    "Provider: {}\nModel: {}\nEndpoint: {}\nStreaming: {}\nMax tokens: {}\nTemperature: {}",
                    self.ai_config.provider,
                    self.ai_config.model,
                    self.ai_config.endpoint,
                    self.ai_config.stream,
                    self.ai_config.max_tokens,
                    self.ai_config.temperature,
                );
                self.conversation.push(ConversationEntry {
                    question: "/model".to_string(),
                    answer: info,
                    context_label: ContextLabel {
                        view: "System".to_string(),
                        focused: "Command".to_string(),
                    },
                    is_complete: true,
                });
                self.input.clear();
                self.auto_scroll = true;
                self.scroll_offset = usize::MAX;
            }
            "/history" => {
                let total = self.conversation.len();
                let complete = self.conversation.iter().filter(|e| e.is_complete).count();
                self.conversation.push(ConversationEntry {
                    question: "/history".to_string(),
                    answer: format!(
                        "Conversation entries: {}\nCompleted: {}\nQuery history buffer: {} items",
                        total,
                        complete,
                        self.input.history.len()
                    ),
                    context_label: ContextLabel {
                        view: "System".to_string(),
                        focused: "Command".to_string(),
                    },
                    is_complete: true,
                });
                self.input.clear();
                self.auto_scroll = true;
                self.scroll_offset = usize::MAX;
            }
            "/export" => {
                let mut content = String::new();
                for entry in &self.conversation {
                    content.push_str(&format!("[{}]\n", entry.context_label.view));
                    content.push_str(&format!("Q: {}\n", entry.question));
                    content.push_str(&format!("A: {}\n\n", entry.answer));
                }
                match std::fs::write("morpharch-ai-conversation.txt", &content) {
                    Ok(_) => {
                        self.conversation.push(ConversationEntry {
                            question: "/export".to_string(),
                            answer: "Conversation exported to morpharch-ai-conversation.txt"
                                .to_string(),
                            context_label: ContextLabel {
                                view: "System".to_string(),
                                focused: "Command".to_string(),
                            },
                            is_complete: true,
                        });
                    }
                    Err(e) => {
                        self.conversation.push(ConversationEntry {
                            question: "/export".to_string(),
                            answer: format!("Export failed: {}", e),
                            context_label: ContextLabel {
                                view: "System".to_string(),
                                focused: "Command".to_string(),
                            },
                            is_complete: true,
                        });
                    }
                }
                self.input.clear();
                self.auto_scroll = true;
                self.scroll_offset = usize::MAX;
            }
            "/diff" => {
                // Parse /diff HEAD~N or /diff N
                let arg = trimmed.strip_prefix("/diff").unwrap_or("").trim();
                let n = if arg.is_empty() {
                    1 // default to previous commit
                } else if let Some(num_str) = arg.strip_prefix("HEAD~") {
                    num_str.parse::<usize>().unwrap_or(1)
                } else {
                    arg.parse::<usize>().unwrap_or(1)
                };
                self.pending_diff_query = Some(n);
                self.input.clear();
            }
            _ => {
                self.conversation.push(ConversationEntry {
                    question: trimmed.to_string(),
                    answer: format!(
                        "Unknown command: {}. Type /help for available commands.",
                        cmd
                    ),
                    context_label: ContextLabel {
                        view: "System".to_string(),
                        focused: "Command".to_string(),
                    },
                    is_complete: true,
                });
                self.input.clear();
                self.auto_scroll = true;
                self.scroll_offset = usize::MAX;
            }
        }
        true
    }

    /// After a response completes, update suggestions with navigation
    /// options for modules mentioned in the AI response.
    pub fn update_post_response_suggestions(&mut self) {
        if self.known_modules.is_empty() {
            return;
        }

        // Get the last completed response
        let last_answer = match self.conversation.last() {
            Some(entry) if entry.is_complete => entry.answer.clone(),
            _ => return,
        };

        // Find mentioned modules (known_modules is sorted by length desc)
        let mut mentioned: Vec<&str> = Vec::new();
        let lower_answer = last_answer.to_lowercase();

        for module in &self.known_modules {
            let lower_module = module.to_lowercase();
            if let Some(pos) = lower_answer.find(&lower_module) {
                let before_ok = pos == 0
                    || lower_answer[..pos]
                        .chars()
                        .next_back()
                        .is_some_and(is_module_boundary);
                let end_pos = pos + lower_module.len();
                let after_ok = end_pos >= lower_answer.len()
                    || lower_answer[end_pos..]
                        .chars()
                        .next()
                        .is_some_and(is_module_boundary);

                if before_ok && after_ok {
                    mentioned.push(module);
                }
            }
        }

        // Build navigation suggestions (max 5, nav suggestions first)
        if !mentioned.is_empty() {
            let mut nav_suggestions: Vec<String> = mentioned
                .iter()
                .take(5)
                .map(|m| format!("\u{2192} Inspect {}", m))
                .collect();

            // Keep up to 2 existing non-navigation suggestions
            let existing: Vec<String> = self
                .suggestions
                .iter()
                .filter(|s| !s.starts_with("\u{2192} "))
                .take(2)
                .cloned()
                .collect();

            nav_suggestions.extend(existing);
            nav_suggestions.truncate(5);
            self.suggestions = nav_suggestions;
        }
    }

    /// Get command suggestions when input starts with `/`.
    pub fn get_command_suggestions(&self) -> Vec<String> {
        let prefix = self.input.text.trim();
        let commands = ["/clear", "/diff", "/help", "/model", "/history", "/export"];
        commands
            .iter()
            .filter(|c| c.starts_with(prefix))
            .map(|c| c.to_string())
            .collect()
    }
}

// ─── Suggestion generation ──────────────────────────────────────────────────

/// Information extracted from App state to generate context-aware suggestions.
pub struct SuggestionContext {
    pub view_name: String,
    pub selected_entity: Option<String>,
    pub insight_tab: String,
    pub cycle_debt: f64,
    pub health_percent: u8,
    pub trend_declining: bool,
    pub has_blast_data: bool,
    pub top_brittle: Option<String>,
    pub is_latest_commit: bool,
    pub commit_message: Option<String>,
}

/// Generate context-aware suggested questions based on the current view state.
pub fn generate_suggestions(ctx: &SuggestionContext) -> Vec<String> {
    let mut suggestions = Vec::with_capacity(5);

    // Timeline-aware suggestions
    if !ctx.is_latest_commit {
        suggestions.insert(0, "What changed in this commit?".to_string());
    }

    let has_entity = matches!(ctx.view_name.as_str(), "PackageDetail" | "ModuleInspect")
        && ctx.selected_entity.is_some();

    if has_entity {
        let name = ctx.selected_entity.as_ref().unwrap();
        suggestions.push(format!("Why is {} fragile?", name));
        suggestions.push(format!("What breaks if {} changes?", name));
        suggestions.push(format!("How to reduce {}'s coupling?", name));
    } else {
        suggestions.push("Which cluster has the highest coupling?".to_string());
        suggestions.push("What causes the most cycle debt?".to_string());
    }

    // Tab-specific
    match ctx.insight_tab.as_str() {
        "Hotspots" => {
            suggestions.push("What do the top 3 hotspots have in common?".to_string());
            if let Some(ref name) = ctx.top_brittle {
                suggestions.push(format!("How to stabilize {}?", name));
            }
        }
        "Blast" => {
            suggestions.push("Which keystones need the most test coverage?".to_string());
            suggestions.push("What are the critical dependency chains?".to_string());
        }
        _ => {}
    }

    // Conditional suggestions based on health
    if ctx.cycle_debt > 30.0 {
        suggestions.push("How can I break the circular dependencies?".to_string());
    }
    if ctx.health_percent < 50 {
        suggestions.push("What are the top 3 things to improve health?".to_string());
    }
    if ctx.trend_declining {
        suggestions.push("What caused the recent health decline?".to_string());
    }

    suggestions.truncate(5);
    suggestions
}

// ─── Module name highlighting ───────────────────────────────────────────────

/// Style used for highlighted module names in AI responses.
const MODULE_HIGHLIGHT: Color = Color::Rgb(180, 160, 255);

/// Check if a character is a word boundary (not alphanumeric, underscore, dot,
/// or path separator) suitable for module-name matching.
fn is_module_boundary(c: char) -> bool {
    !c.is_alphanumeric() && c != '_' && c != '.' && c != '/' && c != '\\'
}

/// Apply module highlighting to all spans in a Line, splitting text spans that
/// contain module names into highlighted and non-highlighted segments.
/// Returns owned `Span<'static>` because we produce new string slices.
fn highlight_line_modules(spans: Vec<Span<'_>>, known_modules: &[String]) -> Vec<Span<'static>> {
    if known_modules.is_empty() {
        return spans
            .into_iter()
            .map(|s| Span::styled(s.content.into_owned(), s.style))
            .collect();
    }
    let mut result = Vec::with_capacity(spans.len());
    for span in spans {
        let text = span.content.into_owned();
        let highlighted = highlight_modules_owned(&text, span.style, known_modules);
        result.extend(highlighted);
    }
    result
}

/// Like `highlight_modules` but returns owned spans (avoids lifetime issues
/// when the source text is an owned String from tui-markdown).
fn highlight_modules_owned(
    text: &str,
    base_style: Style,
    known_modules: &[String],
) -> Vec<Span<'static>> {
    if known_modules.is_empty() || text.is_empty() {
        return vec![Span::styled(text.to_owned(), base_style)];
    }

    let highlight_style = Style::default()
        .fg(MODULE_HIGHLIGHT)
        .add_modifier(Modifier::BOLD);

    let mut result: Vec<Span<'static>> = Vec::new();
    let mut remaining = text;

    while !remaining.is_empty() {
        let mut best_match: Option<(usize, usize)> = None;

        for module in known_modules {
            if module.len() > remaining.len() {
                continue;
            }
            if let Some(pos) = remaining.find(module.as_str()) {
                let before_ok = pos == 0
                    || remaining
                        .as_bytes()
                        .get(pos - 1)
                        .is_none_or(|&b| is_module_boundary(b as char));
                let after_pos = pos + module.len();
                let after_ok = after_pos >= remaining.len()
                    || remaining
                        .as_bytes()
                        .get(after_pos)
                        .is_none_or(|&b| is_module_boundary(b as char));

                if before_ok && after_ok {
                    match best_match {
                        None => best_match = Some((pos, module.len())),
                        Some((bp, bl)) => {
                            if pos < bp || (pos == bp && module.len() > bl) {
                                best_match = Some((pos, module.len()));
                            }
                        }
                    }
                }
            }
        }

        match best_match {
            Some((pos, len)) => {
                if pos > 0 {
                    result.push(Span::styled(remaining[..pos].to_owned(), base_style));
                }
                result.push(Span::styled(
                    remaining[pos..pos + len].to_owned(),
                    highlight_style,
                ));
                remaining = &remaining[pos + len..];
            }
            None => {
                result.push(Span::styled(remaining.to_owned(), base_style));
                break;
            }
        }
    }

    result
}

// ─── Rendering ──────────────────────────────────────────────────────────────

/// Render the AI assistant inline panel.
pub fn render_ai_panel(frame: &mut Frame, area: Rect, state: &mut AiPanelState) {
    if area.height < 4 || area.width < 20 {
        return;
    }

    state.panel_area = area;

    let outer_block = Block::default()
        .title(Line::from(vec![Span::styled(
            " AI Assistant ",
            Style::default()
                .fg(ACCENT_LAVENDER)
                .add_modifier(Modifier::BOLD),
        )]))
        .title_alignment(ratatui::layout::Alignment::Left)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(BORDER_FOCUSED))
        .style(Style::default().bg(BG_BASE));

    let hint_spans = vec![
        Span::styled("Tab", Style::default().fg(FG_OVERLAY)),
        Span::styled(":Suggest ", Style::default().fg(FG_SUBTEXT)),
        Span::styled("/", Style::default().fg(FG_OVERLAY)),
        Span::styled(":Cmds ", Style::default().fg(FG_SUBTEXT)),
        Span::styled("Esc", Style::default().fg(FG_OVERLAY)),
        Span::styled(":Close ", Style::default().fg(FG_SUBTEXT)),
    ];
    let title_right = Line::from(hint_spans);

    let outer_block = outer_block
        .title_bottom(title_right)
        .title_alignment(ratatui::layout::Alignment::Right);
    frame.render_widget(outer_block, area);

    let inner = Rect {
        x: area.x + 1,
        y: area.y + 1,
        width: area.width.saturating_sub(2),
        height: area.height.saturating_sub(2),
    };

    if inner.height < 2 || inner.width < 10 {
        return;
    }

    // Split: input (1) + suggestions (1) + conversation (rest)
    let is_command_input = state.input.text.trim_start().starts_with('/');
    let effective_suggestions = if is_command_input {
        state.get_command_suggestions()
    } else {
        state.suggestions.clone()
    };
    let has_suggestions = !effective_suggestions.is_empty();
    let suggestion_height = if has_suggestions { 1 } else { 0 };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),                 // Input line
            Constraint::Length(suggestion_height), // Suggestion chips
            Constraint::Min(0),                    // Conversation history
        ])
        .split(inner);

    // ── Input line ──
    render_input_line(frame, chunks[0], state);

    // ── Suggestion chips (command suggestions when typing `/`) ──
    if has_suggestions {
        render_suggestion_chips(frame, chunks[1], state, &effective_suggestions);
    }

    // ── Conversation history ──
    render_conversation(frame, chunks[2], state);
}

fn render_input_line(frame: &mut Frame, area: Rect, state: &AiPanelState) {
    let prompt_style = Style::default()
        .fg(ACCENT_LAVENDER)
        .add_modifier(Modifier::BOLD);
    let text_style = Style::default().fg(FG_TEXT);

    let display_text = if state.is_streaming {
        format!("{} Thinking...", state.spinner_char())
    } else {
        state.input.text.clone()
    };

    let line = Line::from(vec![
        Span::styled("> ", prompt_style),
        Span::styled(&display_text, text_style),
    ]);

    frame.render_widget(Paragraph::new(line), area);

    // Render cursor
    if !state.is_streaming {
        let cursor_x = area.x + 2 + state.input.cursor as u16;
        if cursor_x < area.x + area.width {
            frame.set_cursor_position((cursor_x, area.y));
        }
    }
}

fn render_suggestion_chips(
    frame: &mut Frame,
    area: Rect,
    state: &AiPanelState,
    suggestions: &[String],
) {
    let mut spans = Vec::new();

    for (i, suggestion) in suggestions.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled(" ", Style::default()));
        }

        let is_selected = state.suggestion_index == Some(i);
        let truncated = if suggestion.chars().count() > 40 {
            let end = suggestion
                .char_indices()
                .nth(37)
                .map(|(i, _)| i)
                .unwrap_or(suggestion.len());
            format!("{}...", &suggestion[..end])
        } else {
            suggestion.clone()
        };

        if is_selected {
            spans.push(Span::styled(
                format!("[{}]", truncated),
                Style::default()
                    .fg(BG_BASE)
                    .bg(ACCENT_LAVENDER)
                    .add_modifier(Modifier::BOLD),
            ));
        } else {
            spans.push(Span::styled(
                format!("[{}]", truncated),
                Style::default().fg(FG_OVERLAY),
            ));
        }
    }

    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn render_conversation(frame: &mut Frame, area: Rect, state: &mut AiPanelState) {
    if area.height == 0 {
        return;
    }

    let mut lines: Vec<Line> = Vec::new();

    if state.conversation.is_empty() && !state.is_streaming {
        lines.push(Line::from(Span::styled(
            "Ask anything about the architecture. Examples:",
            Style::default().fg(FG_SUBTEXT),
        )));
        lines.push(Line::from(Span::styled(
            "  \"Which modules have the highest blast radius?\"",
            Style::default().fg(FG_OVERLAY),
        )));
        lines.push(Line::from(Span::styled(
            "  \"What causes the most cycle debt?\"",
            Style::default().fg(FG_OVERLAY),
        )));
        lines.push(Line::from(Span::styled(
            "  \"How has health changed over the last 10 commits?\"",
            Style::default().fg(FG_OVERLAY),
        )));
    } else {
        for (i, entry) in state.conversation.iter().enumerate() {
            if i > 0 {
                lines.push(Line::from(""));
            }

            // Context label (small, subtle)
            lines.push(Line::from(vec![Span::styled(
                format!("  {} ", entry.context_label.view),
                Style::default().fg(FG_OVERLAY),
            )]));

            // Question
            lines.push(Line::from(vec![
                Span::styled(
                    "Q: ",
                    Style::default()
                        .fg(ACCENT_LAVENDER)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(entry.question.clone(), Style::default().fg(FG_TEXT)),
            ]));

            // Answer
            if !entry.answer.is_empty() {
                let markdown_text = tui_markdown::from_str(&entry.answer);
                for (j, line) in markdown_text.lines.into_iter().enumerate() {
                    let prefix = if j == 0 { "A: " } else { "   " };
                    let mut spans = vec![Span::styled(
                        prefix,
                        Style::default()
                            .fg(COLOR_HEALTHY)
                            .add_modifier(Modifier::BOLD),
                    )];

                    // Convert tui-markdown spans to ratatui 0.29 Span type,
                    // then apply module-name highlighting.
                    let md_spans: Vec<Span> = line
                        .spans
                        .into_iter()
                        .map(|s| Span::styled(s.content, s.style))
                        .collect();
                    let highlighted = highlight_line_modules(md_spans, &state.known_modules);
                    spans.extend(highlighted);

                    lines.push(Line::from(spans));
                }

                // Streaming spinner at end of last line
                if !entry.is_complete {
                    lines.push(Line::from(Span::styled(
                        format!("   {} ", state.spinner_char()),
                        Style::default().fg(ACCENT_LAVENDER),
                    )));
                }
            } else if !entry.is_complete {
                lines.push(Line::from(Span::styled(
                    format!("A: {} Thinking...", state.spinner_char()),
                    Style::default().fg(FG_SUBTEXT),
                )));
            }
        }
    }

    // Reserve 1 column on the right for the scrollbar
    let content_area = Rect {
        x: area.x,
        y: area.y,
        width: area.width.saturating_sub(1),
        height: area.height,
    };
    let scrollbar_area = area; // Scrollbar renders on the right edge of the full area

    // Use ratatui's own line_count to get accurate wrapped line count that matches
    // the actual word-wrapping algorithm used by Paragraph. The previous manual
    // calculation using chars().count() + div_ceil did not account for word-boundary
    // wrapping, causing max_scroll to be underestimated and auto-scroll to fail
    // (answers appeared truncated, previous answers became visible on next query).
    let para = Paragraph::new(lines).wrap(Wrap { trim: false });
    let wrapped_total = para.line_count(content_area.width);

    state.total_lines = wrapped_total;

    // Clamp scroll using wrapped line count
    let visible = content_area.height as usize;
    let max_scroll = wrapped_total.saturating_sub(visible);
    state.scroll_offset = state.scroll_offset.min(max_scroll);

    // Re-enable auto_scroll when scrolled to bottom
    if state.scroll_offset >= max_scroll {
        state.auto_scroll = true;
    }

    let para = para.scroll((state.scroll_offset as u16, 0));

    frame.render_widget(para, content_area);

    // Render scrollbar only when content overflows
    if wrapped_total > visible {
        let mut scrollbar_state = ScrollbarState::new(max_scroll).position(state.scroll_offset);
        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .thumb_style(Style::default().fg(ACCENT_LAVENDER))
            .track_style(Style::default().fg(Color::Rgb(49, 50, 68)))
            .begin_symbol(None)
            .end_symbol(None);
        frame.render_stateful_widget(scrollbar, scrollbar_area, &mut scrollbar_state);
    }
}
