use std::{cell::Cell, rc::Rc, time::Duration};

use gpui::{
    AnimationExt as _, App, AppContext, Context, Entity, FocusHandle, Focusable, IntoElement,
    ParentElement, Render, SharedString, Styled, Window, div, prelude::FluentBuilder as _, px,
};

use gpui_component::{
    ActiveTheme, ElevationToken, FloatingSidebar, Icon, IconName, Side, Sizable, StyledExt,
    animation::{PresenceOptions, keyed_presence, reduced_motion, theme_animation},
    button::Button,
    h_flex,
    radio::RadioGroup,
    sidebar::{SidebarFooter, SidebarGroup, SidebarHeader, SidebarMenu, SidebarMenuItem},
    switch::Switch,
    v_flex,
};

use crate::section;

const STORY_SIDEBAR_WIDTH: gpui::Pixels = px(240.0);
const STORY_COLLAPSED_WIDTH: gpui::Pixels = px(48.0);
const STORY_DEFAULT_INSET: gpui::Pixels = px(8.0);

pub struct FloatingSidebarStory {
    focus_handle: FocusHandle,
    collapsed: bool,
    content_visual_offset: Rc<Cell<gpui::Pixels>>,
    content_transition_from: gpui::Pixels,
    content_transition_duration_ms: u16,
    content_transition_generation: u64,
    content_last_collapsed: bool,
    side: Side,
    wide_inset: bool,
    elevation: ElevationToken,
}

impl FloatingSidebarStory {
    fn new(_: &mut Window, cx: &mut Context<Self>) -> Self {
        Self {
            focus_handle: cx.focus_handle(),
            collapsed: false,
            content_visual_offset: Rc::new(Cell::new(
                STORY_SIDEBAR_WIDTH + STORY_DEFAULT_INSET * 2.0,
            )),
            content_transition_from: STORY_SIDEBAR_WIDTH + STORY_DEFAULT_INSET * 2.0,
            content_transition_duration_ms: 0,
            content_transition_generation: 0,
            content_last_collapsed: false,
            side: Side::Left,
            wide_inset: false,
            elevation: ElevationToken::Sm,
        }
    }

    pub fn view(window: &mut Window, cx: &mut App) -> Entity<Self> {
        cx.new(|cx| Self::new(window, cx))
    }

    fn elevation_index(&self) -> usize {
        match self.elevation {
            ElevationToken::None => 0,
            ElevationToken::Lg => 2,
            _ => 1,
        }
    }

