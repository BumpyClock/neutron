use crate::{
    ActiveTheme, Anchor, ElementExt, Placement, StyledExt, Theme, ThemeModePreference,
    dialog::Dialog,
    focus_trap::FocusTrapManager,
    input::InputState,
    menu::AppMenuBar,
    notification::{Notification, NotificationList},
    sheet::Sheet,
    title_bar::TITLE_BAR_HEIGHT,
    window_border,
};
use gpui::{
    AnyView, App, AppContext, Context, DefiniteLength, Entity, FocusHandle, InteractiveElement,
    IntoElement, KeyBinding, ParentElement as _, Render, StyleRefinement, Styled, Subscription,
    WeakFocusHandle, Window, actions, div, prelude::FluentBuilder as _,
};
use std::{any::TypeId, rc::Rc};

actions!(root, [Tab, TabPrev]);

const CONTEXT: &str = "Root";
pub(crate) fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("tab", Tab, Some(CONTEXT)),
        KeyBinding::new("shift-tab", TabPrev, Some(CONTEXT)),
    ]);
}

/// Root is a view for the App window for as the top level view (Must be the first view in the window).
///
/// It is used to manage the Sheet, Dialog, and Notification.
pub struct Root {
    pub(crate) active_sheet: Option<ActiveSheet>,
    pub(crate) active_dialogs: Vec<ActiveDialog>,
    next_dialog_id: u64,
    pub(super) focused_input: Option<Entity<InputState>>,
    pub notification: Entity<NotificationList>,
    _appearance_subscription: Subscription,
    sheet_size: Option<DefiniteLength>,
    app_menu_bar: Option<Entity<AppMenuBar>>,
    view: AnyView,
    style: StyleRefinement,
}

#[derive(Clone)]
pub(crate) struct ActiveSheet {
    focus_handle: FocusHandle,
    /// The previous focused handle before opening the Sheet.
    previous_focused_handle: Option<WeakFocusHandle>,
    placement: Placement,
    closing: bool,
    builder: Rc<dyn Fn(Sheet, &mut Window, &mut App) -> Sheet + 'static>,
}

#[derive(Clone)]
pub(crate) struct ActiveDialog {
    id: u64,
    closing: bool,
    focus_handle: FocusHandle,
    /// The previous focused handle before opening the Dialog.
    previous_focused_handle: Option<WeakFocusHandle>,
    builder: Rc<dyn Fn(Dialog, &mut Window, &mut App) -> Dialog + 'static>,
}

impl ActiveDialog {
    pub(crate) fn new(
        id: u64,
        focus_handle: FocusHandle,
        previous_focused_handle: Option<WeakFocusHandle>,
        builder: impl Fn(Dialog, &mut Window, &mut App) -> Dialog + 'static,
    ) -> Self {
        Self {
            id,
            closing: false,
            focus_handle,
            previous_focused_handle,
            builder: Rc::new(builder),
        }
    }
}

impl Root {
    /// Create a new Root view.
    pub fn new(view: impl Into<AnyView>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let appearance_subscription = cx.observe_window_appearance(window, |root, window, cx| {
            root.on_window_appearance_changed(window, cx)
        });

        Self {
            active_sheet: None,
            active_dialogs: Vec::new(),
            next_dialog_id: 1,
            focused_input: None,
            notification: cx.new(|cx| NotificationList::new(window, cx)),
            _appearance_subscription: appearance_subscription,
            sheet_size: None,
            app_menu_bar: None,
            view: view.into(),
            style: StyleRefinement::default(),
        }
    }

    fn on_window_appearance_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !matches!(
            Theme::global(cx).mode_preference,
            ThemeModePreference::System
        ) {
            return;
        }

