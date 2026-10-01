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

use std::{cell::Cell, ops::Range, rc::Rc};

use hgpui::{
    AppContext as _, Bounds, Context, Entity, EntityInputHandler, Focusable as _,
    InteractiveElement as _, Keystroke, NavigationDirection, ParentElement as _, Render,
    Styled as _, TestAppContext, Window, WindowHandle, div, point, px, size,
};

use crate::editable_text::{
    CaretShape, CaretStyle, EditableTextState, StringStorage, TextBoundary,
    actions::{DEFAULT_INPUT_CONTEXT, EditableTextActionHandler as _, Enter, default_bindings},
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

/// Lets the helpers below work with any view that owns one editable text state.
trait HasInput {
    fn input_entity(&self) -> &Entity<EditableTextState>;
}

impl HasInput for TestView {
    fn input_entity(&self) -> &Entity<EditableTextState> {
        &self.input
    }
}

/// Runs `f` against the state with a window available.
fn with_state<V: HasInput + Render + 'static, R>(
    cx: &mut TestAppContext,
    window: &WindowHandle<V>,
    f: impl FnOnce(&mut EditableTextState, &mut Window, &mut Context<EditableTextState>) -> R,
) -> R {
    window
        .update(cx, |view, window, cx| {
            view.input_entity()
                .update(cx, |state, cx| f(state, window, cx))
        })
        .expect("window update failed")
}

/// Simulates the platform handing typed (or pasted) text to the focused input.
fn type_text<V: HasInput + Render + 'static>(
    cx: &mut TestAppContext,
    window: &WindowHandle<V>,
    text: &str,
) {
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

// ---------------------------------------------------------------------------
// Form patterns: intercepting Enter from an ancestor
// ---------------------------------------------------------------------------

/// A form: an ancestor `div` that wants `Enter` (submit), wrapping a single-line input.
///
/// `capture` selects which phase the ancestor listens in, which is exactly what decides
/// whether it ever sees the key: the focused element handles actions during the bubble
/// phase and stops propagation there, so only a capture-phase listener runs first.
struct FormView {
    input: Entity<EditableTextState>,
    submits: Rc<Cell<usize>>,
    capture: bool,
}

impl HasInput for FormView {
    fn input_entity(&self) -> &Entity<EditableTextState> {
        &self.input
    }
}

impl Render for FormView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl hgpui::IntoElement {
        let on_enter = cx.listener(move |this, _: &Enter, _window, cx| {
            this.submits.set(this.submits.get() + 1);
            cx.stop_propagation();
        });

        let form = if self.capture {
            div().capture_action(on_enter)
        } else {
            div().on_action(on_enter)
        };

        form.child(text_input("form-input").state(self.input.downgrade()))
    }
}

fn setup_form(cx: &mut TestAppContext, capture: bool) -> (WindowHandle<FormView>, Entity<EditableTextState>) {
    cx.update(|cx| {
        cx.bind_keys(default_bindings().as_keybindings(Some(DEFAULT_INPUT_CONTEXT)));
    });

    let input = cx.update(|cx| cx.new(|cx| EditableTextState::new(StringStorage::default(), cx)));
    let input_for_view = input.clone();
    let window = cx.open_window(size(px(600.), px(400.)), move |_, _cx| FormView {
        input: input_for_view,
        submits: Rc::new(Cell::new(0)),
        capture,
    });
    cx.run_until_parked();

    // Focus the input the way a click would, so key dispatch targets the element.
    window
        .update(cx, |view, window, cx| {
            view.input.read(cx).focus_handle(cx).focus(window, cx);
        })
        .expect("window update failed");
    cx.run_until_parked();

    (window, input)
}

/// Presses a key through the real key dispatch path (keymap → action → listeners).
///
/// Uses `TestAppContext::dispatch_keystroke` rather than `WindowHandle::update` because
/// dispatching a key may draw the window, and drawing re-renders the root view — which
/// would re-enter the view lease held by `WindowHandle::update`.
fn press_key(cx: &mut TestAppContext, window: &WindowHandle<FormView>, key: &str) {
    cx.dispatch_keystroke(**window, Keystroke::parse(key).unwrap());
    cx.run_until_parked();
}

#[hgpui::test]
fn single_line_enter_is_a_no_op_for_the_input_itself(cx: &mut TestAppContext) {
    let (window, input) = setup_form(cx, false);
    type_text(cx, &window, "hello");
    with_state(cx, &window, |state, window, cx| {
        state.focus_handle(cx).focus(window, cx);
    });
    cx.run_until_parked();

    press_key(cx, &window, "enter");

    assert_eq!(
        content(cx, &input),
        "hello",
        "single-line fields must not gain a newline from enter"
    );
}

#[hgpui::test]
fn an_ancestor_capture_action_receives_enter_before_the_input(cx: &mut TestAppContext) {
    let (window, input) = setup_form(cx, true);
    type_text(cx, &window, "hello");

    press_key(cx, &window, "enter");

    let submits = cx.update(|cx| window.read(cx).expect("window read failed").submits.get());
    assert_eq!(
        submits, 1,
        "a capture-phase listener on an ancestor must see enter and be able to stop it"
    );
    assert_eq!(
        content(cx, &input),
        "hello",
        "submitting must not disturb the text"
    );
}

#[hgpui::test]
fn an_ancestor_bubble_action_never_receives_enter(cx: &mut TestAppContext) {
    let (window, _input) = setup_form(cx, false);

    press_key(cx, &window, "enter");

    let submits = cx.update(|cx| window.read(cx).expect("window read failed").submits.get());
    assert_eq!(
        submits, 0,
        "actions stop propagating at the focused element, so a bubble-phase listener \
         on an ancestor is unreachable while the input has focus"
    );
}

