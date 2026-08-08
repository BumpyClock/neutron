use crate::actions::{Cancel, Confirm, SelectDown, SelectUp};
use crate::actions::{SelectLeft, SelectRight};
use crate::animation::{FlyoutSlide, PresenceOptions, flyout_motion, flyout_presence};
use crate::global_state::GlobalState;
use crate::menu::menu_item::MenuItemElement;
use crate::scroll::ScrollableElement;
use crate::{
    ActiveTheme, ElementExt, FlyoutTokens, Icon, IconName, Sizable as _, SurfaceContext,
    flyout_disabled_foreground, flyout_primary_foreground, flyout_secondary_foreground,
    flyout_selected_secondary_foreground,
};
use crate::{Side, Size, SurfacePreset, h_flex, kbd::Kbd, v_flex};
use gpui::{
    Action, AnyElement, App, AppContext, Bounds, Context, Corner, DismissEvent, Edges, Entity,
    EventEmitter, FocusHandle, Focusable, InteractiveElement, IntoElement, KeyBinding,
    ParentElement, Pixels, Render, Role, ScrollHandle, SharedString, StatefulInteractiveElement,
    Styled, Toggled, WeakEntity, Window, anchored, div, prelude::FluentBuilder, px,
};
use gpui::{ClickEvent, MouseDownEvent, OwnedMenuItem, Point, Subscription};
use std::rc::Rc;

const CONTEXT: &str = "PopupMenu";

pub fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("enter", Confirm { secondary: false }, Some(CONTEXT)),
        KeyBinding::new("escape", Cancel, Some(CONTEXT)),
        KeyBinding::new("up", SelectUp, Some(CONTEXT)),
        KeyBinding::new("down", SelectDown, Some(CONTEXT)),
        KeyBinding::new("left", SelectLeft, Some(CONTEXT)),
        KeyBinding::new("right", SelectRight, Some(CONTEXT)),
    ]);
}

/// An menu item in a popup menu.
pub enum PopupMenuItem {
    /// A menu separator item.
    Separator,
    /// A non-interactive label item.
    Label(SharedString),
    /// A standard menu item.
    Item {
        icon: Option<Icon>,
        label: SharedString,
        disabled: bool,
        checked: bool,
        is_link: bool,
        action: Option<Box<dyn Action>>,
        // For link item
        handler: Option<Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>>,
    },
    /// A menu item with custom element render.
    ElementItem {
        icon: Option<Icon>,
        disabled: bool,
        checked: bool,
        action: Option<Box<dyn Action>>,
        render: Box<dyn Fn(&mut Window, &mut App) -> AnyElement + 'static>,
        handler: Option<Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>>,
    },
    /// A submenu item that opens another popup menu.
    ///
    /// NOTE: This is only supported when the parent menu is not `scrollable`.
    Submenu {
        icon: Option<Icon>,
        label: SharedString,
        disabled: bool,
        menu: Entity<PopupMenu>,
    },
}

impl FluentBuilder for PopupMenuItem {}
impl PopupMenuItem {
    /// Create a new menu item with the given label.
    #[inline]
    pub fn new(label: impl Into<SharedString>) -> Self {
        PopupMenuItem::Item {
            icon: None,
            label: label.into(),
            disabled: false,
            checked: false,
            action: None,
            is_link: false,
            handler: None,
        }
    }

    /// Create a new menu item with custom element render.
    #[inline]
    pub fn element<F, E>(builder: F) -> Self
    where
        F: Fn(&mut Window, &mut App) -> E + 'static,
        E: IntoElement,
    {
        PopupMenuItem::ElementItem {
            icon: None,
            disabled: false,
            checked: false,
            action: None,
            render: Box::new(move |window, cx| builder(window, cx).into_any_element()),
            handler: None,
        }
    }

    /// Create a new submenu item that opens another popup menu.
    #[inline]
    pub fn submenu(label: impl Into<SharedString>, menu: Entity<PopupMenu>) -> Self {
        PopupMenuItem::Submenu {
            icon: None,
            label: label.into(),
            disabled: false,
            menu,
        }
    }

    /// Create a separator menu item.
    #[inline]
    pub fn separator() -> Self {
        PopupMenuItem::Separator
    }

    /// Creates a label menu item.
    #[inline]
    pub fn label(label: impl Into<SharedString>) -> Self {
        PopupMenuItem::Label(label.into())
    }

    /// Set the icon for the menu item.
    ///
    /// Only works for [`PopupMenuItem::Item`], [`PopupMenuItem::ElementItem`] and [`PopupMenuItem::Submenu`].
    pub fn icon(mut self, icon: impl Into<Icon>) -> Self {
        match &mut self {
            PopupMenuItem::Item { icon: i, .. } => {
                *i = Some(icon.into());
            }
            PopupMenuItem::ElementItem { icon: i, .. } => {
                *i = Some(icon.into());
            }
            PopupMenuItem::Submenu { icon: i, .. } => {
                *i = Some(icon.into());
            }
            _ => {}
        }
        self
    }

    /// Set the action for the menu item.
    ///
    /// Only works for [`PopupMenuItem::Item`] and [`PopupMenuItem::ElementItem`].
    pub fn action(mut self, action: Box<dyn Action>) -> Self {
        match &mut self {
            PopupMenuItem::Item { action: a, .. } => {
                *a = Some(action);
            }
            PopupMenuItem::ElementItem { action: a, .. } => {
                *a = Some(action);
            }
            _ => {}
        }
        self
    }

    /// Set the disabled state for the menu item.
    ///
    /// Only works for [`PopupMenuItem::Item`], [`PopupMenuItem::ElementItem`] and [`PopupMenuItem::Submenu`].
    pub fn disabled(mut self, disabled: bool) -> Self {
        match &mut self {
            PopupMenuItem::Item { disabled: d, .. } => {
                *d = disabled;
            }
            PopupMenuItem::ElementItem { disabled: d, .. } => {
                *d = disabled;
            }
            PopupMenuItem::Submenu { disabled: d, .. } => {
                *d = disabled;
            }
            _ => {}
        }
        self
    }

