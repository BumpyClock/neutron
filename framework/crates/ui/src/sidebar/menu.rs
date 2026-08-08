use crate::{
    ActiveTheme as _, Anchor, Collapsible, FlyoutTokens, Icon, IconName, Selectable, Sizable as _,
    StyledExt, SurfaceContext, SurfacePreset,
    animation::{
        PresenceOptions, PresencePhase, PresenceTransition, exit_animation,
        expand_collapse_durations, expand_collapse_layout_animation, keyed_presence,
        spring_animation,
    },
    button::{Button, ButtonVariants as _},
    flyout_primary_foreground,
    global_state::GlobalState,
    h_flex,
    menu::{ContextMenuExt, PopupMenu, PopupMenuItem},
    popover::Popover,
    sidebar::SidebarItem,
    v_flex,
};
use gpui::{
    Animation, AnimationExt as _, AnyElement, App, ClickEvent, Context, DismissEvent, Div,
    ElementId, Entity, Focusable, InteractiveElement as _, IntoElement, MouseButton,
    ParentElement as _, RenderOnce, SharedString, StatefulInteractiveElement as _, StyleRefinement,
    Styled, Window, div, percentage, prelude::FluentBuilder, px,
};
use std::rc::Rc;
use std::time::Duration;

/// Generous max for animated submenu reveal.
const SUBMENU_CONTENT_MAX_H: f32 = 1200.0;
const SIDEBAR_ITEM_CONTENT_MAX_W: f32 = 320.0;

fn submenu_height_progress(progress: f32) -> f32 {
    progress.clamp(0.0, 1.0).powf(3.0)
}

fn sidebar_item_content_progress(progress: f32) -> f32 {
    progress.clamp(0.0, 1.0).powf(1.35)
}

fn animate_item_content(
    el: Div,
    id: &ElementId,
    presence: PresenceTransition,
    layout_anim: Option<Animation>,
    transform_anim: Option<Animation>,
) -> AnyElement {
    if !presence.transition_active() {
        return el.into_any_element();
    }

    let entering = matches!(presence.phase, PresencePhase::Entering);
    let layout_animated = if let Some(anim) = layout_anim {
        el.with_animation(
            SharedString::from(format!("{}-item-content-layout-{}", id, u8::from(entering))),
            anim,
            move |el, delta| {
                let progress = presence.progress(delta);
                let shaped = sidebar_item_content_progress(progress);
                el.max_w(px(SIDEBAR_ITEM_CONTENT_MAX_W * shaped))
                    .opacity(progress)
            },
        )
        .into_any_element()
    } else {
        el.into_any_element()
    };

    if let Some(anim) = transform_anim {
        return div()
            .child(layout_animated)
            .with_animation(
                SharedString::from(format!(
                    "{}-item-content-transform-{}",
                    id,
                    u8::from(entering)
                )),
                anim,
                move |el, delta| {
                    let progress = if entering { delta } else { 1.0 - delta };
                    el.translate_x(px(6.0 * (1.0 - progress)))
                },
            )
            .into_any_element();
    }

    layout_animated
}

#[derive(Default)]
struct SidebarCollapsedSubmenuState {
    menu: Option<Entity<PopupMenu>>,
}

#[derive(IntoElement)]
struct SidebarCollapsedSubmenuTrigger {
    selected: bool,
    element: AnyElement,
}

impl SidebarCollapsedSubmenuTrigger {
    fn new(element: AnyElement) -> Self {
        Self {
            selected: false,
            element,
        }
    }
}

impl Selectable for SidebarCollapsedSubmenuTrigger {
    fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    fn is_selected(&self) -> bool {
        self.selected
    }
}

impl RenderOnce for SidebarCollapsedSubmenuTrigger {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        self.element
    }
}

