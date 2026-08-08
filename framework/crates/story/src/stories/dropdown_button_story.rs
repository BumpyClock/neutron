use gpui::{
    Action, App, AppContext as _, Context, Corner, Entity, Focusable, IntoElement,
    ParentElement as _, Render, Styled as _, Window, prelude::FluentBuilder as _,
};
use serde::Deserialize;

use crate::section;
use gpui_component::{
    ActiveTheme, Disableable, IconName, Selectable as _, Sizable as _, Theme,
    button::{Button, ButtonVariants as _, DropdownButton},
    checkbox::Checkbox,
    h_flex,
    menu::DropdownMenu as _,
    v_flex,
};

#[derive(Clone, Action, PartialEq, Eq, Deserialize)]
#[action(namespace = dropdown_button_story, no_json)]
enum ButtonAction {
    Disabled,
    Loading,
    Selected,
    Compact,
}

pub struct DropdownButtonStory {
    focus_handle: gpui::FocusHandle,
    disabled: bool,
    loading: bool,
    selected: bool,
    compact: bool,
}

impl DropdownButtonStory {
    pub fn view(_: &mut Window, cx: &mut App) -> Entity<Self> {
        cx.new(|cx| Self {
            focus_handle: cx.focus_handle(),
            disabled: false,
            loading: false,
            selected: false,
            compact: false,
        })
    }
}

impl super::Story for DropdownButtonStory {
    fn title() -> &'static str {
        "DropdownButton"
    }

    fn description() -> &'static str {
        "A button with an attached dropdown menu for additional options."
    }

    fn closable() -> bool {
        false
    }

    fn new_view(window: &mut Window, cx: &mut App) -> Entity<impl Render> {
        Self::view(window, cx)
    }
}

