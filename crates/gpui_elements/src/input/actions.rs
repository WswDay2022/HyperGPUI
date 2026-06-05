/// The key context used for input element keybindings.
pub const DEFAULT_INPUT_CONTEXT: &str = "Input";

gpui::actions!(
    actions,
    [
        /// Delete the character before the cursor.
        Backspace,
        /// Delete the character after the cursor.
        Delete,
        /// Blur focus from the input.
        Escape,
        /// Delete the word before the cursor.
        DeleteWordLeft,
        /// Delete the word after the cursor.
        DeleteWordRight,
        /// Delete from the cursor to the beginning of the line.
        DeleteToBeginningOfLine,
        /// Delete from the cursor to the end of the line.
        DeleteToEndOfLine,
        /// Insert a tab character at the cursor position.
        Tab,
        /// Move the cursor one character to the left.
        Left,
        /// Move the cursor one character to the right.
        Right,
        /// Move the cursor up one visual line.
        Up,
        /// Move the cursor down one visual line.
        Down,
        /// Extend selection one character to the left.
        SelectLeft,
        /// Extend selection one character to the right.
        SelectRight,
        /// Extend selection up one visual line.
        SelectUp,
        /// Extend selection down one visual line.
        SelectDown,
        /// Select all text content.
        SelectAll,
        /// Move cursor to the start of the current line.
        Home,
        /// Move cursor to the end of the current line.
        End,
        /// Extend selection to the beginning of the content.
        SelectToBeginning,
        /// Extend selection to the end of the content.
        SelectToEnd,
        /// Move cursor to the beginning of the content.
        MoveToBeginning,
        /// Move cursor to the end of the content.
        MoveToEnd,
        /// Paste from clipboard at the cursor position.
        Paste,
        /// Cut selected text to clipboard.
        Cut,
        /// Copy selected text to clipboard.
        Copy,
        /// Insert a newline at the cursor position.
        Enter,
        /// Move cursor one word to the left.
        WordLeft,
        /// Move cursor one word to the right.
        WordRight,
        /// Extend selection one word to the left.
        SelectWordLeft,
        /// Extend selection one word to the right.
        SelectWordRight,
        /// Undo the last edit.
        Undo,
        /// Redo the last undone edit.
        Redo,
    ]
);

pub fn default_bindings() -> gpui::ActionBindingCollection {
    let mut bindings = gpui::ActionBindingCollection::default();

    #[cfg(target_os = "macos")]
    {
        bindings = bindings
            .with::<Backspace>("backspace")
            .with::<Delete>("delete")
            .with::<DeleteWordLeft>("alt-backspace")
            .with::<DeleteWordRight>("alt-delete")
            .with::<DeleteToBeginningOfLine>("cmd-backspace")
            .with::<DeleteToEndOfLine>("ctrl-k")
            .with::<Tab>("tab")
            .with::<Enter>("enter")
            .with::<Left>("left")
            .with::<Right>("right")
            .with::<Up>("up")
            .with::<Down>("down")
            .with::<SelectLeft>("shift-left")
            .with::<SelectRight>("shift-right")
            .with::<SelectUp>("shift-up")
            .with::<SelectDown>("shift-down")
            .with::<SelectAll>("cmd-a")
            // Mac keyboards don't have Home/End keys, so cmd-left/right are standard
            .with::<Home>("cmd-left")
            .with::<End>("cmd-right")
            .with::<MoveToBeginning>("cmd-up")
            .with::<MoveToEnd>("cmd-down")
            .with::<SelectToBeginning>("cmd-shift-up")
            .with::<SelectToEnd>("cmd-shift-down")
            .with::<WordLeft>("alt-left")
            .with::<WordRight>("alt-right")
            .with::<SelectWordLeft>("alt-shift-left")
            .with::<SelectWordRight>("alt-shift-right")
            .with::<Copy>("cmd-c")
            .with::<Cut>("cmd-x")
            .with::<Paste>("cmd-v")
            .with::<Undo>("cmd-z")
            .with::<Redo>("cmd-shift-z")
            .with::<Escape>("escape");
    }

    #[cfg(not(target_os = "macos"))]
    {
        bindings = bindings
            .with::<Backspace>("backspace")
            .with::<Delete>("delete")
            .with::<DeleteWordLeft>("ctrl-backspace")
            .with::<DeleteWordRight>("ctrl-delete")
            .with::<DeleteToBeginningOfLine>("ctrl-shift-backspace")
            .with::<DeleteToEndOfLine>("ctrl-shift-delete")
            .with::<Tab>("tab")
            .with::<Enter>("enter")
            .with::<Left>("left")
            .with::<Right>("right")
            .with::<Up>("up")
            .with::<Down>("down")
            .with::<SelectLeft>("shift-left")
            .with::<SelectRight>("shift-right")
            .with::<SelectUp>("shift-up")
            .with::<SelectDown>("shift-down")
            .with::<SelectAll>("ctrl-a")
            .with::<Home>("home")
            .with::<End>("end")
            .with::<MoveToBeginning>("ctrl-home")
            .with::<MoveToEnd>("ctrl-end")
            .with::<SelectToBeginning>("ctrl-shift-home")
            .with::<SelectToEnd>("ctrl-shift-end")
            .with::<WordLeft>("ctrl-left")
            .with::<WordRight>("ctrl-right")
            .with::<SelectWordLeft>("ctrl-shift-left")
            .with::<SelectWordRight>("ctrl-shift-right")
            .with::<Copy>("ctrl-c")
            .with::<Cut>("ctrl-x")
            .with::<Paste>("ctrl-v")
            .with::<Undo>("ctrl-z")
            .with::<Redo>("ctrl-shift-z")
            .with::<Escape>("escape");
    }

    bindings
}
