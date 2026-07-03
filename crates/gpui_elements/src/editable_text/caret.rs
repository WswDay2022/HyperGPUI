use std::time::Duration;

use gpui::{Context, Entity, EventEmitter, Subscription, Task};
use smallvec::SmallVec;

/// Default interval for caret blinking (500ms).
pub const DEFAULT_BLINK_INTERVAL: Duration = Duration::from_millis(500);

/// Events emitted that the [`Caret`] listens to.
pub enum CaretNotify {
    /// The caret should pause blinking in response to a user-action
    PauseBlinking,
}

/// State of an EditableText caret cursor, which supports features like blinking.
/// Blinking is disabled by default.
pub struct Caret {
    /// The frequency at which the caret blinks
    interval: Duration,
    generation: usize,
    /// Whether the caret is presently visible in this frame
    visible: bool,
    /// Whether the caret's EditableText element is currently focused.
    /// Caret is only eligible to be blinking if currently focused.
    has_focus: bool,
    /// true when blinking is active but paused for some number of frames
    paused: bool,
    #[allow(dead_code)]
    subscriptions: SmallVec<[Subscription; 2]>,
    /// Tracks whether we were focused on the last update.
    was_focused: bool,
}
impl Default for Caret {
    fn default() -> Self {
        Self {
            interval: Duration::ZERO,
            generation: Default::default(),
            visible: false,
            has_focus: false,
            paused: false,
            subscriptions: SmallVec::new(),
            was_focused: false,
        }
    }
}

impl Caret {
    /// Sets the blinking interval of the caret to the global "default".
    /// The true default of the caret is "do not blink".
    pub fn blink_interval_default(mut self) -> Self {
        self.interval = DEFAULT_BLINK_INTERVAL;
        self
    }

    /// Sets the blinking interval of the caret.
    pub fn blink_interval(mut self, interval: Duration) -> Self {
        self.interval = interval;
        self
    }

    /// Listens for CaretNotify events on an entity (e.g. [`EditableTextState`]).
    pub fn subscribe_to<E>(&mut self, emitter: &Entity<E>, cx: &mut Context<Self>)
    where
        E: EventEmitter<CaretNotify>,
    {
        let handle = cx.subscribe(emitter, |state, _emitter, event, cx| match event {
            CaretNotify::PauseBlinking => {
                if state.interval.is_zero() {
                    return;
                }

                // Temporarily pauses blinking and leaves the caret visible. Blinking will resume after
                // the pre-established interval elapses from the time this is called.
                if !state.visible {
                    state.visible = true;
                }
                state.paused = true;
                state.restart_blink_ticker(cx);
                cx.notify();
            }
        });
        self.subscriptions.push(handle);
    }

    /// Processes updates during prepaint and returns whether the caret is currently visible.
    pub(super) fn update_focus(&mut self, is_focused: bool, cx: &mut Context<Self>) -> bool {
        let was_focused = self.was_focused;
        self.was_focused = is_focused;

        // Caret has no blinking interval, it is always visible
        if self.interval.is_zero() {
            return true;
        }

        match (is_focused, was_focused) {
            // Caret has a blinking interval, and gained focused.
            (true, false) => {
                self.has_focus = true;
                self.paused = false;

                // Render in this frame and restart the blinking ticker.
                self.visible = true;
                self.restart_blink_ticker(cx);
                true
            }
            // Caret has a blinking interval and lost focus
            (false, true) => {
                self.has_focus = false;
                self.visible = false;
                self.paused = false;
                cx.notify();
                false
            }
            // Has a blinking interval, but focus has not changed.
            // Only render if currently visible (based on blink ticker).
            _ => self.visible,
        }
    }

    fn restart_blink_ticker(&mut self, cx: &mut Context<Self>) {
        let generation = self.generation.wrapping_add(1);
        self.generation = generation;

        let interval = self.interval;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(interval).await;

            let Some(this) = this.upgrade() else { return };
            this.update(cx, |this, cx| {
                // If the generation has changed, that means a new task was spawned.
                // This one should be no-op since a new task is owning the blinking state.
                if this.generation == generation {
                    // PauseBlinking increments the generation via restart_ticker,
                    // so we can always unpause the blinking if the generation is unchanged.
                    this.paused = false;

                    // This was the last tick/blink before we lost focus.
                    // Should now go inert until focus is regained.
                    if !this.has_focus {
                        return;
                    }

                    // We still have focus, toggle whether caret is visible and make sure the owning element re-renders.
                    this.visible = !this.visible;
                    cx.notify();

                    // Start a fresh cycle by respawning the task.
                    this.restart_blink_ticker(cx);
                }
            });
        })
        .detach();
    }
}