impl Focusable for DropdownButtonStory {
    fn focus_handle(&self, _: &gpui::App) -> gpui::FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for DropdownButtonStory {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let disabled = self.disabled;
        let loading = self.loading;
        let selected = self.selected;
        let compact = self.compact;

        v_flex()
            .gap_6()
            .child(
                h_flex()
                    .gap_3()
                    .child(
                        Checkbox::new("disabled-button")
                            .label("Disabled")
                            .checked(self.disabled)
                            .on_click(cx.listener(|view, _, _, cx| {
                                view.disabled = !view.disabled;
                                cx.notify();
                            })),
                    )
                    .child(
                        Checkbox::new("loading-button")
                            .label("Loading")
                            .checked(self.loading)
                            .on_click(cx.listener(|view, _, _, cx| {
                                view.loading = !view.loading;
                                cx.notify();
                            })),
                    )
                    .child(
                        Checkbox::new("selected-button")
                            .label("Selected")
                            .checked(self.selected)
                            .on_click(cx.listener(|view, _, _, cx| {
                                view.selected = !view.selected;
                                cx.notify();
                            })),
                    )
                    .child(
                        Checkbox::new("compact-button")
                            .label("Compact")
                            .checked(self.compact)
                            .on_click(cx.listener(|view, _, _, cx| {
                                view.compact = !view.compact;
                                cx.notify();
                            })),
                    )
                    .child(
                        Checkbox::new("shadow-button")
                            .label("Shadow")
                            .checked(cx.theme().shadow)
                            .on_click(cx.listener(|_, _, window, cx| {
                                let mut theme = cx.theme().clone();
                                theme.shadow = !theme.shadow;
                                cx.set_global::<Theme>(theme);
                                window.refresh();
                            })),
                    ),
            )
            .child(
                section("Interactive split")
                    .sub_title("Tab moves from the primary action to the menu trigger")
                    .child(
                        DropdownButton::new("btn0")
                            .primary()
                            .button(Button::new("primary-action").label("Run task"))
                            .when(self.compact, |this| this.compact())
                            .loading(self.loading)
                            .disabled(self.disabled)
                            .selected(selected)
                            .dropdown_menu_with_anchor(Corner::BottomRight, move |this, _, _| {
                                this.menu_with_check(
                                    "Disabled",
                                    disabled,
                                    Box::new(ButtonAction::Disabled),
                                )
                                .menu_with_check(
                                    "Loading",
                                    loading,
                                    Box::new(ButtonAction::Loading),
                                )
                                .menu_with_check(
                                    "Selected",
                                    selected,
                                    Box::new(ButtonAction::Selected),
                                )
                                .menu_with_check(
                                    "Compact",
                                    compact,
                                    Box::new(ButtonAction::Compact),
                                )
                            }),
                    ),
            )
            .child(
                section("State review")
                    .sub_title("Stable states for screenshot comparison")
                    .child(
                        h_flex()
                            .w_full()
                            .flex_wrap()
                            .justify_center()
                            .gap_6()
                            .child(
                                v_flex()
                                    .items_start()
                                    .gap_2()
                                    .child(
                                        gpui::div()
                                            .text_xs()
                                            .text_color(cx.theme().muted_foreground)
                                            .child("Default"),
                                    )
                                    .child(
                                        DropdownButton::new("state-default")
                                            .button(
                                                Button::new("state-default-action").label("Run"),
                                            )
                                            .dropdown_menu(|menu, _, _| {
                                                menu.menu(
                                                    "Run with options",
                                                    Box::new(ButtonAction::Selected),
                                                )
                                            }),
                                    ),
                            )
                            .child(
                                v_flex()
                                    .items_start()
                                    .gap_2()
                                    .child(
                                        gpui::div()
                                            .text_xs()
                                            .text_color(cx.theme().muted_foreground)
                                            .child("Selected"),
                                    )
                                    .child(
                                        DropdownButton::new("state-selected")
                                            .button(
                                                Button::new("state-selected-action").label("Run"),
                                            )
                                            .selected(true)
                                            .dropdown_menu(|menu, _, _| {
                                                menu.menu(
                                                    "Run with options",
                                                    Box::new(ButtonAction::Selected),
                                                )
                                            }),
                                    ),
                            )
                            .child(
                                v_flex()
                                    .items_start()
                                    .gap_2()
                                    .child(
                                        gpui::div()
                                            .text_xs()
                                            .text_color(cx.theme().muted_foreground)
                                            .child("Loading"),
                                    )
                                    .child(
                                        DropdownButton::new("state-loading")
                                            .button(
                                                Button::new("state-loading-action")
                                                    .label("Running"),
                                            )
                                            .loading(true)
                                            .dropdown_menu(|menu, _, _| {
                                                menu.menu(
                                                    "Run with options",
                                                    Box::new(ButtonAction::Selected),
                                                )
                                            }),
                                    ),
                            )
                            .child(
                                v_flex()
                                    .items_start()
                                    .gap_2()
                                    .child(
                                        gpui::div()
                                            .text_xs()
                                            .text_color(cx.theme().muted_foreground)
                                            .child("Disabled"),
                                    )
                                    .child(
                                        DropdownButton::new("state-disabled")
                                            .button(
                                                Button::new("state-disabled-action").label("Run"),
                                            )
                                            .disabled(true)
                                            .dropdown_menu(|menu, _, _| {
                                                menu.menu(
                                                    "Run with options",
                                                    Box::new(ButtonAction::Selected),
                                                )
                                            }),
                                    ),
                            ),
                    ),
            )
            .child(
                section("Scale and density")
                    .sub_title(
                        "Caret stays optically quiet while the hit area tracks control height",
                    )
                    .child(
                        v_flex()
                            .items_start()
                            .gap_2()
                            .child(
                                gpui::div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child("Extra small"),
                            )
                            .child(
                                DropdownButton::new("size-xs")
                                    .xsmall()
                                    .button(Button::new("size-xs-action").label("Run"))
                                    .dropdown_menu(|menu, _, _| {
                                        menu.menu(
                                            "Run with options",
                                            Box::new(ButtonAction::Selected),
                                        )
                                    }),
                            ),
                    )
                    .child(
                        v_flex()
                            .items_start()
                            .gap_2()
                            .child(
                                gpui::div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child("Small"),
                            )
                            .child(
                                DropdownButton::new("size-sm")
                                    .small()
                                    .button(Button::new("size-sm-action").label("Run"))
                                    .dropdown_menu(|menu, _, _| {
                                        menu.menu(
                                            "Run with options",
                                            Box::new(ButtonAction::Selected),
                                        )
                                    }),
                            ),
                    )
                    .child(
                        v_flex()
                            .items_start()
                            .gap_2()
                            .child(
                                gpui::div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child("Compact"),
                            )
                            .child(
                                DropdownButton::new("size-compact")
                                    .compact()
                                    .button(Button::new("size-compact-action").label("Run"))
                                    .dropdown_menu(|menu, _, _| {
                                        menu.menu(
                                            "Run with options",
                                            Box::new(ButtonAction::Selected),
                                        )
                                    }),
                            ),
                    ),
            )
            .child(
                section("Borderless Modes")
                    .child(
                        DropdownButton::new("btn-borderless")
                            .ghost()
                            .bordered(false)
                            .button(Button::new("borderless-action").label("Borderless Dropdown"))
                            .when(self.compact, |this| this.compact())
                            .loading(self.loading)
                            .disabled(self.disabled)
                            .selected(selected)
                            .dropdown_menu(move |this, _, _| {
                                this.menu_with_check(
                                    "Disabled",
                                    disabled,
                                    Box::new(ButtonAction::Disabled),
                                )
                                .menu_with_check(
                                    "Loading",
                                    loading,
                                    Box::new(ButtonAction::Loading),
                                )
                                .menu_with_check(
                                    "Selected",
                                    selected,
                                    Box::new(ButtonAction::Selected),
                                )
                                .menu_with_check(
                                    "Compact",
                                    compact,
                                    Box::new(ButtonAction::Compact),
                                )
                            }),
                    )
                    .child(
                        DropdownButton::new("btn-icon")
                            .ghost()
                            .bordered(false)
                            .icon(IconName::Ellipsis)
                            .tooltip("More actions")
                            .when(self.compact, |this| this.compact())
                            .loading(self.loading)
                            .disabled(self.disabled)
                            .selected(selected)
                            .dropdown_menu(move |this, _, _| {
                                this.menu_with_check(
                                    "Disabled",
                                    disabled,
                                    Box::new(ButtonAction::Disabled),
                                )
                                .menu_with_check(
                                    "Loading",
                                    loading,
                                    Box::new(ButtonAction::Loading),
                                )
                                .menu_with_check(
                                    "Selected",
                                    selected,
                                    Box::new(ButtonAction::Selected),
                                )
                                .menu_with_check(
                                    "Compact",
                                    compact,
                                    Box::new(ButtonAction::Compact),
                                )
                            }),
                    ),
            )
            .child(
                section("Small Size").child(
                    DropdownButton::new("btn-sm")
                        .small()
                        .button(Button::new("small-action").label("Small Dropdown"))
                        .when(self.compact, |this| this.compact())
                        .loading(self.loading)
                        .disabled(self.disabled)
                        .selected(selected)
                        .dropdown_menu(move |this, _, _| {
                            this.menu_with_check(
                                "Disabled",
                                disabled,
                                Box::new(ButtonAction::Disabled),
                            )
                            .menu_with_check("Loading", loading, Box::new(ButtonAction::Loading))
                            .menu_with_check("Selected", selected, Box::new(ButtonAction::Selected))
                            .menu_with_check(
                                "Compact",
                                compact,
                                Box::new(ButtonAction::Compact),
                            )
                        }),
                ),
            )
            .child(
                section("Outline").child(
                    DropdownButton::new("btn-outline")
                        .outline()
                        .danger()
                        .button(Button::new("outline-action").label("Outline Dropdown"))
                        .when(self.compact, |this| this.compact())
                        .loading(self.loading)
                        .disabled(self.disabled)
                        .selected(selected)
                        .dropdown_menu(move |this, _, _| {
                            this.menu_with_check(
                                "Disabled",
                                disabled,
                                Box::new(ButtonAction::Disabled),
                            )
                            .menu_with_check("Loading", loading, Box::new(ButtonAction::Loading))
                            .menu_with_check("Selected", selected, Box::new(ButtonAction::Selected))
                            .menu_with_check(
                                "Compact",
                                compact,
                                Box::new(ButtonAction::Compact),
                            )
                        }),
                ),
            )
            .child(
                section("Ghost").child(
                    Button::new("btn-ghost")
                        .ghost()
                        .label("Ghost Dropdown")
                        .dropdown_caret(true)
                        .when(self.compact, |this| this.compact())
                        .loading(self.loading)
                        .disabled(self.disabled)
                        .selected(selected)
                        .dropdown_menu(move |this, _, _| {
                            this.menu_with_check(
                                "Disabled",
                                disabled,
                                Box::new(ButtonAction::Disabled),
                            )
                            .menu_with_check("Loading", loading, Box::new(ButtonAction::Loading))
                            .menu_with_check("Selected", selected, Box::new(ButtonAction::Selected))
                            .menu_with_check(
                                "Compact",
                                compact,
                                Box::new(ButtonAction::Compact),
                            )
                        }),
                ),
            )
    }
}