fn build_collapsed_submenu(
    mut menu: PopupMenu,
    items: Vec<SidebarMenuItem>,
    window: &mut Window,
    cx: &mut Context<PopupMenu>,
) -> PopupMenu {
    for item in items {
        let icon = item.icon.clone();
        let label = item.label.clone();
        if item.children.is_empty() {
            let handler = item.handler.clone();
            menu = if let Some(suffix) = item.suffix.clone() {
                let row_label = label.clone();
                let menu_item = PopupMenuItem::element(move |window, cx| {
                    h_flex()
                        .w_full()
                        .items_center()
                        .justify_between()
                        .gap_x_2()
                        .child(div().child(row_label.clone()))
                        .child(
                            div()
                                .on_mouse_down(MouseButton::Left, |_, _, cx| {
                                    cx.stop_propagation();
                                })
                                .on_mouse_up(MouseButton::Left, |_, _, cx| {
                                    cx.stop_propagation();
                                })
                                .child(suffix(window, cx)),
                        )
                })
                .disabled(item.disabled)
                .checked(item.active)
                .when_some(icon, |this, icon| this.icon(icon))
                .on_click(move |ev, window, cx| {
                    (handler)(ev, window, cx);
                });
                menu.item(menu_item)
            } else {
                menu.item(
                    PopupMenuItem::new(label)
                        .disabled(item.disabled)
                        .checked(item.active)
                        .when_some(icon, |this, icon| this.icon(icon))
                        .on_click(move |ev, window, cx| {
                            (handler)(ev, window, cx);
                        }),
                )
            };
            continue;
        }

        let children = item.children.clone();
        menu = menu.submenu_with_icon(icon, label, window, cx, move |submenu, window, cx| {
            build_collapsed_submenu(submenu, children.clone(), window, cx)
        });
    }

    menu
}

fn collapsed_submenu_has_suffix(items: &[SidebarMenuItem]) -> bool {
    items
        .iter()
        .any(|item| item.suffix.is_some() || collapsed_submenu_has_suffix(&item.children))
}

/// Menu for the [`super::Sidebar`]
#[derive(Clone)]
pub struct SidebarMenu {
    style: StyleRefinement,
    collapsed: bool,
    items: Vec<SidebarMenuItem>,
}

impl SidebarMenu {
    /// Create a new SidebarMenu
    pub fn new() -> Self {
        Self {
            style: StyleRefinement::default(),
            items: Vec::new(),
            collapsed: false,
        }
    }

    /// Add a [`SidebarMenuItem`] child menu item to the sidebar menu.
    ///
    /// See also [`SidebarMenu::children`].
    pub fn child(mut self, child: impl Into<SidebarMenuItem>) -> Self {
        self.items.push(child.into());
        self
    }

    /// Add multiple [`SidebarMenuItem`] child menu items to the sidebar menu.
    pub fn children(
        mut self,
        children: impl IntoIterator<Item = impl Into<SidebarMenuItem>>,
    ) -> Self {
        self.items = children.into_iter().map(Into::into).collect();
        self
    }
}

impl Collapsible for SidebarMenu {
    fn is_collapsed(&self) -> bool {
        self.collapsed
    }

    fn collapsed(mut self, collapsed: bool) -> Self {
        self.collapsed = collapsed;
        self
    }
}

impl SidebarItem for SidebarMenu {
    fn render(
        self,
        id: impl Into<ElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> impl IntoElement {
        let id = id.into();

        v_flex()
            .gap_2()
            .refine_style(&self.style)
            .children(self.items.into_iter().enumerate().map(|(ix, item)| {
                let id = SharedString::from(format!("{}-{}", id, ix));
                item.collapsed(self.collapsed)
                    .render(id, window, cx)
                    .into_any_element()
            }))
    }
}

impl Styled for SidebarMenu {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

/// Menu item for the [`SidebarMenu`]
#[derive(Clone)]
pub struct SidebarMenuItem {
    icon: Option<Icon>,
    label: SharedString,
    handler: Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>,
    active: bool,
    default_open: bool,
    click_to_open: bool,
    collapsed: bool,
    children: Vec<Self>,
    suffix: Option<Rc<dyn Fn(&mut Window, &mut App) -> AnyElement + 'static>>,
    disabled: bool,
    context_menu: Option<Rc<dyn Fn(PopupMenu, &mut Window, &mut App) -> PopupMenu + 'static>>,
}

impl SidebarMenuItem {
    /// Create a new [`SidebarMenuItem`] with a label.
    pub fn new(label: impl Into<SharedString>) -> Self {
        Self {
            icon: None,
            label: label.into(),
            handler: Rc::new(|_, _, _| {}),
            active: false,
            collapsed: false,
            default_open: false,
            click_to_open: false,
            children: Vec::new(),
            suffix: None,
            disabled: false,
            context_menu: None,
        }
    }

