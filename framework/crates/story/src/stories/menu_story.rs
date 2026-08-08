use gpui::{
    Action, App, AppContext, Context, Corner, Entity, InteractiveElement, IntoElement, KeyBinding,
    ParentElement as _, Render, SharedString, Styled as _, Window, actions, div, px,
};
use gpui_component::{
    ActiveTheme as _, IconName, Side, StyledExt,
    button::Button,
    flyout_secondary_foreground, h_flex,
    menu::{ContextMenuExt, DropdownMenu as _, PopupMenuItem},
    v_flex,
};
use serde::Deserialize;

use crate::section;

#[derive(Action, Clone, PartialEq, Deserialize)]
#[action(namespace = menu_story, no_json)]
struct Info(usize);

actions!(menu_story, [Copy, Paste, Cut, SearchAll, ToggleCheck]);

const CONTEXT: &str = "menu_story";
pub fn init(cx: &mut App) {
    cx.bind_keys([
        #[cfg(target_os = "macos")]
        KeyBinding::new("cmd-c", Copy, Some(CONTEXT)),
        #[cfg(not(target_os = "macos"))]
        KeyBinding::new("ctrl-c", Copy, Some(CONTEXT)),
        #[cfg(target_os = "macos")]
        KeyBinding::new("cmd-v", Paste, Some(CONTEXT)),
        #[cfg(not(target_os = "macos"))]
        KeyBinding::new("ctrl-v", Paste, Some(CONTEXT)),
        #[cfg(target_os = "macos")]
        KeyBinding::new("cmd-x", Cut, Some(CONTEXT)),
        #[cfg(not(target_os = "macos"))]
        KeyBinding::new("ctrl-x", Cut, Some(CONTEXT)),
        #[cfg(target_os = "macos")]
        KeyBinding::new("cmd-shift-f", SearchAll, Some(CONTEXT)),
        #[cfg(not(target_os = "macos"))]
        KeyBinding::new("ctrl-shift-f", SearchAll, Some(CONTEXT)),
        KeyBinding::new("ctrl-shift-alt-t", ToggleCheck, Some(CONTEXT)),
    ])
}

pub struct MenuStory {
    check_side: Option<Side>,
    message: String,
}

impl super::Story for MenuStory {
    fn title() -> &'static str {
        "Menu"
    }

    fn description() -> &'static str {
        "Popup menu and context menu"
    }

    fn new_view(window: &mut Window, cx: &mut App) -> Entity<impl Render> {
        Self::view(window, cx)
    }
}

impl MenuStory {
    pub fn view(window: &mut Window, cx: &mut App) -> Entity<Self> {
        cx.new(|cx| Self::new(window, cx))
    }

    fn new(_: &mut Window, _: &mut Context<Self>) -> Self {
        Self {
            check_side: None,
            message: "Open a menu and choose an action.".to_string(),
        }
    }

    fn on_copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        self.message = "Copied selection".to_string();
        cx.notify()
    }

    fn on_cut(&mut self, _: &Cut, _: &mut Window, cx: &mut Context<Self>) {
        self.message = "Cut selection".to_string();
        cx.notify()
    }

    fn on_paste(&mut self, _: &Paste, _: &mut Window, cx: &mut Context<Self>) {
        self.message = "Pasted from clipboard".to_string();
        cx.notify()
    }

    fn on_search_all(&mut self, _: &SearchAll, _: &mut Window, cx: &mut Context<Self>) {
        self.message = "Opened workspace search".to_string();
        cx.notify()
    }

    fn on_action_info(&mut self, info: &Info, _: &mut Window, cx: &mut Context<Self>) {
        self.message = format!("Opened recent project {}", info.0 + 1);
        cx.notify()
    }

    fn on_action_toggle_check(&mut self, _: &ToggleCheck, _: &mut Window, cx: &mut Context<Self>) {
        self.check_side = if self.check_side == Some(Side::Left) {
            Some(Side::Right)
        } else if self.check_side == Some(Side::Right) {
            None
        } else {
            Some(Side::Left)
        };

        self.message = format!(
            "Check alignment: {}",
            match self.check_side {
                Some(Side::Left) => "left",
                Some(Side::Right) => "right",
                _ => "hidden",
            }
        );
        cx.notify()
    }
}

impl Render for MenuStory {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let check_side = self.check_side;
        let view = cx.entity();

