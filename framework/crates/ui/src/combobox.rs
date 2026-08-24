use gpui::{
    AnyElement, App, Bounds, ClickEvent, Context, DismissEvent, Edges, ElementId, Entity,
    EventEmitter, FocusHandle, Focusable, Hsla, InteractiveElement, IntoElement, KeyBinding,
    Length, MouseDownEvent, ParentElement, Pixels, Render, RenderOnce, Role, SharedString,
    StatefulInteractiveElement, StyleRefinement, Styled, Window, anchored, deferred, div,
    prelude::FluentBuilder, px, rems,
};

use rust_i18n::t;

use crate::{
    ActiveTheme, Disableable, ElementExt as _, FlyoutTokens, Icon, IconName, IndexPath, Sizable,
    Size, StyleSized, StyledExt, SurfaceContext, SurfacePreset,
    actions::{Cancel, Confirm, SelectDown, SelectUp},
    animation::{FlyoutSlide, PresenceOptions, PresenceTransition, flyout_motion, flyout_presence},
    flyout_primary_foreground, h_flex,
    input::clear_button,
    list::{List, ListState},
    searchable_list::{
        SearchableListAdapter, SearchableListChange, SearchableListDelegate, SearchableListItem,
        SearchableListState,
    },
    v_flex,
};

const CONTEXT: &str = "Combobox";

#[derive(IntoElement)]
pub struct Caret {
    size: Size,
    color: Option<Hsla>,
}

impl Caret {
    pub fn new(size: Size) -> Self {
        Self { size, color: None }
    }

    pub fn text_color(mut self, color: Hsla) -> Self {
        self.color = Some(color);
        self
    }
}

impl RenderOnce for Caret {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        Icon::new(IconName::ChevronDown)
            .with_size(match self.size {
                Size::XSmall => Size::XSmall,
                Size::Small => Size::Small,
                _ => Size::Medium,
            })
            .when_some(self.color, |this, color| this.text_color(color))
    }
}

fn input_style(disabled: bool, cx: &App) -> (Hsla, Hsla) {
    if disabled {
        (cx.theme().muted, cx.theme().muted_foreground)
    } else {
        (cx.theme().background, cx.theme().foreground)
    }
}

pub(crate) fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("up", SelectUp, Some(CONTEXT)),
        KeyBinding::new("down", SelectDown, Some(CONTEXT)),
        KeyBinding::new("enter", Confirm { secondary: false }, Some(CONTEXT)),
        KeyBinding::new(
            "secondary-enter",
            Confirm { secondary: true },
            Some(CONTEXT),
        ),
        KeyBinding::new("escape", Cancel, Some(CONTEXT)),
    ])
}

// MARK: ComboboxTriggerCtx

/// Context passed to the `render_trigger` closure on [`Combobox`].
pub struct ComboboxTriggerCtx<'a, D: SearchableListDelegate + 'static> {
    pub selection: &'a [(IndexPath, D::Item)],
    pub placeholder: Option<&'a SharedString>,
    pub open: bool,
    pub disabled: bool,
    pub size: Size,
}

// MARK: ComboboxChange

/// Back-compat alias — new code should use [`SearchableListChange`] directly.
pub type ComboboxChange = SearchableListChange;

// MARK: ComboboxOptions

struct ComboboxOptions {
    style: StyleRefinement,
    size: Size,
    cleanable: bool,
    placeholder: Option<SharedString>,
    search_placeholder: Option<SharedString>,
    menu_width: Length,
    menu_max_h: Length,
    disabled: bool,
    appearance: bool,
    trigger_icon: Option<Icon>,
    check_icon: Option<Icon>,
}

impl Default for ComboboxOptions {
    fn default() -> Self {
        Self {
            style: StyleRefinement::default(),
            size: Size::default(),
            cleanable: false,
            placeholder: None,
            search_placeholder: None,
            menu_width: Length::Auto,
            menu_max_h: rems(20.).into(),
            disabled: false,
            appearance: true,
            trigger_icon: None,
            check_icon: None,
        }
    }
}

// MARK: ComboboxState

/// State of the [`Combobox`] component.
pub struct ComboboxState<D: SearchableListDelegate + 'static>
where
    <D::Item as SearchableListItem>::Value: PartialEq + Clone,
{
    pub(crate) state: SearchableListState<D>,

    // Combobox-specific fields
    multiple: bool,
    searchable: bool,
    trigger_icon: Option<Icon>,
    check_icon: Option<Icon>,
    render_trigger:
        Option<Box<dyn Fn(&ComboboxTriggerCtx<D>, &mut Window, &mut App) -> AnyElement + 'static>>,
    footer: Option<Box<dyn Fn(&mut Window, &mut App) -> AnyElement + 'static>>,
}

/// Events emitted by [`ComboboxState`].
pub enum ComboboxEvent<D: SearchableListDelegate + 'static>
where
    <D::Item as SearchableListItem>::Value: PartialEq + Clone,
{
    /// Emitted on every toggle (item added or removed).
    Change(Vec<<D::Item as SearchableListItem>::Value>),
    /// Emitted when the popover closes.
    Confirm(Vec<<D::Item as SearchableListItem>::Value>),
}

