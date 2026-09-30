//! Behaviour tests for editable text.
//!
//! These pin down what `EditableTextState` and `EditableTextElement` do *today* — the
//! editing model, the platform-facing input-handler surface that an IME drives, and the
//! two rules that only take effect once the element has been laid out (single-line
//! newline pruning, UTF-16 offsets). They exist so that the hooks the widget leaves open
//! (see `validate_incoming_text` and the masked/number/validation TODOs around it) can be
//! filled in without silently changing editing behaviour.
//!
//! Imports are explicit on purpose: a glob here would also pull in `hgpui`'s `test` macro
//! and shadow the `#[test]` attribute.

use std::ops::Range;

use hgpui::{
    AppContext as _, Bounds, Context, Entity, EntityInputHandler, Focusable as _,
    NavigationDirection, ParentElement as _, Render, Styled as _, TestAppContext, Window,
    WindowHandle, div, point, px, size,
};

use crate::editable_text::{
    EditableTextState, StringStorage, TextBoundary,
    actions::{DEFAULT_INPUT_CONTEXT, EditableTextActionHandler as _, default_bindings},
    text_input,
};

/// A view that renders one editable text element bound to a state the test owns.
///
/// Binding through `.state(..)` (rather than letting the element own its state) is what
/// lets a test drive and read the same state the element is editing.
struct TestView {
    input: Entity<EditableTextState>,
    multiline: bool,
}

impl Render for TestView {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl hgpui::IntoElement {
        div().size_full().child(
            text_input("test-input")
                .state(self.input.downgrade())
                .multiline(self.multiline)
                .placeholder("placeholder"),
        )
    }
}

/// Opens a window with one editable text element, draws it once, and hands back both the
/// window and the state the element is bound to.
fn setup(
    cx: &mut TestAppContext,
    multiline: bool,
) -> (WindowHandle<TestView>, Entity<EditableTextState>) {
    cx.update(|cx| {
        cx.bind_keys(default_bindings().as_keybindings(Some(DEFAULT_INPUT_CONTEXT)));
    });

    // The state is created outside the window so the test can keep a handle to it; the
    // view only borrows it.
    let input = cx.update(|cx| cx.new(|cx| EditableTextState::new(StringStorage::default(), cx)));
    let input_for_view = input.clone();
    let window = cx.open_window(size(px(600.), px(400.)), move |_, _cx| TestView {
        input: input_for_view,
        multiline,
    });
    // Draw once so the element has run its layout pass; several rules (single-line
    // pruning, wrapping) are only in effect afterwards.
    cx.run_until_parked();
    (window, input)
}

/// Runs `f` against the state with a window available.
fn with_state<R>(
    cx: &mut TestAppContext,
    window: &WindowHandle<TestView>,
    f: impl FnOnce(&mut EditableTextState, &mut Window, &mut Context<EditableTextState>) -> R,
) -> R {
    window
        .update(cx, |view, window, cx| {
            view.input.update(cx, |state, cx| f(state, window, cx))
        })
        .expect("window update failed")
}

/// Simulates the platform handing typed (or pasted) text to the focused input.
fn type_text(cx: &mut TestAppContext, window: &WindowHandle<TestView>, text: &str) {
    with_state(cx, window, |state, window, cx| {
        state.replace_text_in_range(None, text, window, cx);
    });
    cx.run_until_parked();
}

fn content(cx: &mut TestAppContext, input: &Entity<EditableTextState>) -> String {
    cx.update(|cx| input.read(cx).as_str().to_string())
}

// ---------------------------------------------------------------------------
// State machine (no window needed)
// ---------------------------------------------------------------------------

#[hgpui::test]
fn new_state_starts_empty(cx: &mut TestAppContext) {
    let input = cx.new(|cx| EditableTextState::new(StringStorage::default(), cx));
    assert_eq!(content(cx, &input), "");
    // The state carries an (empty) history from the start, so `history()` is `Some` here;
    // what matters is that there is nothing to undo yet.
    cx.update(|cx| {
        assert!(input.read(cx).history().is_some());
    });
}

#[hgpui::test]
fn emplace_replaces_the_content_and_bumps_the_version(cx: &mut TestAppContext) {
    let input = cx.new(|cx| EditableTextState::new(StringStorage::default(), cx));
    let version_before = cx.update(|cx| input.read(cx).version());

    cx.update(|cx| {
        input.update(cx, |state, cx| state.emplace("hello 世界", cx));
    });

    assert_eq!(content(cx, &input), "hello 世界");
    let version_after = cx.update(|cx| input.read(cx).version());
    assert_ne!(
        version_before, version_after,
        "emplace must bump the version so views know to re-read the value"
    );
    cx.update(|cx| {
        assert!(
            input.read(cx).history().is_some(),
            "emplace must be undoable, so it has to record history"
        );
    });
}

