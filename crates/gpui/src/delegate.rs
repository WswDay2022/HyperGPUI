use crate::{AnyElement, ImageStyle, InteractiveElement, Interactivity, ParentElement, StatefulInteractiveElement, StyleRefinement, Styled, StyledImage};

pub trait ElementDelegate {
    type Target;
    fn delegate(&mut self) -> &mut Self::Target;
}

impl<R: ElementDelegate> Styled for R
where R::Target: Styled + Sized {
    fn style(&mut self) -> &mut StyleRefinement {
        self.delegate().style()
    }
}

impl<R: ElementDelegate> ParentElement for R
where  R::Target: ParentElement {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.delegate().extend(elements)
    }
}

impl<R: ElementDelegate> InteractiveElement for R
where R::Target: InteractiveElement + Sized {
    fn interactivity(&mut self) -> &mut Interactivity {
        self.delegate().interactivity()
    }
}

impl<R: ElementDelegate> StatefulInteractiveElement for R
where R::Target: StatefulInteractiveElement + Sized {}

impl<R: ElementDelegate> StyledImage for R
where R::Target: StyledImage + Sized {
    fn image_style(&mut self) -> &mut ImageStyle {
        self.delegate().image_style()
    }
}