    /// Set the icon for the menu item
    pub fn icon(mut self, icon: impl Into<Icon>) -> Self {
        self.icon = Some(icon.into());
        self
    }

    /// Set the active state of the menu item
    pub fn active(mut self, active: bool) -> Self {
        self.active = active;
        self
    }

    /// Add a click handler to the menu item
    pub fn on_click(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.handler = Rc::new(handler);
        self
    }

    /// Set the collapsed state of the menu item
    pub fn collapsed(mut self, collapsed: bool) -> Self {
        self.collapsed = collapsed;
        self
    }

    /// Set the default open state of the Submenu, default is `false`.
    ///
    /// This only used on initial render, the internal state will be used afterwards.
    pub fn default_open(mut self, open: bool) -> Self {
        self.default_open = open;
        self
    }

    /// Set whether clicking the menu item open the submenu.
    ///
    /// Default is `false`.
    ///
    /// If `false` we only handle open/close via the caret button.
    pub fn click_to_open(mut self, click_to_open: bool) -> Self {
        self.click_to_open = click_to_open;
        self
    }

    pub fn children(mut self, children: impl IntoIterator<Item = impl Into<Self>>) -> Self {
        self.children = children.into_iter().map(Into::into).collect();
        self
    }

    /// Set the suffix for the menu item.
    pub fn suffix<F, E>(mut self, builder: F) -> Self
    where
        F: Fn(&mut Window, &mut App) -> E + 'static,
        E: IntoElement,
    {
        self.suffix = Some(Rc::new(move |window, cx| {
            builder(window, cx).into_any_element()
        }));
        self
    }

    /// Set disabled flat for menu item.
    pub fn disable(mut self, disable: bool) -> Self {
        self.disabled = disable;
        self
    }

    fn is_submenu(&self) -> bool {
        self.children.len() > 0
    }

    /// Set the context menu for the menu item.
    pub fn context_menu(
        mut self,
        f: impl Fn(PopupMenu, &mut Window, &mut App) -> PopupMenu + 'static,
    ) -> Self {
        self.context_menu = Some(Rc::new(f));
        self
    }
}

impl FluentBuilder for SidebarMenuItem {}

impl Collapsible for SidebarMenuItem {
    fn is_collapsed(&self) -> bool {
        self.collapsed
    }