        Theme::sync_system_appearance(Some(window), cx);
        cx.refresh_windows();
    }

    /// Add an in-window application menu bar above the root content.
    pub fn with_app_menu_bar(mut self, app_menu_bar: Entity<AppMenuBar>) -> Self {
        self.app_menu_bar = Some(app_menu_bar);
        self
    }

    pub fn update<F, R>(window: &mut Window, cx: &mut App, f: F) -> R
    where
        F: FnOnce(&mut Self, &mut Window, &mut Context<Self>) -> R,
    {
        let root = window
            .root::<Root>()
            .flatten()
            .expect("BUG: window first layer should be a neutron_components::Root.");

        root.update(cx, |root, cx| f(root, window, cx))
    }

    pub fn read<'a>(window: &'a Window, cx: &'a App) -> &'a Self {
        &window
            .root::<Root>()
            .expect("The window root view should be of type `ui::Root`.")
            .unwrap()
            .read(cx)
    }

    // Render Notification layer.
    pub fn render_notification_layer(
        window: &mut Window,
        cx: &mut App,
    ) -> Option<impl IntoElement + use<>> {
        let root = window.root::<Root>()??;

        let active_sheet_placement = root.read(cx).active_sheet.clone().map(|d| d.placement);

        let sheet_size = root.read(cx).sheet_size;
        let (mt, mr, mb, ml) = match active_sheet_placement {
            Some(Placement::Top) => (sheet_size, None, None, None),
            Some(Placement::Right) => (None, sheet_size, None, None),
            Some(Placement::Bottom) => (None, None, sheet_size, None),
            Some(Placement::Left) => (None, None, None, sheet_size),
            _ => (None, None, None, None),
        };

        let placement = cx.theme().notification.placement;

        Some(
            div()
                .absolute()
                .when(matches!(placement, Anchor::TopRight), |this| {
                    this.top_0().right_0()
                })
                .when(matches!(placement, Anchor::TopLeft), |this| {
                    this.top_0().left_0()
                })
                .when(matches!(placement, Anchor::TopCenter), |this| {
                    this.top_0().mx_auto()
                })
                .when(matches!(placement, Anchor::BottomRight), |this| {
                    this.bottom_0().right_0()
                })
                .when(matches!(placement, Anchor::BottomLeft), |this| {
                    this.bottom_0().left_0()
                })
                .when(matches!(placement, Anchor::BottomCenter), |this| {
                    this.bottom_0().mx_auto()
                })
                .when_some(mt, |this, offset| this.mt(offset))
                .when_some(mr, |this, offset| this.mr(offset))
                .when_some(mb, |this, offset| this.mb(offset))
                .when_some(ml, |this, offset| this.ml(offset))
                .child(root.read(cx).notification.clone()),
        )
    }

    /// Render the Sheet layer.
    pub fn render_sheet_layer(
        window: &mut Window,
        cx: &mut App,
    ) -> Option<impl IntoElement + use<>> {
        let root = window.root::<Root>()??;

        if let Some(active_sheet) = root.read(cx).active_sheet.clone() {
            let mut sheet = Sheet::new(window, cx);
            sheet = (active_sheet.builder)(sheet, window, cx);
            sheet.focus_handle = active_sheet.focus_handle.clone();
            sheet.placement = active_sheet.placement;
            sheet.closing = active_sheet.closing;

            let size = sheet.size;

            return Some(
                div()
                    .relative()
                    .child(sheet)
                    .on_prepaint(move |_, _, cx| root.update(cx, |r, _| r.sheet_size = Some(size))),
            );
        }

        None
    }

    /// Render the Dialog layer.
    pub fn render_dialog_layer(
        window: &mut Window,
        cx: &mut App,
    ) -> Option<impl IntoElement + use<>> {
        let root = window.root::<Root>()??;

        let active_dialogs = root.read(cx).active_dialogs.clone();

        if active_dialogs.is_empty() {
            return None;
        }

        let mut show_overlay_ix = None;

        let mut dialogs = active_dialogs
            .iter()
            .enumerate()
            .map(|(i, active_dialog)| {
                let mut dialog = Dialog::new(window, cx);

                dialog = (active_dialog.builder)(dialog, window, cx);

                // Give the dialog the focus handle, because `dialog` is a temporary value, is not possible to
                // keep the focus handle in the dialog.
                //
                // So we keep the focus handle in the `active_dialog`, this is owned by the `Root`.
                dialog.focus_handle = active_dialog.focus_handle.clone();

                dialog.id = active_dialog.id;
                dialog.layer_ix = i;
                dialog.closing = active_dialog.closing;
                // Find the dialog which one needs to show overlay.
                if dialog.has_overlay() {
                    show_overlay_ix = Some(i);
                }

                dialog
            })
            .collect::<Vec<_>>();

        if let Some(ix) = show_overlay_ix {
            if let Some(dialog) = dialogs.get_mut(ix) {
                dialog.overlay_visible = true;
            }
        }

        Some(div().children(dialogs))
    }

    pub fn open_dialog<F>(&mut self, build: F, window: &mut Window, cx: &mut Context<'_, Root>)
    where
        F: Fn(Dialog, &mut Window, &mut App) -> Dialog + 'static,
    {
        let previous_focused_handle = self
            .active_dialogs
            .last()
            .filter(|dialog| dialog.closing)
            .map(|dialog| dialog.previous_focused_handle.clone())
            .unwrap_or_else(|| window.focused(cx).map(|handle| handle.downgrade()));
        let focus_handle = cx.focus_handle();
        focus_handle.focus(window, cx);
        let dialog_id = self.next_dialog_id;
        self.next_dialog_id += 1;

        self.active_dialogs.push(ActiveDialog::new(
            dialog_id,
            focus_handle,
            previous_focused_handle,
            build,
        ));
        cx.notify();
    }

    fn finalize_dialog_close(
        &mut self,
        dialog_id: u64,
        restore_focus: Option<FocusHandle>,
        window: &mut Window,
        cx: &mut Context<'_, Root>,
    ) {
        if let Some(ix) = self.active_dialogs.iter().position(|d| d.id == dialog_id) {
            self.focused_input = None;
            let was_top = ix + 1 == self.active_dialogs.len();
            self.active_dialogs.remove(ix);
            if was_top && let Some(handle) = restore_focus {
                window.focus(&handle, cx);
            }
            cx.notify();
        }
    }

    pub fn close_dialog(&mut self, window: &mut Window, cx: &mut Context<'_, Root>) {
        let Some(active_dialog) = self.active_dialogs.last_mut() else {
            return;
        };

        if active_dialog.closing {
            return;
        }

        let restore_focus = active_dialog
            .previous_focused_handle
            .as_ref()
            .and_then(|h| h.upgrade());
        let dialog_id = active_dialog.id;

        let should_defer_close = {
            let mut dialog = Dialog::new(window, cx);
            dialog = (active_dialog.builder)(dialog, window, cx);
            dialog.should_defer_close(cx)
        };

        if !should_defer_close {
            self.finalize_dialog_close(dialog_id, restore_focus, window, cx);
            return;
        }

        active_dialog.closing = true;
        // The deferral window doubles as the ceiling: teardown happens when it
        // elapses whether or not the content finished animating.
        let duration = crate::animation::exit_duration(&cx.theme().motion);
        window
            .spawn(cx, async move |cx| {
                cx.background_executor().timer(duration).await;
                _ = cx.update(|window, cx| {
                    Root::update(window, cx, |root, window, cx| {
                        root.finalize_dialog_close(dialog_id, restore_focus.clone(), window, cx);
                    });
                });
            })
            .detach();
        cx.notify();
    }

    pub(crate) fn defer_close_dialog(&mut self, window: &mut Window, cx: &mut Context<'_, Root>) {
        self.close_dialog(window, cx);
    }

    pub fn close_all_dialogs(&mut self, window: &mut Window, cx: &mut Context<'_, Root>) {
        self.focused_input = None;
        let previous_focused_handle = self
            .active_dialogs
            .first()
            .and_then(|d| d.previous_focused_handle.clone());
        self.active_dialogs.clear();
        if let Some(handle) = previous_focused_handle.and_then(|h| h.upgrade()) {
            window.focus(&handle, cx);
        }
        cx.notify();
    }

    pub fn open_sheet_at<F>(
        &mut self,
        placement: Placement,
        build: F,
        window: &mut Window,
        cx: &mut Context<'_, Root>,
    ) where
        F: Fn(Sheet, &mut Window, &mut App) -> Sheet + 'static,
    {
        let previous_focused_handle = self
            .active_sheet
            .take()
            .and_then(|sheet| sheet.previous_focused_handle)
            .or_else(|| window.focused(cx).map(|handle| handle.downgrade()));

        let focus_handle = cx.focus_handle();
        focus_handle.focus(window, cx);
        self.active_sheet = Some(ActiveSheet {
            focus_handle,
            previous_focused_handle,
            placement,
            closing: false,
            builder: Rc::new(build),
        });
        cx.notify();
    }

    fn finalize_sheet_close(&mut self, window: &mut Window, cx: &mut Context<'_, Root>) {
        self.focused_input = None;
        if let Some(previous_handle) = self
            .active_sheet
            .as_ref()
            .and_then(|s| s.previous_focused_handle.as_ref())
            .and_then(|h| h.upgrade())
        {
            window.focus(&previous_handle, cx);
        }
        self.active_sheet = None;
        cx.notify();
    }

    pub fn close_sheet(&mut self, window: &mut Window, cx: &mut Context<'_, Root>) {
        let Some(active_sheet) = self.active_sheet.as_mut() else {
            return;
        };
        if active_sheet.closing {
            return;
        }

        if crate::animation::reduced_motion(cx) {
            self.finalize_sheet_close(window, cx);
            return;
        }

        // Keep the sheet mounted for the exit window so it can slide out; the
        // window is also the ceiling — teardown happens when it elapses.
        active_sheet.closing = true;
        let duration = crate::animation::exit_duration(&cx.theme().motion);
        window
            .spawn(cx, async move |cx| {
                cx.background_executor().timer(duration).await;
                _ = cx.update(|window, cx| {
                    Root::update(window, cx, |root, window, cx| {
                        if root.active_sheet.as_ref().is_some_and(|s| s.closing) {
                            root.finalize_sheet_close(window, cx);
                        }
                    });
                });
            })
            .detach();
        cx.notify();
    }

    pub fn push_notification(
        &mut self,
        note: impl Into<Notification>,
        window: &mut Window,
        cx: &mut Context<'_, Root>,
    ) {
        self.notification
            .update(cx, |view, cx| view.push(note, window, cx));
        cx.notify();
    }

    pub fn remove_notification<T: Sized + 'static>(
        &mut self,
        window: &mut Window,
        cx: &mut Context<'_, Root>,
    ) {
        self.notification.update(cx, |view, cx| {
            let id = TypeId::of::<T>();
            view.close(id, window, cx);
        });
        cx.notify();
    }

    pub fn clear_notifications(&mut self, window: &mut Window, cx: &mut Context<'_, Root>) {
        self.notification
            .update(cx, |view, cx| view.clear(window, cx));
        cx.notify();
    }

    /// Return the root view of the Root.
    pub fn view(&self) -> &AnyView {
        &self.view
    }

    fn on_action_tab(&mut self, _: &Tab, window: &mut Window, cx: &mut Context<Self>) {
        // Check if we're inside a focus trap
        if let Some(container_focus_handle) = FocusTrapManager::find_active_trap(window, cx) {
            // We're in a focus trap - try to focus next, then check if we're still inside
            let before_focus = window.focused(cx);

            // Try normal focus navigation
            window.focus_next(cx);

            // Check if we're still in the trap
            if !container_focus_handle.contains_focused(window, cx) {
                // We jumped out of the trap - need to cycle back to the beginning
                // Find the first focusable element in the trap by continuing to focus_next
                let mut attempts = 0;
                const MAX_ATTEMPTS: usize = 100; // Prevent infinite loop

                while !container_focus_handle.contains_focused(window, cx)
                    && attempts < MAX_ATTEMPTS
                {
                    window.focus_next(cx);
                    attempts += 1;

                    // If we cycled back to where we started, restore original focus
                    if window.focused(cx) == before_focus {
                        break;
                    }
                }
            }
            return;
        }

        // Normal tab navigation
        window.focus_next(cx);
    }

    fn on_action_tab_prev(&mut self, _: &TabPrev, window: &mut Window, cx: &mut Context<Self>) {
        // Check if we're inside a focus trap
        if let Some(container_focus_handle) = FocusTrapManager::find_active_trap(window, cx) {
            // We're in a focus trap - try to focus previous, then check if we're still inside
            let before_focus = window.focused(cx);

            // Try normal focus navigation
            window.focus_prev(cx);

            // Check if we're still in the trap
            if !container_focus_handle.contains_focused(window, cx) {
                // We jumped out of the trap - need to cycle back to the end
                // Find the last focusable element in the trap by continuing to focus_prev
                let mut attempts = 0;
                const MAX_ATTEMPTS: usize = 100; // Prevent infinite loop

                while !container_focus_handle.contains_focused(window, cx)
                    && attempts < MAX_ATTEMPTS
                {
                    window.focus_prev(cx);
                    attempts += 1;

                    // If we cycled back to where we started, restore original focus
                    if window.focused(cx) == before_focus {
                        break;
                    }
                }
            }
            return;
        }

        // Normal tab navigation
        window.focus_prev(cx);
    }
}

