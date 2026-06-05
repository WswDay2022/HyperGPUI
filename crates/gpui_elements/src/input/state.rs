use super::actions::*;
use crate::input::{InputLayout, unicode::UnicodeString};
use gpui::{
    App, AppContext, ClipboardItem, Context, Entity, EntityId, EntityInputHandler, EventEmitter,
    FocusHandle, Focusable, Pixels, Point, SharedString, Subscription, TextRun, TextStyle, Window,
    WrappedLine, point, px,
};
use std::{
    ops::Range,
    sync::Arc,
    time::{Duration, Instant},
};
use unicode_segmentation::UnicodeSegmentation;

/// Events emitted by InputState when significant changes occur.
#[derive(Clone, Debug)]
pub enum InputStateEvent {
    /// Emitted when the input gains focus.
    Focus,
    /// Emitted when the input loses focus.
    Blur,
    /// Emitted when the text content changes.
    TextChanged,
    /// Emitted when an undo operation is performed.
    Undo,
    /// Emitted when a redo operation is performed.
    Redo,
}

impl EventEmitter<InputStateEvent> for InputState {}

/// `Input` is the state model for text input components. It handles:
/// - Text content storage and manipulation
/// - Selection and cursor management
/// - Keyboard navigation and editing actions
/// - IME (Input Method Editor) support via `EntityInputHandler`
pub struct InputState {
    entity_id: EntityId,
    focus_handle: FocusHandle,
    content: String,
    placeholder: SharedString,
    pub(super) selected_range: Range<usize>,
    pub(super) selection_reversed: bool,
    pub(super) marked_range: Option<Range<usize>>,
    pub(super) line_height: Pixels,
    pub(super) line_layouts: Vec<InputLineLayout>,
    pub(super) wrap_width: Option<Pixels>,
    pub(super) text_style: Option<TextStyle>,
    pub(super) needs_layout: bool,
    is_selecting: bool,
    last_click_position: Option<Point<Pixels>>,
    click_count: usize,
    /// Scroll offset - vertical for multiline, horizontal for single-line
    pub(super) scroll_offset: Pixels,
    pub(super) available_height: Pixels,
    pub(super) available_width: Pixels,
    pub(super) layout: InputLayout,
    /// Stack of previous states for undo.
    undo_stack: Vec<super::HistoryEntry>,
    /// Stack of undone states for redo.
    redo_stack: Vec<super::HistoryEntry>,
    /// Optional entity and subscription tracking the blinking of the text cursor.
    cursor_blink: Option<(Entity<super::CursorBlink>, Subscription)>,
    /// Tracks whether we were focused on the last update.
    was_focused: bool,
    /// Cached UTF-16 length of content for faster IME operations.
    /// Lazily computed when None.
    pub(super) cached_utf16_len: Option<usize>,
}

/// Layout information for a single logical line of text in an input.
///
/// A logical line corresponds to content between newlines in the input text.
/// When text wrapping is enabled, a logical line may span multiple visual lines.
#[derive(Clone, Debug)]
pub(super) struct InputLineLayout {
    /// The byte range in the content string that this line covers.
    pub text_range: Range<usize>,
    /// The shaped and wrapped text for this line, if available.
    pub wrapped_line: Option<Arc<WrappedLine>>,
    /// The vertical offset from the top of the text area in pixels.
    pub y_offset: Pixels,
    /// The number of visual lines this logical line spans (due to wrapping).
    pub visual_line_count: usize,
}

pub enum CursorBlinkType<'app> {
    Disabled,
    Enabled {
        app: &'app mut App,
        interval: Option<Duration>,
    },
}

impl Focusable for InputState {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl InputState {
    /// Creates a new `Input` with the specified multiline setting.
    /// Cursor blinking is enabled by default.
    pub fn new(cx: &mut Context<Self>) -> Self {
        let mut this = Self {
            entity_id: cx.entity_id(),
            focus_handle: cx.focus_handle(),
            content: String::new(),
            placeholder: SharedString::default(),
            selected_range: 0..0,
            selection_reversed: false,
            marked_range: None,
            line_height: px(0.),
            line_layouts: Vec::new(),
            wrap_width: None,
            text_style: None,
            needs_layout: true,
            is_selecting: false,
            last_click_position: None,
            click_count: 0,
            scroll_offset: px(0.),
            available_height: px(0.),
            available_width: px(0.),
            layout: InputLayout::SingleLine,
            undo_stack: Vec::new(),
            cached_utf16_len: None,
            redo_stack: Vec::new(),
            cursor_blink: None,
            was_focused: false,
        };
        this = this.cursor_blink(CursorBlinkType::Enabled {
            app: cx,
            interval: None,
        });
        this
    }