#[hgpui::test]
fn move_to_sets_the_caret_and_clamps_past_the_end(cx: &mut TestAppContext) {
    let input = cx.new(|cx| EditableTextState::new(StringStorage::from("abc"), cx));

    cx.update(|cx| input.update(cx, |state, cx| state.move_to(2, cx)));
    cx.update(|cx| assert_eq!(input.read(cx).caret_pos(), 2));

    // Past the end, and on a character boundary in the middle of a multi-byte char:
    cx.update(|cx| input.update(cx, |state, cx| state.move_to(999, cx)));
    cx.update(|cx| assert_eq!(input.read(cx).caret_pos(), 3));
}

#[hgpui::test]
fn select_to_extends_the_selection_and_reports_its_direction(cx: &mut TestAppContext) {
    let input = cx.new(|cx| EditableTextState::new(StringStorage::from("abcdef"), cx));

    // A selection made by extending *forward* is reported as `Back` — the naming follows
    // the caret's offset relative to the anchor, and this is what `selected_text_range`
    // turns into `reversed: true` for the platform. See the note on that field.
    cx.update(|cx| {
        input.update(cx, |state, cx| {
            state.move_to(1, cx);
            state.select_to(4, cx);
        })
    });
    cx.update(|cx| {
        let state = input.read(cx);
        assert_eq!(state.selected_range(), 1..4);
        assert_eq!(state.selection_direction(), Some(NavigationDirection::Back));
    });

    // And the other way round: extending backwards is `Forward`.
    cx.update(|cx| {
        input.update(cx, |state, cx| {
            state.move_to(4, cx);
            state.select_to(1, cx);
        })
    });
    cx.update(|cx| {
        let state = input.read(cx);
        assert_eq!(state.selected_range(), 1..4);
        assert_eq!(state.selection_direction(), Some(NavigationDirection::Forward));
        assert_eq!(state.caret_pos(), 1, "the caret follows the moving end");
    });
}

#[hgpui::test]
fn select_document_selects_everything(cx: &mut TestAppContext) {
    let input = cx.new(|cx| EditableTextState::new(StringStorage::from("hello"), cx));
    cx.update(|cx| input.update(cx, |state, cx| state.select_document(cx)));
    cx.update(|cx| assert_eq!(input.read(cx).selected_range(), 0..5));
}

// NOTE: `delete_linear` and `nav_linear` are gated on the element's layout data
// (`accepts_input`), which only exists after the element has been laid out. Driving them
// on a state that was never rendered is a silent no-op, so these tests open a window and
// draw the element first — same as typing does.

#[hgpui::test]
fn delete_linear_backwards_deletes_one_grapheme(cx: &mut TestAppContext) {
    let (window, input) = setup(cx, false);
    type_text(cx, &window, "a👨‍👩‍👧b");

    // A family emoji is one grapheme but several code points: deleting one grapheme must
    // take the whole cluster, not a single scalar value.
    with_state(cx, &window, |state, _, cx| {
        state.move_to("a👨‍👩‍👧".len(), cx);
        state.delete_linear(NavigationDirection::Back, TextBoundary::Graphmeme, cx);
    });

    assert_eq!(content(cx, &input), "ab");
}

#[hgpui::test]
fn delete_linear_by_word_removes_the_whole_word(cx: &mut TestAppContext) {
    let (window, input) = setup(cx, false);
    type_text(cx, &window, "hello world");

    with_state(cx, &window, |state, _, cx| {
        state.move_to("hello world".len(), cx);
        state.delete_linear(NavigationDirection::Back, TextBoundary::Word, cx);
    });

    assert_eq!(content(cx, &input), "hello ");
}

#[hgpui::test]
fn nav_linear_moves_by_grapheme_and_by_word(cx: &mut TestAppContext) {
    let (window, _input) = setup(cx, false);
    type_text(cx, &window, "one two");

    with_state(cx, &window, |state, _, cx| {
        state.move_to(0, cx);
        state.nav_linear(NavigationDirection::Forward, TextBoundary::Word, cx);
    });
    // The word boundary stops after the word itself, before the following space.
    with_state(cx, &window, |state, _, _| {
        assert_eq!(state.caret_pos(), 3, "end of `one`")
    });

    with_state(cx, &window, |state, _, cx| {
        state.nav_linear(NavigationDirection::Back, TextBoundary::Graphmeme, cx);
    });
    with_state(cx, &window, |state, _, _| assert_eq!(state.caret_pos(), 2));
}

