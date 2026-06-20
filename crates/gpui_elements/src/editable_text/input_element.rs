use gpui::{
    App, Element, ElementId, Entity, Hitbox, InteractiveElement, Interactivity, IntoElement,
    Length, SharedString, StyleRefinement, Styled, TextStyle,
};

use crate::editable_text::{TextInputState, UnicodeTextStorage};

#[track_caller]
pub fn input(id: impl Into<ElementId>) -> TextInputElement {
    let mut this = TextInputElement {
        id: id.into(),
        placeholder: None,
        interactivity: Interactivity::new(),
        init_storage: None,
    };
    this = this.key_context(super::DEFAULT_INPUT_CONTEXT);
    this
}

// TODO: Disabled flag/state?
pub struct TextInputElement {
    id: ElementId,
    placeholder: Option<SharedString>,
    interactivity: Interactivity,
    init_storage: Option<Box<dyn Fn(&mut App) -> Box<dyn UnicodeTextStorage>>>,
}

impl InteractiveElement for TextInputElement {
    fn interactivity(&mut self) -> &mut Interactivity {
        &mut self.interactivity
    }
}

impl Styled for TextInputElement {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.interactivity.base_style
    }
}

impl IntoElement for TextInputElement {
    type Element = Self;
    fn into_element(self) -> Self::Element {
        self
    }
}

#[doc(hidden)]
pub struct LayoutState {
    state: Entity<TextInputState>,
    text_style: TextStyle,
}

#[doc(hidden)]
pub struct PrepaintState {
    hitbox: Option<Hitbox>,
}

impl Element for TextInputElement {
    type RequestLayoutState = LayoutState;
    type PrepaintState = PrepaintState;

    fn id(&self) -> Option<ElementId> {
        self.interactivity.element_id.clone()
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        self.interactivity.source_location()
    }

    fn request_layout(
        &mut self,
        global_id: Option<&gpui::GlobalElementId>,
        inspector_id: Option<&gpui::InspectorElementId>,
        window: &mut gpui::Window,
        cx: &mut gpui::App,
    ) -> (gpui::LayoutId, Self::RequestLayoutState) {
        let mut resolved_text_style = None;

        // Get the state from the app using the element's id as the key.
        // If it doesnt exist, initialize a new state with the user's desired storage medium.
        let state = window.use_keyed_state(self.id.clone(), cx, |_window, cx| {
            let storage = match &self.init_storage {
                None => Box::new(String::new()),
                Some(init_storage) => (*init_storage)(cx),
            };
            TextInputState::new(storage, cx)
        });

        let layout_id = self.interactivity.request_layout(
            global_id,
            inspector_id,
            window,
            cx,
            |element_style, window, cx| {
                window.with_text_style(element_style.text_style().cloned(), |window| {
                    resolved_text_style = Some(window.text_style());

                    let style = element_style.clone();
                    // TODO: Does this need to propagate the line_height as the element's height?
                    window.request_layout(style, None, cx)
                })
            },
        );

        let layout_state = LayoutState {
            state,
            text_style: resolved_text_style.unwrap_or_else(|| window.text_style()),
        };
        (layout_id, layout_state)
    }

    fn prepaint(
        &mut self,
        id: Option<&gpui::GlobalElementId>,
        inspector_id: Option<&gpui::InspectorElementId>,
        bounds: gpui::Bounds<gpui::Pixels>,
        request_layout: &mut Self::RequestLayoutState,
        window: &mut gpui::Window,
        cx: &mut gpui::App,
    ) -> Self::PrepaintState {
        todo!()
    }

    fn paint(
        &mut self,
        id: Option<&gpui::GlobalElementId>,
        inspector_id: Option<&gpui::InspectorElementId>,
        bounds: gpui::Bounds<gpui::Pixels>,
        request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut gpui::Window,
        cx: &mut gpui::App,
    ) {
        todo!()
    }
}
