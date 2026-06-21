use crate::editable_text::{
    InitStorage, TextAreaState,
    actions::{DEFAULT_INPUT_CONTEXT, EditableInputActionElement},
};
use gpui::{
    Element, ElementId, Entity, Hitbox, InteractiveElement, Interactivity, IntoElement,
    SharedString, StyleRefinement, Styled, TextStyle, WeakEntity,
};
use std::{cell::RefCell, rc::Rc};

pub fn text_area(id: impl Into<ElementId>) -> TextAreaElement {
    let mut this = TextAreaElement {
        interactivity: Interactivity::new(),
        state_entity: Rc::new(RefCell::new(WeakEntity::new_invalid())),
        init_storage: InitStorage::default(),
        placeholder: None,
    };
    this.interactivity.element_id = Some(id.into());

    this = this.key_context(DEFAULT_INPUT_CONTEXT);
    this.register_actions();

    this
}

// TODO: Disabled flag/state?
pub struct TextAreaElement {
    interactivity: Interactivity,
    // Populated on first render with an entity stored/attached to the view.
    // This reference is shared with the action handlers, which are processed between renders
    // and therefore cannot otherwise access state attached to the view.
    state_entity: Rc<RefCell<WeakEntity<TextAreaState>>>,
    init_storage: InitStorage,
    placeholder: Option<SharedString>,
}

impl InteractiveElement for TextAreaElement {
    fn interactivity(&mut self) -> &mut Interactivity {
        &mut self.interactivity
    }
}

impl Styled for TextAreaElement {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.interactivity.base_style
    }
}

impl IntoElement for TextAreaElement {
    type Element = Self;
    fn into_element(self) -> Self::Element {
        self
    }
}

impl EditableInputActionElement for TextAreaElement {
    type State = TextAreaState;
    fn state_entity_rc(&self) -> &Rc<RefCell<WeakEntity<Self::State>>> {
        &self.state_entity
    }
}

pub mod element {
    use super::*;

    #[doc(hidden)]
    pub struct LayoutState {
        pub state: Entity<TextAreaState>,
        pub text_style: TextStyle,
    }

    #[doc(hidden)]
    pub struct PrepaintState {
        pub hitbox: Option<Hitbox>,
    }
}

impl Element for TextAreaElement {
    type RequestLayoutState = element::LayoutState;
    type PrepaintState = element::PrepaintState;

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
        // Fetches or initializes the internal state of the field
        let state = match &self.interactivity.element_id {
            None => unimplemented!("all input elements must be assigned an id"),
            Some(element_id) => {
                let state = window.use_keyed_state(element_id.clone(), cx, |_window, cx| {
                    TextAreaState::new(self.init_storage.exec(cx), cx)
                });
                // store a reference to the entity owned by the element for access in action handlers
                *self.state_entity.borrow_mut() = state.downgrade();
                state
            }
        };

        let mut resolved_text_style = None;

        let layout_id = self.interactivity.request_layout(
            global_id,
            inspector_id,
            window,
            cx,
            |element_style, window, cx| {
                window.with_text_style(element_style.text_style().cloned(), |window| {
                    resolved_text_style = Some(window.text_style());

                    let mut style = element_style.clone();
                    if let gpui::Length::Auto = style.size.width {
                        style.size.width = gpui::relative(1.).into();
                    }
                    if let gpui::Length::Auto = style.size.height {
                        style.size.height = gpui::relative(1.).into();
                    }
                    window.request_layout(style, None, cx)
                })
            },
        );

        let layout_state = element::LayoutState {
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