    /// Set checked state for the menu item.
    ///
    /// NOTE: If `check_side` is [`Side::Left`], the icon will replace with a check icon.
    /// Use [`PopupMenu::menu_with_check`] for an unchecked checkable item; a standalone
    /// `PopupMenuItem::checked(false)` is indistinguishable from an ordinary item.
    pub fn checked(mut self, checked: bool) -> Self {
        match &mut self {
            PopupMenuItem::Item { checked: value, .. }
            | PopupMenuItem::ElementItem { checked: value, .. } => {
                *value = checked;
            }
            _ => {}
        }
        self
    }

    /// Add a click handler for the menu item.
    ///
    /// Only works for [`PopupMenuItem::Item`] and [`PopupMenuItem::ElementItem`].
    pub fn on_click<F>(mut self, handler: F) -> Self
    where
        F: Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    {
        match &mut self {
            PopupMenuItem::Item { handler: h, .. } => {
                *h = Some(Rc::new(handler));
            }
            PopupMenuItem::ElementItem { handler: h, .. } => {
                *h = Some(Rc::new(handler));
            }
            _ => {}
        }
        self
    }

    /// Create a link menu item.
    #[inline]
    pub fn link(label: impl Into<SharedString>, href: impl Into<String>) -> Self {
        let href = href.into();
        PopupMenuItem::Item {
            icon: None,
            label: label.into(),
            disabled: false,
            checked: false,
            action: None,
            is_link: true,
            handler: Some(Rc::new(move |_, _, cx| cx.open_url(&href))),
        }
    }

    #[inline]
    fn is_clickable(&self) -> bool {
        !matches!(self, PopupMenuItem::Separator)
            && matches!(
                self,
                PopupMenuItem::Item {
                    disabled: false,
                    ..
                } | PopupMenuItem::ElementItem {
                    disabled: false,
                    ..
                } | PopupMenuItem::Submenu {
                    disabled: false,
                    ..
                }
            )
    }

    #[inline]
    fn is_separator(&self) -> bool {
        matches!(self, PopupMenuItem::Separator)
    }

    #[inline]
    fn is_disabled(&self) -> bool {
        matches!(
            self,
            PopupMenuItem::Item { disabled: true, .. }
                | PopupMenuItem::ElementItem { disabled: true, .. }
                | PopupMenuItem::Submenu { disabled: true, .. }
        )
    }

    fn has_left_icon(&self, check_side: Side) -> bool {
        match self {
            PopupMenuItem::Item { icon, checked, .. } => {
                icon.is_some() || (check_side.is_left() && *checked)
            }
            PopupMenuItem::ElementItem { icon, checked, .. } => {
                icon.is_some() || (check_side.is_left() && *checked)
            }
            PopupMenuItem::Submenu { icon, .. } => icon.is_some(),
            _ => false,
        }
    }

    #[inline]
    fn is_checked(&self) -> bool {
        match self {
            PopupMenuItem::Item { checked, .. } => *checked,
            PopupMenuItem::ElementItem { checked, .. } => *checked,
            _ => false,
        }
    }

    fn a11y(&self, checkable: bool) -> (Option<Role>, Option<SharedString>, Option<Toggled>) {
        let role = if checkable {
            Role::MenuItemCheckBox
        } else {
            Role::MenuItem
        };
        let toggled = checkable.then_some(if self.is_checked() {
            Toggled::True
        } else {
            Toggled::False
        });

        match self {
            PopupMenuItem::Separator => (None, None, None),
            PopupMenuItem::Label(label) => (Some(Role::Label), Some(label.clone()), None),
            PopupMenuItem::Item { label, .. } => (Some(role), Some(label.clone()), toggled),
            PopupMenuItem::ElementItem { .. } => (Some(role), None, toggled),
            PopupMenuItem::Submenu { label, .. } => {
                (Some(Role::MenuItem), Some(label.clone()), None)
            }
        }
    }
}

pub struct PopupMenu {
    pub(crate) focus_handle: FocusHandle,
    pub(crate) menu_items: Vec<PopupMenuItem>,
    item_checkability: Vec<bool>,
    /// The focus handle of Entity to handle actions.
    pub(crate) action_context: Option<FocusHandle>,
    selected_index: Option<usize>,
    min_width: Option<Pixels>,
    max_width: Option<Pixels>,
    max_height: Option<Pixels>,
    bounds: Bounds<Pixels>,
    size: Size,
    check_side: Side,

    /// The parent menu of this menu, if this is a submenu
    parent_menu: Option<WeakEntity<Self>>,
    scrollable: bool,
    external_link_icon: bool,
    scroll_handle: ScrollHandle,
    // This will update on render
    submenu_anchor: (Corner, Pixels),

    _subscriptions: Vec<Subscription>,
}

impl PopupMenu {
    pub(crate) fn new(cx: &mut App) -> Self {
        Self {
            focus_handle: cx.focus_handle(),
            action_context: None,
            parent_menu: None,
            menu_items: Vec::new(),
            item_checkability: Vec::new(),
            selected_index: None,
            min_width: None,
            max_width: None,
            max_height: None,
            check_side: Side::Left,
            bounds: Bounds::default(),
            scrollable: false,
            scroll_handle: ScrollHandle::default(),
            external_link_icon: true,
            size: Size::default(),
            submenu_anchor: (Corner::TopLeft, Pixels::ZERO),
            _subscriptions: vec![],
        }
    }

    pub fn build(
        window: &mut Window,
        cx: &mut App,
        f: impl FnOnce(Self, &mut Window, &mut Context<PopupMenu>) -> Self,
    ) -> Entity<Self> {
        cx.new(|cx| f(Self::new(cx), window, cx))
    }

    fn push_item(&mut self, item: PopupMenuItem, checkable: bool) {
        self.menu_items.push(item);
        self.item_checkability.push(checkable);
    }

    fn item_a11y(&self, ix: usize) -> (Option<Role>, Option<SharedString>, Option<Toggled>) {
        let item = &self.menu_items[ix];
        let checkable = self
            .item_checkability
            .get(ix)
            .copied()
            .unwrap_or_else(|| item.is_checked());
        item.a11y(checkable)
    }

    pub(crate) fn replace_menu_items(&mut self, menu: Self) {
        self.menu_items = menu.menu_items;
        self.item_checkability = menu.item_checkability;
    }

    /// Set the focus handle of Entity to handle actions.
    ///
    /// When the menu is dismissed or before an action is triggered, the focus will be returned to this handle.
    ///
    /// Then the action will be dispatched to this handle.
    pub fn action_context(mut self, handle: FocusHandle) -> Self {
        self.action_context = Some(handle);
        self
    }