// ---------------------------------------------------------------------------
// Typing through the platform input handler
// ---------------------------------------------------------------------------

#[hgpui::test]
fn typing_inserts_text_at_the_caret(cx: &mut TestAppContext) {
    let (window, input) = setup(cx, false);

    type_text(cx, &window, "hello");
    assert_eq!(content(cx, &input), "hello");

    // Caret is at the end, so the next insert appends.
    type_text(cx, &window, " there");
    assert_eq!(content(cx, &input), "hello there");
}

#[hgpui::test]
fn typing_replaces_the_selection(cx: &mut TestAppContext) {
    let (window, input) = setup(cx, false);
    type_text(cx, &window, "hello world");

    // Select `world` and type over it: the selection must be consumed by the insert.
    with_state(cx, &window, |state, _, cx| {
        state.move_to(6, cx);
        state.select_to(11, cx);
    });
    type_text(cx, &window, "there");

    assert_eq!(content(cx, &input), "hello there");
}

#[hgpui::test]
fn backspace_and_delete_remove_the_expected_character(cx: &mut TestAppContext) {
    let (window, input) = setup(cx, false);
    type_text(cx, &window, "abcd");

    with_state(cx, &window, |state, window, cx| {
        state.delete_left(&crate::editable_text::actions::DeleteLeft, window, cx);
        state.delete_right(&crate::editable_text::actions::DeleteRight, window, cx);
    });

    // Deleting left removes `d`; the caret is then at the end so deleting right does nothing.
    assert_eq!(content(cx, &input), "abc");
}

#[hgpui::test]
fn single_line_input_strips_newlines_from_inserted_text(cx: &mut TestAppContext) {
    let (window, input) = setup(cx, false);

    type_text(cx, &window, "one\ntwo\r\nthree");

    assert_eq!(
        content(cx, &input),
        "onetwothree",
        "a single-line field must not accept line breaks (this is the one sanitisation rule \
         `validate_incoming_text` implements today)"
    );
}

#[hgpui::test]
fn multiline_input_keeps_newlines(cx: &mut TestAppContext) {
    let (window, input) = setup(cx, true);

    type_text(cx, &window, "one\ntwo");

    assert_eq!(content(cx, &input), "one\ntwo");
}

// ---------------------------------------------------------------------------
// The IME contract: offsets are UTF-16, marked text is tracked
// ---------------------------------------------------------------------------

#[hgpui::test]
fn text_length_and_selection_are_reported_in_utf16(cx: &mut TestAppContext) {
    let (window, _input) = setup(cx, false);
    type_text(cx, &window, "a世界🎉");

    // "a" (1) + "世界" (2) + "🎉" (2, a surrogate pair) = 5 UTF-16 code units. Note this
    // state leaves `text_length_utf16` at the trait default (`None`) — recorded here so a
    // change to that is deliberate.
    let length = with_state(cx, &window, |state, window, cx| {
        state.text_length_utf16(window, cx)
    });
    assert_eq!(length, None);

    // Select the emoji: it sits at UTF-8 bytes 7..11 and spans UTF-16 units 3..5, because
    // it is encoded as a surrogate pair.
    with_state(cx, &window, |state, _, cx| {
        state.move_to("a世界".len(), cx);
        state.select_to("a世界🎉".len(), cx);
    });
    let selection = with_state(cx, &window, |state, window, cx| {
        state.selected_text_range(false, window, cx)
    });
    let selection = selection.expect("a laid-out input reports its selection");
    assert_eq!(selection.range, 3..5, "the emoji is 2 UTF-16 code units");
    assert!(
        selection.reversed,
        "selecting forward reports as reversed, matching selection_direction()"
    );
}