impl<D> ComboboxState<D>
where
    D: SearchableListDelegate + 'static,
    <D::Item as SearchableListItem>::Value: PartialEq + Clone,
{
    /// Create a new `Combobox` state.
    pub fn new(
        delegate: D,
        selected_indices: Vec<IndexPath>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let weak = cx.entity().downgrade();
        let weak_confirm = weak.clone();
        let weak_cancel = weak.clone();
        let weak_empty = weak;

        let state = SearchableListState::new_with_empty(
            delegate,
            selected_indices,
            move |selected_index, _secondary, window, cx| {
                cx.defer_in(window, {
                    let weak_confirm = weak_confirm.clone();
                    move |list_state, window, cx| {
                        let Some(index) = selected_index else {
                            return;
                        };

                        let Some(item) = list_state.delegate().delegate.item(index).cloned() else {
                            return;
                        };

                        let ix = index;

                        let Some(weak) = weak_confirm.upgrade() else {
                            return;
                        };

                        let (disabled, multiple, mut selection) = {
                            let s = weak.read(cx);
                            (s.state.disabled, s.multiple, s.state.selection.clone())
                        };
                        if disabled {
                            return;
                        }

                        // on_will_change is called directly — entity-handle access would
                        // re-enter the ListState lock that defer_in holds for this callback.
                        let (changed, should_close) = {
                            let adapter = list_state.delegate_mut();
                            Self::apply_selection(
                                multiple,
                                &mut selection,
                                ix,
                                &item,
                                &mut adapter.delegate,
                            )
                        };

                        let new_selection = weak_confirm.update(cx, |this, cx| {
                            this.state.selection = selection;

                            if changed {
                                cx.emit(ComboboxEvent::Change(this.selected_values()));
                                cx.notify();
                            }

                            this.state.selection.clone()
                        });

                        // Sync snapshot before the deferred committed-close callback.
                        if let Ok(new_selection) = new_selection {
                            list_state
                                .delegate_mut()
                                .update_selection_snapshot(new_selection);

                            if should_close {
                                let weak_confirm = weak_confirm.clone();
                                window.defer(cx, move |window, cx| {
                                    _ = weak_confirm.update(cx, |this, cx| {
                                        this.commit_close(window, cx, true);
                                    });
                                });
                            }
                        }
                    }
                });
            },
            // on_cancel — defer the committed close until the list borrow ends
            move |_final_selected_index, window, cx| {
                cx.defer_in(window, {
                    let weak_cancel = weak_cancel.clone();
                    move |_list_state, window, cx| {
                        window.defer(cx, move |window, cx| {
                            _ = weak_cancel.update(cx, |this, cx| {
                                this.commit_close(window, cx, true);
                            });
                        });
                    }
                });
            },
            // on_render_empty
            Some(Box::new(move |window, cx| {
                weak_empty
                    .upgrade()
                    .and_then(|e| e.read(cx).state.empty.as_ref().and_then(|f| f(window, cx)))
            })),
            Self::on_blur,
            window,
            cx,
        );

        Self {
            state,
            multiple: false,
            searchable: false,
            trigger_icon: None,
            check_icon: None,
            render_trigger: None,
            footer: None,
        }
    }

    /// Enable multi-select mode.
    ///
    /// When `true`, clicking an item toggles it in the selection and the popover stays open.
    /// When `false` (default), clicking an item replaces the selection and closes the popover.
    pub fn multiple(mut self, multiple: bool) -> Self {
        self.multiple = multiple;
        self
    }

    /// Enable or disable the search input at the top of the dropdown.
    pub fn searchable(mut self, searchable: bool) -> Self {
        self.searchable = searchable;
        self
    }

    /// Return the currently selected values.
    pub fn selected_values(&self) -> Vec<<D::Item as SearchableListItem>::Value> {
        self.state.selected_values()
    }

    /// Return the first selected value, or `None` when nothing is selected.
    ///
    /// Convenience for single-select mode (`.multiple(false)`).
    pub fn selected_value(&self) -> Option<<D::Item as SearchableListItem>::Value> {
        self.state.selected_values().into_iter().next()
    }

    /// Return the currently selected `(IndexPath, Item)` pairs.
    pub fn selection(&self) -> &[(IndexPath, D::Item)] {
        self.state.selection()
    }

    /// Replace the entire selection set by item values.
    ///
    /// Values are resolved through the current delegate. Values that cannot be resolved are
    /// ignored. This updates the committed selection and snapshot without emitting a
    /// [`ComboboxEvent`].
    pub fn set_selected_values(
        &mut self,
        values: &[<D::Item as SearchableListItem>::Value],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let selected_indices = {
            let list = self.state.list.read(cx);
            let delegate = &list.delegate().delegate;

            values
                .iter()
                .filter_map(|value| delegate.position(value))
                .collect::<Vec<_>>()
        };

        self.set_selected_indices(selected_indices, window, cx);
    }

    /// Replace the entire selection set.
    pub fn set_selected_indices(
        &mut self,
        indices: impl IntoIterator<Item = IndexPath>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.state.set_selected_indices(indices, cx);
        self.state.sync_snapshot(cx);
        cx.notify();
    }

    /// Add a single index to the selection, if not already present, returning whether it was added.
    pub fn add_selected_index(&mut self, index: IndexPath, cx: &mut Context<Self>) -> bool {
        let added = self.state.add_selected_index(index, cx);

        if added {
            self.state.sync_snapshot(cx);
            cx.notify();
        }

        added
    }

    /// Remove a single index from the selection, returning whether it was removed.
    pub fn remove_selected_index(&mut self, index: IndexPath, cx: &mut Context<Self>) -> bool {
        let removed = self.state.remove_selected_index(index);

        if removed {
            self.state.sync_snapshot(cx);
            cx.notify();
        }

        removed
    }

    /// Clear all selected values.
    pub fn clear_selection(&mut self, cx: &mut Context<Self>) {
        self.state.selection.clear();
        self.state.sync_snapshot(cx);
        cx.emit(ComboboxEvent::Change(self.selected_values()));
        cx.notify();
    }

    /// Replace the underlying delegate (item data source).
    pub fn set_items(&mut self, items: D, window: &mut Window, cx: &mut Context<Self>) {
        let selected_values = self.selected_values();
        let cursor_value = self.state.list.read(cx).selected_index().and_then(|index| {
            self.state
                .list
                .read(cx)
                .delegate()
                .delegate
                .item(index)
                .map(|item| item.value().clone())
        });

        let selection = self.state.list.update(cx, |list, list_cx| {
            list.delegate_mut().delegate = items;

            let delegate = &list.delegate().delegate;
            let selection = selected_values
                .iter()
                .filter_map(|value| {
                    let index = delegate.position(value)?;
                    let item = delegate.item(index)?.clone();
                    Some((index, item))
                })
                .collect::<Vec<_>>();
            let cursor = cursor_value
                .as_ref()
                .and_then(|value| delegate.position(value));

            list.set_selected_index(cursor, window, list_cx);
            list.delegate_mut()
                .update_selection_snapshot(selection.clone());
            list_cx.notify();
            selection
        });

        self.state.selection = selection;
        cx.notify();
    }

    /// Focus the trigger.
    pub fn focus(&self, window: &mut Window, cx: &mut App) {
        self.state.focus_handle.focus(window, cx);
    }

    /// Return the current search query without its filtering normalization.
    pub fn query(&self, cx: &App) -> SharedString {
        self.state.list.read(cx).query_input.read(cx).value()
    }

    /// Set the search query and refresh filtered items.
    pub fn set_query(&self, query: impl Into<SharedString>, window: &mut Window, cx: &mut App) {
        let query = query.into();
        self.state.list.update(cx, |list, cx| {
            list.set_query(query.as_ref(), window, cx);
        });
    }

    fn selection_changes(
        multiple: bool,
        selection: &[(IndexPath, D::Item)],
        ix: IndexPath,
        item: &D::Item,
    ) -> Vec<SearchableListChange> {
        let is_selected = selection
            .iter()
            .any(|(_, selected_item)| selected_item.value() == item.value());

        if multiple {
            if is_selected {
                vec![SearchableListChange::Deselect { index: ix }]
            } else {
                vec![SearchableListChange::Select { index: ix }]
            }
        } else {
            let mut changes: Vec<SearchableListChange> = selection
                .iter()
                .map(|(cur_ix, _)| SearchableListChange::Deselect { index: *cur_ix })
                .collect();
            changes.push(SearchableListChange::Select { index: ix });
            changes
        }
    }

    fn apply_selection(
        multiple: bool,
        selection: &mut Vec<(IndexPath, D::Item)>,
        ix: IndexPath,
        item: &D::Item,
        delegate: &mut D,
    ) -> (bool, bool) {
        let changes = Self::selection_changes(multiple, selection, ix, item);
        let was_selected = selection
            .iter()
            .any(|(_, selected_item)| selected_item.value() == item.value());
        let before: Vec<_> = selection
            .iter()
            .map(|(ix, item)| (*ix, item.value().clone()))
            .collect();

        delegate.on_will_change(selection, &changes);

        let after: Vec<_> = selection
            .iter()
            .map(|(ix, item)| (*ix, item.value().clone()))
            .collect();
        let changed = before != after;
        (changed, !multiple && (changed || was_selected))
    }

    /// Process an item click: single-select replaces the selection and closes; multi-select toggles.
    ///
    /// Calls `delegate.on_will_change` before committing and `delegate.on_confirm` when closing.
    #[cfg(test)]
    pub(crate) fn handle_item_select(
        &mut self,
        ix: IndexPath,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.state.disabled {
            return;
        }

        let (item, enabled) = {
            let list = self.state.list.read(cx);
            let delegate = &list.delegate().delegate;
            let Some(item) = delegate.item(ix) else {
                return;
            };

            (item.clone(), delegate.is_item_enabled(ix, item, cx))
        };
        if !enabled {
            return;
        }

        let mut selection = self.state.selection.clone();
        let (changed, should_close) = self.state.list.update(cx, |list, _cx| {
            let delegate = &mut list.delegate_mut().delegate;
            Self::apply_selection(self.multiple, &mut selection, ix, &item, delegate)
        });

        self.state.selection = selection;
        self.state.sync_snapshot(cx);

        if changed {
            cx.emit(ComboboxEvent::Change(self.selected_values()));
            cx.notify();
        }

        if should_close {
            self.commit_close(window, cx, true);
        }
    }

    fn on_blur(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.state.open
            || self.state.list.read(cx).is_focused(window, cx)
            || self.state.focus_handle.is_focused(window)
        {
            return;
        }

        self.commit_close(window, cx, false);
    }

    fn up(&mut self, _: &SelectUp, window: &mut Window, cx: &mut Context<Self>) {
        if self.state.disabled {
            cx.propagate();
            return;
        }

        if !self.state.open {
            self.set_open(true, cx);
        }

        self.state.list.focus_handle(cx).focus(window, cx);
        cx.propagate();
    }

    fn down(&mut self, _: &SelectDown, window: &mut Window, cx: &mut Context<Self>) {
        if self.state.disabled {
            cx.propagate();
            return;
        }

        if !self.state.open {
            self.set_open(true, cx);
        }

        self.state.list.focus_handle(cx).focus(window, cx);
        cx.propagate();
    }

    fn enter(&mut self, _: &Confirm, window: &mut Window, cx: &mut Context<Self>) {
        if self.state.disabled {
            cx.propagate();
            return;
        }

        cx.propagate();

        if !self.state.open {
            self.set_open(true, cx);
            cx.notify();
        }

        self.state.list.focus_handle(cx).focus(window, cx);
    }

    fn toggle_menu(&mut self, _: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();

        if self.state.open {
            self.commit_close(window, cx, true);
        } else {
            self.set_open(true, cx);
            self.state.list.focus_handle(cx).focus(window, cx);
        }

        cx.notify();
    }

    /// Close the menu when a press lands outside the popup.
    ///
    /// A press on the trigger is left to propagate: swallowing it here would keep nested
    /// controls (a tag remove button, the clear button) from ever seeing the press, and
    /// `toggle_menu` closes the menu on release anyway.
    fn dismiss(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if !self.state.open || self.state.bounds.contains(&event.position) {
            return;
        }

        cx.stop_propagation();
        self.commit_close(window, cx, true);
    }

    fn escape(&mut self, _: &Cancel, window: &mut Window, cx: &mut Context<Self>) {
        if self.state.disabled {
            cx.propagate();
            return;
        }

        if !self.state.open {
            cx.propagate();
            return;
        }

        cx.stop_propagation();
        self.commit_close(window, cx, true);
    }

    fn commit_close(&mut self, window: &mut Window, cx: &mut Context<Self>, restore_focus: bool) {
        if !self.state.open || self.state.disabled {
            return;
        }

        let final_selection = self.state.selection.clone();
        self.state.list.update(cx, |list, _| {
            list.delegate_mut().delegate.on_confirm(&final_selection);
        });

        cx.emit(ComboboxEvent::Confirm(self.selected_values()));
        self.set_open(false, cx);
        if restore_focus {
            self.focus(window, cx);
        }
        cx.notify();
    }

    fn set_open(&mut self, open: bool, cx: &mut Context<Self>) {
        self.state.open = open && !self.state.disabled;
        cx.notify();
    }

    fn clean(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        self.clear_selection(cx);
    }

    fn default_trigger_body(&self, _window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let placeholder_text = self
            .state
            .placeholder
            .clone()
            .unwrap_or_else(|| t!("ComboBox.placeholder").into());

        if self.state.selection.is_empty() {
            return div()
                .text_color(cx.theme().muted_foreground)
                .child(placeholder_text)
                .into_any_element();
        }

        if self.multiple {
            let items: Vec<SharedString> = self
                .state
                .selection
                .iter()
                .map(|(_, i)| i.title())
                .collect();

            div()
                .w_full()
                .overflow_hidden()
                .whitespace_nowrap()
                .truncate()
                .child(items.join(", "))
                .into_any_element()
        } else {
            let title = self
                .state
                .selection
                .first()
                .map(|(_, i)| i.title())
                .unwrap_or_default();

            div()
                .w_full()
                .overflow_hidden()
                .whitespace_nowrap()
                .truncate()
                .child(title)
                .into_any_element()
        }
    }
}