    /// Set min width of the popup menu, default is 120px
    pub fn min_w(mut self, width: impl Into<Pixels>) -> Self {
        self.min_width = Some(width.into());
        self
    }

    /// Set max width of the popup menu, default is 500px
    pub fn max_w(mut self, width: impl Into<Pixels>) -> Self {
        self.max_width = Some(width.into());
        self
    }

    /// Set max height of the popup menu, default is half of the window height
    pub fn max_h(mut self, height: impl Into<Pixels>) -> Self {
        self.max_height = Some(height.into());
        self
    }

    /// Set the menu to be scrollable to show vertical scrollbar.
    ///
    /// NOTE: If this is true, the sub-menus will cannot be support.
    pub fn scrollable(mut self, scrollable: bool) -> Self {
        self.scrollable = scrollable;
        self
    }

    /// Set the side to show check icon, default is `Side::Left`.
    pub fn check_side(mut self, side: Side) -> Self {
        self.check_side = side;
        self
    }

    /// Set the menu to show external link icon, default is true.
    pub fn external_link_icon(mut self, visible: bool) -> Self {
        self.external_link_icon = visible;
        self
    }

    /// Add Menu Item
    pub fn menu(self, label: impl Into<SharedString>, action: Box<dyn Action>) -> Self {
        self.menu_with_disabled(label, action, false)
    }

    /// Add Menu Item with enable state
    pub fn menu_with_enable(
        mut self,
        label: impl Into<SharedString>,
        action: Box<dyn Action>,
        enable: bool,
    ) -> Self {
        self.add_menu_item(label, None, action, !enable, false, false);
        self
    }

    /// Add Menu Item with disabled state
    pub fn menu_with_disabled(
        mut self,
        label: impl Into<SharedString>,
        action: Box<dyn Action>,
        disabled: bool,
    ) -> Self {
        self.add_menu_item(label, None, action, disabled, false, false);
        self
    }

    /// Add label
    pub fn label(mut self, label: impl Into<SharedString>) -> Self {
        self.push_item(PopupMenuItem::label(label.into()), false);
        self
    }

    /// Add Menu to open link
    pub fn link(self, label: impl Into<SharedString>, href: impl Into<String>) -> Self {
        self.link_with_disabled(label, href, false)
    }

    /// Add Menu to open link with disabled state
    pub fn link_with_disabled(
        mut self,
        label: impl Into<SharedString>,
        href: impl Into<String>,
        disabled: bool,
    ) -> Self {
        let href = href.into();
        self.push_item(PopupMenuItem::link(label, href).disabled(disabled), false);
        self
    }

    /// Add Menu to open link
    pub fn link_with_icon(
        self,
        label: impl Into<SharedString>,
        icon: impl Into<Icon>,
        href: impl Into<String>,
    ) -> Self {
        self.link_with_icon_and_disabled(label, icon, href, false)
    }

    /// Add Menu to open link with icon and disabled state
    fn link_with_icon_and_disabled(
        mut self,
        label: impl Into<SharedString>,
        icon: impl Into<Icon>,
        href: impl Into<String>,
        disabled: bool,
    ) -> Self {
        let href = href.into();
        self.push_item(
            PopupMenuItem::link(label, href)
                .icon(icon)
                .disabled(disabled),
            false,
        );
        self
    }

    /// Add Menu Item with Icon.
    pub fn menu_with_icon(
        self,
        label: impl Into<SharedString>,
        icon: impl Into<Icon>,
        action: Box<dyn Action>,
    ) -> Self {
        self.menu_with_icon_and_disabled(label, icon, action, false)
    }

    /// Add Menu Item with Icon and disabled state
    pub fn menu_with_icon_and_disabled(
        mut self,
        label: impl Into<SharedString>,
        icon: impl Into<Icon>,
        action: Box<dyn Action>,
        disabled: bool,
    ) -> Self {
        self.add_menu_item(label, Some(icon.into()), action, disabled, false, false);
        self
    }

    /// Add Menu Item with check icon
    pub fn menu_with_check(
        self,
        label: impl Into<SharedString>,
        checked: bool,
        action: Box<dyn Action>,
    ) -> Self {
        self.menu_with_check_and_disabled(label, checked, action, false)
    }

    /// Add Menu Item with check icon and disabled state
    pub fn menu_with_check_and_disabled(
        mut self,
        label: impl Into<SharedString>,
        checked: bool,
        action: Box<dyn Action>,
        disabled: bool,
    ) -> Self {
        self.add_menu_item(label, None, action, disabled, checked, true);
        self
    }

    /// Add Menu Item with custom element render.
    pub fn menu_element<F, E>(self, action: Box<dyn Action>, builder: F) -> Self
    where
        F: Fn(&mut Window, &mut App) -> E + 'static,
        E: IntoElement,
    {
        self.menu_element_with_disabled(action, false, builder)
    }

    /// Add Menu Item with custom element render with disabled state.
    pub fn menu_element_with_disabled<F, E>(
        mut self,
        action: Box<dyn Action>,
        disabled: bool,
        builder: F,
    ) -> Self
    where
        F: Fn(&mut Window, &mut App) -> E + 'static,
        E: IntoElement,
    {
        self.push_item(
            PopupMenuItem::element(builder)
                .action(action)
                .disabled(disabled),
            false,
        );
        self
    }

    /// Add Menu Item with custom element render with icon.
    pub fn menu_element_with_icon<F, E>(
        self,
        icon: impl Into<Icon>,
        action: Box<dyn Action>,
        builder: F,
    ) -> Self
    where
        F: Fn(&mut Window, &mut App) -> E + 'static,
        E: IntoElement,
    {
        self.menu_element_with_icon_and_disabled(icon, action, false, builder)
    }

    /// Add Menu Item with custom element render with check state
    pub fn menu_element_with_check<F, E>(
        self,
        checked: bool,
        action: Box<dyn Action>,
        builder: F,
    ) -> Self
    where
        F: Fn(&mut Window, &mut App) -> E + 'static,
        E: IntoElement,
    {
        self.menu_element_with_check_and_disabled(checked, action, false, builder)
    }

