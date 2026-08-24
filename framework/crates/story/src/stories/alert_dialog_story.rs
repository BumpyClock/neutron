use gpui::{
    App, AppContext as _, Context, Entity, FocusHandle, Focusable, IntoElement, ParentElement as _,
    Render, Styled as _, Window, px,
};
use gpui_component::{
    ActiveTheme, Icon, IconName, WindowExt as _,
    button::{Button, ButtonVariant, ButtonVariants as _},
    dialog::DialogButtonProps,
    v_flex,
};

use crate::section;

pub struct AlertDialogStory {
    focus_handle: FocusHandle,
}

impl AlertDialogStory {
    fn new(_: &mut Window, cx: &mut Context<Self>) -> Self {
        Self {
            focus_handle: cx.focus_handle(),
        }
    }

    pub fn view(window: &mut Window, cx: &mut App) -> Entity<Self> {
        cx.new(|cx| Self::new(window, cx))
    }
}

impl super::Story for AlertDialogStory {
    fn title() -> &'static str {
        "Alert Dialog"
    }

    fn description() -> &'static str {
        "Requires a focused user response."
    }

    fn new_view(window: &mut Window, cx: &mut App) -> Entity<impl Render> {
        Self::view(window, cx)
    }
}

impl Focusable for AlertDialogStory {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for AlertDialogStory {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .gap_4()
            .child(
                section("Default").child(
                    Button::new("open-alert-dialog")
                        .label("Open alert")
                        .primary()
                        .on_click(cx.listener(|_, _, window, cx| {
                            window.open_alert_dialog(cx, |alert, _, _| {
                                alert
                                    .title("Changes saved")
                                    .description(
                                        "Your settings are available in every open window.",
                                    )
                                    .on_ok(|_, window, cx| {
                                        window.push_notification("Alert confirmed", cx);
                                        true
                                    })
                            });
                        })),
                ),
            )
            .child(
                section("Confirmation").child(
                    Button::new("open-confirm-alert-dialog")
                        .label("Delete project")
                        .danger()
                        .on_click(cx.listener(|_, _, window, cx| {
                            window.open_alert_dialog(cx, |alert, _, cx| {
                                alert
                                    .confirm()
                                    .icon(
                                        Icon::new(IconName::TriangleAlert)
                                            .size_8()
                                            .text_color(cx.theme().danger),
                                    )
                                    .title("Delete this project?")
                                    .description("This action cannot be undone.")
                                    .button_props(
                                        DialogButtonProps::default()
                                            .ok_text("Delete")
                                            .ok_variant(ButtonVariant::Danger)
                                            .cancel_text("Keep project"),
                                    )
                                    .on_ok(|_, window, cx| {
                                        window.push_notification("Project deleted", cx);
                                        true
                                    })
                            });
                        })),
                ),
            )
            .child(
                section("Custom width").child(
                    Button::new("open-wide-alert-dialog")
                        .label("Open wide alert")
                        .outline()
                        .on_click(cx.listener(|_, _, window, cx| {
                            window.open_alert_dialog(cx, |alert, _, _| {
                                alert
                                    .width(px(560.))
                                    .title("Network access needed")
                                    .description(
                                        "Allow this application to contact the configured service?",
                                    )
                                    .show_cancel(true)
                            });
                        })),
                ),
            )
    }
}