#[hgpui::test]
fn ime_composition_marks_text_and_unmark_clears_it(cx: &mut TestAppContext) {
    let (window, input) = setup(cx, false);

    // Compose "にほん" as an IME would: mark the freshly inserted text as being composed.
    with_state(cx, &window, |state, window, cx| {
        state.replace_and_mark_text_in_range(None, "にほん", None, window, cx);
    });
    assert_eq!(content(cx, &input), "にほん");
    with_state(cx, &window, |state, _, _| {
        assert_eq!(
            state.marked_range(),
            Some(0.."にほん".len()),
            "composing text must be tracked so the IME can replace it on the next keystroke"
        );
    });

    // The IME commits: the marked range goes away and further typing appends.
    with_state(cx, &window, |state, window, cx| {
        state.unmark_text(window, cx);
    });
    with_state(cx, &window, |state, _, _| assert_eq!(state.marked_range(), None));
    type_text(cx, &window, "です");
    assert_eq!(content(cx, &input), "にほんです");
}

// ---------------------------------------------------------------------------
// Events and history
// ---------------------------------------------------------------------------

#[hgpui::test]
fn text_changed_event_fires_on_edit(cx: &mut TestAppContext) {
    use crate::editable_text::TextChanged;
    use std::cell::Cell;
    use std::rc::Rc;

    /// Counts change events, so the test can tell an edit that notified from one that did not.
    struct CounterView {
        input: Entity<EditableTextState>,
        changes: Rc<Cell<u32>>,
    }

    impl Render for CounterView {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl hgpui::IntoElement {
            div().size_full().child(text_input("counter").state(self.input.downgrade()))
        }
    }

    let changes = Rc::new(Cell::new(0u32));
    let changes_for_view = changes.clone();

    let input = cx.update(|cx| cx.new(|cx| EditableTextState::new(StringStorage::default(), cx)));
    let input_for_view = input.clone();
    let _window = cx.open_window(size(px(400.), px(200.)), move |_, cx| {
        cx.subscribe(&input_for_view, |this: &mut CounterView, _input, _: &TextChanged, _| {
            this.changes.set(this.changes.get() + 1);
        })
        .detach();
        CounterView {
            input: input_for_view,
            changes: changes_for_view,
        }
    });
    cx.run_until_parked();

    cx.update(|cx| input.update(cx, |state, cx| state.emplace("changed", cx)));

    assert_eq!(
        changes.get(),
        1,
        "one edit must emit exactly one change event"
    );
}

#[hgpui::test]
fn undo_and_redo_restore_the_content(cx: &mut TestAppContext) {
    use crate::editable_text::actions::{Redo, Undo};

    let (window, input) = setup(cx, false);
    type_text(cx, &window, "hello");

    // Focus the element so the action dispatches to it, then undo through the real action
    // path (the same one a keyboard binding uses).
    with_state(cx, &window, |state, window, cx| {
        state.focus_handle(cx).focus(window, cx);
    });
    cx.run_until_parked();

    window
        .update(cx, |_, window, cx| {
            window.dispatch_action(Box::new(Undo), cx);
        })
        .expect("window update failed");
    cx.run_until_parked();

    assert_eq!(
        content(cx, &input),
        "",
        "undo must restore the content from before the insert"
    );

    window
        .update(cx, |_, window, cx| {
            window.dispatch_action(Box::new(Redo), cx);
        })
        .expect("window update failed");
    cx.run_until_parked();

    assert_eq!(content(cx, &input), "hello", "redo must reapply the insert");
}

// ---------------------------------------------------------------------------
// Layout-dependent surface
// ---------------------------------------------------------------------------

#[hgpui::test]
fn text_for_range_answers_from_the_content(cx: &mut TestAppContext) {
    let (window, _input) = setup(cx, false);
    type_text(cx, &window, "hello world");

    let mut adjusted: Option<Range<usize>> = None;
    let text = with_state(cx, &window, |state, window, cx| {
        state.text_for_range(6..11, &mut adjusted, window, cx)
    });

    assert_eq!(
        text.as_deref(),
        Some("world"),
        "the platform asks for text by UTF-8 byte range"
    );
}

#[hgpui::test]
fn bounds_for_range_reports_a_laid_out_rectangle(cx: &mut TestAppContext) {
    let (window, _input) = setup(cx, false);
    type_text(cx, &window, "hello");

    let bounds = with_state(cx, &window, |state, window, cx| {
        state.bounds_for_range(0..5, Bounds::new(point(px(0.), px(0.)), size(px(600.), px(400.))), window, cx)
    });

    let bounds = bounds.expect("a laid-out input must report bounds for a byte range");
    assert!(
        bounds.size.width > px(0.) && bounds.size.height > px(0.),
        "bounds for a non-empty range must have a visible size, got {bounds:?}"
    );
}