    /// Add Menu Item with custom element render with icon and disabled state
    fn menu_element_with_icon_and_disabled<F, E>(
        mut self,
        icon: impl Into<Icon>,
        action: Box<dyn Action>,
        disabled: bool,
        builder: F,
    ) -> Self
    where
        F: Fn(&mut Window, &mut App) -> E + 'static,
        E: IntoElement,
    {
        self.push_item(
            PopupMenuItem::element(builder)
                .action(action)
                .icon(icon)
                .disabled(disabled),
            false,
        );
        self
    }

    /// Add Menu Item with custom element render with check state and disabled state
    fn menu_element_with_check_and_disabled<F, E>(
        mut self,
        checked: bool,
        action: Box<dyn Action>,
        disabled: bool,
        builder: F,
    ) -> Self
    where
        F: Fn(&mut Window, &mut App) -> E + 'static,
        E: IntoElement,
    {
        self.push_item(
            PopupMenuItem::element(builder)
                .action(action)
                .checked(checked)
                .disabled(disabled),
            true,
        );
        self
    }

    /// Add a separator Menu Item
    pub fn separator(mut self) -> Self {
        if self.menu_items.is_empty() {
            return self;
        }

        if let Some(PopupMenuItem::Separator) = self.menu_items.last() {
            return self;
        }

        self.push_item(PopupMenuItem::separator(), false);
        self
    }

    /// Add a Submenu
    pub fn submenu(
        self,
        label: impl Into<SharedString>,
        window: &mut Window,
        cx: &mut Context<Self>,
        f: impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static,
    ) -> Self {
        self.submenu_with_icon(None, label, window, cx, f)
    }

    /// Add a Submenu item with icon
    pub fn submenu_with_icon(
        mut self,
        icon: Option<Icon>,
        label: impl Into<SharedString>,
        window: &mut Window,
        cx: &mut Context<Self>,
        f: impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static,
    ) -> Self {
        let submenu = PopupMenu::build(window, cx, f);
        let parent_menu = cx.entity().downgrade();
        submenu.update(cx, |view, _| {
            view.parent_menu = Some(parent_menu);
        });

        self.push_item(
            PopupMenuItem::submenu(label, submenu).when_some(icon, |this, icon| this.icon(icon)),
            false,
        );
        self
    }

    /// Add menu item.
    pub fn item(mut self, item: impl Into<PopupMenuItem>) -> Self {
        let item: PopupMenuItem = item.into();
        let checkable = item.is_checked();
        self.push_item(item, checkable);
        self
    }

    /// Use small size, the menu item will have smaller height.
    pub(crate) fn small(mut self) -> Self {
        self.size = Size::Small;
        self
    }

    fn add_menu_item(
        &mut self,
        label: impl Into<SharedString>,
        icon: Option<Icon>,
        action: Box<dyn Action>,
        disabled: bool,
        checked: bool,
        checkable: bool,
    ) -> &mut Self {
        let item = PopupMenuItem::new(label)
            .when_some(icon, |item, icon| item.icon(icon))
            .disabled(disabled)
            .action(action);
        self.push_item(
            if checkable {
                item.checked(checked)
            } else {
                item
            },
            checkable,
        );
        self
    }

