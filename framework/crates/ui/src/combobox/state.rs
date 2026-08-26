use gpui::{
    AnyElement, App, Bounds, ClickEvent, Context, DismissEvent, EventEmitter, FocusHandle,
    Focusable, IntoElement, MouseDownEvent, ParentElement, Pixels, Render, SharedString, Styled,
    Window, deferred, div, prelude::FluentBuilder,
};
use rust_i18n::t;

use crate::{
    ActiveTheme, Disableable, Icon, IndexPath, Sizable,
    actions::{Cancel, Confirm, SelectDown, SelectUp},
    animation::{PresenceOptions, flyout_presence},
    input::clear_button,
    searchable_list::{
        SearchableListChange, SearchableListDelegate, SearchableListItem, SearchableListState,
    },
};

use super::{
    ComboboxTriggerCtx,
    render::{Caret, input_style, render_popup_shell, render_trigger_container},
};

/// State of the [`super::Combobox`] component.
pub struct ComboboxState<D: SearchableListDelegate + 'static>
where
    <D::Item as SearchableListItem>::Value: PartialEq + Clone,
{
    pub(crate) state: SearchableListState<D>,

    // Combobox-specific fields
    multiple: bool,
    searchable: bool,
    pub(super) trigger_icon: Option<Icon>,
    pub(super) check_icon: Option<Icon>,
    pub(super) render_trigger:
        Option<Box<dyn Fn(&ComboboxTriggerCtx<D>, &mut Window, &mut App) -> AnyElement + 'static>>,
    pub(super) footer: Option<Box<dyn Fn(&mut Window, &mut App) -> AnyElement + 'static>>,
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
    /// [`ComboboxEvent`]. Single-select mode keeps the first resolved value.
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

    /// Replace the entire selection set. Single-select mode keeps the first valid index.
    pub fn set_selected_indices(
        &mut self,
        indices: impl IntoIterator<Item = IndexPath>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.state.set_selected_indices(indices, cx);
        if !self.multiple {
            self.state.selection.truncate(1);
        }
        self.state.sync_snapshot(cx);
        cx.notify();
    }

    /// Add a single index to the selection, if not already present, returning whether it was added.
    /// In single-select mode, a new index replaces the current selection.
    pub fn add_selected_index(&mut self, index: IndexPath, cx: &mut Context<Self>) -> bool {
        let (added, changed) = if self.multiple {
            let added = self.state.add_selected_index(index, cx);
            (added, added)
        } else {
            let already_selected = self
                .state
                .selection
                .iter()
                .any(|(selected_index, _)| *selected_index == index);
            let item = self
                .state
                .list
                .read(cx)
                .delegate()
                .delegate
                .item(index)
                .cloned();

            let Some(item) = item else {
                return false;
            };

            if self.state.selection.len() == 1 && already_selected {
                return false;
            }

            self.state.selection = vec![(index, item)];
            (!already_selected, true)
        };

        if changed {
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

    pub(super) fn up(&mut self, _: &SelectUp, window: &mut Window, cx: &mut Context<Self>) {
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

    pub(super) fn down(&mut self, _: &SelectDown, window: &mut Window, cx: &mut Context<Self>) {
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

    pub(super) fn enter(&mut self, _: &Confirm, window: &mut Window, cx: &mut Context<Self>) {
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

    pub(super) fn escape(&mut self, _: &Cancel, window: &mut Window, cx: &mut Context<Self>) {
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

#[cfg(test)]
mod tests;