impl Styled for Root {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl Render for Root {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        window.set_rem_size(cx.theme().font_size);

        let view = self.app_menu_bar.clone().map_or_else(
            || self.view.clone().into_any_element(),
            |app_menu_bar| {
                div()
                    .flex()
                    .flex_col()
                    .size_full()
                    .child(
                        div()
                            .debug_selector(|| "root-app-menu-bar".to_string())
                            .w_full()
                            .h(TITLE_BAR_HEIGHT)
                            .flex_none()
                            .overflow_hidden()
                            .child(app_menu_bar),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_h_0()
                            .overflow_hidden()
                            .child(self.view.clone()),
                    )
                    .into_any_element()
            },
        );

        window_border().child(
            div()
                .id("root")
                .key_context(CONTEXT)
                .on_action(cx.listener(Self::on_action_tab))
                .on_action(cx.listener(Self::on_action_tab_prev))
                .relative()
                .size_full()
                .font_family(cx.theme().font_family.clone())
                .bg(cx.theme().background)
                .text_color(cx.theme().foreground)
                .refine_style(&self.style)
                .child(view),
        )
    }
}

#[cfg(test)]
mod tests {
    use std::{
        cell::{Cell, RefCell},
        rc::Rc,
    };

    use gpui::{Empty, Role, TestAppContext, VisualTestContext, div, px, size};