    pub(super) fn with_menu_items<I>(
        mut self,
        items: impl IntoIterator<Item = I>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self
    where
        I: Into<OwnedMenuItem>,
    {
        for item in items {
            match item.into() {
                OwnedMenuItem::Action {
                    name,
                    action,
                    checked,
                    disabled,
                    ..
                } => {
                    // OwnedMenuItem cannot distinguish an unchecked checkbox from a regular action.
                    self = if checked {
                        self.menu_with_check_and_disabled(
                            name,
                            true,
                            action.boxed_clone(),
                            disabled,
                        )
                    } else {
                        self.menu_with_disabled(name, action.boxed_clone(), disabled)
                    }
                }
                OwnedMenuItem::Separator => {
                    self = self.separator();
                }
                OwnedMenuItem::Submenu(submenu) => {
                    self = self.submenu(submenu.name, window, cx, move |menu, window, cx| {
                        menu.with_menu_items(submenu.items.clone(), window, cx)
                    })
                }
                OwnedMenuItem::SystemMenu(_) => {}
            }
        }

        if self.menu_items.len() > 20 {
            self.scrollable = true;
        }

        self
    }

    pub(crate) fn active_submenu(&self) -> Option<Entity<PopupMenu>> {
        if let Some(ix) = self.selected_index {
            if let Some(item) = self.menu_items.get(ix) {
                return match item {
                    PopupMenuItem::Submenu { menu, .. } => Some(menu.clone()),
                    _ => None,
                };
            }
        }

        None
    }

    pub fn is_empty(&self) -> bool {
        self.menu_items.is_empty()
    }

    fn clickable_menu_items(&self) -> impl Iterator<Item = (usize, &PopupMenuItem)> {
        self.menu_items
            .iter()
            .enumerate()
            .filter(|(_, item)| item.is_clickable())
    }

    fn on_click(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        window.prevent_default();
        self.selected_index = Some(ix);
        self.confirm(&Confirm { secondary: false }, window, cx);
    }

    fn confirm(&mut self, _: &Confirm, window: &mut Window, cx: &mut Context<Self>) {
        match self.selected_index {
            Some(index) => {
                let item = self.menu_items.get(index);
                match item {
                    Some(PopupMenuItem::Item {
                        handler, action, ..
                    }) => {
                        if let Some(handler) = handler {
                            handler(&ClickEvent::default(), window, cx);
                        } else if let Some(action) = action.as_ref() {
                            self.dispatch_confirm_action(action, window, cx);
                        }

                        self.dismiss(&Cancel, window, cx)
                    }
                    Some(PopupMenuItem::ElementItem {
                        handler, action, ..
                    }) => {
                        if let Some(handler) = handler {
                            handler(&ClickEvent::default(), window, cx);
                        } else if let Some(action) = action.as_ref() {
                            self.dispatch_confirm_action(action, window, cx);
                        }
                        self.dismiss(&Cancel, window, cx)
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }

    fn dispatch_confirm_action(
        &self,
        action: &Box<dyn Action>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(context) = self.action_context.as_ref() {
            context.focus(window, cx);
        }

        window.dispatch_action(action.boxed_clone(), cx);
    }

    fn set_selected_index(&mut self, ix: usize, cx: &mut Context<Self>) {
        if self.selected_index != Some(ix) {
            self.selected_index = Some(ix);
            self.scroll_handle.scroll_to_item(ix);
            cx.notify();
        }
    }

    fn select_up(&mut self, _: &SelectUp, _: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        let ix = self.selected_index.unwrap_or(0);

        if let Some((prev_ix, _)) = self
            .menu_items
            .iter()
            .enumerate()
            .rev()
            .find(|(i, item)| *i < ix && item.is_clickable())
        {
            self.set_selected_index(prev_ix, cx);
            return;
        }

        let last_clickable_ix = self.clickable_menu_items().last().map(|(ix, _)| ix);
        self.set_selected_index(last_clickable_ix.unwrap_or(0), cx);
    }

    fn select_down(&mut self, _: &SelectDown, _: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        let Some(ix) = self.selected_index else {
            self.set_selected_index(0, cx);
            return;
        };

        if let Some((next_ix, _)) = self
            .menu_items
            .iter()
            .enumerate()
            .find(|(i, item)| *i > ix && item.is_clickable())
        {
            self.set_selected_index(next_ix, cx);
            return;
        }

        self.set_selected_index(0, cx);
    }

    fn select_left(&mut self, _: &SelectLeft, window: &mut Window, cx: &mut Context<Self>) {
        let handled = if matches!(self.submenu_anchor.0, Corner::TopLeft | Corner::BottomLeft) {
            self._unselect_submenu(window, cx)
        } else {
            self._select_submenu(window, cx)
        };

        if self.parent_side(cx).is_left() {
            self._focus_parent_menu(window, cx);
        }

        if handled {
            return;
        }

        // For parent AppMenuBar to handle.
        if self.parent_menu.is_none() {
            cx.propagate();
        }
    }

    fn select_right(&mut self, _: &SelectRight, window: &mut Window, cx: &mut Context<Self>) {
        let handled = if matches!(self.submenu_anchor.0, Corner::TopLeft | Corner::BottomLeft) {
            self._select_submenu(window, cx)
        } else {
            self._unselect_submenu(window, cx)
        };

        if self.parent_side(cx).is_right() {
            self._focus_parent_menu(window, cx);
        }

        if handled {
            return;
        }

        // For parent AppMenuBar to handle.
        if self.parent_menu.is_none() {
            cx.propagate();
        }
    }

    fn _select_submenu(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if let Some(active_submenu) = self.active_submenu() {
            // Focus the submenu, so that can be handle the action.
            active_submenu.update(cx, |view, cx| {
                view.set_selected_index(0, cx);
                view.focus_handle.focus(window, cx);
            });
            cx.notify();
            return true;
        }

        return false;
    }

    fn _unselect_submenu(&mut self, _: &mut Window, cx: &mut Context<Self>) -> bool {
        if let Some(active_submenu) = self.active_submenu() {
            active_submenu.update(cx, |view, cx| {
                view.selected_index = None;
                cx.notify();
            });
            return true;
        }

        return false;
    }

    fn _focus_parent_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(parent) = self.parent_menu.as_ref() else {
            return;
        };
        let Some(parent) = parent.upgrade() else {
            return;
        };

        self.selected_index = None;
        parent.update(cx, |view, cx| {
            view.focus_handle.focus(window, cx);
            cx.notify();
        });
    }

    fn parent_side(&self, cx: &App) -> Side {
        let Some(parent) = self.parent_menu.as_ref() else {
            return Side::Left;
        };

        let Some(parent) = parent.upgrade() else {
            return Side::Left;
        };

        match parent.read(cx).submenu_anchor.0 {
            Corner::TopLeft | Corner::BottomLeft => Side::Left,
            Corner::TopRight | Corner::BottomRight => Side::Right,
        }
    }

    fn dismiss(&mut self, _: &Cancel, window: &mut Window, cx: &mut Context<Self>) {
        if self.active_submenu().is_some() {
            return;
        }

        cx.emit(DismissEvent);

        // Focus back to the previous focused handle.
        if let Some(action_context) = self.action_context.as_ref() {
            window.focus(action_context, cx);
        }

        let Some(parent_menu) = self.parent_menu.clone() else {
            return;
        };

        // Dismiss parent menu, when this menu is dismissed
        _ = parent_menu.update(cx, |view, cx| {
            view.selected_index = None;
            view.dismiss(&Cancel, window, cx);
        });
    }

    fn handle_dismiss(
        &mut self,
        position: &Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Do not dismiss, if click inside the parent menu
        if let Some(parent) = self.parent_menu.as_ref() {
            if let Some(parent) = parent.upgrade() {
                if parent.read(cx).bounds.contains(position) {
                    return;
                }
            }
        }

        self.dismiss(&Cancel, window, cx);
    }

    fn on_mouse_down_out(
        &mut self,
        e: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.handle_dismiss(&e.position, window, cx);
    }

    fn render_key_binding(
        &self,
        action: Option<&dyn Action>,
        selected: bool,
        disabled: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Kbd> {
        let action = action?;
        let color = if selected && !disabled {
            flyout_selected_secondary_foreground(cx)
        } else if disabled {
            flyout_disabled_foreground(cx)
        } else {
            flyout_secondary_foreground(cx)
        };

        match self
            .action_context
            .as_ref()
            .and_then(|handle| Kbd::binding_for_action_in(action, handle, window))
        {
            Some(kbd) => Some(kbd),
            // Fallback to App level key binding
            None => Kbd::binding_for_action(action, None, window),
        }
        .map(|this| {
            this.p_0()
                .flex_nowrap()
                .border_0()
                .text_color(color)
                .bg(gpui::transparent_white())
        })
    }

    fn render_icon(
        has_icon: bool,
        checked: bool,
        icon: Option<Icon>,
        selected: bool,
        disabled: bool,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<impl IntoElement> {
        if !has_icon {
            return None;
        }

        let icon = if let Some(icon) = icon {
            icon
        } else if checked {
            Icon::new(IconName::Check)
        } else {
            Icon::empty()
        };

        let color = if selected && !disabled {
            cx.theme().primary_foreground
        } else if checked && !disabled {
            cx.theme().primary
        } else if disabled {
            flyout_disabled_foreground(cx)
        } else {
            flyout_secondary_foreground(cx)
        };

        Some(
            h_flex()
                .w_4()
                .justify_center()
                .child(icon.xsmall().text_color(color)),
        )
    }

    #[inline]
    fn max_width(&self, cx: &App) -> Pixels {
        self.max_width
            .unwrap_or_else(|| FlyoutTokens::sized(self.size, cx).max_width)
    }

    /// Calculate the anchor corner and left offset for child submenu
    fn update_submenu_menu_anchor(&mut self, window: &Window, cx: &App) {
        let bounds = self.bounds;
        let max_width = self.max_width(cx);
        let (anchor, left) = if max_width + bounds.origin.x > window.bounds().size.width {
            (Corner::TopRight, -px(16.))
        } else {
            (Corner::TopLeft, bounds.size.width - px(8.))
        };

        let is_bottom_pos = bounds.origin.y + bounds.size.height > window.bounds().size.height;
        self.submenu_anchor = if is_bottom_pos {
            (anchor.other_side_corner_along(gpui::Axis::Vertical), left)
        } else {
            (anchor, left)
        };
    }

    fn render_item(
        &self,
        ix: usize,
        item: &PopupMenuItem,
        options: RenderOptions,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> MenuItemElement {
        let has_left_icon = options.has_left_icon;
        let is_left_check = options.check_side.is_left() && item.is_checked();
        let selected = self.selected_index == Some(ix);
        let disabled = item.is_disabled();
        let right_check_icon = if options.check_side.is_right() && item.is_checked() {
            Some(
                Icon::new(IconName::Check)
                    .xsmall()
                    .text_color(if selected && !disabled {
                        cx.theme().primary_foreground
                    } else if disabled {
                        flyout_disabled_foreground(cx)
                    } else {
                        cx.theme().primary
                    }),
            )
        } else {
            None
        };

        /// Margin kept between a submenu and the window edge when it snaps.
        const EDGE_PADDING: Pixels = px(4.);

        let tokens = options.tokens;
        let is_submenu = matches!(item, PopupMenuItem::Submenu { .. });
        let group_name = format!("{}:item-{}", cx.entity().entity_id(), ix);
        let item_height = tokens.item_height;
        let (a11y_role, a11y_label, a11y_toggled) = self.item_a11y(ix);

        let this = MenuItemElement::new(ix, &group_name)
            .a11y(a11y_role, a11y_label, a11y_toggled)
            .relative()
            .text_size(tokens.label_size)
            .py_0()
            .px(tokens.item_padding_x)
            .rounded(tokens.item_radius)
            .items_center()
            .selected(selected)
            .on_hover(cx.listener(move |this, hovered, _, cx| {
                if *hovered {
                    this.selected_index = Some(ix);
                } else if !is_submenu && this.selected_index == Some(ix) {
                    // TODO: Better handle the submenu unselection when hover out
                    this.selected_index = None;
                }

                cx.notify();
            }));

        match item {
            // The rule spans the full row box so it shares one alignment line with
            // the row hover/selection background instead of introducing a second.
            PopupMenuItem::Separator => this
                .h_auto()
                .p_0()
                .my(tokens.separator_margin)
                .mx_0()
                .border_b(px(1.))
                .border_color(cx.theme().border)
                .disabled(true),
            PopupMenuItem::Label(label) => this
                .h(tokens.section_header_height)
                .text_size(tokens.meta_size)
                .font_weight(tokens.section_weight)
                .disabled(true)
                .cursor_default()
                .child(
                    h_flex()
                        .cursor_default()
                        // `disabled(true)` above only makes the header inert; its text
                        // stays on the secondary step so it separates from genuinely
                        // disabled rows one step further down.
                        .text_color(flyout_secondary_foreground(cx))
                        .items_center()
                        .gap(tokens.icon_gap)
                        .children(Self::render_icon(
                            has_left_icon,
                            false,
                            None,
                            false,
                            true,
                            window,
                            cx,
                        ))
                        .child(div().flex_1().child(label.clone())),
                ),
            PopupMenuItem::ElementItem {
                render,
                icon,
                disabled,
                ..
            } => this
                .when(!disabled, |this| {
                    this.on_click(
                        cx.listener(move |this, _, window, cx| this.on_click(ix, window, cx)),
                    )
                })
                .disabled(*disabled)
                .child(
                    h_flex()
                        .flex_1()
                        .min_h(item_height)
                        .items_center()
                        .gap(tokens.icon_gap)
                        .children(Self::render_icon(
                            has_left_icon,
                            is_left_check,
                            icon.clone(),
                            selected,
                            *disabled,
                            window,
                            cx,
                        ))
                        .child((render)(window, cx))
                        .children(right_check_icon.map(|icon| icon.ml(tokens.accessory_gap))),
                ),
            PopupMenuItem::Item {
                icon,
                label,
                action,
                disabled,
                is_link,
                ..
            } => {
                let show_link_icon = *is_link && self.external_link_icon;
                let key =
                    self.render_key_binding(action.as_deref(), selected, *disabled, window, cx);
                let accessory_color = if selected && !disabled {
                    flyout_selected_secondary_foreground(cx)
                } else if *disabled {
                    flyout_disabled_foreground(cx)
                } else {
                    flyout_secondary_foreground(cx)
                };

                this.when(!disabled, |this| {
                    this.on_click(
                        cx.listener(move |this, _, window, cx| this.on_click(ix, window, cx)),
                    )
                })
                .disabled(*disabled)
                .h(item_height)
                .gap(tokens.icon_gap)
                .children(Self::render_icon(
                    has_left_icon,
                    is_left_check,
                    icon.clone(),
                    selected,
                    *disabled,
                    window,
                    cx,
                ))
                .child(
                    h_flex()
                        .w_full()
                        .gap(tokens.accessory_gap)
                        .items_center()
                        .justify_between()
                        .when(!show_link_icon, |this| this.child(label.clone()))
                        .children(right_check_icon)
                        .when(show_link_icon, |this| {
                            this.child(
                                h_flex()
                                    .w_full()
                                    .justify_between()
                                    .gap(tokens.icon_gap)
                                    .child(label.clone())
                                    .child(
                                        Icon::new(IconName::ExternalLink)
                                            .xsmall()
                                            .text_color(accessory_color),
                                    ),
                            )
                        })
                        .children(key),
                )
            }
            PopupMenuItem::Submenu {
                icon,
                label,
                menu,
                disabled,
            } => {
                let reduced_motion = GlobalState::global(cx).reduced_motion();
                let motion = cx.theme().motion.clone();
                let submenu_presence = flyout_presence(
                    SharedString::from(format!(
                        "popup-menu-submenu-presence-{}-{}",
                        cx.entity().entity_id(),
                        ix
                    )),
                    selected,
                    PresenceOptions::default(),
                    window,
                    cx,
                );

                this.selected(selected)
                    .disabled(*disabled)
                    .items_start()
                    .child(
                        h_flex()
                            .min_h(item_height)
                            .size_full()
                            .items_center()
                            .gap(tokens.icon_gap)
                            .children(Self::render_icon(
                                has_left_icon,
                                false,
                                icon.clone(),
                                selected,
                                *disabled,
                                window,
                                cx,
                            ))
                            .child(
                                h_flex()
                                    .flex_1()
                                    .gap(tokens.accessory_gap)
                                    .items_center()
                                    .justify_between()
                                    .child(label.clone())
                                    .child(Icon::new(IconName::ChevronRight).xsmall().text_color(
                                        if selected && !disabled {
                                            flyout_selected_secondary_foreground(cx)
                                        } else if *disabled {
                                            flyout_disabled_foreground(cx)
                                        } else {
                                            flyout_secondary_foreground(cx)
                                        },
                                    )),
                            ),
                    )
                    .when(submenu_presence.should_render(), |this| {
                        this.child({
                            let (anchor, left) = self.submenu_anchor;
                            let is_bottom_pos =
                                matches!(anchor, Corner::BottomLeft | Corner::BottomRight);
                            let direction =
                                if matches!(anchor, Corner::TopLeft | Corner::BottomLeft) {
                                    -1.0
                                } else {
                                    1.0
                                };
                            let submenu_inner = div()
                                .id("submenu")
                                .occlude()
                                .when(is_bottom_pos, |this| this.bottom_0())
                                .when(!is_bottom_pos, |this| this.top_neg_1())
                                .left(left)
                                .child(menu.clone());

                            // The motion wraps the *content inside* `anchored`:
                            // wrappers outside it have no visual effect, because
                            // anchored repositions its children after layout.
                            let animated_inner = flyout_motion(
                                SharedString::from(format!("popup-submenu-{}", ix)),
                                submenu_presence,
                                FlyoutSlide::horizontal(direction),
                                &motion,
                                reduced_motion,
                                submenu_inner,
                            );

                            gpui::deferred(
                                anchored()
                                    .anchor(anchor)
                                    .child(animated_inner)
                                    .snap_to_window_with_margin(Edges::all(EDGE_PADDING)),
                            )
                            .with_priority(2)
                            .into_any_element()
                        })
                    })
            }
        }
    }
}

impl FluentBuilder for PopupMenu {}
impl EventEmitter<DismissEvent> for PopupMenu {}
impl Focusable for PopupMenu {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

#[derive(Clone, Copy)]
struct RenderOptions {
    has_left_icon: bool,
    check_side: Side,
    tokens: FlyoutTokens,
}

impl Render for PopupMenu {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.update_submenu_menu_anchor(window, cx);

        let tokens = FlyoutTokens::sized(self.size, cx);
        let view = cx.entity();
        let items_count = self.menu_items.len();

        let max_height = self.max_height.unwrap_or_else(|| {
            let window_half_height = window.window_bounds().get_bounds().size.height * 0.5;
            window_half_height.min(px(450.))
        });

        let has_left_icon = self
            .menu_items
            .iter()
            .any(|item| item.has_left_icon(self.check_side));

        let max_width = self.max_width(cx);
        let options = RenderOptions {
            has_left_icon,
            check_side: self.check_side,
            tokens,
        };

        let surface_ctx = SurfaceContext::new(cx);
        let surface_width = if self.bounds.size.width > px(0.) {
            self.bounds.size.width
        } else {
            max_width
        };
        let surface_height = if self.bounds.size.height > px(0.) {
            self.bounds.size.height
        } else {
            max_height
        };

        let content = v_flex()
            .id("popup-menu")
            .role(Role::Menu)
            .key_context(CONTEXT)
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::select_up))
            .on_action(cx.listener(Self::select_down))
            .on_action(cx.listener(Self::select_left))
            .on_action(cx.listener(Self::select_right))
            .on_action(cx.listener(Self::confirm))
            .on_action(cx.listener(Self::dismiss))
            .on_mouse_down_out(cx.listener(Self::on_mouse_down_out))
            .text_color(flyout_primary_foreground(cx))
            .relative()
            .occlude()
            .child(
                v_flex()
                    .id("items")
                    .p(tokens.inset)
                    .gap(tokens.item_gap)
                    .min_w(tokens.min_width)
                    .when_some(self.min_width, |this, min_width| this.min_w(min_width))
                    .max_w(max_width)
                    .when(self.scrollable, |this| {
                        this.max_h(max_height)
                            .overflow_y_scroll()
                            .track_scroll(&self.scroll_handle)
                    })
                    .children(
                        self.menu_items
                            .iter()
                            .enumerate()
                            // Ignore last separator
                            .filter(|(ix, item)| !(*ix + 1 == items_count && item.is_separator()))
                            .map(|(ix, item)| self.render_item(ix, item, options, window, cx)),
                    )
                    .on_prepaint(move |bounds, _, cx| view.update(cx, |r, _| r.bounds = bounds)),
            )
            .when(self.scrollable, |this| {
                // TODO: When the menu is limited by `overflow_y_scroll`, the sub-menu will cannot be displayed.
                this.vertical_scrollbar(&self.scroll_handle)
            });

        SurfacePreset::flyout()
            .with_radius(tokens.radius)
            .wrap_with_bounds(
                content,
                surface_width,
                surface_height,
                window,
                cx,
                surface_ctx,
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    use gpui::{AccessibleAction, Corner, Empty, TestAppContext, VisualTestContext, actions};

    actions!(popup_menu_test, [Toggle]);

    fn new_popup_menu(
        cx: &mut TestAppContext,
    ) -> (
        gpui::WindowHandle<Empty>,
        VisualTestContext,
        Entity<PopupMenu>,
    ) {
        let window = cx.update(|cx| {
            cx.open_window(Default::default(), |_, cx| cx.new(|_| Empty))
                .unwrap()
        });
        let mut visual_cx = VisualTestContext::from_window(window.into(), cx);
        let menu = visual_cx.update(|window, cx| {
            PopupMenu::build(window, cx, |menu, window, cx| {
                menu.item(PopupMenuItem::new("Open"))
                    .submenu("More", window, cx, |submenu, _, _| {
                        submenu
                            .item(PopupMenuItem::new("Child 1"))
                            .item(PopupMenuItem::new("Child 2"))
                    })
                    .item(PopupMenuItem::new("Quit"))
            })
        });
        (window, visual_cx, menu)
    }

    #[gpui::test]
    fn test_popup_menu_select_right_focuses_active_submenu(cx: &mut TestAppContext) {
        let (_window, mut cx, menu) = new_popup_menu(cx);

        menu.update_in(&mut cx, |menu, window, cx| {
            menu.selected_index = Some(1);
            menu.submenu_anchor = (Corner::TopLeft, Pixels::ZERO);
            menu.select_right(&SelectRight, window, cx);

            let submenu = menu.active_submenu().expect("submenu to be active");
            let submenu = submenu.read(cx);
            assert_eq!(submenu.selected_index, Some(0));
            assert!(submenu.focus_handle.is_focused(window));
        });
    }

    #[gpui::test]
    fn test_popup_menu_dismiss_clears_parent_selection(cx: &mut TestAppContext) {
        let (_window, mut cx, menu) = new_popup_menu(cx);

        let submenu = menu.update_in(&mut cx, |menu, _, _| {
            menu.selected_index = Some(1);
            menu.active_submenu().expect("submenu to exist")
        });

        submenu.update_in(&mut cx, |submenu, window, cx| {
            submenu.dismiss(&Cancel, window, cx);
        });

        menu.read_with(&cx, |menu, _| {
            assert_eq!(menu.selected_index, None);
        });
    }

    #[gpui::test]
    fn test_popup_menu_projects_owned_action_disabled_state(cx: &mut TestAppContext) {
        let (_window, mut cx, _) = new_popup_menu(cx);
        let menu = cx.update(|window, cx| {
            PopupMenu::build(window, cx, |menu, window, cx| {
                menu.with_menu_items(
                    [OwnedMenuItem::Action {
                        name: "Toggle".to_string(),
                        action: Box::new(Toggle),
                        os_action: None,
                        checked: true,
                        disabled: true,
                    }],
                    window,
                    cx,
                )
            })
        });

        menu.read_with(&cx, |menu, _| {
            let PopupMenuItem::Item {
                checked, disabled, ..
            } = &menu.menu_items[0]
            else {
                panic!("owned action should project to a popup menu item");
            };
            assert!(*checked);
            assert!(*disabled);
        });
    }

    #[gpui::test]
    fn checkable_popup_menu_builders_preserve_accessibility_state(cx: &mut TestAppContext) {
        let (_window, mut cx, _) = new_popup_menu(cx);
        let menu = cx.update(|window, cx| {
            PopupMenu::build(window, cx, |menu, _, _| {
                menu.menu_with_check_and_disabled("Off", false, Box::new(Toggle), true)
                    .menu_with_check("On", true, Box::new(Toggle))
                    .item(PopupMenuItem::new("Direct").checked(true))
            })
        });

        menu.read_with(&cx, |menu, _| {
            let (role, label, toggled) = menu.item_a11y(0);
            assert_eq!(role, Some(Role::MenuItemCheckBox));
            assert_eq!(label.as_deref(), Some("Off"));
            assert_eq!(toggled, Some(Toggled::False));
            assert!(menu.menu_items[0].is_disabled());

            let (role, label, toggled) = menu.item_a11y(1);
            assert_eq!(role, Some(Role::MenuItemCheckBox));
            assert_eq!(label.as_deref(), Some("On"));
            assert_eq!(toggled, Some(Toggled::True));

            let (role, label, toggled) = menu.item_a11y(2);
            assert_eq!(role, Some(Role::MenuItemCheckBox));
            assert_eq!(label.as_deref(), Some("Direct"));
            assert_eq!(toggled, Some(Toggled::True));
        });
    }

    #[gpui::test]
    fn popup_menu_projects_accessibility_tree(cx: &mut TestAppContext) {
        let menu_slot = Rc::new(RefCell::new(None));
        let menu_for_root = menu_slot.clone();
        let window = cx.update(|cx| {
            crate::init(cx);
            cx.open_window(Default::default(), |window, cx| {
                let menu = PopupMenu::build(window, cx, |menu, _, _| {
                    menu.menu_with_check_and_disabled("Off", false, Box::new(Toggle), true)
                        .menu_with_check("On", true, Box::new(Toggle))
                });
                menu_for_root.replace(Some(menu.clone()));
                cx.new(|cx| crate::Root::new(menu, window, cx))
            })
            .unwrap()
        });
        let menu = menu_slot.borrow_mut().take().unwrap();
        let mut visual_cx = VisualTestContext::from_window(window.into(), cx);

        let update = visual_cx.update(|window, cx| {
            menu.update(cx, |menu, cx| menu.focus_handle.focus(window, cx));
            window.set_a11y_active_for_test(true);
            window.draw(cx).clear();
            let update = window
                .last_a11y_tree_for_test()
                .cloned()
                .expect("accessibility tree should be captured after drawing");
            window.set_a11y_active_for_test(false);
            update
        });

        let (menu_id, menu_node) = update
            .nodes
            .iter()
            .find_map(|(id, node)| (node.role() == Role::Menu).then_some((*id, node)))
            .expect("popup menu node should be present");
        assert!(menu_node.supports_action(AccessibleAction::Focus));
        assert_eq!(update.focus, menu_id);

        let off = update
            .nodes
            .iter()
            .find_map(|(_, node)| {
                (node.role() == Role::MenuItemCheckBox && node.label() == Some("Off"))
                    .then_some(node)
            })
            .expect("unchecked checkable item should be present");
        assert_eq!(off.toggled(), Some(Toggled::False));
        assert!(off.is_disabled());
        assert!(!off.supports_action(AccessibleAction::Click));

        let on = update
            .nodes
            .iter()
            .find_map(|(_, node)| {
                (node.role() == Role::MenuItemCheckBox && node.label() == Some("On"))
                    .then_some(node)
            })
            .expect("checked checkable item should be present");
        assert_eq!(on.toggled(), Some(Toggled::True));
        assert!(on.supports_action(AccessibleAction::Click));
    }
}
