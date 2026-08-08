use std::{cell::Cell, rc::Rc};

use crate::{
    App, AppContext, Context, Element, ElementId, Entity, IntoElement, Render, RenderOnce,
    StyleRefinement, Styled, TestAppContext, View, ViewElement, Window, div, px,
};

#[derive(IntoElement)]
struct StatelessComponent {
    render_count: Rc<Cell<usize>>,
}

impl RenderOnce for StatelessComponent {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        self.render_count.set(self.render_count.get() + 1);
        div()
    }
}

struct StatelessRoot {
    render_count: Rc<Cell<usize>>,
}

impl Render for StatelessRoot {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        StatelessComponent {
            render_count: self.render_count.clone(),
        }
    }
}

#[gpui::test]
fn derived_render_once_uses_the_stateless_view_path(cx: &mut TestAppContext) {
    let render_count = Rc::new(Cell::new(0));
    let (_, _cx) = cx.add_window_view(|_, _| StatelessRoot {
        render_count: render_count.clone(),
    });

    assert_eq!(render_count.get(), 1);
}

struct IdentityModel;

impl Render for IdentityModel {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

struct EntityBackedView {
    model: Entity<IdentityModel>,
}

impl View for EntityBackedView {
    fn entity_id(&self) -> Option<crate::EntityId> {
        Some(self.model.entity_id())
    }

    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        self.model
    }
}

#[gpui::test]
fn manual_view_uses_its_backing_entity_as_element_identity(cx: &mut TestAppContext) {
    let model = cx.new(|_| IdentityModel);
    let element = ViewElement::new(EntityBackedView {
        model: model.clone(),
    });

    assert_eq!(element.id(), Some(ElementId::View(model.entity_id())));
}

struct CachedChild {
    render_count: Rc<Cell<usize>>,
}

impl Render for CachedChild {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        self.render_count.set(self.render_count.get() + 1);
        div()
    }
}

struct CachedRoot {
    child: Entity<CachedChild>,
}

impl Render for CachedRoot {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        self.child
            .clone()
            .cached(StyleRefinement::default().w(px(20.)).h(px(20.)))
    }
}

#[gpui::test]
fn cached_entity_reuses_until_notified_and_rebuilds_for_a11y(cx: &mut TestAppContext) {
    let render_count = Rc::new(Cell::new(0));
    let (root, cx) = cx.add_window_view({
        let render_count = render_count.clone();
        move |_, cx| CachedRoot {
            child: cx.new(|_| CachedChild {
                render_count: render_count.clone(),
            }),
        }
    });

    assert_eq!(render_count.get(), 1);

    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    assert_eq!(render_count.get(), 1, "an unchanged cached view is reused");

    let child = root.read_with(cx, |root, _| root.child.clone());
    child.update(cx, |_, cx| cx.notify());
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    assert_eq!(render_count.get(), 2, "notification invalidates the cache");

    cx.update(|window, cx| {
        window.a11y.set_active_for_test(true);
        let _ = window.draw(cx);
        window.a11y.set_active_for_test(false);
    });
    assert_eq!(
        render_count.get(),
        3,
        "active accessibility rebuilds instead of replaying cached paint"
    );
}
