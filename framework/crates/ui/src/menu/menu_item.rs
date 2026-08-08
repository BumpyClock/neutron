use crate::{
    ActiveTheme, Disableable, StyledExt, animation::spring_animation, global_state::GlobalState,
    h_flex,
};
use gpui::{
    AnimationExt as _, AnyElement, App, ClickEvent, ElementId, InteractiveElement, IntoElement,
    MouseButton, ParentElement, RenderOnce, Role, SharedString, StatefulInteractiveElement as _,
    StyleRefinement, Styled, Toggled, Window, prelude::FluentBuilder as _, px,
};
use smallvec::SmallVec;

#[derive(IntoElement)]
pub(crate) struct MenuItemElement {
    id: ElementId,
    group_name: SharedString,
    style: StyleRefinement,
    disabled: bool,
    selected: bool,
    a11y_role: Option<Role>,
    a11y_label: Option<SharedString>,
    a11y_toggled: Option<Toggled>,
    on_click: Option<Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>>,
    on_hover: Option<Box<dyn Fn(&bool, &mut Window, &mut App) + 'static>>,
    children: SmallVec<[AnyElement; 2]>,
}

impl MenuItemElement {
    /// Create a new MenuItem with the given ID and group name.
    pub(crate) fn new(id: impl Into<ElementId>, group_name: impl Into<SharedString>) -> Self {
        let id: ElementId = id.into();
        Self {
            id,
            group_name: group_name.into(),
            style: StyleRefinement::default(),
            disabled: false,
            selected: false,
            a11y_role: None,
            a11y_label: None,
            a11y_toggled: None,
            on_click: None,
            on_hover: None,
            children: SmallVec::new(),
        }
    }

    /// Set ListItem as the selected item style.
    pub(crate) fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    pub(crate) fn a11y(
        mut self,
        role: Option<Role>,
        label: Option<SharedString>,
        toggled: Option<Toggled>,
    ) -> Self {
        self.a11y_role = role;
        self.a11y_label = label;
        self.a11y_toggled = toggled;
        self
    }

    /// Set the disabled state of the MenuItem.
    pub(crate) fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Set a handler for when the MenuItem is clicked.
    pub(crate) fn on_click(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_click = Some(Box::new(handler));
        self
    }

    /// Set a handler for when the mouse enters the MenuItem.
    #[allow(unused)]
    pub fn on_hover(mut self, handler: impl Fn(&bool, &mut Window, &mut App) + 'static) -> Self {
        self.on_hover = Some(Box::new(handler));
        self
    }
}

impl Disableable for MenuItemElement {
    fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
}

impl Styled for MenuItemElement {
    fn style(&mut self) -> &mut gpui::StyleRefinement {
        &mut self.style
    }
}

impl ParentElement for MenuItemElement {
    fn extend(&mut self, elements: impl IntoIterator<Item = gpui::AnyElement>) {
        self.children.extend(elements);
    }
}

impl RenderOnce for MenuItemElement {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let a11y_role = self.a11y_role;
        let a11y_label = self.a11y_label;
        let a11y_toggled = self.a11y_toggled;
        let selected_state = window.use_keyed_state(
            ElementId::Name(SharedString::from(format!("{}:selected", self.group_name))),
            cx,
            |_, _| self.selected,
        );
        let became_selected = {
            let was_selected = *selected_state.read(cx);
            if was_selected != self.selected {
                selected_state.update(cx, |selected, _| *selected = self.selected);
            }
            !self.disabled && self.selected && !was_selected
        };
        let reduced_motion = GlobalState::global(cx).reduced_motion();
        let selection_animation = became_selected
            .then(|| spring_animation(&cx.theme().motion, reduced_motion))
            .flatten();
        let selected = self.selected;
        let selection_animation_id =
            SharedString::from(format!("{}:selection-feedback", self.group_name));

        h_flex()
            .id(self.id)
            .when_some(a11y_role, |this, role| this.role(role))
            .when_some(a11y_label, |this, label| this.aria_label(label))
            .when_some(a11y_toggled, |this, toggled| this.aria_toggled(toggled))
            .aria_disabled(self.disabled)
            .group(&self.group_name)
            .gap_x_1()
            .py_1()
            .px_2()
            .text_base()
            .text_color(cx.theme().foreground)
            .relative()
            .items_center()
            .justify_between()
            .refine_style(&self.style)
            .when_some(self.on_hover, |this, on_hover| {
                this.on_hover(move |hovered, window, cx| (on_hover)(hovered, window, cx))
            })
            .when(!self.disabled, |this| {
                this.group_hover(self.group_name, |this| {
                    if selected {
                        this.bg(cx.theme().primary)
                            .text_color(cx.theme().primary_foreground)
                    } else {
                        this.bg(cx.theme().list_hover)
                    }
                })
                .when(self.selected, |this| {
                    this.bg(cx.theme().primary)
                        .text_color(cx.theme().primary_foreground)
                })
                .when_some(self.on_click, |this, on_click| {
                    this.on_mouse_down(MouseButton::Left, move |_, _, cx| {
                        cx.stop_propagation();
                    })
                    .on_click(on_click)
                })
            })
            .when(self.disabled, |this| {
                this.text_color(crate::flyout_disabled_foreground(cx))
            })
            .children(self.children)
            .map(|this| {
                if let Some(animation) = selection_animation {
                    this.with_animation(selection_animation_id, animation, |this, delta| {
                        this.translate_x(px(1.5 * (delta - 1.0)))
                    })
                    .into_any_element()
                } else {
                    this.into_any_element()
                }
            })
    }
}