#[hgpui::test]
fn tab_inserts_a_literal_tab_into_a_single_line_field(cx: &mut TestAppContext) {
    // Documented backlog item in the module docs ("disabling insert_tab in favor of tab
    // being used to change focus between elements"); pinned here so a change is noticed.
    let (window, input) = setup(cx, false);
    with_state(cx, &window, |state, window, cx| {
        state.focus_handle(cx).focus(window, cx);
    });
    cx.run_until_parked();

    cx.dispatch_keystroke(*window, Keystroke::parse("tab").unwrap());
    cx.run_until_parked();

    assert_eq!(
        content(cx, &input),
        "\t",
        "tab is currently inserted as text; intercept it in the capture phase to use it \
         for focus movement instead"
    );
}

// ---------------------------------------------------------------------------
// "User is done editing": detecting focus loss
// ---------------------------------------------------------------------------

/// Two inputs where the first one wants to know when the user leaves it.
///
/// The element has no "editing finished" callback, so the view derives it: it remembers
/// whether the field was focused last frame and reacts to the focused→unfocused edge.
/// (`Context::on_blur` is the intended API for this, but its focus events are only
/// populated for an active window, which the test harness never has.)
struct BlurView {
    first: Entity<EditableTextState>,
    second: Entity<EditableTextState>,
    was_focused: bool,
    blurs: Rc<Cell<usize>>,
}

impl Render for BlurView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl hgpui::IntoElement {
        let focused = self.first.read(cx).focus_handle(cx).is_focused(window);
        if self.was_focused && !focused {
            self.blurs.set(self.blurs.get() + 1);
        }
        self.was_focused = focused;

        div()
            .flex()
            .flex_col()
            .child(text_input("blur-input-a").state(self.first.downgrade()))
            .child(text_input("blur-input-b").state(self.second.downgrade()))
    }
}

/// Draws the window once. Focus arrives as a *render* input (`is_focused`) and focus
/// events are dispatched at the end of a draw, so a redraw is what makes focus changes
/// observable.
fn draw(cx: &mut TestAppContext, window: &WindowHandle<BlurView>) {
    cx.update_window(**window, |_, window, cx| {
        let _ = window.draw(cx);
    })
    .expect("window update failed");
    cx.run_until_parked();
}

#[hgpui::test]
fn leaving_a_field_is_observable_in_render(cx: &mut TestAppContext) {
    use hgpui::Focusable as _;

    let first = cx.new(|cx| EditableTextState::new(StringStorage::default(), cx));
    let second = cx.new(|cx| EditableTextState::new(StringStorage::default(), cx));
    let blurs = Rc::new(Cell::new(0));
    let (first_for_view, second_for_view, blurs_for_view) =
        (first.clone(), second.clone(), blurs.clone());
    let window = cx.open_window(size(px(600.), px(400.)), move |_, _cx| BlurView {
        first: first_for_view,
        second: second_for_view,
        was_focused: false,
        blurs: blurs_for_view,
    });
    cx.run_until_parked();

    window
        .update(cx, |view, window, cx| {
            view.first.read(cx).focus_handle(cx).focus(window, cx);
        })
        .expect("window update failed");
    draw(cx, &window);
    assert_eq!(blurs.get(), 0, "focusing a field is not losing focus");

    // Focus the other field, which is what tabbing or clicking away does.
    window
        .update(cx, |view, window, cx| {
            view.second.read(cx).focus_handle(cx).focus(window, cx);
        })
        .expect("window update failed");
    draw(cx, &window);

    assert_eq!(
        blurs.get(),
        1,
        "the focused -> unfocused edge must be visible while rendering; it is the hook          to commit a value on, since the element has no 'editing finished' event"
    );
}

// ---------------------------------------------------------------------------
// Caret styling
// ---------------------------------------------------------------------------

/// Renders one input per caret shape so the styling setters are exercised through the
/// element itself (the geometry they produce is pinned in `element`'s unit tests).
struct CaretView {
    input: Entity<EditableTextState>,
}

impl HasInput for CaretView {
    fn input_entity(&self) -> &Entity<EditableTextState> {
        &self.input
    }
}

impl Render for CaretView {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl hgpui::IntoElement {
        div()
            .flex()
            .flex_col()
            .child(text_input("caret-bar").state(self.input.downgrade()))
            .child(
                text_input("caret-underline")
                    .state(self.input.downgrade())
                    .caret_shape(CaretShape::Underline)
                    .caret_width(px(3.0))
                    .caret_radius(px(1.5)),
            )
            .child(
                text_input("caret-full-style")
                    .state(self.input.downgrade())
                    .caret_style(CaretStyle {
                        color: hgpui::hsla(0.133, 0.845, 0.531, 1.0),
                        shape: CaretShape::Bar,
                        width: px(4.0),
                        radius: px(2.0),
                        height_ratio: 0.7,
                    }),
            )
    }
}

#[hgpui::test]
fn caret_styling_setters_render(cx: &mut TestAppContext) {
    let input = cx.new(|cx| EditableTextState::new(StringStorage::from("hello"), cx));
    let input_for_view = input.clone();
    let window = cx.open_window(size(px(600.), px(400.)), move |_, _cx| CaretView {
        input: input_for_view,
    });
    cx.run_until_parked();

    window
        .update(cx, |view, window, cx| {
            view.input.read(cx).focus_handle(cx).focus(window, cx);
        })
        .expect("window update failed");
    cx.run_until_parked();

    // The caret is drawn while focused; painting every shape must both succeed and leave
    // editing untouched. (The caret starts at offset 0, so the text lands before it.)
    type_text(cx, &window, "!");
    assert_eq!(content(cx, &input), "!hello");
}