    pub fn cursor_blink<'app>(mut self, args: CursorBlinkType<'app>) -> Self {
        self.cursor_blink = match args {
            CursorBlinkType::Disabled => None,
            CursorBlinkType::Enabled { app: cx, interval } => {
                let interval = interval.unwrap_or(super::DEFAULT_BLINK_INTERVAL);
                let cursor_blink = cx.new(|cx| super::CursorBlink::new(interval, cx));
                let entity_id = self.entity_id;
                let subscription = cx.observe(&cursor_blink, move |_, cx| cx.notify(entity_id));
                Some((cursor_blink, subscription))
            }
        };
        self
    }

    /// Returns whether the cursor should be visible (for blinking).
    ///
    /// If blinking is not enabled, always returns `true`.
    /// This method also updates the blink manager's enabled state based on focus.
    pub fn cursor_visible(&mut self, is_focused: bool, cx: &mut Context<Self>) -> bool {
        // Update cursor blink based on focus changes
        if let Some((cursor_blink, _)) = &self.cursor_blink {
            if is_focused && !self.was_focused {
                cursor_blink.update(cx, |cb, cx| cb.enable(cx));
                cx.emit(InputStateEvent::Focus);
            } else if !is_focused && self.was_focused {
                cursor_blink.update(cx, |cb, cx| cb.disable(cx));
                cx.emit(InputStateEvent::Blur);
            }
        }
        self.was_focused = is_focused;

        self.cursor_blink
            .as_ref()
            .map(|(cb, _)| cb.read(cx).visible())
            .unwrap_or(true)
    }

    /// Pauses cursor blinking temporarily (e.g., during typing).
    pub(super) fn pause_cursor_blink(&self, cx: &mut Context<Self>) {
        if let Some((cursor_blink, _)) = &self.cursor_blink {
            cursor_blink.update(cx, |cb, cx| cb.pause_blinking(cx));
        }
    }

    /// Sets the text style used for layout. Marks layout as dirty if the style changed.
    pub(crate) fn set_text_style(&mut self, style: &TextStyle) {
        let changed = self
            .text_style
            .as_ref()
            .map_or(true, |current| current != style);

        if changed {
            self.text_style = Some(style.clone());
            self.needs_layout = true;
        }
    }

    /// Returns the current text content.
    pub fn content(&self) -> &str {
        &self.content
    }

    pub(super) fn content_mut(&mut self) -> &mut String {
        &mut self.content
    }

    pub fn get_layout(&self) -> InputLayout {
        self.layout
    }

    /// Sets the text content, resetting selection to the beginning.
    /// This clears the undo/redo history.
    pub fn set_content(&mut self, content: impl AsRef<str>, cx: &mut Context<Self>) {
        self.content = self.layout.sanitize_content(content.as_ref()).to_string();
        self.selected_range = 0..0;
        self.selection_reversed = false;
        self.marked_range = None;
        self.needs_layout = true;
        self.undo_stack.clear();
        self.redo_stack.clear();
        self.cached_utf16_len = None;
        self.pause_cursor_blink(cx);
        cx.emit(InputStateEvent::TextChanged);
        cx.notify();
    }

    /// Returns whether undo is available.
    pub fn can_undo(&self) -> bool {
        !self.undo_stack.is_empty()
    }

    /// Returns whether redo is available.
    pub fn can_redo(&self) -> bool {
        !self.redo_stack.is_empty()
    }

    /// Records a patch for undo. Called before making changes to content.
    /// Returns true if a new entry was created, false if grouped with previous.
    pub(super) fn push_undo_patch(&mut self, range: Range<usize>, new_text_len: usize) {
        // Don't record during IME composition
        if self.marked_range.is_some() {
            return;
        }

        let now = Instant::now();

        // Check if we should group with the last entry
        if let Some(last) = self.undo_stack.last() {
            if now.duration_since(last.timestamp) < super::DEFAULT_GROUP_INTERVAL {
                // Within group interval - extend the existing patch
                // We need to merge this edit with the previous one
                return;
            }
        }

        // Capture the text that will be replaced
        let old_text = self.content[range.clone()].to_string();

        self.undo_stack.push(super::HistoryEntry {
            range: range.start..range.start + new_text_len,
            old_text,
            new_text_len,
            selected_range: self.selected_range.clone(),
            selection_reversed: self.selection_reversed,
            timestamp: now,
        });

        // Limit history size
        if self.undo_stack.len() > super::MAX_HISTORY_LEN {
            self.undo_stack.remove(0);
        }

        // New edit invalidates redo stack
        self.redo_stack.clear();
    }

    /// Undoes the last edit by applying the reverse patch.
    pub(crate) fn undo(&mut self, _: &Undo, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(entry) = self.undo_stack.pop() {
            // Remember selection to restore
            let selected_range = entry.selected_range.clone();
            let selection_reversed = entry.selection_reversed;

            // Apply the undo patch and get the redo patch
            let redo_entry = entry.apply_undo(&mut self.content);
            self.redo_stack.push(redo_entry);

            // Restore selection state
            self.selected_range = selected_range;
            self.selection_reversed = selection_reversed;
            self.needs_layout = true;
            self.cached_utf16_len = None;
            self.scroll_to_cursor();
            cx.emit(InputStateEvent::Undo);
            cx.notify();
        }
    }

