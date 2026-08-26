use gpui::{
    AnyElement, ClickEvent, ElementId, InteractiveElement, IntoElement, MouseButton, ParentElement,
    RenderOnce, Role, SharedString, StatefulInteractiveElement, StyleRefinement, Styled, div,
    prelude::FluentBuilder as _,
};

use crate::{ActiveTheme as _, StyledExt};

/// A Link element like a `<a>` tag in HTML.
#[derive(IntoElement)]
pub struct Link {
    id: ElementId,
    style: StyleRefinement,
    href: Option<SharedString>,
    aria_label: Option<SharedString>,
    disabled: bool,
    on_click: Option<Box<dyn Fn(&ClickEvent, &mut gpui::Window, &mut gpui::App) + 'static>>,
    children: Vec<AnyElement>,
}

impl Link {
    /// Create a new Link element.
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self {
            id: id.into(),
            style: StyleRefinement::default(),
            href: None,
            aria_label: None,
            on_click: None,
            disabled: false,
            children: Vec::new(),
        }
    }

    /// Set the href of the link.
    pub fn href(mut self, href: impl Into<SharedString>) -> Self {
        self.href = Some(href.into());
        self
    }

    /// Set the accessible label for the link.
    pub fn aria_label(mut self, label: impl Into<SharedString>) -> Self {
        self.aria_label = Some(label.into());
        self
    }

    /// Set the click handler of the link.
    ///
    /// If this set, the handler will be called when the link is clicked.
    /// Otherwise, the link will only open the href if set.
    pub fn on_click(
        mut self,
        handler: impl Fn(&ClickEvent, &mut gpui::Window, &mut gpui::App) + 'static,
    ) -> Self {
        self.on_click = Some(Box::new(handler));
        self
    }

    /// Set the disabled state, default false.
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
}

impl Styled for Link {
    fn style(&mut self) -> &mut gpui::StyleRefinement {
        &mut self.style
    }
}

impl ParentElement for Link {
    fn extend(&mut self, elements: impl IntoIterator<Item = gpui::AnyElement>) {
        self.children.extend(elements)
    }
}

impl RenderOnce for Link {
    fn render(self, window: &mut gpui::Window, cx: &mut gpui::App) -> impl IntoElement {
        let href = self.href.clone();
        let on_click = self.on_click;
        let disabled = self.disabled;
        let focus_handle = window
            .use_keyed_state(self.id.clone(), cx, |_, cx| cx.focus_handle())
            .read(cx)
            .clone();

        div()
            .id(self.id)
            .role(Role::Link)
            .when_some(self.aria_label, |this, label| this.aria_label(label))
            .aria_disabled(disabled)
            .when(!disabled, |this| {
                this.track_focus(&focus_handle.tab_stop(true))
            })
            .text_color(cx.theme().link)
            .text_decoration_1()
            .text_decoration_color(cx.theme().link)
            .cursor_default()
            .when(!disabled, |this| {
                this.hover(|this| {
                    this.text_color(cx.theme().link.opacity(0.8))
                        .text_decoration_1()
                })
                .active(|this| {
                    this.text_color(cx.theme().link.opacity(0.6))
                        .text_decoration_1()
                })
                .cursor_pointer()
            })
            .when(disabled, |this| this.opacity(0.5))
            .refine_style(&self.style)
            .on_mouse_down(MouseButton::Left, |_, _, cx| {
                cx.stop_propagation();
            })
            .when(!disabled, |this| {
                this.on_click({
                    move |e, window, cx| {
                        if let Some(href) = &href {
                            cx.open_url(&href.clone());
                        }
                        if let Some(on_click) = &on_click {
                            on_click(e, window, cx);
                        }
                    }
                })
            })
            .children(self.children)
    }
}

#[cfg(test)]
mod tests {
    use std::{cell::Cell, rc::Rc};

    use gpui::{
        Context, Modifiers, Render, StatefulInteractiveElement as _, TestAppContext,
        VisualTestContext, px,
    };

    use super::*;

    struct LinkHarness {
        disabled: bool,
        link_clicks: Rc<Cell<usize>>,
        parent_clicks: Rc<Cell<usize>>,
    }

    impl Render for LinkHarness {
        fn render(&mut self, _: &mut gpui::Window, _: &mut Context<Self>) -> impl IntoElement {
            let link_clicks = self.link_clicks.clone();
            let parent_clicks = self.parent_clicks.clone();

            div()
                .id("link-parent")
                .size(px(100.))
                .on_click(move |_, _, _| parent_clicks.set(parent_clicks.get() + 1))
                .child(
                    Link::new("test-link")
                        .size_full()
                        .href("https://example.com")
                        .disabled(self.disabled)
                        .on_click(move |_, _, _| link_clicks.set(link_clicks.get() + 1))
                        .child("Example"),
                )
        }
    }

    fn harness(
        cx: &mut TestAppContext,
        disabled: bool,
    ) -> (&mut VisualTestContext, Rc<Cell<usize>>, Rc<Cell<usize>>) {
        cx.update(crate::init);
        let link_clicks = Rc::new(Cell::new(0));
        let parent_clicks = Rc::new(Cell::new(0));
        let (_, cx) = cx.add_window_view({
            let link_clicks = link_clicks.clone();
            let parent_clicks = parent_clicks.clone();
            move |_, _| LinkHarness {
                disabled,
                link_clicks,
                parent_clicks,
            }
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
        (cx, link_clicks, parent_clicks)
    }

    #[gpui::test]
    fn enabled_link_opens_url_and_isolates_parent(cx: &mut TestAppContext) {
        let (cx, link_clicks, parent_clicks) = harness(cx, false);
        cx.simulate_click(gpui::point(px(50.), px(50.)), Modifiers::none());

        assert_eq!(cx.opened_url().as_deref(), Some("https://example.com"));
        assert_eq!(link_clicks.get(), 1);
        assert_eq!(parent_clicks.get(), 0);
    }

    #[gpui::test]
    fn disabled_link_is_inert_and_isolates_parent(cx: &mut TestAppContext) {
        let (cx, link_clicks, parent_clicks) = harness(cx, true);
        cx.simulate_click(gpui::point(px(50.), px(50.)), Modifiers::none());

        assert_eq!(cx.opened_url(), None);
        assert_eq!(link_clicks.get(), 0);
        assert_eq!(parent_clicks.get(), 0);
    }
}