impl<D> Render for ComboboxState<D>
where
    D: SearchableListDelegate + 'static,
    <D::Item as SearchableListItem>::Value: PartialEq + Clone,
{
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let searchable = self.searchable;
        let is_focused = self.state.focus_handle.is_focused(window);
        let show_clean = self.state.cleanable && !self.state.selection.is_empty();
        let bounds = self.state.bounds;
        let allow_open = !self.state.disabled;
        let outline_visible = self.state.open || (is_focused && !self.state.disabled);
        let disabled = self.state.disabled;

        let (bg, fg) = input_style(disabled, cx);

        self.state.list.update(cx, |list, cx| {
            list.set_searchable(searchable, cx);
            list.delegate_mut().size = self.state.size;
            list.delegate_mut().check_icon = self.check_icon.clone();
        });

        let selection = &self.state.selection;
        let placeholder = self.state.placeholder.as_ref();
        let open = self.state.open;
        let size = self.state.size;
        let has_custom_trigger = self.render_trigger.is_some();

        let trigger_body = if let Some(render_trigger) = &self.render_trigger {
            let ctx = ComboboxTriggerCtx {
                selection,
                placeholder,
                open,
                disabled,
                size,
            };

            render_trigger(&ctx, window, cx)
        } else {
            self.default_trigger_body(window, cx)
        };

        let trailing: AnyElement = if has_custom_trigger {
            div().into_any_element()
        } else if show_clean {
            clear_button(cx)
                .map(|this| {
                    if disabled {
                        this.disabled(true)
                    } else {
                        this.on_click(cx.listener(Self::clean))
                    }
                })
                .into_any_element()
        } else if let Some(icon) = self.trigger_icon.clone() {
            icon.xsmall()
                .text_color(cx.theme().muted_foreground)
                .into_any_element()
        } else {
            Caret::new(size)
                .text_color(cx.theme().muted_foreground)
                .into_any_element()
        };

        let toggle_handler: Option<Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>> =
            if allow_open {
                Some(Box::new(cx.listener(Self::toggle_menu)))
            } else {
                None
            };

        let prepaint_handler: Box<dyn Fn(Bounds<Pixels>, &mut Window, &mut App) + 'static> = {
            let state = cx.entity();
            Box::new(move |bounds, _, cx| state.update(cx, |r, _| r.state.bounds = bounds))
        };

        let footer_el = self.footer.as_ref().map(|f| f(window, cx));

        let dismiss_handler: Box<dyn Fn(&MouseDownEvent, &mut Window, &mut App) + 'static> =
            Box::new(cx.listener(Self::dismiss));

        let motion = cx.theme().motion.clone();
        let reduced_motion = crate::animation::reduced_motion(cx);
        let presence = flyout_presence(
            SharedString::from(format!(
                "combobox-popup-presence-{}",
                cx.entity().entity_id()
            )),
            self.state.open,
            PresenceOptions::default(),
            window,
            cx,
        );

        div()
            .size_full()
            .relative()
            .child(render_trigger_container(
                disabled,
                self.state.appearance,
                self.state.size,
                &self.state.style,
                bg,
                fg,
                outline_visible,
                allow_open,
                trigger_body,
                trailing,
                toggle_handler,
                prepaint_handler,
                cx,
            ))
            .when(presence.should_render(), |this| {
                this.child(
                    deferred(render_popup_shell(
                        &self.state.list,
                        self.state.menu_width,
                        self.state.search_placeholder.clone(),
                        self.state.size,
                        self.state.menu_max_h,
                        bounds,
                        footer_el,
                        dismiss_handler,
                        presence,
                        &motion,
                        reduced_motion,
                        cx,
                    ))
                    .with_priority(1),
                )
            })
    }
}

impl<D> EventEmitter<ComboboxEvent<D>> for ComboboxState<D>
where
    D: SearchableListDelegate + 'static,
    <D::Item as SearchableListItem>::Value: PartialEq + Clone,
{
}
impl<D> EventEmitter<DismissEvent> for ComboboxState<D>
where
    D: SearchableListDelegate + 'static,
    <D::Item as SearchableListItem>::Value: PartialEq + Clone,
{
}

impl<D> Focusable for ComboboxState<D>
where
    D: SearchableListDelegate + 'static,
    <D::Item as SearchableListItem>::Value: PartialEq + Clone,
{
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        if self.state.open {
            self.state.list.focus_handle(cx)
        } else {
            self.state.focus_handle.clone()
        }
    }
}

// MARK: Combobox element

/// A combo box with support for single and multi-select.
///
/// Clicking an item toggles it in the selection; the dropdown stays open until the user
/// presses Escape or clicks outside.
#[derive(IntoElement)]
pub struct Combobox<D: SearchableListDelegate + 'static>
where
    <D::Item as SearchableListItem>::Value: PartialEq + Clone,
{
    id: ElementId,
    state: Entity<ComboboxState<D>>,
    options: ComboboxOptions,
    render_trigger:
        Option<Box<dyn Fn(&ComboboxTriggerCtx<D>, &mut Window, &mut App) -> AnyElement + 'static>>,
    footer: Option<Box<dyn Fn(&mut Window, &mut App) -> AnyElement + 'static>>,
    empty: Option<Box<dyn Fn(&mut Window, &App) -> Option<AnyElement> + 'static>>,
}

impl<D> Combobox<D>
where
    D: SearchableListDelegate + 'static,
    <D::Item as SearchableListItem>::Value: PartialEq + Clone,
{
    pub fn new(state: &Entity<ComboboxState<D>>) -> Self {
        Self {
            id: ("multi-combo-box", state.entity_id()).into(),
            state: state.clone(),
            options: ComboboxOptions::default(),
            render_trigger: None,
            footer: None,
            empty: None,
        }
    }

    /// Set the width of the dropdown menu.
    pub fn menu_width(mut self, width: impl Into<Length>) -> Self {
        self.options.menu_width = width.into();
        self
    }

    /// Set the maximum height of the dropdown menu.
    pub fn menu_max_h(mut self, max_h: impl Into<Length>) -> Self {
        self.options.menu_max_h = max_h.into();
        self
    }

    /// Set the placeholder text shown when no items are selected.
    pub fn placeholder(mut self, placeholder: impl Into<SharedString>) -> Self {
        self.options.placeholder = Some(placeholder.into());
        self
    }

    /// Override the trigger chevron icon.
    pub fn icon(mut self, icon: impl Into<Icon>) -> Self {
        self.options.trigger_icon = Some(icon.into());
        self
    }

    /// Override the trailing check icon shown next to selected items.
    pub fn check_icon(mut self, icon: impl Into<Icon>) -> Self {
        self.options.check_icon = Some(icon.into());
        self
    }

    /// Set the placeholder text for the search input.
    pub fn search_placeholder(mut self, placeholder: impl Into<SharedString>) -> Self {
        self.options.search_placeholder = Some(placeholder.into());
        self
    }

    /// Show a clear button when at least one item is selected.
    pub fn cleanable(mut self, cleanable: bool) -> Self {
        self.options.cleanable = cleanable;
        self
    }

    /// Set the disabled state.
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.options.disabled = disabled;
        self
    }

    /// Set a custom closure that renders the empty-state element.
    pub fn empty<E: IntoElement + 'static>(
        mut self,
        builder: impl Fn(&mut Window, &App) -> E + 'static,
    ) -> Self {
        self.empty = Some(Box::new(move |window, cx| {
            Some(builder(window, cx).into_any_element())
        }));
        self
    }

    /// Control whether the trigger shows a border and background.
    pub fn appearance(mut self, appearance: bool) -> Self {
        self.options.appearance = appearance;
        self
    }

    /// Override the entire trigger element.
    pub fn render_trigger<E: IntoElement + 'static>(
        mut self,
        f: impl Fn(&ComboboxTriggerCtx<D>, &mut Window, &mut App) -> E + 'static,
    ) -> Self {
        self.render_trigger = Some(Box::new(move |ctx, window, cx| {
            f(ctx, window, cx).into_any_element()
        }));
        self
    }

    /// Render an element below a separator at the bottom of the dropdown.
    pub fn footer<E: IntoElement + 'static>(
        mut self,
        f: impl Fn(&mut Window, &mut App) -> E + 'static,
    ) -> Self {
        self.footer = Some(Box::new(move |window, cx| f(window, cx).into_any_element()));
        self
    }
}