        v_flex()
            .key_context(CONTEXT)
            .on_action(cx.listener(Self::on_copy))
            .on_action(cx.listener(Self::on_cut))
            .on_action(cx.listener(Self::on_paste))
            .on_action(cx.listener(Self::on_search_all))
            .on_action(cx.listener(Self::on_action_info))
            .on_action(cx.listener(Self::on_action_toggle_check))
            .size_full()
            .min_h(px(400.))
            .gap_6()
            .child(
                section("Menu states")
                    .v_flex()
                    .gap_4()
                    .child(
                        h_flex()
                            .gap_2()
                            .flex_wrap()
                            .child(
                                Button::new("menu-states")
                                    .outline()
                                    .label("Open menu")
                                    .dropdown_menu(move |this, window, cx| {
                                        this.check_side(check_side.unwrap_or(Side::Left))
                                            .label("Edit")
                                            .menu_with_icon(
                                                "Find in workspace",
                                                IconName::Search,
                                                Box::new(SearchAll),
                                            )
                                            .menu("Copy", Box::new(Copy))
                                            .menu("Cut", Box::new(Cut))
                                            .menu_with_disabled("Paste", Box::new(Paste), true)
                                            .separator()
                                            .label("View")
                                            .menu_with_check(
                                                "Cycle check alignment",
                                                check_side.is_some(),
                                                Box::new(ToggleCheck),
                                            )
                                            .menu_with_icon_and_disabled(
                                                "Reveal minimap",
                                                IconName::Eye,
                                                Box::new(Info(0)),
                                                true,
                                            )
                                            .separator()
                                            .item(
                                                PopupMenuItem::element(|_, cx| {
                                                    v_flex().child("Inspect selection").child(
                                                        div()
                                                            .text_xs()
                                                            // Custom flyout content should use the
                                                            // flyout secondary role, not the window
                                                            // `muted_foreground`, which is sub-AA on
                                                            // the flyout material in dark themes.
                                                            .text_color(
                                                                flyout_secondary_foreground(cx),
                                                            )
                                                            .child("Open details panel"),
                                                    )
                                                })
                                                .on_click(window.listener_for(
                                                    &view,
                                                    |this, _, _, cx| {
                                                        this.message =
                                                            "Opened selection details".to_string();
                                                        cx.notify();
                                                    },
                                                )),
                                            )
                                            .separator()
                                            .submenu_with_icon(
                                                Some(IconName::FolderOpen.into()),
                                                "Open recent",
                                                window,
                                                cx,
                                                |menu, _, _| {
                                                    menu.menu("gpui-component", Box::new(Info(0)))
                                                        .menu("Atlas editor", Box::new(Info(1)))
                                                        .menu(
                                                            "Window shell demo",
                                                            Box::new(Info(2)),
                                                        )
                                                },
                                            )
                                            .separator()
                                            .link_with_icon(
                                                "Component docs",
                                                IconName::ExternalLink,
                                                "https://bumpyclock.github.io/gpui-component/",
                                            )
                                    }),
                            )
                            .child(
                                Button::new("menu-right-checks")
                                    .outline()
                                    .label("Right-side checks")
                                    .dropdown_menu(|this, _, _| {
                                        this.check_side(Side::Right)
                                            .label("Panels")
                                            .menu_with_check(
                                                "Project navigator",
                                                true,
                                                Box::new(Info(0)),
                                            )
                                            .menu_with_check(
                                                "Debug console",
                                                false,
                                                Box::new(Info(1)),
                                            )
                                            .menu_with_disabled("Timeline", Box::new(Info(2)), true)
                                    }),
                            ),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .p_3()
                            .rounded_md()
                            .border_1()
                            .border_color(cx.theme().border)
                            .child(
                                div()
                                    .text_xs()
                                    .font_medium()
                                    .text_color(cx.theme().muted_foreground)
                                    .child("Last action"),
                            )
                            .child(self.message.clone()),
                    ),
            )
            .child(
                section("Context menu").v_flex().gap_4().child(
                    v_flex()
                        .w_full()
                        .p_4()
                        .items_center()
                        .justify_center()
                        .min_h_20()
                        .rounded_md()
                        .border_1()
                        .border_dashed()
                        .border_color(cx.theme().border)
                        .child("Right-click to open")
                        .context_menu({
                            move |this, window, cx| {
                                this.check_side(check_side.unwrap_or(Side::Left))
                                    .label("Selection")
                                    .menu("Cut", Box::new(Cut))
                                    .menu("Copy", Box::new(Copy))
                                    .menu("Paste", Box::new(Paste))
                                    .separator()
                                    .submenu("Refactor", window, cx, move |menu, _, _| {
                                        menu.menu("Extract method", Box::new(Info(0)))
                                            .menu("Rename symbol", Box::new(Info(1)))
                                            .menu_with_disabled(
                                                "Move to module",
                                                Box::new(Info(2)),
                                                true,
                                            )
                                    })
                                    .separator()
                                    .menu_with_icon(
                                        "Find references",
                                        IconName::Search,
                                        Box::new(SearchAll),
                                    )
                            }
                        })
                        .child(
                            div()
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child("Reopen repeatedly to compare enter and exit motion."),
                        ),
                ),
            )
            .child(
                section("Scrollable menu")
                    .child(
                        Button::new("dropdown-menu-scrollable-1")
                            .outline()
                            .label("100 items")
                            .dropdown_menu_with_anchor(Corner::TopRight, move |this, _, _| {
                                let mut this = this
                                    .scrollable(true)
                                    .max_h(px(300.))
                                    .label("100 workspace files");
                                for i in 0..100 {
                                    if i > 0 && i % 5 == 0 {
                                        this = this.separator();
                                    }

                                    this = this.menu(
                                        SharedString::from(format!(
                                            "workspace-file-{:02}.rs",
                                            i + 1
                                        )),
                                        Box::new(Info(i)),
                                    )
                                }
                                this.min_w(px(200.))
                            }),
                    )
                    .child(
                        Button::new("dropdown-menu-scrollable-2")
                            .outline()
                            .label("5 items")
                            .dropdown_menu_with_anchor(Corner::TopRight, move |this, _, _| {
                                let mut this = this
                                    .scrollable(true)
                                    .max_h(px(300.))
                                    .label("5 recent files");
                                for i in 0..5 {
                                    this = this.menu(
                                        SharedString::from(format!("recent-file-{}.rs", i + 1)),
                                        Box::new(Info(i)),
                                    )
                                }
                                this.min_w(px(180.))
                            }),
                    ),
            )
    }
}