    /// Redoes the last undone edit by applying the forward patch.
    pub(crate) fn redo(&mut self, _: &Redo, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(entry) = self.redo_stack.pop() {
            // Apply the redo patch and get the undo patch
            let undo_entry = entry.apply_redo(&mut self.content);

            // The undo entry contains the selection state after the original edit
            // We need to restore cursor to end of inserted text
            let cursor_pos = undo_entry.range.start;
            self.selected_range = cursor_pos..cursor_pos;
            self.selection_reversed = false;

            self.undo_stack.push(undo_entry);
            self.needs_layout = true;
            self.cached_utf16_len = None;
            self.scroll_to_cursor();
            cx.emit(InputStateEvent::Redo);
            cx.notify();
        }
    }

    /// Returns the placeholder text shown when content is empty.
    pub fn placeholder(&self) -> &SharedString {
        &self.placeholder
    }

    /// Sets the placeholder text.
    pub fn set_placeholder(
        &mut self,
        placeholder: impl Into<SharedString>,
        cx: &mut Context<Self>,
    ) {
        self.placeholder = placeholder.into();
        cx.notify();
    }

    /// Returns the current selection range.
    pub fn selected_range(&self) -> &Range<usize> {
        &self.selected_range
    }

    /// Returns true if the selection is reversed (cursor at start).
    pub fn selection_reversed(&self) -> bool {
        self.selection_reversed
    }

    /// Returns the current cursor offset.
    pub fn cursor_offset(&self) -> usize {
        if self.selection_reversed {
            self.selected_range.start
        } else {
            self.selected_range.end
        }
    }

    /// Returns the marked text range (for IME composition).
    pub fn marked_range(&self) -> Option<&Range<usize>> {
        self.marked_range.as_ref()
    }

    /// Sets the selection range directly.
    pub fn set_selected_range(&mut self, range: Range<usize>) {
        let range = range.start.min(self.content.len())..range.end.min(self.content.len());
        self.selected_range = range;
        self.selection_reversed = false;
    }

    /// Returns the selected text range in UTF-16 offsets (for IME).
    pub fn selected_text_range_utf16(&self) -> Range<usize> {
        self.utf_range_8to16(&self.selected_range)
    }

    /// Inserts text at the current cursor position, replacing any selection.
    pub fn insert_text(&mut self, text: &str, cx: &mut Context<Self>) {
        let range = self
            .marked_range
            .clone()
            .unwrap_or(self.selected_range.clone());
        let range = range.start.min(self.content.len())..range.end.min(self.content.len());

        let text_to_insert = self.layout.sanitize_content(text);

        // Record patch for undo before modifying content
        self.push_undo_patch(range.clone(), text_to_insert.len());

        // Update cached UTF-16 length incrementally if available
        if let Some(cached_len) = self.cached_utf16_len {
            let removed_utf16_len: usize = self.content[range.clone()]
                .chars()
                .map(|c| c.len_utf16())
                .sum();
            let added_utf16_len: usize = text_to_insert.chars().map(|c| c.len_utf16()).sum();
            self.cached_utf16_len = Some(cached_len - removed_utf16_len + added_utf16_len);
        }

        self.content.replace_range(range.clone(), &text_to_insert);
        self.selected_range =
            range.start + text_to_insert.len()..range.start + text_to_insert.len();
        self.marked_range.take();
        self.needs_layout = true;
        self.pause_cursor_blink(cx);
        cx.emit(InputStateEvent::TextChanged);
        cx.notify();
    }