    fn collapsed(mut self, collapsed: bool) -> Self {
        self.collapsed = collapsed;
        self
    }
}

impl SidebarItem for SidebarMenuItem {
    fn render(
        self,
        id: impl Into<ElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> impl IntoElement {
        let click_to_open = self.click_to_open;
        let default_open = self.default_open;
        let id = id.into();
        let state_key = SharedString::from(format!("sidebar-menu-state-{}", id));
        let open_state = window.use_keyed_state(state_key.clone(), cx, |_, _| default_open);
        let handler = self.handler.clone();
        let is_collapsed = self.collapsed;
        let is_active = self.active;
        let is_hoverable = !is_active && !self.disabled;
        let is_disabled = self.disabled;
        let is_submenu = self.is_submenu();
        let is_open = is_submenu && !is_collapsed && *open_state.read(cx);
        let show_collapsed_submenu = is_submenu && is_collapsed;
        let reduced_motion = GlobalState::global(cx).reduced_motion();
        let motion = cx.theme().motion.clone();
        let (open_duration, close_duration) = expand_collapse_durations(&motion);
        let submenu_presence = keyed_presence(
            SharedString::from(format!("{}-submenu-presence", state_key)),
            is_open,
            !reduced_motion,
            open_duration,
            close_duration,
            PresenceOptions::default(),
            window,
            cx,
        );
        let submenu_visible = submenu_presence.should_render();
        let submenu_entering = matches!(submenu_presence.phase, PresencePhase::Entering);
        let submenu_layout_anim =
            expand_collapse_layout_animation(&motion, reduced_motion, submenu_entering);
        // Direction-aware like the chevron below: the spring is a reveal, so a
        // collapse runs the exit token rather than springing back.
        let submenu_transform_anim = if submenu_entering {
            spring_animation(&motion, reduced_motion)
        } else {
            exit_animation(&motion, reduced_motion)
        };
        let chevron_open_anim = spring_animation(&motion, reduced_motion);
        let chevron_close_anim = exit_animation(&motion, reduced_motion);
        let item_content_presence = keyed_presence(
            SharedString::from(format!("{}-item-content-presence", state_key)),
            !is_collapsed,
            !reduced_motion,
            open_duration,
            close_duration,
            PresenceOptions::default(),
            window,
            cx,
        );
        let item_content_visible = item_content_presence.should_render();
        let item_content_layout_anim = expand_collapse_layout_animation(
            &motion,
            reduced_motion,
            matches!(item_content_presence.phase, PresencePhase::Entering),
        );
        let item_content_transform_anim = spring_animation(&motion, reduced_motion);

        let item_element = h_flex()
            .size_full()
            .id("item")
            .overflow_x_hidden()
            .flex_shrink_0()
            .p_2()
            .gap_x_2()
            .rounded(cx.theme().radius)
            .text_sm()
            .when(is_hoverable, |this| {
                this.hover(|this| {
                    this.bg(cx.theme().sidebar_accent.opacity(0.8))
                        .text_color(cx.theme().sidebar_accent_foreground)
                })
            })
            .when(is_active, |this| {
                this.font_medium()
                    .bg(cx.theme().sidebar_accent)
                    .text_color(cx.theme().sidebar_accent_foreground)
            })
            .when_some(self.icon.clone(), |this, icon| this.child(icon))
            .when(!item_content_visible, |this| {
                this.justify_center().when(is_active, |this| {
                    this.bg(cx.theme().sidebar_accent)
                        .text_color(cx.theme().sidebar_accent_foreground)
                })
            })
            .when(item_content_visible, |this| {
                this.h_7()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .child(
                                h_flex()
                                    .flex_1()
                                    .w_full()
                                    .min_w_0()
                                    .gap_x_2()
                                    .justify_between()
                                    .overflow_x_hidden()
                                    .child(
                                        h_flex()
                                            .flex_1()
                                            .min_w_0()
                                            .overflow_x_hidden()
                                            .child(self.label.clone()),
                                    )
                                    .when_some(self.suffix.clone(), |this, suffix| {
                                        this.child(
                                            div()
                                                .flex_shrink_0()
                                                .child(suffix(window, cx).into_any_element()),
                                        )
                                    }),
                            )
                            .map(|el| {
                                animate_item_content(
                                    el,
                                    &id,
                                    item_content_presence,
                                    item_content_layout_anim,
                                    item_content_transform_anim,
                                )
                            }),
                    )
                    .when(is_submenu, |this| {
                        let caret_base = Icon::new(IconName::ChevronRight).size_4();
                        let caret_icon = if reduced_motion || !submenu_presence.transition_active()
                        {
                            let icon = if is_open {
                                caret_base.rotate(percentage(0.25))
                            } else {
                                caret_base
                            };
                            icon.into_any_element()
                        } else {
                            let anim = if matches!(submenu_presence.phase, PresencePhase::Entering)
                            {
                                chevron_open_anim
                            } else {
                                chevron_close_anim
                            };
                            if let Some(anim) = anim {
                                let animation_id = SharedString::from(format!(
                                    "{}-submenu-caret-{}",
                                    id,
                                    u8::from(matches!(
                                        submenu_presence.phase,
                                        PresencePhase::Entering
                                    ))
                                ));
                                caret_base
                                    .with_animation(animation_id, anim, move |icon, delta| {
                                        let progress = if matches!(
                                            submenu_presence.phase,
                                            PresencePhase::Entering
                                        ) {
                                            delta
                                        } else {
                                            1.0 - delta
                                        };
                                        icon.rotate(percentage(0.25 * progress))
                                    })
                                    .into_any_element()
                            } else {
                                caret_base.into_any_element()
                            }
                        };
                        this.child(
                            Button::new("caret")
                                .xsmall()
                                .ghost()
                                .child(caret_icon)
                                .on_click({
                                    let open_state = open_state.clone();
                                    move |_, _, cx| {
                                        // Avoid trigger item click, just expand/collapse submenu
                                        cx.stop_propagation();
                                        open_state.update(cx, |is_open, cx| {
                                            *is_open = !*is_open;
                                            cx.notify();
                                        })
                                    }
                                }),
                        )
                    })
            })
            .when(is_disabled, |this| {
                this.text_color(cx.theme().muted_foreground)
            })
            .when(!is_disabled && !show_collapsed_submenu, |this| {
                this.on_click({
                    let open_state = open_state.clone();
                    move |ev, window, cx| {
                        if click_to_open {
                            open_state.update(cx, |is_open, cx| {
                                *is_open = true;
                                cx.notify();
                            });
                        }

                        handler(ev, window, cx)
                    }
                })
            })
            .map(|this| {
                if let Some(context_menu) = self.context_menu {
                    this.context_menu(move |menu, window, cx| context_menu(menu, window, cx))
                        .into_any_element()
                } else {
                    this.into_any_element()
                }
            });

        let item_element = if show_collapsed_submenu && !is_disabled {
            let children = self.children.clone();
            let has_suffix_controls = collapsed_submenu_has_suffix(&children);
            let collapsed_content_id = id.clone();
            let menu_state = if has_suffix_controls {
                None
            } else {
                Some(window.use_keyed_state(
                    SharedString::from(format!("{}-collapsed-submenu-menu", state_key)),
                    cx,
                    |_, _| SidebarCollapsedSubmenuState::default(),
                ))
            };

            Popover::new(SharedString::from(format!(
                "{}-collapsed-submenu-popover",
                id
            )))
            .appearance(false)
            .overlay_closable(has_suffix_controls)
            .anchor(Anchor::TopRight)
            .trigger(SidebarCollapsedSubmenuTrigger::new(item_element))
            .content(move |_, window, cx| {
                if has_suffix_controls {
                    let list_id =
                        SharedString::from(format!("{}-collapsed-submenu", collapsed_content_id));
                    let tokens = FlyoutTokens::new(cx);
                    return SurfacePreset::flyout()
                        .with_radius(tokens.radius)
                        .apply_material(v_flex().id(list_id.clone()), cx, SurfaceContext::new(cx))
                        .text_color(flyout_primary_foreground(cx))
                        .p(tokens.inset)
                        .gap(tokens.item_gap)
                        .w(px(220.))
                        .children(children.clone().into_iter().enumerate().map(|(ix, item)| {
                            let item_id = SharedString::from(format!("{}-{}", list_id, ix));
                            item.collapsed(false)
                                .render(item_id, window, cx)
                                .into_any_element()
                        }))
                        .into_any_element();
                }

                let Some(menu_state) = menu_state.as_ref() else {
                    return PopupMenu::build(window, cx, |menu, _, _| menu).into_any_element();
                };
                let menu = match menu_state.read(cx).menu.clone() {
                    Some(menu) => menu,
                    None => {
                        let menu_items = children.clone();
                        let menu = PopupMenu::build(window, cx, move |menu, window, cx| {
                            build_collapsed_submenu(menu, menu_items, window, cx)
                        });
                        menu_state.update(cx, |state, _| {
                            state.menu = Some(menu.clone());
                        });
                        menu.focus_handle(cx).focus(window, cx);

                        let popover_state = cx.entity();
                        window
                            .subscribe(&menu, cx, {
                                let menu_state = menu_state.clone();
                                move |_, _: &DismissEvent, window, cx| {
                                    popover_state.update(cx, |state, cx| {
                                        state.dismiss(window, cx);
                                        let dismiss_duration = Duration::from_millis(u64::from(
                                            cx.theme().motion.fade_duration_ms,
                                        ));
                                        cx.spawn({
                                            let menu_state = menu_state.clone();
                                            async move |_, cx| {
                                                cx.background_executor()
                                                    .timer(dismiss_duration)
                                                    .await;
                                                _ = menu_state.update(cx, |state, _| {
                                                    state.menu = None;
                                                });
                                            }
                                        })
                                        .detach();
                                    });
                                }
                            })
                            .detach();

                        menu.clone()
                    }
                };

                menu.into_any_element()
            })
            .into_any_element()
        } else {
            item_element
        };

        div()
            .id(id.clone())
            .w_full()
            .child(item_element)
            .when(submenu_visible, |this| {
                this.child(
                    div()
                        .id("submenu-wrapper")
                        .overflow_hidden()
                        .w_full()
                        .child(
                            v_flex()
                                .id("submenu")
                                .w_full()
                                .border_l_1()
                                .border_color(cx.theme().sidebar_border)
                                .gap_1()
                                .ml_3p5()
                                .pl_2p5()
                                .py_0p5()
                                .children(self.children.into_iter().enumerate().map(
                                    |(ix, item)| {
                                        let id = format!("{}-{}", id, ix);
                                        item.render(id, window, cx).into_any_element()
                                    },
                                )),
                        )
                        .map(|el| {
                            if !submenu_presence.transition_active() {
                                return el.into_any_element();
                            }

                            let layout_anim = submenu_layout_anim;
                            let layout_animated = if let Some(anim) = layout_anim {
                                el.with_animation(
                                    SharedString::from(format!(
                                        "{}-submenu-expand-{}",
                                        id,
                                        u8::from(matches!(
                                            submenu_presence.phase,
                                            PresencePhase::Entering
                                        ))
                                    )),
                                    anim,
                                    move |el, delta| {
                                        let progress = submenu_presence.progress(delta);
                                        let clamped = progress.clamp(0.0, 1.0);
                                        el.max_h(px(SUBMENU_CONTENT_MAX_H
                                            * submenu_height_progress(clamped)))
                                            .opacity(clamped)
                                    },
                                )
                                .into_any_element()
                            } else {
                                el.into_any_element()
                            };

                            let transform_anim = submenu_transform_anim;

                            if let Some(anim) = transform_anim {
                                return div()
                                    .child(layout_animated)
                                    .with_animation(
                                        SharedString::from(format!(
                                            "{}-submenu-transform-{}",
                                            id,
                                            u8::from(matches!(
                                                submenu_presence.phase,
                                                PresencePhase::Entering
                                            ))
                                        )),
                                        anim,
                                        move |el, delta| {
                                            let progress = if matches!(
                                                submenu_presence.phase,
                                                PresencePhase::Entering
                                            ) {
                                                delta
                                            } else {
                                                1.0 - delta
                                            };
                                            el.translate_y(px(3.0 * (1.0 - progress)))
                                        },
                                    )
                                    .into_any_element();
                            }

                            layout_animated
                        }),
                )
            })
    }
}