    fn render_example(
        &mut self,
        id: &'static str,
        inset: gpui::Pixels,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let side = self.side;
        let reduced_motion = reduced_motion(cx);
        let motion = cx.theme().motion.clone();
        let content_target_offset = if self.collapsed {
            STORY_COLLAPSED_WIDTH
        } else {
            STORY_SIDEBAR_WIDTH
        } + inset * 2.0;
        if self.content_last_collapsed != self.collapsed {
            self.content_last_collapsed = self.collapsed;
            self.content_transition_generation += 1;

            self.content_transition_from = self.content_visual_offset.get();
            let full_distance = (STORY_SIDEBAR_WIDTH - STORY_COLLAPSED_WIDTH).as_f32();
            let remaining_distance = (content_target_offset - self.content_transition_from)
                .as_f32()
                .abs()
                .min(full_distance);
            // max() against the enter duration made the collapse branch a no-op
            // whenever exit is the shorter token, which is the usual case.
            let base_duration_ms = if self.collapsed {
                motion.exit_duration_ms
            } else {
                motion.enter_duration_ms
            };
            self.content_transition_duration_ms =
                ((f32::from(base_duration_ms) * remaining_distance / full_distance).round() as u16)
                    .max(1);
        }
        let content_presence = keyed_presence(
            SharedString::from(format!("{id}-content-offset")),
            !self.collapsed,
            !reduced_motion,
            Duration::from_millis(u64::from(self.content_transition_duration_ms.max(1))),
            Duration::from_millis(u64::from(self.content_transition_duration_ms.max(1))),
            PresenceOptions::default(),
            window,
            cx,
        );
        let content_transition = if content_presence.transition_active() {
            theme_animation(
                self.content_transition_duration_ms,
                &motion.standard_easing,
                reduced_motion,
            )
            .map(|animation| {
                (
                    self.content_transition_from,
                    self.content_transition_generation,
                    animation,
                )
            })
        } else {
            self.content_visual_offset.set(content_target_offset);
            None
        };

        let sidebar = FloatingSidebar::new(id)
            .side(self.side)
            .collapsed(self.collapsed)
            .width(STORY_SIDEBAR_WIDTH)
            .inset(inset)
            .elevation(self.elevation)
            .header_with(move |collapsed, _, cx| {
                SidebarHeader::new()
                    .child(
                        h_flex()
                            .min_w_0()
                            .gap_2()
                            .child(
                                div()
                                    .flex_shrink_0()
                                    .size_6()
                                    .items_center()
                                    .justify_center()
                                    .rounded(cx.theme().radius)
                                    .bg(cx.theme().sidebar_primary)
                                    .text_color(cx.theme().sidebar_primary_foreground)
                                    .child(
                                        Icon::new(if side.is_left() {
                                            IconName::PanelLeft
                                        } else {
                                            IconName::PanelRight
                                        })
                                        .size_4(),
                                    ),
                            )
                            .when(!collapsed, |this| {
                                this.child(
                                    v_flex()
                                        .min_w_0()
                                        .line_height(px(16.0))
                                        .child(div().text_sm().font_medium().child("Granite"))
                                        .child(
                                            div()
                                                .text_xs()
                                                .text_color(
                                                    cx.theme().sidebar_foreground.opacity(0.64),
                                                )
                                                .child("Product workspace"),
                                        ),
                                )
                            }),
                    )
                    .when(!collapsed, |this| {
                        this.child(Icon::new(IconName::ChevronsUpDown).size_4())
                    })
            })
            .child(
                SidebarGroup::new("Navigation").child(
                    SidebarMenu::new().children([
                        SidebarMenuItem::new("Overview")
                            .icon(IconName::LayoutDashboard)
                            .active(true),
                        SidebarMenuItem::new("Projects")
                            .icon(IconName::Folder)
                            .suffix(|_, cx| {
                                div()
                                    .h_5()
                                    .min_w_5()
                                    .px_1()
                                    .items_center()
                                    .justify_center()
                                    .rounded(cx.theme().radius)
                                    .bg(cx.theme().sidebar_accent)
                                    .text_xs()
                                    .text_color(cx.theme().sidebar_accent_foreground)
                                    .child("7")
                            }),
                        SidebarMenuItem::new("Inbox")
                            .icon(IconName::Inbox)
                            .suffix(|_, cx| {
                                div()
                                    .h_5()
                                    .min_w_5()
                                    .px_1()
                                    .items_center()
                                    .justify_center()
                                    .rounded(cx.theme().radius)
                                    .bg(cx.theme().sidebar_accent)
                                    .text_xs()
                                    .text_color(cx.theme().sidebar_accent_foreground)
                                    .child("3")
                            }),
                        SidebarMenuItem::new("Settings").icon(IconName::Settings2),
                    ]),
                ),
            )
            .footer_with(|collapsed, _, cx| {
                SidebarFooter::new()
                    .child(
                        h_flex()
                            .min_w_0()
                            .gap_2()
                            .child(
                                div()
                                    .flex_shrink_0()
                                    .size_6()
                                    .items_center()
                                    .justify_center()
                                    .rounded(px(999.0))
                                    .bg(cx.theme().secondary)
                                    .text_xs()
                                    .font_medium()
                                    .child("MC"),
                            )
                            .when(!collapsed, |this| {
                                this.child(
                                    v_flex()
                                        .min_w_0()
                                        .line_height(px(16.0))
                                        .child(div().text_sm().font_medium().child("Mira Chen"))
                                        .child(
                                            div()
                                                .text_xs()
                                                .text_color(
                                                    cx.theme().sidebar_foreground.opacity(0.64),
                                                )
                                                .child("mira@granite.tools"),
                                        ),
                                )
                            }),
                    )
                    .when(!collapsed, |this| {
                        this.child(Icon::new(IconName::ChevronsUpDown).size_4())
                    })
            });

        let content = div()
            .size_full()
            .when(self.side.is_left(), |this| this.pl(content_target_offset))
            .when(self.side.is_right(), |this| this.pr(content_target_offset))
            .child(
                v_flex()
                    .size_full()
                    .gap_4()
                    .p_4()
                    .child(
                        h_flex()
                            .justify_between()
                            .gap_3()
                            .child(
                                v_flex()
                                    .gap_1()
                                    .child(div().text_lg().font_semibold().child("Overview"))
                                    .child(
                                        div()
                                            .text_sm()
                                            .text_color(cx.theme().muted_foreground)
                                            .child("Current workspace activity"),
                                    ),
                            )
                            .child(
                                Button::new("floating-sidebar-canvas-toggle")
                                    .small()
                                    .outline()
                                    .icon(if self.collapsed {
                                        if self.side.is_left() {
                                            IconName::PanelLeftOpen
                                        } else {
                                            IconName::PanelRightOpen
                                        }
                                    } else if self.side.is_left() {
                                        IconName::PanelLeftClose
                                    } else {
                                        IconName::PanelRightClose
                                    })
                                    .label(if self.collapsed { "Expand" } else { "Collapse" })
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.collapsed = !this.collapsed;
                                        cx.notify();
                                    })),
                            ),
                    )
                    .child(
                        h_flex()
                            .gap_3()
                            .child(
                                v_flex()
                                    .flex_1()
                                    .gap_2()
                                    .rounded(cx.theme().radius)
                                    .border_1()
                                    .border_color(cx.theme().border)
                                    .bg(cx.theme().card)
                                    .p_3()
                                    .child(
                                        div()
                                            .text_sm()
                                            .text_color(cx.theme().muted_foreground)
                                            .child("Open projects"),
                                    )
                                    .child(div().text_xl().font_semibold().child("7")),
                            )
                            .child(
                                v_flex()
                                    .flex_1()
                                    .gap_2()
                                    .rounded(cx.theme().radius)
                                    .border_1()
                                    .border_color(cx.theme().border)
                                    .bg(cx.theme().card)
                                    .p_3()
                                    .child(
                                        div()
                                            .text_sm()
                                            .text_color(cx.theme().muted_foreground)
                                            .child("Ready for review"),
                                    )
                                    .child(div().text_xl().font_semibold().child("3")),
                            ),
                    )
                    .child(
                        v_flex()
                            .gap_3()
                            .rounded(cx.theme().radius)
                            .border_1()
                            .border_color(cx.theme().border)
                            .bg(cx.theme().card)
                            .p_3()
                            .child(div().text_sm().font_medium().child("Recent activity"))
                            .child(
                                h_flex()
                                    .gap_2()
                                    .text_sm()
                                    .child(Icon::new(IconName::Folder).size_4())
                                    .child("Mira updated desktop navigation"),
                            )
                            .child(
                                h_flex()
                                    .gap_2()
                                    .text_sm()
                                    .child(Icon::new(IconName::Settings2).size_4())
                                    .child("Workspace permissions changed"),
                            ),
                    ),
            )
            .map(move |content| {
                let Some((from, generation, animation)) = content_transition else {
                    return content.into_any_element();
                };
                let visual_offset = self.content_visual_offset.clone();
                content
                    .with_animation(
                        format!("floating-sidebar-content-offset-{generation}"),
                        animation,
                        move |content, delta| {
                            let offset = from + (content_target_offset - from) * delta;
                            visual_offset.set(offset);
                            if side.is_left() {
                                content.pl(offset)
                            } else {
                                content.pr(offset)
                            }
                        },
                    )
                    .into_any_element()
            });

        div()
            .relative()
            .w(px(720.0))
            .h(px(420.0))
            .overflow_hidden()
            .rounded(cx.theme().radius_lg)
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().background)
            .child(content)
            .child(sidebar)
    }
}