    /// Deletes the character before the cursor (convenience method for benchmarks).
    pub fn delete_backward(&mut self, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.select_to(self.previous_boundary(self.cursor_offset()), cx);
        }
        self.insert_text("", cx);
    }

    /// Undoes the last edit (convenience method without Window).
    pub fn undo_action(&mut self, cx: &mut Context<Self>) {
        if let Some(entry) = self.undo_stack.pop() {
            let selected_range = entry.selected_range.clone();
            let selection_reversed = entry.selection_reversed;

            let redo_entry = entry.apply_undo(&mut self.content);
            self.redo_stack.push(redo_entry);

            self.selected_range = selected_range;
            self.selection_reversed = selection_reversed;
            self.needs_layout = true;
            self.cached_utf16_len = None;
            self.scroll_to_cursor();
            cx.emit(InputStateEvent::Undo);
            cx.notify();
        }
    }

    /// Redoes the last undone edit (convenience method without Window).
    pub fn redo_action(&mut self, cx: &mut Context<Self>) {
        if let Some(entry) = self.redo_stack.pop() {
            let undo_entry = entry.apply_redo(&mut self.content);

            let cursor_pos = undo_entry.range.start;
            self.selected_range = cursor_pos..cursor_pos;
            self.selection_reversed = false;

            self.undo_stack.push(undo_entry);
            self.needs_layout = true;
            self.cached_utf16_len = None;
            self.scroll_to_cursor();
            cx.emit(InputStateEvent::Redo);
            cx.notify();
        }
    }

    /// Selects all text.
    pub fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.selected_range = 0..self.content.len();
        self.selection_reversed = false;
        cx.notify();
    }

    pub(crate) fn left(&mut self, _: &Left, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            let new_pos = self.previous_boundary(self.cursor_offset());
            self.move_to(new_pos, cx);
        } else {
            self.move_to(self.selected_range.start, cx);
        }
    }

    pub(crate) fn right(&mut self, _: &Right, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            let new_pos = self.next_boundary(self.cursor_offset());
            self.move_to(new_pos, cx);
        } else {
            self.move_to(self.selected_range.end, cx);
        }
    }

    pub(crate) fn up(&mut self, _: &Up, _window: &mut Window, cx: &mut Context<Self>) {
        self.pause_cursor_blink(cx);
        match self.layout {
            InputLayout::SingleLine => {
                // In single-line mode, up moves to start
                self.selected_range = 0..0;
                self.selection_reversed = false;
                self.scroll_to_cursor();
                cx.notify();
            }
            InputLayout::MultiLine => {
                if let Some(new_offset) = self.move_vertically(self.cursor_offset(), -1) {
                    self.selected_range = new_offset..new_offset;
                    self.selection_reversed = false;
                    self.scroll_to_cursor();
                    cx.notify();
                }
            }
        }
    }

    pub(crate) fn down(&mut self, _: &Down, _window: &mut Window, cx: &mut Context<Self>) {
        self.pause_cursor_blink(cx);
        match self.layout {
            InputLayout::SingleLine => {
                // In single-line mode, down moves to end
                let end = self.content.len();
                self.selected_range = end..end;
                self.selection_reversed = false;
                self.scroll_to_cursor();
                cx.notify();
            }
            InputLayout::MultiLine => {
                if let Some(new_offset) = self.move_vertically(self.cursor_offset(), 1) {
                    self.selected_range = new_offset..new_offset;
                    self.selection_reversed = false;
                    self.scroll_to_cursor();
                    cx.notify();
                }
            }
        }
    }

    pub(crate) fn select_left(&mut self, _: &SelectLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.previous_boundary(self.cursor_offset()), cx);
    }

    pub(crate) fn select_right(&mut self, _: &SelectRight, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.next_boundary(self.cursor_offset()), cx);
    }

    pub(crate) fn select_up(&mut self, _: &SelectUp, _window: &mut Window, cx: &mut Context<Self>) {
        self.pause_cursor_blink(cx);
        match self.layout {
            InputLayout::SingleLine => {
                // In single-line mode, select_up selects to start
                self.select_to(0, cx);
            }
            InputLayout::MultiLine => {
                if let Some(new_offset) = self.move_vertically(self.cursor_offset(), -1) {
                    if self.selection_reversed {
                        self.selected_range.start = new_offset;
                    } else {
                        self.selected_range.end = new_offset;
                    }
                    if self.selected_range.end < self.selected_range.start {
                        self.selection_reversed = !self.selection_reversed;
                        self.selected_range = self.selected_range.end..self.selected_range.start;
                    }
                    self.scroll_to_cursor();
                    cx.notify();
                }
            }
        }
    }

    pub(crate) fn select_down(
        &mut self,
        _: &SelectDown,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.pause_cursor_blink(cx);
        match self.layout {
            InputLayout::SingleLine => {
                // In single-line mode, select_down selects to end
                self.select_to(self.content.len(), cx);
            }
            InputLayout::MultiLine => {
                if let Some(new_offset) = self.move_vertically(self.cursor_offset(), 1) {
                    if self.selection_reversed {
                        self.selected_range.start = new_offset;
                    } else {
                        self.selected_range.end = new_offset;
                    }
                    if self.selected_range.end < self.selected_range.start {
                        self.selection_reversed = !self.selection_reversed;
                        self.selected_range = self.selected_range.end..self.selected_range.start;
                    }
                    self.scroll_to_cursor();
                    cx.notify();
                }
            }
        }
    }

    pub(crate) fn home(&mut self, _: &Home, _: &mut Window, cx: &mut Context<Self>) {
        let line_start = self.find_line_start(self.cursor_offset());
        self.move_to(line_start, cx);
    }

    pub(crate) fn end(&mut self, _: &End, _: &mut Window, cx: &mut Context<Self>) {
        let line_end = self.find_line_end(self.cursor_offset());
        self.move_to(line_end, cx);
    }

    pub(crate) fn move_to_beginning(
        &mut self,
        _: &MoveToBeginning,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_to(0, cx);
    }

    pub(crate) fn move_to_end(&mut self, _: &MoveToEnd, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.content.len(), cx);
    }

    pub(crate) fn select_to_beginning(
        &mut self,
        _: &SelectToBeginning,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.select_to(0, cx);
    }

    pub(crate) fn select_to_end(
        &mut self,
        _: &SelectToEnd,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.select_to(self.content.len(), cx);
    }

    pub(crate) fn word_left(&mut self, _: &WordLeft, _: &mut Window, cx: &mut Context<Self>) {
        let new_pos = self.previous_word_boundary(self.cursor_offset());
        self.move_to(new_pos, cx);
    }

    pub(crate) fn word_right(&mut self, _: &WordRight, _: &mut Window, cx: &mut Context<Self>) {
        let new_pos = self.next_word_boundary(self.cursor_offset());
        self.move_to(new_pos, cx);
    }

    pub(crate) fn select_word_left(
        &mut self,
        _: &SelectWordLeft,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let new_pos = self.previous_word_boundary(self.cursor_offset());
        self.select_to(new_pos, cx);
    }

    pub(crate) fn select_word_right(
        &mut self,
        _: &SelectWordRight,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let new_pos = self.next_word_boundary(self.cursor_offset());
        self.select_to(new_pos, cx);
    }

    pub(crate) fn enter(&mut self, _: &Enter, window: &mut Window, cx: &mut Context<Self>) {
        if matches!(&self.layout, InputLayout::MultiLine) {
            self.replace_text_in_range(None, "\n", window, cx);
        }
    }

    pub(crate) fn tab(&mut self, _: &Tab, window: &mut Window, cx: &mut Context<Self>) {
        self.replace_text_in_range(None, "\t", window, cx);
    }

    pub(crate) fn backspace(&mut self, _: &Backspace, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.select_to(self.previous_boundary(self.cursor_offset()), cx);
        }
        self.replace_text_in_range(None, "", window, cx);
    }

    pub(crate) fn delete(&mut self, _: &Delete, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.select_to(self.next_boundary(self.cursor_offset()), cx);
        }
        self.replace_text_in_range(None, "", window, cx);
    }

    pub(crate) fn delete_word_left(
        &mut self,
        _: &DeleteWordLeft,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.selected_range.is_empty() {
            self.select_to(self.previous_word_boundary(self.cursor_offset()), cx);
        }
        self.replace_text_in_range(None, "", window, cx);
    }

    pub(crate) fn delete_word_right(
        &mut self,
        _: &DeleteWordRight,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.selected_range.is_empty() {
            self.select_to(self.next_word_boundary(self.cursor_offset()), cx);
        }
        self.replace_text_in_range(None, "", window, cx);
    }

    pub(crate) fn delete_to_beginning_of_line(
        &mut self,
        _: &DeleteToBeginningOfLine,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.selected_range.is_empty() {
            self.select_to(self.find_line_start(self.cursor_offset()), cx);
        }
        self.replace_text_in_range(None, "", window, cx);
    }

    pub(crate) fn delete_to_end_of_line(
        &mut self,
        _: &DeleteToEndOfLine,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.selected_range.is_empty() {
            self.select_to(self.find_line_end(self.cursor_offset()), cx);
        }
        self.replace_text_in_range(None, "", window, cx);
    }

    pub(crate) fn paste(&mut self, _: &Paste, window: &mut Window, cx: &mut Context<Self>) {
        let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) else {
            return;
        };
        let text = self.layout.sanitize_content(&text);
        self.replace_text_in_range(None, &text, window, cx);
    }

    pub(crate) fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        if !self.selected_range.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(
                self.content[self.selected_range.clone()].to_string(),
            ));
        }
    }

    pub(crate) fn cut(&mut self, _: &Cut, window: &mut Window, cx: &mut Context<Self>) {
        if !self.selected_range.is_empty() {
            // Cut selected text
            cx.write_to_clipboard(ClipboardItem::new_string(
                self.content[self.selected_range.clone()].to_string(),
            ));
            self.replace_text_in_range(None, "", window, cx);
        } else {
            // No selection: cut the entire current line (including newline)
            let cursor = self.cursor_offset();
            let line_start = self.find_line_start(cursor);
            let line_end = self.find_line_end(cursor);

            // Include the newline character if there is one after the line
            let cut_end = if line_end < self.content.len() {
                line_end + 1 // Include the newline
            } else if line_start > 0 {
                // Last line with no trailing newline - include preceding newline instead
                line_end
            } else {
                line_end
            };

            // For last line, also remove the preceding newline if it exists
            let cut_start = if line_end >= self.content.len() && line_start > 0 {
                line_start - 1 // Include preceding newline for last line
            } else {
                line_start
            };

            let line_text = self.content[cut_start..cut_end].to_string();
            cx.write_to_clipboard(ClipboardItem::new_string(line_text));

            self.selected_range = cut_start..cut_end;
            self.replace_text_in_range(None, "", window, cx);
        }
    }

    pub(crate) fn on_mouse_down(
        &mut self,
        position: Point<Pixels>,
        click_count: usize,
        shift: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.focus_handle, cx);
        self.is_selecting = true;

        let is_same_position = self
            .last_click_position
            .map(|last| {
                let threshold = px(4.);
                (position.x - last.x).abs() < threshold && (position.y - last.y).abs() < threshold
            })
            .unwrap_or(false);

        if is_same_position && click_count > 1 {
            self.click_count = click_count;
        } else {
            self.click_count = 1;
        }
        self.last_click_position = Some(position);

        let clicked_offset = self.index_for_position(position);

        match self.click_count {
            2 => {
                let (word_start, word_end) = self.word_range_at(clicked_offset);
                self.selected_range = word_start..word_end;
                self.selection_reversed = false;
                cx.notify();
            }
            3 => {
                let line_start = self.find_line_start(clicked_offset);
                let line_end = self.find_line_end(clicked_offset);
                let line_end_with_newline = if line_end < self.content.len() {
                    line_end + 1
                } else {
                    line_end
                };
                self.selected_range = line_start..line_end_with_newline;
                self.selection_reversed = false;
                cx.notify();
            }
            _ => {
                if shift {
                    self.select_to(clicked_offset, cx);
                } else {
                    self.move_to(clicked_offset, cx);
                }
            }
        }
    }

    pub(crate) fn on_mouse_up(&mut self, _cx: &mut Context<Self>) {
        self.is_selecting = false;
    }

    pub(crate) fn on_mouse_move(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        if self.is_selecting && self.click_count == 1 {
            self.select_to(self.index_for_position(position), cx);
        }
    }

    fn move_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        self.pause_cursor_blink(cx);
        let offset = offset.min(self.content.len());
        self.selected_range = offset..offset;
        self.selection_reversed = false;
        self.scroll_to_cursor();
        cx.notify();
    }

    fn select_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        self.pause_cursor_blink(cx);
        let offset = offset.min(self.content.len());
        if self.selection_reversed {
            self.selected_range.start = offset;
        } else {
            self.selected_range.end = offset;
        }
        if self.selected_range.end < self.selected_range.start {
            self.selection_reversed = !self.selection_reversed;
            self.selected_range = self.selected_range.end..self.selected_range.start;
        }
        self.scroll_to_cursor();
        cx.notify();
    }

    pub(crate) fn find_line_start(&self, offset: usize) -> usize {
        self.content[..offset.min(self.content.len())]
            .rfind('\n')
            .map(|pos| pos + 1)
            .unwrap_or(0)
    }

    pub(crate) fn find_line_end(&self, offset: usize) -> usize {
        self.content[offset.min(self.content.len())..]
            .find('\n')
            .map(|pos| offset + pos)
            .unwrap_or(self.content.len())
    }

    fn move_vertically(&self, offset: usize, direction: i32) -> Option<usize> {
        let (visual_line_idx, x_pixels) = self.find_visual_line_and_x_offset(offset);
        let target_visual_line_idx = (visual_line_idx as i32 + direction).max(0) as usize;

        let mut current_visual_line = 0;
        for layout in self.line_layouts.iter() {
            let visual_lines_in_layout = layout.visual_line_count;

            if target_visual_line_idx < current_visual_line + visual_lines_in_layout {
                let visual_line_within_layout = target_visual_line_idx - current_visual_line;

                if layout.text_range.is_empty() {
                    return Some(layout.text_range.start);
                }

                if let Some(wrapped) = &layout.wrapped_line {
                    let y_within_wrapped = self.line_height * visual_line_within_layout as f32;
                    let target_point = point(px(x_pixels), y_within_wrapped);

                    let closest_result =
                        wrapped.closest_index_for_position(target_point, self.line_height);

                    let closest_idx = closest_result.unwrap_or_else(|closest| closest);
                    let clamped = closest_idx.min(wrapped.text.len());
                    let result = layout.text_range.start + clamped;

                    return Some(result);
                }

                return Some(layout.text_range.start);
            }

            current_visual_line += visual_lines_in_layout;
        }

        if direction > 0 {
            Some(self.content.len())
        } else {
            None
        }
    }

    fn find_visual_line_and_x_offset(&self, offset: usize) -> (usize, f32) {
        if self.line_layouts.is_empty() {
            return (0, 0.0);
        }

        let mut visual_line_idx = 0;

        for line in &self.line_layouts {
            if line.text_range.is_empty() {
                if offset == line.text_range.start {
                    return (visual_line_idx, 0.0);
                }
            } else if offset >= line.text_range.start && offset <= line.text_range.end {
                if let Some(wrapped) = &line.wrapped_line {
                    let local_offset = (offset - line.text_range.start).min(wrapped.text.len());
                    if let Some(position) =
                        wrapped.position_for_index(local_offset, self.line_height)
                    {
                        let visual_line_within = (position.y / self.line_height).floor() as usize;
                        return (visual_line_idx + visual_line_within, position.x.into());
                    }
                }
                return (visual_line_idx, 0.0);
            }
            visual_line_idx += line.visual_line_count;
        }

        (visual_line_idx.saturating_sub(1), 0.0)
    }

    pub(crate) fn index_for_position(&self, position: Point<Pixels>) -> usize {
        if self.content.is_empty() {
            return 0;
        }

        for line in self.line_layouts.iter() {
            let line_height_total = self.line_height * line.visual_line_count as f32;

            if position.y >= line.y_offset && position.y < line.y_offset + line_height_total {
                if line.text_range.is_empty() {
                    return line.text_range.start;
                }

                if let Some(wrapped) = &line.wrapped_line {
                    let relative_y = position.y - line.y_offset;
                    let relative_point = point(position.x, relative_y);

                    let closest_result =
                        wrapped.closest_index_for_position(relative_point, self.line_height);

                    let local_idx = closest_result.unwrap_or_else(|closest| closest);
                    let clamped = local_idx.min(wrapped.text.len());
                    return line.text_range.start + clamped;
                }
                return line.text_range.start;
            }
        }

        self.content.len()
    }

    pub(crate) fn scroll_to_cursor(&mut self) {
        if self.line_layouts.is_empty() {
            return;
        }

        let cursor_offset = self.cursor_offset();
        match self.layout {
            InputLayout::SingleLine => self.scroll_to_cursor_horizontal(cursor_offset),
            InputLayout::MultiLine => self.scroll_to_cursor_vertical(cursor_offset),
        }
    }

    fn scroll_to_cursor_vertical(&mut self, cursor_offset: usize) {
        if self.available_height <= px(0.) {
            return;
        }

        let line_height = self.line_height;

        for line in &self.line_layouts {
            let is_cursor_in_line = if line.text_range.is_empty() {
                cursor_offset == line.text_range.start
            } else {
                line.text_range.contains(&cursor_offset)
                    || (cursor_offset == line.text_range.end && cursor_offset == self.content.len())
            };

            if is_cursor_in_line {
                let cursor_visual_y = if let Some(wrapped) = &line.wrapped_line {
                    let local_offset = cursor_offset.saturating_sub(line.text_range.start);
                    if let Some(position) =
                        wrapped.position_for_index(local_offset, self.line_height)
                    {
                        line.y_offset + position.y
                    } else {
                        line.y_offset
                    }
                } else {
                    line.y_offset
                };

                let visible_top = self.scroll_offset;
                let visible_bottom = self.scroll_offset + self.available_height;

                if cursor_visual_y < visible_top {
                    self.scroll_offset = cursor_visual_y;
                } else if cursor_visual_y + line_height > visible_bottom {
                    self.scroll_offset = (cursor_visual_y + line_height) - self.available_height;
                }

                self.scroll_offset = self.scroll_offset.max(px(0.));
                break;
            }
        }
    }

    fn scroll_to_cursor_horizontal(&mut self, cursor_offset: usize) {
        if self.available_width <= px(0.) {
            return;
        }

        // For single-line input, get cursor x position from the first (only) line
        let Some(line) = self.line_layouts.first() else {
            return;
        };

        let cursor_x = if let Some(wrapped) = &line.wrapped_line {
            let local_offset = cursor_offset.saturating_sub(line.text_range.start);
            wrapped
                .position_for_index(local_offset, self.line_height)
                .map(|p| p.x)
                .unwrap_or(px(0.))
        } else {
            px(0.)
        };

        let visible_left = self.scroll_offset;
        let visible_right = self.scroll_offset + self.available_width;

        // Add some padding so cursor isn't right at the edge
        let padding = px(2.0);

        if cursor_x < visible_left + padding {
            self.scroll_offset = (cursor_x - padding).max(px(0.));
        } else if cursor_x > visible_right - padding {
            self.scroll_offset = cursor_x - self.available_width + padding;
        }

        self.scroll_offset = self.scroll_offset.max(px(0.));
    }

    pub(crate) fn update_line_layouts(
        &mut self,
        width: Pixels,
        line_height: Pixels,
        text_style: &TextStyle,
        window: &mut Window,
    ) {
        self.line_height = line_height;
        self.set_text_style(text_style);

        if !self.needs_layout && self.wrap_width == Some(width) {
            return;
        }

        self.line_layouts.clear();
        self.wrap_width = Some(width);

        let text_color = text_style.color;
        let font_size = text_style.font_size.to_pixels(window.rem_size());

        if self.content.is_empty() {
            self.line_layouts.push(InputLineLayout {
                text_range: 0..0,
                wrapped_line: None,
                y_offset: px(0.),
                visual_line_count: 1,
            });
            self.needs_layout = false;
            return;
        }

        let mut y_offset = px(0.);
        let mut current_pos = 0;

        while current_pos < self.content.len() {
            let line_end = self.content[current_pos..]
                .find('\n')
                .map(|pos| current_pos + pos)
                .unwrap_or(self.content.len());

            let line_text = &self.content[current_pos..line_end];

            if line_text.is_empty() {
                self.line_layouts.push(InputLineLayout {
                    text_range: current_pos..current_pos,
                    wrapped_line: None,
                    y_offset,
                    visual_line_count: 1,
                });
                y_offset += line_height;
            } else {
                let run = TextRun {
                    len: line_text.len(),
                    font: text_style.font(),
                    color: text_color,
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                };

                let wrapped_lines = window
                    .text_system()
                    .shape_text(
                        SharedString::from(line_text.to_string()),
                        font_size,
                        &[run],
                        Some(width),
                        None,
                    )
                    .unwrap_or_default();

                for wrapped in wrapped_lines {
                    let visual_line_count = wrapped.wrap_boundaries().len() + 1;
                    let line_height_total = line_height * visual_line_count as f32;

                    self.line_layouts.push(InputLineLayout {
                        text_range: current_pos..line_end,
                        wrapped_line: Some(Arc::new(wrapped)),
                        y_offset,
                        visual_line_count,
                    });

                    y_offset += line_height_total;
                }
            }

            current_pos = if line_end < self.content.len() {
                line_end + 1
            } else {
                self.content.len()
            };
        }

        if self.content.ends_with('\n') {
            self.line_layouts.push(InputLineLayout {
                text_range: self.content.len()..self.content.len(),
                wrapped_line: None,
                y_offset,
                visual_line_count: 1,
            });
        }

        self.needs_layout = false;
        self.scroll_to_cursor();
    }

    pub(crate) fn total_content_height(&self) -> Pixels {
        self.line_layouts
            .last()
            .map(|last| last.y_offset + self.line_height * last.visual_line_count as f32)
            .unwrap_or(px(0.))
    }

    /// Returns true if the scroll position is at the top.
    pub fn at_top(&self) -> bool {
        self.scroll_offset <= px(0.)
    }

    /// Returns true if the scroll position is at the bottom.
    pub fn at_bottom(&self) -> bool {
        let content_height = self.total_content_height();
        let visible_height = self.available_height;

        if content_height <= visible_height {
            return true;
        }

        self.scroll_offset + visible_height >= content_height
    }

    /// Returns the scroll progress as a value from 0.0 (top) to 1.0 (bottom).
    pub fn scroll_progress(&self) -> f32 {
        let content_height = self.total_content_height();
        let visible_height = self.available_height;
        let max_scroll = content_height - visible_height;

        if max_scroll <= px(0.) {
            return 0.0;
        }

        (self.scroll_offset / max_scroll).clamp(0.0, 1.0)
    }

    /// Returns how far the content is scrolled from the top in pixels.
    pub fn distance_from_top(&self) -> Pixels {
        self.scroll_offset.max(px(0.))
    }

    /// Returns how far the content is from the bottom in pixels.
    pub fn distance_from_bottom(&self) -> Pixels {
        let content_height = self.total_content_height();
        let visible_height = self.available_height;
        let max_scroll = content_height - visible_height;

        if max_scroll <= px(0.) {
            return px(0.);
        }

        (max_scroll - self.scroll_offset).max(px(0.))
    }

    fn previous_boundary(&self, offset: usize) -> usize {
        if offset == 0 {
            return 0;
        }

        let text_before = &self.content[..offset.min(self.content.len())];
        text_before
            .grapheme_indices(true)
            .map(|(i, _)| i)
            .next_back()
            .unwrap_or(0)
    }

    fn next_boundary(&self, offset: usize) -> usize {
        if offset >= self.content.len() {
            return self.content.len();
        }

        let text_after = &self.content[offset..];
        text_after
            .grapheme_indices(true)
            .nth(1)
            .map(|(i, _)| offset + i)
            .unwrap_or(self.content.len())
    }

    fn previous_word_boundary(&self, offset: usize) -> usize {
        if offset == 0 {
            return 0;
        }

        let text_before = &self.content[..offset.min(self.content.len())];

        let mut last_word_start = 0;
        for (idx, _) in text_before.unicode_word_indices() {
            if idx < offset {
                last_word_start = idx;
            }
        }

        if last_word_start == 0 && offset > 0 {
            let trimmed = text_before.trim_end();
            if trimmed.is_empty() {
                return 0;
            }
            for (idx, _) in trimmed.unicode_word_indices() {
                last_word_start = idx;
            }
        }

        last_word_start
    }

    fn next_word_boundary(&self, offset: usize) -> usize {
        if offset >= self.content.len() {
            return self.content.len();
        }

        let text_after = &self.content[offset..];

        for (idx, word) in text_after.unicode_word_indices() {
            let word_end = offset + idx + word.len();
            if word_end > offset {
                return word_end;
            }
        }

        self.content.len()
    }

    fn word_range_at(&self, offset: usize) -> (usize, usize) {
        let offset = offset.min(self.content.len());

        for (idx, word) in self.content.unicode_word_indices() {
            let word_end = idx + word.len();
            if offset >= idx && offset <= word_end {
                return (idx, word_end);
            }
        }

        (offset, offset)
    }
}