    use crate::ThemeMode;
    use crate::menu::AppMenuBar;

    use super::*;

    struct LayoutProbe;

    impl Render for LayoutProbe {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
                .debug_selector(|| "root-content".to_string())
                .size_full()
        }
    }

    struct RenderProbe(Rc<Cell<usize>>);

    impl Render for RenderProbe {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            self.0.set(self.0.get() + 1);
            div().size_full()
        }
    }

    fn root_window(
        cx: &mut TestAppContext,
    ) -> (gpui::WindowHandle<Root>, VisualTestContext, FocusHandle) {
        let original_focus = Rc::new(RefCell::new(None));
        let original_focus_for_window = original_focus.clone();
        let window = cx.update(|cx| {
            crate::init(cx);
            cx.open_window(Default::default(), |window, cx| {
                original_focus_for_window.replace(Some(cx.focus_handle()));
                let content = cx.new(|_| Empty);
                cx.new(|cx| Root::new(content, window, cx))
            })
            .unwrap()
        });
        let visual_cx = VisualTestContext::from_window(window.into(), cx);
        let original_focus = original_focus.borrow_mut().take().unwrap();
        (window, visual_cx, original_focus)
    }

    #[gpui::test]
    fn replacing_sheet_preserves_original_focus(cx: &mut TestAppContext) {
        let (window, mut cx, original_focus) = root_window(cx);
        let root = window.root(&mut cx).unwrap();

        cx.update(|window, cx| original_focus.focus(window, cx));
        root.update_in(&mut cx, |root, window, cx| {
            root.open_sheet_at(Placement::Right, |sheet, _, _| sheet, window, cx);
            root.open_sheet_at(Placement::Left, |sheet, _, _| sheet, window, cx);

            let previous_focus = root
                .active_sheet
                .as_ref()
                .and_then(|sheet| sheet.previous_focused_handle.as_ref())
                .and_then(WeakFocusHandle::upgrade)
                .expect("replacement sheet should retain original focus");
            assert_eq!(previous_focus, original_focus);

            root.finalize_sheet_close(window, cx);
            assert!(original_focus.is_focused(window));
        });
    }

    #[gpui::test]
    fn configured_root_renders_app_menu_bar(cx: &mut TestAppContext) {
        let window = cx.update(|cx| {
            crate::init(cx);
            cx.open_window(Default::default(), |window, cx| {
                let content = cx.new(|_| Empty);
                let app_menu_bar = AppMenuBar::new(cx);
                cx.new(|cx| Root::new(content, window, cx).with_app_menu_bar(app_menu_bar))
            })
            .unwrap()
        });
        let mut visual_cx = VisualTestContext::from_window(window.into(), cx);

        let a11y_tree = visual_cx.update(|window, cx| {
            window.set_a11y_active_for_test(true);
            window.draw(cx).clear(cx);
            let tree = window
                .last_a11y_tree_for_test()
                .cloned()
                .expect("accessibility tree should be captured after drawing");
            window.set_a11y_active_for_test(false);
            tree
        });

        assert!(
            a11y_tree
                .nodes
                .iter()
                .any(|(_, node)| node.role() == Role::MenuBar)
        );
    }

    #[gpui::test]
    fn root_resyncs_system_theme_only_for_system_preference(cx: &mut TestAppContext) {
        let (window, mut visual_cx, _) = root_window(cx);
        let root = window.root(&mut visual_cx).unwrap();

        root.update_in(&mut visual_cx, |_, _, cx| {
            Theme::change(ThemeMode::Dark, None, cx);
            Theme::global_mut(cx).mode_preference = ThemeModePreference::System;
        });

        visual_cx.simulate_window_appearance(window.into(), gpui::WindowAppearance::Light);
        visual_cx.run_until_parked();
        assert_eq!(cx.read(|cx| Theme::global(cx).mode), ThemeMode::Light);

        root.update_in(&mut visual_cx, |_, _, cx| {
            Theme::change(ThemeMode::Light, None, cx);
            Theme::global_mut(cx).mode_preference = ThemeModePreference::Light;
        });

        visual_cx.simulate_window_appearance(window.into(), gpui::WindowAppearance::Dark);
        visual_cx.run_until_parked();
        assert_eq!(cx.read(|cx| Theme::global(cx).mode), ThemeMode::Light);
    }

    #[gpui::test]
    fn system_theme_change_refreshes_all_root_windows(cx: &mut TestAppContext) {
        let second_renders = Rc::new(Cell::new(0));
        let first = cx.update(|cx| {
            crate::init(cx);
            cx.open_window(Default::default(), |window, cx| {
                let content = cx.new(|_| RenderProbe(Rc::new(Cell::new(0))));
                cx.new(|cx| Root::new(content, window, cx))
            })
            .unwrap()
        });
        let second_renders_for_window = second_renders.clone();
        let second = cx.update(|cx| {
            cx.open_window(Default::default(), |window, cx| {
                let content = cx.new(|_| RenderProbe(second_renders_for_window));
                cx.new(|cx| Root::new(content, window, cx))
            })
            .unwrap()
        });
        let mut visual_cx = VisualTestContext::from_window(first.into(), cx);
        let renders_before_change = second_renders.get();

        let first_root = first.root(&mut visual_cx).unwrap();
        first_root.update_in(&mut visual_cx, |_, _, cx| {
            Theme::change(ThemeMode::Dark, None, cx);
            Theme::global_mut(cx).mode_preference = ThemeModePreference::System;
        });
        visual_cx.simulate_window_appearance(first.into(), gpui::WindowAppearance::Dark);
        visual_cx.run_until_parked();

        assert!(second_renders.get() > renders_before_change);
        let _ = second;
    }

    #[gpui::test]
    fn app_menu_bar_reserves_title_bar_height(cx: &mut TestAppContext) {
        let window = cx.update(|cx| {
            crate::init(cx);
            cx.open_window(Default::default(), |window, cx| {
                let content = cx.new(|_| LayoutProbe);
                let app_menu_bar = AppMenuBar::new(cx);
                cx.new(|cx| Root::new(content, window, cx).with_app_menu_bar(app_menu_bar))
            })
            .unwrap()
        });
        let mut visual_cx = VisualTestContext::from_window(window.into(), cx);
        visual_cx.simulate_resize(size(px(800.), px(600.)));
        visual_cx.update(|window, cx| window.draw(cx).clear(cx));

        let menu_bar_bounds = visual_cx
            .debug_bounds("root-app-menu-bar")
            .expect("app menu bar bounds should be captured");
        let content_bounds = visual_cx
            .debug_bounds("root-content")
            .expect("root content bounds should be captured");
        assert_eq!(menu_bar_bounds.origin.y, px(0.));
        assert_eq!(menu_bar_bounds.size.width, px(800.));
        assert_eq!(menu_bar_bounds.size.height, TITLE_BAR_HEIGHT);
        assert_eq!(content_bounds.origin.y, TITLE_BAR_HEIGHT);
        assert_eq!(content_bounds.size.width, px(800.));
        assert_eq!(content_bounds.size.height, px(600.) - TITLE_BAR_HEIGHT);
    }

    #[gpui::test]
    fn opening_dialog_during_exit_preserves_original_focus(cx: &mut TestAppContext) {
        let (window, mut cx, original_focus) = root_window(cx);
        let root = window.root(&mut cx).unwrap();

        cx.update(|window, cx| original_focus.focus(window, cx));
        root.update_in(&mut cx, |root, window, cx| {
            root.open_dialog(|dialog, _, _| dialog, window, cx);
            let first_id = root.active_dialogs[0].id;
            root.active_dialogs[0].closing = true;

            root.open_dialog(|dialog, _, _| dialog, window, cx);
            let second = root.active_dialogs.last().unwrap().clone();
            let previous_focus = second
                .previous_focused_handle
                .as_ref()
                .and_then(WeakFocusHandle::upgrade)
                .expect("replacement dialog should retain original focus");
            assert_eq!(previous_focus, original_focus);
            assert!(second.focus_handle.is_focused(window));

            root.finalize_dialog_close(first_id, Some(original_focus.clone()), window, cx);
            assert!(second.focus_handle.is_focused(window));

            root.finalize_dialog_close(second.id, Some(original_focus.clone()), window, cx);
            assert!(original_focus.is_focused(window));
        });
    }
}
