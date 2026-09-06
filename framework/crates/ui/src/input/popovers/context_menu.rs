use gpui::{
    App, AppContext as _, Context, Corner, DismissEvent, Entity, IntoElement, MouseDownEvent,
    ParentElement as _, Pixels, Point, Render, Styled, Subscription, Window, anchored, deferred,
    div, prelude::FluentBuilder as _, px,
};
use rust_i18n::t;

use crate::{
    ActiveTheme as _,
    input::{self, InputState, popovers::ContextMenu},
    menu::PopupMenu,
};

/// Context menu for mouse right clicks.
pub(crate) struct MouseContextMenu {
    editor: Entity<InputState>,
    menu: Entity<PopupMenu>,
    mouse_position: Point<Pixels>,
    open: bool,

    _subscriptions: Vec<Subscription>,
}

impl InputState {
    pub(crate) fn handle_right_click_menu(
        &mut self,
        event: &MouseDownEvent,
        offset: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Show Mouse context menu
        if !self.selected_range.contains(offset) {
            self.move_to(offset, None, cx);
        }

        self.context_menu = Some(ContextMenu::MouseContext(self.mouse_context_menu.clone()));

        let is_code_editor = self.mode.is_code_editor();
        if is_code_editor {
            self.handle_hover_definition(offset, window, cx);
        }

        let is_enable = !self.disabled;
        let has_goto_definition = is_enable && self.lsp.definition_provider.is_some();
        let has_code_action = is_enable && !self.lsp.code_action_providers.is_empty();
        let is_copyable = self.is_copyable();
        let has_paste = is_enable && cx.read_from_clipboard().is_some();

        let action_context = self.focus_handle.clone();
        self.mouse_context_menu.update(cx, |this, cx| {
            this.mouse_position = event.position;
            this.menu.update(cx, |menu, cx| {
                let new_menu = PopupMenu::new(cx)
                    .when(is_code_editor, |m| {
                        m.menu_with_enable(
                            t!("Input.Go to Definition"),
                            Box::new(input::GoToDefinition),
                            has_goto_definition,
                        )
                        .menu_with_enable(
                            t!("Input.Show Code Actions"),
                            Box::new(input::ToggleCodeActions),
                            has_code_action,
                        )
                        .separator()
                    })
                    .menu_with_enable(
                        t!("Input.Cut"),
                        Box::new(input::Cut),
                        is_enable && is_copyable,
                    )
                    .menu_with_enable(t!("Input.Copy"), Box::new(input::Copy), is_copyable)
                    .menu_with_enable(t!("Input.Paste"), Box::new(input::Paste), has_paste)
                    .separator()
                    .menu(t!("Input.Select All"), Box::new(input::SelectAll));

                menu.replace_menu_items(new_menu);
                menu.action_context = Some(action_context);
                cx.notify();
            });
            cx.defer_in(window, |this, _, cx| {
                this.open = true;
                cx.notify();
            });
        });
    }
}

impl MouseContextMenu {
    pub(crate) fn new(
        editor: Entity<InputState>,
        window: &mut Window,
        cx: &mut App,
    ) -> Entity<Self> {
        cx.new(|cx| {
            let menu = cx.new(|cx| PopupMenu::new(cx).small());

            let _subscriptions = vec![cx.subscribe_in(&menu, window, {
                move |this: &mut Self, _, _: &DismissEvent, window, cx| {
                    this.close(window, cx);
                }
            })];

            Self {
                editor,
                menu,
                mouse_position: Point::default(),
                open: false,
                _subscriptions,
            }
        })
    }

    #[inline]
    pub(crate) fn is_open(&self) -> bool {
        self.open
    }

    #[inline]
    pub(crate) fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open = false;
        self.editor.update(cx, |this, cx| {
            this.focus(window, cx);
        });
    }
}

impl Render for MouseContextMenu {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.open {
            return div().into_any_element();
        }

        deferred(
            anchored()
                .snap_to_window_with_margin(px(8.))
                .anchor(Corner::TopLeft)
                .position(self.mouse_position)
                .child(
                    div()
                        .font_family(cx.theme().font_family.clone())
                        .cursor_default()
                        .child(self.menu.clone()),
                ),
        )
        .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::menu::PopupMenuItem;
    use gpui::{ClipboardItem, MouseButton, TestAppContext, VisualTestContext};

    #[gpui::test]
    fn test_masked_input_context_menu(cx: &mut TestAppContext) {
        let window = cx.update(|cx| {
            crate::init(cx);
            cx.open_window(Default::default(), |window, cx| {
                cx.new(|cx| InputState::new(window, cx).default_value("secret two"))
            })
            .unwrap()
        });
        let mut cx = VisualTestContext::from_window(window.into(), cx);
        let input = window.root(&mut cx).unwrap();
        let event = MouseDownEvent {
            button: MouseButton::Right,
            position: Point::default(),
            modifiers: Default::default(),
            click_count: 1,
            first_mouse: false,
        };

        for (masked, selected, copyable) in [
            (true, true, false),
            (false, true, true),
            (false, false, false),
        ] {
            input.update_in(&mut cx, |input, window, cx| {
                input.set_masked(masked, window, cx);
                if selected {
                    input.select_all(&input::SelectAll, window, cx);
                } else {
                    input.move_to(1, None, cx);
                }
                cx.write_to_clipboard(ClipboardItem::new_string("sentinel".into()));
                input.handle_right_click_menu(&event, 1, window, cx);

                let menu = input.mouse_context_menu.read(cx).menu.read(cx);
                let enabled = |action: &dyn gpui::Action| {
                    menu.menu_items
                        .iter()
                        .find_map(|item| match item {
                            PopupMenuItem::Item {
                                action: Some(item_action),
                                disabled,
                                ..
                            } if item_action.partial_eq(action) => Some(!disabled),
                            _ => None,
                        })
                        .expect("input context menu must contain the action")
                };
                assert_eq!(enabled(&input::Cut), copyable);
                assert_eq!(enabled(&input::Copy), copyable);
                assert!(enabled(&input::Paste));
                assert!(enabled(&input::SelectAll));
                assert_eq!(menu.action_context, Some(input.focus_handle.clone()));
            });
        }
    }
}