impl super::Story for FloatingSidebarStory {
    fn title() -> &'static str {
        "Floating Sidebar"
    }

    fn description() -> &'static str {
        "FloatingSidebar combines SidebarShell and Sidebar with built-in resize handling."
    }

    fn new_view(window: &mut Window, cx: &mut App) -> Entity<impl Render> {
        Self::view(window, cx)
    }
}

impl Focusable for FloatingSidebarStory {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for FloatingSidebarStory {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let inset = if self.wide_inset { px(12.0) } else { px(8.0) };

        v_flex().gap_6().child(
            section("Floating Sidebar").child(
                v_flex()
                    .gap_4()
                    .child(
                        h_flex()
                            .flex_wrap()
                            .gap_x_4()
                            .gap_y_3()
                            .child(
                                Switch::new("floating-sidebar-collapsed")
                                    .label("Collapsed")
                                    .checked(self.collapsed)
                                    .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                        this.collapsed = *checked;
                                        cx.notify();
                                    })),
                            )
                            .child(
                                Switch::new("floating-sidebar-side")
                                    .label("Right Side")
                                    .checked(self.side.is_right())
                                    .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                        this.side = if *checked { Side::Right } else { Side::Left };
                                        cx.notify();
                                    })),
                            )
                            .child(
                                Switch::new("floating-sidebar-inset")
                                    .label("Wide Inset")
                                    .checked(self.wide_inset)
                                    .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                        this.wide_inset = *checked;
                                        cx.notify();
                                    })),
                            )
                            .child(
                                RadioGroup::horizontal("floating-sidebar-elevation")
                                    .children(["None", "Small", "Large"])
                                    .selected_index(Some(self.elevation_index()))
                                    .on_click(cx.listener(|this, selected_ix: &usize, _, cx| {
                                        this.elevation = match selected_ix {
                                            0 => ElevationToken::None,
                                            1 => ElevationToken::Sm,
                                            2 => ElevationToken::Lg,
                                            _ => return,
                                        };
                                        cx.notify();
                                    })),
                            ),
                    )
                    .child(self.render_example("floating-sidebar-example", inset, window, cx)),
            ),
        )
    }
}