impl<D> Sizable for Combobox<D>
where
    D: SearchableListDelegate + 'static,
    <D::Item as SearchableListItem>::Value: PartialEq + Clone,
{
    fn with_size(mut self, size: impl Into<Size>) -> Self {
        self.options.size = size.into();
        self
    }
}

impl<D> Styled for Combobox<D>
where
    D: SearchableListDelegate + 'static,
    <D::Item as SearchableListItem>::Value: PartialEq + Clone,
{
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.options.style
    }
}

impl<D> RenderOnce for Combobox<D>
where
    D: SearchableListDelegate + 'static,
    <D::Item as SearchableListItem>::Value: PartialEq + Clone,
{
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let disabled = self.options.disabled;
        let focus_handle = self.state.focus_handle(cx);
        let render_trigger = self.render_trigger;
        let footer = self.footer;
        let empty = self.empty;
        let opts = self.options;

        self.state.update(cx, |this, _| {
            this.state.style = opts.style;
            this.state.size = opts.size;
            this.state.cleanable = opts.cleanable;
            this.state.placeholder = opts.placeholder;
            this.state.search_placeholder = opts.search_placeholder;
            this.state.menu_width = opts.menu_width;
            this.state.menu_max_h = opts.menu_max_h;
            if opts.disabled {
                this.state.open = false;
            }
            this.state.disabled = opts.disabled;
            this.state.appearance = opts.appearance;
            this.trigger_icon = opts.trigger_icon;
            this.check_icon = opts.check_icon;
            this.render_trigger = render_trigger;
            this.footer = footer;

            if let Some(empty) = empty {
                this.state.empty = Some(empty);
            }
        });

        let is_open = self.state.read(cx).state.open;

        div()
            .id(self.id.clone())
            .role(Role::ComboBox)
            .aria_expanded(is_open)
            .aria_disabled(disabled)
            .key_context(CONTEXT)
            .when(!disabled, |this| {
                this.track_focus(&focus_handle.tab_stop(true))
                    .on_action(window.listener_for(&self.state, ComboboxState::up))
                    .on_action(window.listener_for(&self.state, ComboboxState::down))
                    .on_action(window.listener_for(&self.state, ComboboxState::enter))
                    .on_action(window.listener_for(&self.state, ComboboxState::escape))
            })
            .size_full()
            .child(self.state)
    }
}

// MARK: Rendering helpers

/// Renders the styled trigger container.
#[allow(clippy::too_many_arguments)]
fn render_trigger_container(
    disabled: bool,
    appearance: bool,
    size: Size,
    style: &StyleRefinement,
    bg: Hsla,
    fg: Hsla,
    outline_visible: bool,
    allow_open: bool,
    trigger_body: AnyElement,
    trailing: AnyElement,
    toggle_handler: Option<Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>>,
    prepaint_handler: Box<dyn Fn(Bounds<Pixels>, &mut Window, &mut App) + 'static>,
    cx: &mut App,
) -> impl IntoElement {
    div()
        .id("input")
        .relative()
        .flex()
        .items_center()
        .justify_between()
        .border_1()
        .border_color(cx.theme().transparent)
        .when(appearance, |this| {
            this.bg(bg)
                .text_color(fg)
                .when(disabled, |this| this.opacity(0.5))
                .border_color(cx.theme().input)
                .rounded(cx.theme().radius)
        })
        .overflow_hidden()
        .input_size(size)
        .input_text_size(size)
        .refine_style(style)
        .when(outline_visible, |this| this.focused_border(cx))
        .when(allow_open, |this| {
            this.when_some(toggle_handler, |this, handler| this.on_click(handler))
        })
        .child(
            h_flex()
                .id("inner")
                .w_full()
                .items_center()
                .justify_between()
                .gap_1()
                .child(trigger_body)
                .child(trailing),
        )
        .on_prepaint(prepaint_handler)
}

/// Renders the deferred anchored popup shell containing the searchable list and optional footer.
#[allow(clippy::too_many_arguments)]
fn render_popup_shell<D: SearchableListDelegate + 'static>(
    list: &Entity<ListState<SearchableListAdapter<D>>>,
    menu_width: Length,
    search_placeholder: Option<SharedString>,
    size: Size,
    menu_max_h: Length,
    bounds: Bounds<Pixels>,
    footer_el: Option<AnyElement>,
    dismiss_handler: Box<dyn Fn(&MouseDownEvent, &mut Window, &mut App) + 'static>,
    presence: PresenceTransition,
    motion: &crate::theme::ThemeMotion,
    reduced_motion: bool,
    cx: &mut App,
) -> AnyElement {
    let has_footer = footer_el.is_some();
    let tokens = FlyoutTokens::sized(size, cx);
    let surface = SurfacePreset::flyout().with_radius(tokens.radius);

    anchored()
        .snap_to_window_with_margin(px(8.))
        .child(
            div()
                .occlude()
                .map(|this| match menu_width {
                    Length::Auto => this.w(bounds.size.width + px(2.)),
                    Length::Definite(w) => this.w(w),
                })
                .child(
                    surface
                        .apply_material(
                            v_flex()
                                .occlude()
                                .mt_1p5()
                                .text_color(flyout_primary_foreground(cx))
                                .child(
                                    List::new(list)
                                        .when_some(search_placeholder, |this, placeholder| {
                                            this.search_placeholder(placeholder)
                                        })
                                        .with_size(size)
                                        .max_h(menu_max_h)
                                        .paddings(Edges::all(tokens.inset)),
                                )
                                .when(has_footer, |this| {
                                    this.child(
                                        div()
                                            .border_t_1()
                                            .border_color(cx.theme().border)
                                            .p(tokens.inset)
                                            .when_some(footer_el, |this, el| this.child(el)),
                                    )
                                }),
                            cx,
                            SurfaceContext::new(cx),
                        )
                        .map(|el| {
                            flyout_motion(
                                "combobox",
                                presence,
                                FlyoutSlide::vertical(-1.0),
                                motion,
                                reduced_motion,
                                el,
                            )
                        }),
                )
                .on_mouse_down_out(dismiss_handler),
        )
        .into_any_element()
}

// MARK: Tests

#[cfg(test)]
mod tests {
    use std::{cell::Cell, rc::Rc};

    use gpui::{
        AppContext as _, Bounds, ClickEvent, Context, Entity, Focusable as _,
        InteractiveElement as _, IntoElement, Modifiers, MouseButton, MouseDownEvent,
        ParentElement as _, Pixels, Point, Render, Role, SharedString, Subscription,
        TestAppContext, VisualTestContext, Window, div, point, px, size,
    };

    use crate::{
        IndexPath,
        actions::{Cancel, SelectDown},
        combobox::{Combobox, ComboboxEvent, ComboboxState},
        list::ListDelegate as _,
        searchable_list::{
            SearchableListChange, SearchableListDelegate, SearchableListItem, SearchableListState,
            SearchableVec,
        },
    };

    struct TestComboboxEventCollector {
        event_count: Rc<Cell<usize>>,
        _subscription: Subscription,
    }

    impl TestComboboxEventCollector {
        fn new(
            state: &Entity<ComboboxState<SearchableVec<&'static str>>>,
            cx: &mut Context<Self>,
        ) -> Self {
            let event_count = Rc::new(Cell::new(0));
            let event_count_for_subscription = event_count.clone();
            let _subscription = cx.subscribe(
                state,
                move |_, _, _: &ComboboxEvent<SearchableVec<&'static str>>, _| {
                    event_count_for_subscription.set(event_count_for_subscription.get() + 1);
                },
            );

            Self {
                event_count,
                _subscription,
            }
        }
    }

    struct TestComboboxObserver {
        notifications: Rc<Cell<usize>>,
        _subscription: Subscription,
    }

    impl TestComboboxObserver {
        fn new(
            state: &Entity<ComboboxState<SearchableVec<&'static str>>>,
            cx: &mut Context<Self>,
        ) -> Self {
            let notifications = Rc::new(Cell::new(0));
            let notifications_for_observer = notifications.clone();
            let _subscription = cx.observe(state, move |_, _, _| {
                notifications_for_observer.set(notifications_for_observer.get() + 1);
            });

            Self {
                notifications,
                _subscription,
            }
        }
    }

    struct ConfirmTrackingDelegate {
        items: SearchableVec<&'static str>,
        confirms: Rc<Cell<usize>>,
    }

    impl ConfirmTrackingDelegate {
        fn new(confirms: Rc<Cell<usize>>) -> Self {
            Self {
                items: SearchableVec::new(vec!["Rust", "Go"]),
                confirms,
            }
        }
    }

    impl SearchableListDelegate for ConfirmTrackingDelegate {
        type Item = &'static str;

        fn items_count(&self, section: usize) -> usize {
            self.items.items_count(section)
        }

        fn item(&self, ix: IndexPath) -> Option<&&'static str> {
            self.items.item(ix)
        }

        fn position<V>(&self, value: &V) -> Option<IndexPath>
        where
            &'static str: SearchableListItem<Value = V>,
            V: PartialEq,
        {
            self.items.position(value)
        }

        fn on_confirm(&mut self, _: &[(IndexPath, Self::Item)]) {
            self.confirms.set(self.confirms.get() + 1);
        }
    }

    #[derive(Clone)]
    struct VersionedItem {
        title: &'static str,
        value: &'static str,
    }

    impl SearchableListItem for VersionedItem {
        type Value = &'static str;

        fn title(&self) -> SharedString {
            self.title.into()
        }

        fn value(&self) -> &Self::Value {
            &self.value
        }
    }

    struct SameIndexValueDelegate {
        item: VersionedItem,
        replacement: VersionedItem,
    }

    impl SearchableListDelegate for SameIndexValueDelegate {
        type Item = VersionedItem;

        fn items_count(&self, _: usize) -> usize {
            1
        }

        fn item(&self, ix: IndexPath) -> Option<&Self::Item> {
            (ix == IndexPath::new(0)).then_some(&self.item)
        }

        fn position<V>(&self, value: &V) -> Option<IndexPath>
        where
            Self::Item: SearchableListItem<Value = V>,
            V: PartialEq,
        {
            (self.item.value() == value).then_some(IndexPath::new(0))
        }

        fn on_will_change(
            &mut self,
            selection: &mut Vec<(IndexPath, Self::Item)>,
            _: &[SearchableListChange],
        ) {
            *selection = vec![(IndexPath::new(0), self.replacement.clone())];
        }
    }

    struct TrackedEventCollector<D: SearchableListDelegate + 'static> {
        change_count: Rc<Cell<usize>>,
        confirm_count: Rc<Cell<usize>>,
        _subscription: Subscription,
        _marker: std::marker::PhantomData<fn() -> D>,
    }

    impl<D> TrackedEventCollector<D>
    where
        D: SearchableListDelegate + 'static,
        <D::Item as SearchableListItem>::Value: PartialEq + Clone,
    {
        fn new(state: &Entity<ComboboxState<D>>, cx: &mut Context<Self>) -> Self {
            let change_count = Rc::new(Cell::new(0));
            let confirm_count = Rc::new(Cell::new(0));
            let change_count_for_subscription = change_count.clone();
            let confirm_count_for_subscription = confirm_count.clone();
            let _subscription =
                cx.subscribe(
                    state,
                    move |_, _, event: &ComboboxEvent<D>, _| match event {
                        ComboboxEvent::Change(_) => {
                            change_count_for_subscription
                                .set(change_count_for_subscription.get() + 1);
                        }
                        ComboboxEvent::Confirm(_) => {
                            confirm_count_for_subscription
                                .set(confirm_count_for_subscription.get() + 1);
                        }
                    },
                );

            Self {
                change_count,
                confirm_count,
                _subscription,
                _marker: std::marker::PhantomData,
            }
        }
    }

    struct ComboboxEscapeHarness {
        state: Entity<ComboboxState<SearchableVec<&'static str>>>,
        parent_cancels: Rc<Cell<usize>>,
    }

    impl Render for ComboboxEscapeHarness {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let parent_cancels = self.parent_cancels.clone();
            div()
                .on_action(move |_: &Cancel, _, _| {
                    parent_cancels.set(parent_cancels.get() + 1);
                })
                .child(Combobox::new(&self.state))
        }
    }

    struct DisabledComboboxHarness {
        state: Entity<ComboboxState<SearchableVec<&'static str>>>,
        disabled: bool,
    }

    impl Render for DisabledComboboxHarness {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            Combobox::new(&self.state).disabled(self.disabled)
        }
    }

    #[derive(Clone)]
    struct DisabledItem(&'static str);

    impl SearchableListItem for DisabledItem {
        type Value = &'static str;

        fn title(&self) -> SharedString {
            self.0.into()
        }

        fn value(&self) -> &Self::Value {
            &self.0
        }

        fn disabled(&self) -> bool {
            true
        }
    }

    #[derive(Clone)]
    struct A11yItem {
        title: &'static str,
        disabled: bool,
    }

    impl SearchableListItem for A11yItem {
        type Value = &'static str;

        fn title(&self) -> SharedString {
            self.title.into()
        }

        fn value(&self) -> &Self::Value {
            &self.title
        }

        fn disabled(&self) -> bool {
            self.disabled
        }
    }

    struct ComboboxA11yHarness {
        state: Entity<ComboboxState<SearchableVec<A11yItem>>>,
    }

    impl Render for ComboboxA11yHarness {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            Combobox::new(&self.state)
        }
    }

    #[gpui::test]
    fn test_combo_box_builder(cx: &mut TestAppContext) {
        cx.update(crate::init);
        let cx = cx.add_empty_window();
        cx.update(|window, cx| {
            let items = SearchableVec::new(vec!["Rust", "Go", "C++"]);
            let state = cx.new(|cx| ComboboxState::new(items, vec![], window, cx).searchable(true));

            let _cb = Combobox::new(&state)
                .placeholder("Select language")
                .search_placeholder("Search...")
                .menu_width(gpui::px(300.))
                .menu_max_h(gpui::rems(15.))
                .cleanable(true)
                .disabled(false)
                .appearance(true);
        });
    }

    #[gpui::test]
    fn test_combo_box_search_filters_items(cx: &mut TestAppContext) {
        cx.update(crate::init);
        let cx = cx.add_empty_window();
        cx.update(|window, cx| {
            let items = SearchableVec::new(vec!["Rust", "Go", "C++"]);
            let state = cx.new(|cx| ComboboxState::new(items, vec![], window, cx).searchable(true));

            let count_before = state
                .read(cx)
                .state
                .list
                .read(cx)
                .delegate()
                .delegate
                .items_count(0);
            assert_eq!(count_before, 3);

            state.update(cx, |s, cx| {
                s.state.list.update(cx, |list, cx| {
                    let _ = list
                        .delegate_mut()
                        .delegate
                        .perform_search("Rust", window, cx);
                });
            });

            let count_after = state
                .read(cx)
                .state
                .list
                .read(cx)
                .delegate()
                .delegate
                .items_count(0);
            assert_eq!(count_after, 1);
        });
    }

    #[gpui::test]
    fn test_combo_box_set_query_updates_text_and_filters_items(cx: &mut TestAppContext) {
        cx.update(crate::init);
        let cx = cx.add_empty_window();
        cx.update(|window, cx| {
            let items = SearchableVec::new(vec!["Rust", "Go", "C++"]);
            let state = cx.new(|cx| ComboboxState::new(items, vec![], window, cx).searchable(true));

            state.update(cx, |state, cx| state.set_query(" Rust ", window, cx));

            assert_eq!(state.read(cx).query(cx).as_ref(), " Rust ");
            assert_eq!(
                state
                    .read(cx)
                    .state
                    .list
                    .read(cx)
                    .delegate()
                    .delegate
                    .items_count(0),
                1,
            );
        });
    }

    #[gpui::test]
    fn test_multi_combo_box_builder(cx: &mut TestAppContext) {
        cx.update(crate::init);
        let cx = cx.add_empty_window();
        cx.update(|window, cx| {
            let items = SearchableVec::new(vec!["React", "Vue", "Angular"]);
            let state = cx.new(|cx| {
                ComboboxState::new(items, vec![IndexPath::new(0)], window, cx)
                    .multiple(true)
                    .searchable(true)
            });

            let _cb = Combobox::new(&state)
                .placeholder("Select frameworks")
                .search_placeholder("Search...")
                .menu_width(gpui::px(300.))
                .cleanable(true)
                .disabled(false);

            assert_eq!(state.read(cx).selected_values(), vec!["React"]);
        });
    }

    #[gpui::test]
    fn test_combo_box_set_selected_values_uses_current_delegate(cx: &mut TestAppContext) {
        cx.update(crate::init);
        let cx = cx.add_empty_window();
        cx.update(|window, cx| {
            let items = SearchableVec::new(vec!["React", "Vue", "Angular"]);
            let state = cx.new(|cx| ComboboxState::new(items, vec![], window, cx).multiple(true));

            state.update(cx, |state, cx| {
                state.set_selected_values(&["Vue", "Missing"], window, cx);

                assert_eq!(state.selected_values(), vec!["Vue"]);
                assert_eq!(
                    state
                        .selection()
                        .iter()
                        .map(|(index, _)| *index)
                        .collect::<Vec<_>>(),
                    vec![IndexPath::new(1)],
                );
                assert_eq!(
                    state
                        .state
                        .list
                        .read(cx)
                        .delegate()
                        .selection_snapshot
                        .as_slice(),
                    state.selection(),
                );

                state.set_items(SearchableVec::new(vec!["Vue", "Rust", "Go"]), window, cx);
                state.set_selected_values(&["Go", "Vue"], window, cx);

                assert_eq!(state.selected_values(), vec!["Go", "Vue"]);
                assert_eq!(
                    state
                        .selection()
                        .iter()
                        .map(|(index, _)| *index)
                        .collect::<Vec<_>>(),
                    vec![IndexPath::new(2), IndexPath::new(0)],
                );
                assert_eq!(
                    state
                        .state
                        .list
                        .read(cx)
                        .delegate()
                        .selection_snapshot
                        .as_slice(),
                    state.selection(),
                );

                state.set_selected_values(&[], window, cx);

                assert!(state.selection().is_empty());
                assert!(
                    state
                        .state
                        .list
                        .read(cx)
                        .delegate()
                        .selection_snapshot
                        .is_empty()
                );
            });
        });
    }

    #[gpui::test]
    fn test_combo_box_set_selected_values_does_not_emit_events(cx: &mut TestAppContext) {
        cx.update(crate::init);
        let cx = cx.add_empty_window();
        let state = cx.update(|window, cx| {
            let items = SearchableVec::new(vec!["React", "Vue", "Angular"]);
            cx.new(|cx| ComboboxState::new(items, vec![], window, cx).multiple(true))
        });
        let collector = cx.update(|_, cx| cx.new(|cx| TestComboboxEventCollector::new(&state, cx)));

        cx.update(|window, cx| {
            state.update(cx, |state, cx| {
                state.set_selected_values(&["React", "Vue"], window, cx);
            });
        });

        cx.update(|_, cx| {
            assert_eq!(collector.read(cx).event_count.get(), 0);
        });
    }

    #[gpui::test]
    fn test_combo_box_initial_selection_seeds_cursor(cx: &mut TestAppContext) {
        cx.update(crate::init);
        let cx = cx.add_empty_window();
        cx.update(|window, cx| {
            let items = SearchableVec::new(vec!["React", "Vue", "Angular"]);
            let state = cx.new(|cx| {
                ComboboxState::new(items, vec![IndexPath::new(1)], window, cx).multiple(true)
            });

            let state_ref = state.read(cx);
            assert_eq!(
                state_ref.state.list.read(cx).selected_index(),
                Some(IndexPath::new(1)),
                "initial selected_indices should seed ListState.selected_index, not just the snapshot",
            );
            assert_eq!(state_ref.selected_values(), vec!["Vue"]);
        });
    }

    #[gpui::test]
    fn test_multi_combo_box_toggle(cx: &mut TestAppContext) {
        cx.update(crate::init);
        let cx = cx.add_empty_window();
        cx.update(|window, cx| {
            let items = SearchableVec::new(vec!["React", "Vue", "Angular"]);
            let state = cx.new(|cx| ComboboxState::new(items, vec![], window, cx).multiple(true));

            state.update(cx, |s, cx| s.add_selected_index(IndexPath::new(0), cx));
            assert_eq!(state.read(cx).selected_values(), &["React"]);

            state.update(cx, |s, cx| s.add_selected_index(IndexPath::new(1), cx));
            assert_eq!(state.read(cx).selected_values(), &["React", "Vue"]);

            state.update(cx, |s, cx| s.remove_selected_index(IndexPath::new(0), cx));
            assert_eq!(state.read(cx).selected_values(), &["Vue"]);
        });
    }

    #[gpui::test]
    fn test_remove_selected_index_notifies_and_updates_snapshot(cx: &mut TestAppContext) {
        cx.update(crate::init);
        let cx = cx.add_empty_window();
        let state = cx.update(|window, cx| {
            let items = SearchableVec::new(vec!["React", "Vue"]);
            cx.new(|cx| {
                ComboboxState::new(items, vec![IndexPath::new(0)], window, cx).multiple(true)
            })
        });
        let observer = cx.update(|_, cx| cx.new(|cx| TestComboboxObserver::new(&state, cx)));

        let before = cx.update(|_, cx| observer.read(cx).notifications.get());
        cx.update(|_, cx| {
            state.update(cx, |state, cx| {
                assert!(state.remove_selected_index(IndexPath::new(0), cx));
            });
        });
        cx.run_until_parked();

        cx.update(|_, cx| {
            assert!(state.read(cx).selection().is_empty());
            assert!(
                state
                    .read(cx)
                    .state
                    .list
                    .read(cx)
                    .delegate()
                    .selection_snapshot
                    .is_empty()
            );
            assert!(observer.read(cx).notifications.get() > before);
        });
    }

    #[gpui::test]
    fn test_set_items_notifies_and_preserves_valid_selection(cx: &mut TestAppContext) {
        cx.update(crate::init);
        let cx = cx.add_empty_window();
        let state = cx.update(|window, cx| {
            let items = SearchableVec::new(vec!["React", "Vue", "Angular"]);
            cx.new(|cx| ComboboxState::new(items, vec![IndexPath::new(1)], window, cx))
        });
        let observer = cx.update(|_, cx| cx.new(|cx| TestComboboxObserver::new(&state, cx)));

        let before = cx.update(|_, cx| observer.read(cx).notifications.get());
        cx.update(|window, cx| {
            state.update(cx, |state, cx| {
                state.set_items(SearchableVec::new(vec!["Rust", "Vue", "Go"]), window, cx);
            });
        });
        cx.run_until_parked();
        cx.update(|_, cx| {
            let state = state.read(cx);
            assert_eq!(state.selected_values(), vec!["Vue"]);
            assert_eq!(
                state.state.list.read(cx).selected_index(),
                Some(IndexPath::new(1))
            );
            assert_eq!(
                state.state.list.read(cx).delegate().delegate.items_count(0),
                3
            );
            assert!(observer.read(cx).notifications.get() > before);
        });

        cx.update(|window, cx| {
            state.update(cx, |state, cx| {
                state.set_items(SearchableVec::new(vec!["Vue"]), window, cx);
            });
        });
        cx.update(|_, cx| {
            let state = state.read(cx);
            assert_eq!(state.selected_values(), vec!["Vue"]);
            assert_eq!(
                state.state.list.read(cx).selected_index(),
                Some(IndexPath::new(0))
            );
            assert_eq!(
                state.state.list.read(cx).delegate().delegate.items_count(0),
                1
            );
        });

        cx.update(|window, cx| {
            state.update(cx, |state, cx| {
                state.set_items(SearchableVec::new(vec!["Rust"]), window, cx);
            });
        });
        cx.update(|_, cx| {
            let state = state.read(cx);
            assert!(state.selection().is_empty());
            assert_eq!(state.state.list.read(cx).selected_index(), None);
            assert!(
                state
                    .state
                    .list
                    .read(cx)
                    .delegate()
                    .selection_snapshot
                    .is_empty()
            );
        });
    }

    #[gpui::test]
    fn test_disabled_items_do_not_confirm_from_pointer_or_keyboard(cx: &mut TestAppContext) {
        cx.update(crate::init);
        let cx = cx.add_empty_window();
        let state = cx.update(|window, cx| {
            let items = SearchableVec::new(vec![DisabledItem("Locked")]);
            cx.new(|cx| ComboboxState::new(items, vec![], window, cx))
        });

        cx.update(|window, cx| {
            state.update(cx, |state, cx| {
                state.set_open(true, cx);
                state.handle_item_select(IndexPath::new(0), window, cx);
            });
        });
        cx.update(|_, cx| {
            assert!(state.read(cx).selection().is_empty());
            assert!(state.read(cx).state.open);
        });

        cx.update(|window, cx| {
            state.update(cx, |state, cx| {
                state.state.list.update(cx, |list, cx| {
                    list.set_selected_index(Some(IndexPath::new(0)), window, cx);
                    list.delegate_mut().confirm(false, window, cx);
                });
            });
        });
        cx.run_until_parked();
        cx.update(|_, cx| {
            assert!(state.read(cx).selection().is_empty());
            assert!(state.read(cx).state.open);
        });
    }

    #[gpui::test]
    fn test_disabling_open_combobox_closes_and_blocks_interaction(cx: &mut TestAppContext) {
        let window = cx.update(|cx| {
            crate::init(cx);
            cx.open_window(Default::default(), |window, cx| {
                let state = cx.new(|cx| {
                    ComboboxState::new(SearchableVec::new(vec!["Rust", "Go"]), vec![], window, cx)
                });
                cx.new(|_| DisabledComboboxHarness {
                    state,
                    disabled: false,
                })
            })
            .unwrap()
        });
        let mut cx = VisualTestContext::from_window(window.into(), cx);
        let harness = window.root(&mut cx).unwrap();
        let state = harness.read_with(&cx, |harness, _| harness.state.clone());

        state.update_in(&mut cx, |state, _, cx| state.set_open(true, cx));
        cx.update(|window, cx| window.draw(cx).clear(cx));
        state.read_with(&cx, |state, _| assert!(state.state.open));

        harness.update_in(&mut cx, |harness, _, cx| {
            harness.disabled = true;
            cx.notify();
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
        state.read_with(&cx, |state, _| {
            assert!(state.state.disabled);
            assert!(!state.state.open);
        });

        state.update_in(&mut cx, |state, window, cx| {
            state.down(&SelectDown, window, cx);
            state.handle_item_select(IndexPath::new(0), window, cx);
        });
        state.read_with(&cx, |state, _| {
            assert!(!state.state.open);
            assert!(state.selection().is_empty());
        });
    }

    #[gpui::test]
    fn test_combo_box_escape_confirms_once_and_restores_focus(cx: &mut TestAppContext) {
        let parent_cancels = Rc::new(Cell::new(0));
        let window = cx.update(|cx| {
            crate::init(cx);
            cx.open_window(Default::default(), {
                let parent_cancels = parent_cancels.clone();
                move |window, cx| {
                    let state = cx.new(|cx| {
                        ComboboxState::new(
                            SearchableVec::new(vec!["Rust", "Go"]),
                            vec![],
                            window,
                            cx,
                        )
                    });
                    cx.new(|_| ComboboxEscapeHarness {
                        state,
                        parent_cancels,
                    })
                }
            })
            .unwrap()
        });
        let mut cx = VisualTestContext::from_window(window.into(), cx);
        let harness = window.root(&mut cx).unwrap();
        let state = harness.read_with(&cx, |harness, _| harness.state.clone());
        let collector = cx.update(|_, cx| cx.new(|cx| TestComboboxEventCollector::new(&state, cx)));

        state.update_in(&mut cx, |state, window, cx| {
            state.set_open(true, cx);
            state.state.list.focus_handle(cx).focus(window, cx);
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));

        cx.dispatch_action(Cancel);
        cx.run_until_parked();
        state.read_with(&cx, |state, _| assert!(!state.state.open));
        cx.update(|window, cx| assert!(state.read(cx).state.focus_handle.is_focused(window)));
        assert_eq!(
            collector.read_with(&cx, |collector, _| collector.event_count.get()),
            1
        );
        assert_eq!(parent_cancels.get(), 0);

        cx.update(|window, cx| window.draw(cx).clear(cx));
        cx.dispatch_action(Cancel);
        cx.run_until_parked();
        assert_eq!(parent_cancels.get(), 1);
        assert_eq!(
            collector.read_with(&cx, |collector, _| collector.event_count.get()),
            1
        );
    }

    #[gpui::test]
    fn test_escape_and_blur_commit_delegate_once(cx: &mut TestAppContext) {
        cx.update(crate::init);
        let cx = cx.add_empty_window();
        let confirms = Rc::new(Cell::new(0));
        let state = cx.update(|window, cx| {
            cx.new(|cx| {
                ComboboxState::new(
                    ConfirmTrackingDelegate::new(confirms.clone()),
                    vec![],
                    window,
                    cx,
                )
            })
        });
        let collector = cx.update(|_, cx| cx.new(|cx| TrackedEventCollector::new(&state, cx)));

        cx.update(|window, cx| {
            state.update(cx, |state, cx| {
                state.set_open(true, cx);
                state.state.list.update(cx, |list, cx| {
                    list.delegate_mut().cancel(window, cx);
                });
            });
        });
        cx.run_until_parked();
        assert_eq!(confirms.get(), 1);
        assert_eq!(
            collector.read_with(cx, |collector, _| collector.confirm_count.get()),
            1
        );

        cx.update(|window, cx| {
            window.blur();
            state.update(cx, |state, cx| {
                state.set_open(true, cx);
                state.on_blur(window, cx);
            });
        });
        assert_eq!(confirms.get(), 2);
        assert_eq!(
            collector.read_with(cx, |collector, _| collector.confirm_count.get()),
            2
        );
    }

    #[gpui::test]
    fn test_trigger_and_pointer_close_commit_delegate_once(cx: &mut TestAppContext) {
        cx.update(crate::init);
        let cx = cx.add_empty_window();
        let confirms = Rc::new(Cell::new(0));
        let state = cx.update(|window, cx| {
            cx.new(|cx| {
                ComboboxState::new(
                    ConfirmTrackingDelegate::new(confirms.clone()),
                    vec![],
                    window,
                    cx,
                )
                .multiple(true)
            })
        });
        let collector = cx.update(|_, cx| cx.new(|cx| TrackedEventCollector::new(&state, cx)));

        cx.update(|window, cx| {
            state.update(cx, |state, cx| {
                state.set_open(true, cx);
                state.toggle_menu(&ClickEvent::default(), window, cx);
                state.set_open(true, cx);
                state.dismiss(&left_press(point(px(1.), px(1.))), window, cx);
            });
        });

        assert_eq!(confirms.get(), 2);
        assert_eq!(
            collector.read_with(cx, |collector, _| collector.confirm_count.get()),
            2
        );
    }

    #[gpui::test]
    fn test_item_confirm_commits_delegate_once(cx: &mut TestAppContext) {
        cx.update(crate::init);
        let cx = cx.add_empty_window();
        let confirms = Rc::new(Cell::new(0));
        let state = cx.update(|window, cx| {
            cx.new(|cx| {
                ComboboxState::new(
                    ConfirmTrackingDelegate::new(confirms.clone()),
                    vec![],
                    window,
                    cx,
                )
            })
        });
        let collector = cx.update(|_, cx| cx.new(|cx| TrackedEventCollector::new(&state, cx)));

        cx.update(|window, cx| {
            state.update(cx, |state, cx| {
                state.set_open(true, cx);
                state.state.list.update(cx, |list, cx| {
                    list.set_selected_index(Some(IndexPath::new(0)), window, cx);
                    list.delegate_mut().confirm(false, window, cx);
                });
            });
        });
        cx.run_until_parked();

        assert_eq!(confirms.get(), 1);
        assert_eq!(
            collector.read_with(cx, |collector, _| collector.confirm_count.get()),
            1
        );
        assert!(!state.read_with(cx, |state, _| state.state.open));
    }

    #[gpui::test]
    fn test_selected_item_confirm_closes_and_commits_once(cx: &mut TestAppContext) {
        cx.update(crate::init);
        let cx = cx.add_empty_window();
        let confirms = Rc::new(Cell::new(0));
        let state = cx.update(|window, cx| {
            cx.new(|cx| {
                ComboboxState::new(
                    ConfirmTrackingDelegate::new(confirms.clone()),
                    vec![IndexPath::new(0)],
                    window,
                    cx,
                )
            })
        });
        let collector = cx.update(|_, cx| cx.new(|cx| TrackedEventCollector::new(&state, cx)));

        cx.update(|window, cx| {
            state.update(cx, |state, cx| {
                state.set_open(true, cx);
                state.handle_item_select(IndexPath::new(0), window, cx);
            });
        });

        assert_eq!(confirms.get(), 1);
        assert_eq!(
            collector.read_with(cx, |collector, _| collector.confirm_count.get()),
            1
        );
        assert_eq!(
            collector.read_with(cx, |collector, _| collector.change_count.get()),
            0
        );
        assert!(!state.read_with(cx, |state, _| state.state.open));
    }

    #[gpui::test]
    fn test_same_index_value_change_emits_change(cx: &mut TestAppContext) {
        cx.update(crate::init);
        let cx = cx.add_empty_window();
        let state = cx.update(|window, cx| {
            cx.new(|cx| {
                ComboboxState::new(
                    SameIndexValueDelegate {
                        item: VersionedItem {
                            title: "Original",
                            value: "original",
                        },
                        replacement: VersionedItem {
                            title: "Replacement",
                            value: "replacement",
                        },
                    },
                    vec![IndexPath::new(0)],
                    window,
                    cx,
                )
            })
        });
        let collector = cx.update(|_, cx| cx.new(|cx| TrackedEventCollector::new(&state, cx)));

        cx.update(|window, cx| {
            state.update(cx, |state, cx| {
                state.set_open(true, cx);
                state.handle_item_select(IndexPath::new(0), window, cx);
            });
        });

        assert_eq!(
            state.read_with(cx, |state, _| state.selected_values()),
            ["replacement"]
        );
        assert_eq!(
            collector.read_with(cx, |collector, _| collector.change_count.get()),
            1
        );
        assert_eq!(
            collector.read_with(cx, |collector, _| collector.confirm_count.get()),
            1
        );
        assert!(!state.read_with(cx, |state, _| state.state.open));
    }

    #[gpui::test]
    fn test_open_combobox_exposes_selected_and_disabled_options(cx: &mut TestAppContext) {
        let window = cx.update(|cx| {
            crate::init(cx);
            cx.open_window(Default::default(), |window, cx| {
                let state = cx.new(|cx| {
                    ComboboxState::new(
                        SearchableVec::new(vec![
                            A11yItem {
                                title: "Selected option",
                                disabled: false,
                            },
                            A11yItem {
                                title: "Cursor option",
                                disabled: false,
                            },
                            A11yItem {
                                title: "Disabled option",
                                disabled: true,
                            },
                        ]),
                        vec![IndexPath::new(0)],
                        window,
                        cx,
                    )
                });
                state.update(cx, |state, cx| {
                    state.set_open(true, cx);
                    state.state.list.update(cx, |list, cx| {
                        list.set_selected_index(Some(IndexPath::new(1)), window, cx);
                    });
                });
                cx.new(|_| ComboboxA11yHarness { state })
            })
            .unwrap()
        });
        let mut cx = VisualTestContext::from_window(window.into(), cx);

        let update = cx.update(|window, cx| {
            window.set_a11y_active_for_test(true);
            window.draw(cx).clear(cx);
            window
                .last_a11y_tree_for_test()
                .cloned()
                .expect("open combobox should publish an accessibility tree")
        });
        let options = update
            .nodes
            .iter()
            .filter_map(|(_, node)| (node.role() == Role::ListBoxOption).then_some(node))
            .collect::<Vec<_>>();

        assert_eq!(options.len(), 3);
        assert_eq!(options[0].is_selected(), Some(true));
        assert_eq!(options[1].is_selected(), Some(false));
        assert!(
            options
                .iter()
                .any(|node| node.is_selected() == Some(true) && !node.is_disabled())
        );
        assert!(options.iter().any(|node| node.is_disabled()));
    }

    #[gpui::test]
    fn test_multi_combo_box_search_selection_uses_value_identity(cx: &mut TestAppContext) {
        cx.update(crate::init);
        let cx = cx.add_empty_window();
        cx.update(|window, cx| {
            let items = SearchableVec::new(vec!["React", "Vue", "Angular"]);
            let state = cx.new(|cx| ComboboxState::new(items, vec![], window, cx).multiple(true));

            state.update(cx, |s, cx| s.add_selected_index(IndexPath::new(0), cx));
            assert_eq!(state.read(cx).selected_values(), &["React"]);

            state.update(cx, |s, cx| {
                s.state.list.update(cx, |list, cx| {
                    let _ = list
                        .delegate_mut()
                        .delegate
                        .perform_search("Vue", window, cx);
                });
            });

            state.read_with(cx, |s, cx| {
                let selection = s.state.selection.clone();
                let list = s.state.list.read(cx);
                let delegate = &list.delegate().delegate;
                let ix = IndexPath::new(0);
                let item = delegate.item(ix).expect("filtered item exists");

                assert_eq!(item.value(), &"Vue");
                assert!(
                    !delegate.is_item_checked(ix, item, &selection, cx),
                    "filtered row 0 should not inherit React's checked state",
                );
            });

            state.update(cx, |s, cx| {
                s.handle_item_select(IndexPath::new(0), window, cx);
            });
            assert_eq!(state.read(cx).selected_values(), &["React", "Vue"]);
        });
    }

    #[gpui::test]
    fn test_multi_combo_box_search_deselects_by_value(cx: &mut TestAppContext) {
        cx.update(crate::init);
        let cx = cx.add_empty_window();
        cx.update(|window, cx| {
            let items = SearchableVec::new(vec!["React", "Vue", "Angular"]);
            let state = cx.new(|cx| ComboboxState::new(items, vec![], window, cx).multiple(true));

            state.update(cx, |s, cx| s.add_selected_index(IndexPath::new(0), cx));

            state.update(cx, |s, cx| {
                s.state.list.update(cx, |list, cx| {
                    let _ = list
                        .delegate_mut()
                        .delegate
                        .perform_search("React", window, cx);
                });
            });

            state.update(cx, |s, cx| {
                s.handle_item_select(IndexPath::new(0), window, cx);
            });
            assert!(state.read(cx).selected_values().is_empty());
        });
    }

    #[gpui::test]
    fn test_searchable_list_default_change_uses_value_identity(cx: &mut TestAppContext) {
        cx.update(crate::init);
        let cx = cx.add_empty_window();
        cx.update(|window, cx| {
            let mut delegate = SearchableVec::new(vec!["React", "Vue", "Angular"]);
            let mut selection = vec![(IndexPath::new(1), "Vue")];

            let _ = delegate.perform_search("Vue", window, cx);
            delegate.on_will_change(
                &mut selection,
                &[SearchableListChange::Deselect {
                    index: IndexPath::new(0),
                }],
            );
            assert!(selection.is_empty());

            delegate.on_will_change(
                &mut selection,
                &[SearchableListChange::Select {
                    index: IndexPath::new(0),
                }],
            );
            assert_eq!(selection, vec![(IndexPath::new(0), "Vue")]);
        });
    }

    #[gpui::test]
    fn test_multi_combo_box_clear(cx: &mut TestAppContext) {
        cx.update(crate::init);
        let cx = cx.add_empty_window();
        cx.update(|window, cx| {
            let items = SearchableVec::new(vec!["React", "Vue", "Angular"]);
            let state = cx.new(|cx| {
                ComboboxState::new(
                    items,
                    vec![IndexPath::new(0), IndexPath::new(1)],
                    window,
                    cx,
                )
                .multiple(true)
            });

            assert_eq!(state.read(cx).selected_values().len(), 2);
            state.update(cx, |s, cx| s.clear_selection(cx));
            assert!(state.read(cx).selected_values().is_empty());
        });
    }

    #[gpui::test]
    fn test_single_combo_box_mode(cx: &mut TestAppContext) {
        cx.update(crate::init);
        let cx = cx.add_empty_window();
        cx.update(|window, cx| {
            let items = SearchableVec::new(vec!["Rust", "Go", "C++"]);
            let state = cx.new(|cx| ComboboxState::new(items, vec![], window, cx));

            // Default mode is Single.
            state.update(cx, |s, cx| s.add_selected_index(IndexPath::new(0), cx));
            assert_eq!(state.read(cx).selected_values(), &["Rust"]);

            state.update(cx, |s, cx| s.add_selected_index(IndexPath::new(1), cx));
            assert_eq!(state.read(cx).selected_values(), &["Rust", "Go"]);
        });
    }

    // Delegate that vetoes all selections via on_will_change by ignoring the changes.
    struct VetoDelegate(SearchableVec<&'static str>);

    impl SearchableListDelegate for VetoDelegate {
        type Item = &'static str;

        fn items_count(&self, section: usize) -> usize {
            self.0.items_count(section)
        }

        fn item(&self, ix: IndexPath) -> Option<&&'static str> {
            self.0.item(ix)
        }

        fn position<V>(&self, value: &V) -> Option<IndexPath>
        where
            &'static str: SearchableListItem<Value = V>,
            V: PartialEq,
        {
            self.0.position(value)
        }

        fn on_will_change(
            &mut self,
            _selection: &mut Vec<(IndexPath, &'static str)>,
            _changes: &[SearchableListChange],
        ) {
            // Leave selection unchanged — acts as a veto.
        }
    }

    #[gpui::test]
    fn test_on_will_change_veto(cx: &mut TestAppContext) {
        cx.update(crate::init);
        let cx = cx.add_empty_window();
        cx.update(|window, cx| {
            let delegate = VetoDelegate(SearchableVec::new(vec!["Rust", "Go", "C++"]));
            let state = cx.new(|cx| ComboboxState::new(delegate, vec![], window, cx));

            // Pre-select an item directly so we can verify veto prevents changes.
            state.update(cx, |s, cx| s.add_selected_index(IndexPath::new(0), cx));
            assert_eq!(state.read(cx).selected_values(), &["Rust"]);

            // Simulate a click on index 1 via handle_item_select; on_will_change vetoes it.
            state.update(cx, |s, cx| {
                s.handle_item_select(IndexPath::new(1), window, cx);
            });

            // Selection must remain unchanged because on_will_change left it unmodified.
            assert_eq!(state.read(cx).selected_values(), &["Rust"]);
        });
    }

    fn left_press(position: Point<Pixels>) -> MouseDownEvent {
        MouseDownEvent {
            button: MouseButton::Left,
            position,
            modifiers: Modifiers::default(),
            click_count: 1,
            first_mouse: false,
        }
    }

    #[gpui::test]
    fn test_combo_box_dismiss_ignores_press_on_trigger(cx: &mut TestAppContext) {
        cx.update(crate::init);
        let cx = cx.add_empty_window();
        let state = cx.update(|window, cx| {
            let items = SearchableVec::new(vec!["React", "Vue", "Angular"]);
            cx.new(|cx| ComboboxState::new(items, vec![], window, cx).multiple(true))
        });
        let collector = cx.update(|_, cx| cx.new(|cx| TestComboboxEventCollector::new(&state, cx)));

        cx.update(|window, cx| {
            state.update(cx, |state, cx| {
                state.state.bounds = Bounds {
                    origin: point(px(10.), px(10.)),
                    size: size(px(200.), px(32.)),
                };
                state.set_open(true, cx);
                state.dismiss(&left_press(point(px(20.), px(20.))), window, cx);
            });
        });

        cx.update(|_, cx| {
            assert!(
                state.read(cx).state.open,
                "a press on the trigger must reach nested controls, so the menu stays open \
                 and `toggle_menu` closes it on release instead",
            );
            assert_eq!(collector.read(cx).event_count.get(), 0);
        });

        cx.update(|window, cx| {
            state.update(cx, |state, cx| {
                state.dismiss(&left_press(point(px(20.), px(200.))), window, cx);
            });
        });

        cx.update(|_, cx| {
            assert!(!state.read(cx).state.open);
            assert_eq!(
                collector.read(cx).event_count.get(),
                1,
                "dismissing from outside the trigger emits Confirm",
            );
        });
    }

    // Suppress unused import warning for SearchableListState in test module.
    #[allow(unused)]
    fn _uses_state<D: SearchableListDelegate + 'static>(_: &SearchableListState<D>)
    where
        <D::Item as SearchableListItem>::Value: PartialEq + Clone,
    {
    }
}
