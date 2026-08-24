use std::{cell::RefCell, rc::Rc};

use gpui::{
    AbsoluteLength, Anchor, AnyElement, App, Bounds, ClickEvent, Context, DefiniteLength,
    DismissEvent, Edges, ElementId, Entity, EventEmitter, FocusHandle, Focusable,
    InteractiveElement, IntoElement, KeyBinding, Length, ParentElement, Pixels, Render, RenderOnce,
    SharedString, StatefulInteractiveElement, StyleRefinement, Styled, Task, WeakEntity, Window,
    anchored, deferred, div, point, prelude::FluentBuilder, px, rems,
};
use rust_i18n::t;

use crate::{
    ActiveTheme, Disableable, ElementExt as _, FlyoutTokens, Icon, IconName, IndexPath, Sizable,
    Size, StyleSized, StyledExt, SurfaceContext, SurfacePreset,
    actions::{Cancel, Confirm, SelectDown, SelectUp},
    animation::{FlyoutSlide, PresenceOptions, flyout_motion, flyout_presence},
    h_flex,
    input::clear_button,
    list::List,
    searchable_list::{SearchableListDelegate, SearchableListItem, SearchableListState},
    v_flex,
};

const CONTEXT: &str = "Select";
const POPUP_GAP: Pixels = px(6.);
const POPUP_MARGIN: Pixels = px(8.);

#[derive(Debug, Clone, Copy, PartialEq)]
struct SelectMenuPlacement {
    anchor: Anchor,
    position: gpui::Point<Pixels>,
}

fn select_menu_placement(
    trigger: Bounds<Pixels>,
    menu_height: Pixels,
    viewport_height: Pixels,
    margin: Pixels,
    gap: Pixels,
) -> SelectMenuPlacement {
    let fits_below = trigger.bottom() + gap + menu_height <= viewport_height - margin;
    if fits_below {
        SelectMenuPlacement {
            anchor: Anchor::TopLeft,
            position: point(trigger.left(), trigger.bottom() + gap),
        }
    } else {
        SelectMenuPlacement {
            anchor: Anchor::BottomLeft,
            position: point(trigger.left(), trigger.top() - gap),
        }
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

/// An item that can be displayed in a select.
pub trait SelectItem: Clone {
    type Value: Clone;

    fn title(&self) -> SharedString;

    /// Customize the display title used for the selected item in the Select input.
    fn display_title(&self) -> Option<AnyElement> {
        None
    }

    /// Render the item for the Select dropdown menu.
    fn render(&self, _: &mut Window, _: &mut App) -> impl IntoElement {
        self.title().into_element()
    }

    /// Get the value of the item.
    fn value(&self) -> &Self::Value;

    /// Check if the item matches the query for search.
    fn matches(&self, query: &str) -> bool {
        self.title().to_lowercase().contains(&query.to_lowercase())
    }
}

impl SelectItem for String {
    type Value = Self;

    fn title(&self) -> SharedString {
        self.clone().into()
    }

    fn value(&self) -> &Self::Value {
        self
    }
}

impl SelectItem for SharedString {
    type Value = Self;

    fn title(&self) -> SharedString {
        self.clone()
    }

    fn value(&self) -> &Self::Value {
        self
    }
}

impl SelectItem for &'static str {
    type Value = Self;

    fn title(&self) -> SharedString {
        (*self).into()
    }

    fn value(&self) -> &Self::Value {
        self
    }
}

/// A data source for a Select.
pub trait SelectDelegate: Sized {
    type Item: SelectItem;

    fn sections_count(&self, _: &App) -> usize {
        1
    }

    fn section(&self, _section: usize) -> Option<AnyElement> {
        None
    }

    fn items_count(&self, section: usize) -> usize;

    fn item(&self, ix: IndexPath) -> Option<&Self::Item>;

    fn position<V>(&self, _value: &V) -> Option<IndexPath>
    where
        Self::Item: SelectItem<Value = V>,
        V: PartialEq;

    fn perform_search(
        &mut self,
        _query: &str,
        _window: &mut Window,
        _: &mut Context<SelectState<Self>>,
    ) -> Task<()> {
        Task::ready(())
    }
}

impl<T: SelectItem> SelectDelegate for Vec<T> {
    type Item = T;

    fn items_count(&self, _: usize) -> usize {
        self.len()
    }

    fn item(&self, ix: IndexPath) -> Option<&Self::Item> {
        self.get(ix.row)
    }

    fn position<V>(&self, value: &V) -> Option<IndexPath>
    where
        Self::Item: SelectItem<Value = V>,
        V: PartialEq,
    {
        self.iter()
            .position(|item| item.value() == value)
            .map(IndexPath::new)
    }
}

pub use crate::searchable_list::{SearchableGroup as SelectGroup, SearchableVec};

impl<I: SelectItem> SelectDelegate for SearchableVec<I> {
    type Item = I;

    fn items_count(&self, _: usize) -> usize {
        self.matched_items().len()
    }

    fn item(&self, ix: IndexPath) -> Option<&Self::Item> {
        self.matched_items().get(ix.row)
    }

    fn position<V>(&self, value: &V) -> Option<IndexPath>
    where
        Self::Item: SelectItem<Value = V>,
        V: PartialEq,
    {
        self.find_position(value, |item| item.value())
    }

    fn perform_search(
        &mut self,
        query: &str,
        _: &mut Window,
        _: &mut Context<SelectState<Self>>,
    ) -> Task<()> {
        self.filter_items(|item| item.matches(query));
        Task::ready(())
    }
}

impl<I: SelectItem> SelectDelegate for SearchableVec<SelectGroup<I>> {
    type Item = I;

    fn sections_count(&self, _: &App) -> usize {
        self.matched_items().len()
    }

    fn section(&self, section: usize) -> Option<AnyElement> {
        self.matched_items()
            .get(section)
            .map(|group| group.title.clone().into_any_element())
    }

    fn items_count(&self, section: usize) -> usize {
        self.matched_items()
            .get(section)
            .map_or(0, |group| group.items.len())
    }

    fn item(&self, ix: IndexPath) -> Option<&Self::Item> {
        self.matched_items().get(ix.section)?.items.get(ix.row)
    }

    fn position<V>(&self, value: &V) -> Option<IndexPath>
    where
        Self::Item: SelectItem<Value = V>,
        V: PartialEq,
    {
        self.find_group_position(value, |group| group.items.as_slice(), |item| item.value())
    }

    fn perform_search(
        &mut self,
        query: &str,
        _: &mut Window,
        _: &mut Context<SelectState<Self>>,
    ) -> Task<()> {
        let normalized = query.to_lowercase();
        self.filter_groups(
            |group| group.title.to_lowercase().contains(&normalized),
            |group| {
                group.retain_items(|item| item.matches(query));
                !group.items.is_empty()
            },
        );
        Task::ready(())
    }
}

#[derive(Clone)]
pub(crate) struct SelectItemAdapter<I: SelectItem>(I);

impl<I: SelectItem> SearchableListItem for SelectItemAdapter<I> {
    type Value = ();

    fn title(&self) -> SharedString {
        self.0.title()
    }

    fn display_title(&self) -> Option<AnyElement> {
        self.0.display_title()
    }

    fn render(&self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        self.0.render(window, cx)
    }

    fn value(&self) -> &Self::Value {
        static UNIT: () = ();
        &UNIT
    }

    fn matches(&self, query: &str) -> bool {
        self.0.matches(query)
    }
}

/// Bridges the legacy Select delegate into the shared searchable-list delegate.
pub(crate) struct SelectDelegateAdapter<D: SelectDelegate + 'static> {
    delegate: D,
    state: WeakEntity<SelectState<D>>,
    items: Vec<(IndexPath, SelectItemAdapter<D::Item>)>,
}

impl<D: SelectDelegate + 'static> SelectDelegateAdapter<D> {
    fn new(delegate: D, state: WeakEntity<SelectState<D>>) -> Self {
        Self {
            delegate,
            state,
            items: Vec::new(),
        }
    }

    fn refresh_items(&mut self, cx: &App) {
        let mut items = Vec::new();
        for section in 0..self.delegate.sections_count(cx) {
            for row in 0..self.delegate.items_count(section) {
                let ix = IndexPath::default().section(section).row(row);
                if let Some(item) = self.delegate.item(ix) {
                    items.push((ix, SelectItemAdapter(item.clone())));
                }
            }
        }
        self.items = items;
    }

    fn source_item(&self, ix: IndexPath) -> Option<&D::Item> {
        self.delegate.item(ix)
    }
}

impl<D: SelectDelegate + 'static> SearchableListDelegate for SelectDelegateAdapter<D> {
    type Item = SelectItemAdapter<D::Item>;

    fn sections_count(&self, cx: &App) -> usize {
        self.delegate.sections_count(cx)
    }

    fn section(&self, section: usize) -> Option<AnyElement> {
        self.delegate.section(section)
    }

    fn items_count(&self, section: usize) -> usize {
        self.delegate.items_count(section)
    }

    fn item(&self, ix: IndexPath) -> Option<&Self::Item> {
        self.items
            .iter()
            .find_map(|(item_ix, item)| (*item_ix == ix).then_some(item))
    }

    fn position<V>(&self, _value: &V) -> Option<IndexPath>
    where
        Self::Item: SearchableListItem<Value = V>,
        V: PartialEq,
    {
        None
    }

    fn perform_search_with_context<P: 'static>(
        &mut self,
        query: &str,
        window: &mut Window,
        cx: &mut Context<P>,
    ) -> Task<()> {
        let Some(state) = self.state.upgrade() else {
            return Task::ready(());
        };

        let search = state.update(cx, |_, cx| self.delegate.perform_search(query, window, cx));
        let state = state.downgrade();
        cx.spawn_in(window, async move |_, window| {
            search.await;
            _ = state.update_in(window, |state, _, cx| {
                state.state.list.update(cx, |list, list_cx| {
                    list.delegate_mut().delegate.refresh_items(list_cx);
                    list_cx.notify();
                });
            });
        })
    }

    fn is_item_checked(
        &self,
        ix: IndexPath,
        _: &Self::Item,
        selection: &[(IndexPath, Self::Item)],
        _: &App,
    ) -> bool {
        selection.iter().any(|(selected_ix, _)| *selected_ix == ix)
    }
}

/// Events emitted by the [`SelectState`].
pub enum SelectEvent<D: SelectDelegate + 'static> {
    Confirm(Option<<D::Item as SelectItem>::Value>),
}

struct SelectOptions {
    style: StyleRefinement,
    size: Size,
    icon: Option<Icon>,
    cleanable: bool,
    placeholder: Option<SharedString>,
    title_prefix: Option<SharedString>,
    search_placeholder: Option<SharedString>,
    empty: Option<Box<dyn Fn(&mut Window, &App) -> Option<AnyElement> + 'static>>,
    menu_width: Length,
    disabled: bool,
    appearance: bool,
}

impl Default for SelectOptions {
    fn default() -> Self {
        Self {
            style: StyleRefinement::default(),
            size: Size::default(),
            icon: None,
            cleanable: false,
            placeholder: None,
            title_prefix: None,
            empty: None,
            menu_width: Length::Auto,
            disabled: false,
            appearance: true,
            search_placeholder: None,
        }
    }
}

/// State of the [`Select`].
pub struct SelectState<D: SelectDelegate + 'static> {
    pub(crate) state: SearchableListState<SelectDelegateAdapter<D>>,
    searchable: bool,
    icon: Option<Icon>,
    title_prefix: Option<SharedString>,
}

/// A Select element.
#[derive(IntoElement)]
pub struct Select<D: SelectDelegate + 'static> {
    id: ElementId,
    state: Entity<SelectState<D>>,
    options: SelectOptions,
}

impl<D> SelectState<D>
where
    D: SelectDelegate + 'static,
{
    /// Create a new Select state.
    pub fn new(
        delegate: D,
        selected_index: Option<IndexPath>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let weak = cx.entity().downgrade();
        let weak_confirm = weak.clone();
        let weak_cancel = weak.clone();
        let weak_empty = weak.clone();

        let mut delegate = SelectDelegateAdapter::new(delegate, weak);
        delegate.refresh_items(cx);

        let state = SearchableListState::new_with_empty(
            delegate,
            selected_index.into_iter().collect(),
            move |selected_index, _secondary, window, cx| {
                cx.defer_in(window, {
                    let weak_confirm = weak_confirm.clone();
                    move |list_state, window, cx| {
                        let selection = selected_index
                            .and_then(|ix| list_state.delegate().delegate.item(ix).cloned())
                            .map(|item| (selected_index.unwrap(), item))
                            .into_iter()
                            .collect::<Vec<_>>();

                        let new_selection = weak_confirm.update(cx, |this, cx| {
                            this.state.selection = selection;
                            let value = this
                                .state
                                .selection
                                .first()
                                .map(|(_, item)| item.0.value().clone());

                            cx.emit(SelectEvent::Confirm(value));
                            this.state.open = false;
                            this.focus(window, cx);
                            cx.notify();

                            this.state.selection.clone()
                        });

                        if let Ok(new_selection) = new_selection {
                            list_state
                                .delegate_mut()
                                .update_selection_snapshot(new_selection);
                        }
                    }
                });
            },
            move |_final_selected_index, window, cx| {
                cx.defer_in(window, {
                    let weak_cancel = weak_cancel.clone();
                    move |list_state, window, cx| {
                        let committed_ix = weak_cancel.upgrade().and_then(|entity| {
                            entity.read(cx).state.selection.first().map(|(ix, _)| *ix)
                        });

                        list_state.set_selected_index(committed_ix, window, cx);
                        _ = weak_cancel.update(cx, |this, cx| {
                            this.state.open = false;
                            this.focus(window, cx);
                        });
                    }
                });
            },
            Some(Box::new(move |window, cx| {
                weak_empty.upgrade().and_then(|entity| {
                    entity
                        .read(cx)
                        .state
                        .empty
                        .as_ref()
                        .and_then(|f| f(window, cx))
                })
            })),
            Self::on_blur,
            window,
            cx,
        );

        Self {
            state,
            searchable: false,
            icon: None,
            title_prefix: None,
        }
    }

    /// Sets whether the dropdown menu is searchable, default is `false`.
    ///
    /// When `true`, there will be a search input at the top of the dropdown menu.
    pub fn searchable(mut self, searchable: bool) -> Self {
        self.searchable = searchable;
        self
    }

    /// Set the selected index for the select.
    pub fn set_selected_index(
        &mut self,
        selected_index: Option<IndexPath>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.state.list.update(cx, |list, cx| {
            list._set_selected_index(selected_index, window, cx);
        });

        let item = selected_index
            .and_then(|ix| self.state.list.read(cx).delegate().delegate.source_item(ix))
            .cloned();
        self.state.selection = match (selected_index, item) {
            (Some(ix), Some(item)) => vec![(ix, SelectItemAdapter(item))],
            _ => vec![],
        };
        self.state.sync_snapshot(cx);
    }

    /// Set selected value for the select.
    ///
    /// This method will to get position from delegate and set selected index.
    ///
    /// If the value is not found, the None will be sets.
    pub fn set_selected_value(
        &mut self,
        selected_value: &<D::Item as SelectItem>::Value,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) where
        <D::Item as SelectItem>::Value: PartialEq,
    {
        self.state.list.update(cx, |list, cx| {
            if !list.query_input.read(cx).value().is_empty() {
                list.set_query("", window, cx);
            }
        });

        let selected_index = self
            .state
            .list
            .read(cx)
            .delegate()
            .delegate
            .delegate
            .position(selected_value);
        self.set_selected_index(selected_index, window, cx);
    }

    /// Set the items for the select state.
    pub fn set_items(&mut self, items: D, _: &mut Window, cx: &mut Context<Self>) {
        self.state.list.update(cx, |list, list_cx| {
            let delegate = &mut list.delegate_mut().delegate;
            delegate.delegate = items;
            delegate.refresh_items(list_cx);
        });
    }

    /// Get the selected index of the select.
    pub fn selected_index(&self, cx: &App) -> Option<IndexPath> {
        self.state.list.read(cx).selected_index()
    }

    /// Get the selected value of the select.
    pub fn selected_value(&self) -> Option<&<D::Item as SelectItem>::Value> {
        self.state.selection.first().map(|(_, item)| item.0.value())
    }

    /// Focus the select input.
    pub fn focus(&self, window: &mut Window, cx: &mut App) {
        self.state.focus_handle.focus(window, cx);
    }

    fn on_blur(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // When the select and dropdown menu are both not focused, close the dropdown menu.
        if self.state.list.read(cx).is_focused(window, cx)
            || self.state.focus_handle.is_focused(window)
        {
            return;
        }

        let committed_ix = self.state.selection.first().map(|(ix, _)| *ix);
        if self.selected_index(cx) != committed_ix {
            self.state.list.update(cx, |list, cx| {
                list.set_selected_index(committed_ix, window, cx);
            });
        }

        self.state.open = false;
        cx.notify();
    }

    fn up(&mut self, _: &SelectUp, window: &mut Window, cx: &mut Context<Self>) {
        if self.state.disabled {
            cx.propagate();
            return;
        }
        if !self.state.open {
            self.state.open = true;
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
            self.state.open = true;
        }

        self.state.list.focus_handle(cx).focus(window, cx);
        cx.propagate();
    }

    fn enter(&mut self, _: &Confirm, window: &mut Window, cx: &mut Context<Self>) {
        if self.state.disabled {
            cx.propagate();
            return;
        }
        // Propagate the event to the parent view, for example to the Dialog to support ENTER to confirm.
        cx.propagate();

        if !self.state.open {
            self.state.open = true;
            cx.notify();
        }

        self.state.list.focus_handle(cx).focus(window, cx);
    }

    fn toggle_menu(&mut self, _: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();

        self.state.open = !self.state.open;
        if self.state.open {
            self.state.list.focus_handle(cx).focus(window, cx);
        }
        cx.notify();
    }

    fn escape(&mut self, _: &Cancel, window: &mut Window, cx: &mut Context<Self>) {
        if !self.state.open {
            cx.propagate();
            return;
        }

        cx.stop_propagation();
        self.state.open = false;
        self.focus(window, cx);
        cx.notify();
    }

    fn clean(&mut self, _: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        self.set_selected_index(None, window, cx);
        cx.emit(SelectEvent::Confirm(None));
    }

    /// Returns the title element for the select input.
    fn display_title(&mut self, _: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let default_title = div().text_color(cx.theme().muted_foreground).child(
            self.state
                .placeholder
                .clone()
                .unwrap_or_else(|| t!("Select.placeholder").into()),
        );

        let Some(selected_index) = &self.selected_index(cx) else {
            return default_title;
        };

        let Some(title) = self
            .state
            .list
            .read(cx)
            .delegate()
            .delegate
            .source_item(*selected_index)
            .map(|item| {
                if let Some(el) = item.display_title() {
                    el
                } else {
                    if let Some(prefix) = self.title_prefix.as_ref() {
                        format!("{}{}", prefix, item.title()).into_any_element()
                    } else {
                        item.title().into_any_element()
                    }
                }
            })
        else {
            return default_title;
        };

        div()
            .when(self.state.disabled, |this| {
                this.text_color(cx.theme().muted_foreground)
            })
            .child(title)
    }
}

impl<D> Render for SelectState<D>
where
    D: SelectDelegate + 'static,
{
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let searchable = self.searchable;
        let is_focused = self.state.focus_handle.is_focused(window);
        let show_clean = self.state.cleanable && self.selected_index(cx).is_some();
        let bounds = self.state.bounds;
        let allow_open = !(self.state.open || self.state.disabled);
        let outline_visible = self.state.open || is_focused && !self.state.disabled;
        let tokens = FlyoutTokens::sized(self.state.size, cx);
        let surface_ctx = SurfaceContext::new(cx);
        let base_width = bounds.size.width.into();
        let base_height = bounds.size.height.into();
        let rem_size = window.rem_size();
        let menu_width = match self.state.menu_width {
            Length::Auto => bounds.size.width + px(2.),
            Length::Definite(width) => width.to_pixels(base_width, rem_size),
        };
        let menu_height = DefiniteLength::Absolute(AbsoluteLength::Rems(rems(20.)))
            .to_pixels(base_height, rem_size);
        let placement = select_menu_placement(
            bounds,
            menu_height,
            window.viewport_size().height,
            POPUP_MARGIN,
            POPUP_GAP,
        );

        let menu_anchor = anchored()
            .anchor(placement.anchor)
            .position(placement.position)
            .snap_to_window_with_margin(POPUP_MARGIN);

        let size = self.state.size;
        self.state.list.update(cx, |list, cx| {
            list.set_searchable(searchable, cx);
            list.delegate_mut().size = size;
        });

        div()
            .size_full()
            .relative()
            .child(
                div()
                    .id("input")
                    .relative()
                    .flex()
                    .items_center()
                    .justify_between()
                    .border_1()
                    .border_color(cx.theme().transparent)
                    .when(self.state.appearance, |this| {
                        this.bg(cx.theme().background)
                            .border_color(cx.theme().input)
                            .rounded(cx.theme().radius)
                            .when(cx.theme().shadow, |this| this.shadow_xs())
                    })
                    .map(|this| {
                        if self.state.disabled {
                            this.shadow_none()
                        } else {
                            this
                        }
                    })
                    .overflow_hidden()
                    .input_size(self.state.size)
                    .input_text_size(self.state.size)
                    .refine_style(&self.state.style)
                    .when(outline_visible, |this| this.focused_border(cx))
                    .when(allow_open, |this| {
                        this.on_click(cx.listener(Self::toggle_menu))
                    })
                    .child(
                        h_flex()
                            .id("inner")
                            .w_full()
                            .items_center()
                            .justify_between()
                            .gap_1()
                            .child(
                                div()
                                    .id("title")
                                    .w_full()
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .truncate()
                                    .child(self.display_title(window, cx)),
                            )
                            .when(show_clean, |this| {
                                this.child(clear_button(cx).map(|this| {
                                    if self.state.disabled {
                                        this.disabled(true)
                                    } else {
                                        this.on_click(cx.listener(Self::clean))
                                    }
                                }))
                            })
                            .when(!show_clean, |this| {
                                let icon = match self.icon.clone() {
                                    Some(icon) => icon,
                                    None => Icon::new(IconName::ChevronDown),
                                };

                                this.child(icon.xsmall().text_color(match self.state.disabled {
                                    true => cx.theme().muted_foreground.opacity(0.5),
                                    false => cx.theme().muted_foreground,
                                }))
                            }),
                    )
                    .on_prepaint({
                        let state = cx.entity();
                        move |bounds, _, cx| state.update(cx, |r, _| r.state.bounds = bounds)
                    }),
            )
            .map(|this| {
                let motion = cx.theme().motion.clone();
                let reduced_motion = crate::animation::reduced_motion(cx);
                let presence = flyout_presence(
                    SharedString::from(format!(
                        "select-popup-presence-{}",
                        cx.entity().entity_id()
                    )),
                    self.state.open,
                    PresenceOptions::default(),
                    window,
                    cx,
                );
                if !presence.should_render() {
                    return this;
                }

                // Anchor::Top* places the menu below the trigger (see
                // select_menu_placement), which is the same case popover.rs and
                // hover_card.rs give -1.0 — match them so a flyout opening
                // downward always slides the same way.
                let vertical_direction = if matches!(
                    placement.anchor,
                    Anchor::TopLeft | Anchor::TopRight | Anchor::TopCenter
                ) {
                    -1.0
                } else {
                    1.0
                };
                let slide = FlyoutSlide::vertical(vertical_direction);

                this.child(
                    deferred(
                        menu_anchor.child(
                            div()
                                .occlude()
                                .w(menu_width)
                                .child(
                                    SurfacePreset::flyout()
                                        .with_radius(tokens.radius)
                                        .wrap_with_bounds(
                                            v_flex().occlude().child(
                                                List::new(&self.state.list)
                                                    .when_some(
                                                        self.state.search_placeholder.clone(),
                                                        |this, placeholder| {
                                                            this.search_placeholder(placeholder)
                                                        },
                                                    )
                                                    .with_size(self.state.size)
                                                    .max_h(rems(20.))
                                                    .paddings(Edges::all(tokens.inset)),
                                            ),
                                            menu_width,
                                            menu_height,
                                            window,
                                            cx,
                                            surface_ctx,
                                        ),
                                )
                                .on_mouse_down_out(cx.listener(|this, _, window, cx| {
                                    this.escape(&Cancel, window, cx);
                                }))
                                .map(|el| {
                                    flyout_motion(
                                        "select",
                                        presence,
                                        slide,
                                        &motion,
                                        reduced_motion,
                                        el,
                                    )
                                }),
                        ),
                    )
                    .with_priority(1),
                )
            })
    }
}

impl<D> Select<D>
where
    D: SelectDelegate + 'static,
{
    pub fn new(state: &Entity<SelectState<D>>) -> Self {
        Self {
            id: ("select", state.entity_id()).into(),
            state: state.clone(),
            options: SelectOptions::default(),
        }
    }

    /// Set the width of the dropdown menu, default: Length::Auto
    pub fn menu_width(mut self, width: impl Into<Length>) -> Self {
        self.options.menu_width = width.into();
        self
    }

    /// Set the placeholder for display when select value is empty.
    pub fn placeholder(mut self, placeholder: impl Into<SharedString>) -> Self {
        self.options.placeholder = Some(placeholder.into());
        self
    }

    /// Set the right icon for the select input, instead of the default arrow icon.
    pub fn icon(mut self, icon: impl Into<Icon>) -> Self {
        self.options.icon = Some(icon.into());
        self
    }

    /// Set title prefix for the select.
    ///
    /// e.g.: Country: United States
    ///
    /// You should set the label is `Country: `
    pub fn title_prefix(mut self, prefix: impl Into<SharedString>) -> Self {
        self.options.title_prefix = Some(prefix.into());
        self
    }

    /// Set whether to show the clear button when the input field is not empty, default is false.
    pub fn cleanable(mut self, cleanable: bool) -> Self {
        self.options.cleanable = cleanable;
        self
    }

    /// Sets the placeholder text for the search input.
    pub fn search_placeholder(mut self, placeholder: impl Into<SharedString>) -> Self {
        self.options.search_placeholder = Some(placeholder.into());
        self
    }

    /// Set the disable state for the select.
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.options.disabled = disabled;
        self
    }

    /// Set the element to display when the Select list is empty.
    ///
    /// This legacy form consumes `element` on its first render. Use [`Self::empty_with`] for
    /// content that must render more than once.
    pub fn empty(mut self, element: impl IntoElement) -> Self {
        let element = Rc::new(RefCell::new(Some(element.into_any_element())));
        self.options.empty = Some(Box::new(move |_, _| element.borrow_mut().take()));
        self
    }

    /// Set a reusable builder for the empty-state element.
    pub fn empty_with<E: IntoElement + 'static>(
        mut self,
        builder: impl Fn(&mut Window, &App) -> E + 'static,
    ) -> Self {
        self.options.empty = Some(Box::new(move |window, cx| {
            Some(builder(window, cx).into_any_element())
        }));
        self
    }

    /// Set the appearance of the select, if false the select input will no border, background.
    pub fn appearance(mut self, appearance: bool) -> Self {
        self.options.appearance = appearance;
        self
    }
}

impl<D> Sizable for Select<D>
where
    D: SelectDelegate + 'static,
{
    fn with_size(mut self, size: impl Into<Size>) -> Self {
        self.options.size = size.into();
        self
    }
}

impl<D> EventEmitter<SelectEvent<D>> for SelectState<D> where D: SelectDelegate + 'static {}
impl<D> EventEmitter<DismissEvent> for SelectState<D> where D: SelectDelegate + 'static {}
impl<D> Focusable for SelectState<D>
where
    D: SelectDelegate + 'static,
{
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        if self.state.open {
            self.state.list.focus_handle(cx)
        } else {
            self.state.focus_handle.clone()
        }
    }
}

impl<D> Styled for Select<D>
where
    D: SelectDelegate + 'static,
{
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.options.style
    }
}

impl<D> RenderOnce for Select<D>
where
    D: SelectDelegate + 'static,
{
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let disabled = self.options.disabled;
        let focus_handle = self.state.read(cx).state.focus_handle.clone();
        let options = self.options;
        self.state.update(cx, |this, _| {
            this.state.style = options.style;
            this.state.size = options.size;
            this.state.cleanable = options.cleanable;
            this.state.placeholder = options.placeholder;
            this.state.search_placeholder = options.search_placeholder;
            this.state.menu_width = options.menu_width;
            this.state.disabled = options.disabled;
            this.state.appearance = options.appearance;
            this.icon = options.icon;
            this.title_prefix = options.title_prefix;
            this.state.empty = options.empty;
            if disabled {
                this.state.open = false;
            }
        });

        div()
            .id(self.id.clone())
            .key_context(CONTEXT)
            .when(!disabled, |this| {
                this.track_focus(&focus_handle.tab_stop(true))
            })
            .on_action(window.listener_for(&self.state, SelectState::up))
            .on_action(window.listener_for(&self.state, SelectState::down))
            .on_action(window.listener_for(&self.state, SelectState::enter))
            .on_action(window.listener_for(&self.state, SelectState::escape))
            .size_full()
            .child(self.state)
    }
}

#[cfg(test)]
mod tests {
    use std::{cell::Cell, rc::Rc, time::Duration};

    use super::*;
    use gpui::{AppContext, TestAppContext, VisualTestContext, size};

    use crate::list::ListDelegate;

    struct SelectEscapeHarness {
        state: Entity<SelectState<Vec<&'static str>>>,
        parent_cancels: Rc<Cell<usize>>,
    }

    struct SelectEmptyHarness {
        state: Entity<SelectState<Vec<&'static str>>>,
        render_count: Rc<Cell<usize>>,
    }

    struct SelectLegacyEmptyHarness {
        state: Entity<SelectState<Vec<&'static str>>>,
    }

    #[derive(Clone)]
    struct LegacyValue;

    #[derive(Clone)]
    struct LegacyItem {
        title: SharedString,
        value: LegacyValue,
    }

    impl SelectItem for LegacyItem {
        type Value = LegacyValue;

        fn title(&self) -> SharedString {
            self.title.clone()
        }

        fn value(&self) -> &Self::Value {
            &self.value
        }
    }

    struct LegacyDelegate {
        item: LegacyItem,
    }

    impl SelectDelegate for LegacyDelegate {
        type Item = LegacyItem;

        fn items_count(&self, _: usize) -> usize {
            1
        }

        fn item(&self, ix: IndexPath) -> Option<&Self::Item> {
            (ix.row == 0).then_some(&self.item)
        }

        fn position<V>(&self, _: &V) -> Option<IndexPath>
        where
            Self::Item: SelectItem<Value = V>,
            V: PartialEq,
        {
            None
        }

        fn perform_search(
            &mut self,
            _: &str,
            _: &mut Window,
            _: &mut Context<SelectState<Self>>,
        ) -> Task<()> {
            Task::ready(())
        }
    }

    #[derive(Clone)]
    struct BorrowedItem<'a>(&'a str);

    impl<'a> SelectItem for BorrowedItem<'a> {
        type Value = &'a str;

        fn title(&self) -> SharedString {
            self.0.into()
        }

        fn value(&self) -> &Self::Value {
            &self.0
        }
    }

    struct BorrowedDelegate<'a> {
        item: BorrowedItem<'a>,
    }

    impl<'a> SelectDelegate for BorrowedDelegate<'a> {
        type Item = BorrowedItem<'a>;

        fn items_count(&self, _: usize) -> usize {
            1
        }

        fn item(&self, ix: IndexPath) -> Option<&Self::Item> {
            (ix.row == 0).then_some(&self.item)
        }

        fn position<V>(&self, _: &V) -> Option<IndexPath>
        where
            Self::Item: SelectItem<Value = V>,
            V: PartialEq,
        {
            None
        }
    }

    fn assert_select_delegate<D: SelectDelegate>() {}

    #[allow(dead_code)]
    fn borrowed_select_delegate_impl_is_source_compatible<'a>(delegate: BorrowedDelegate<'a>) {
        assert_select_delegate::<BorrowedDelegate<'a>>();
        let _ = delegate;
    }

    struct BorrowedEmpty<'a>(&'a str);

    impl<'a> IntoElement for BorrowedEmpty<'a> {
        type Element = &'static str;

        fn into_element(self) -> Self::Element {
            let _ = self.0;
            "No Data"
        }
    }

    #[allow(dead_code)]
    fn borrowed_select_empty_is_source_compatible<D: SelectDelegate + 'static>(
        select: Select<D>,
        text: &str,
    ) {
        let _ = select.empty(BorrowedEmpty(text));
    }

    #[derive(Clone)]
    struct DelayedItem {
        title: &'static str,
        render_count: Rc<Cell<usize>>,
    }

    impl SelectItem for DelayedItem {
        type Value = &'static str;

        fn title(&self) -> SharedString {
            self.title.into()
        }

        fn value(&self) -> &Self::Value {
            &self.title
        }

        fn render(&self, _: &mut Window, _: &mut App) -> impl IntoElement {
            self.render_count.set(self.render_count.get() + 1);
            self.title.into_element()
        }
    }

    #[derive(Clone)]
    struct CaseSensitiveItem(&'static str);

    impl SelectItem for CaseSensitiveItem {
        type Value = &'static str;

        fn title(&self) -> SharedString {
            self.0.into()
        }

        fn value(&self) -> &Self::Value {
            &self.0
        }

        fn matches(&self, query: &str) -> bool {
            self.0.contains(query)
        }
    }

    struct DelayedDelegate {
        ready: Rc<Cell<bool>>,
        old: DelayedItem,
        new: DelayedItem,
    }

    impl SelectDelegate for DelayedDelegate {
        type Item = DelayedItem;

        fn items_count(&self, _: usize) -> usize {
            1
        }

        fn item(&self, ix: IndexPath) -> Option<&Self::Item> {
            (ix.row == 0).then_some(if self.ready.get() {
                &self.new
            } else {
                &self.old
            })
        }

        fn position<V>(&self, _: &V) -> Option<IndexPath>
        where
            Self::Item: SelectItem<Value = V>,
            V: PartialEq,
        {
            None
        }

        fn perform_search(
            &mut self,
            _: &str,
            _: &mut Window,
            cx: &mut Context<SelectState<Self>>,
        ) -> Task<()> {
            let ready = self.ready.clone();
            cx.spawn(async move |_, cx| {
                cx.background_executor().timer(Duration::ZERO).await;
                ready.set(true);
            })
        }
    }

    impl Render for SelectEscapeHarness {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let parent_cancels = self.parent_cancels.clone();
            div()
                .on_action(move |_: &Cancel, _, _| {
                    parent_cancels.set(parent_cancels.get() + 1);
                })
                .child(Select::new(&self.state))
        }
    }

    impl Render for SelectEmptyHarness {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let render_count = self.render_count.clone();
            Select::new(&self.state).empty_with(move |_, _| {
                render_count.set(render_count.get() + 1);
                div().child("No Data")
            })
        }
    }

    impl Render for SelectLegacyEmptyHarness {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            List::new(&self.state.read(cx).state.list)
        }
    }

    fn new_select_state(
        cx: &mut TestAppContext,
    ) -> (
        gpui::WindowHandle<SelectState<Vec<&'static str>>>,
        VisualTestContext,
    ) {
        let window = cx.update(|cx| {
            crate::init(cx);
            cx.open_window(Default::default(), |window, cx| {
                cx.new(|cx| {
                    SelectState::new(
                        vec!["Alpha", "Beta", "Gamma"],
                        Some(IndexPath::default().row(1)),
                        window,
                        cx,
                    )
                })
            })
            .unwrap()
        });
        let visual_cx = VisualTestContext::from_window(window.into(), cx);
        (window, visual_cx)
    }

    #[gpui::test]
    fn legacy_select_delegate_without_partial_eq_compiles(cx: &mut TestAppContext) {
        cx.update(|cx| {
            crate::init(cx);
            cx.open_window(Default::default(), |window, cx| {
                cx.new(|cx| {
                    SelectState::new(
                        LegacyDelegate {
                            item: LegacyItem {
                                title: "Legacy".into(),
                                value: LegacyValue,
                            },
                        },
                        None,
                        window,
                        cx,
                    )
                })
            })
            .unwrap()
        });
    }

    #[gpui::test]
    fn legacy_async_search_refreshes_shared_adapter_items(cx: &mut TestAppContext) {
        cx.update(crate::init);
        let cx = cx.add_empty_window();
        let ready = Rc::new(Cell::new(false));
        let render_count = Rc::new(Cell::new(0));
        let state = cx.update(|window, cx| {
            cx.new(|cx| {
                SelectState::new(
                    DelayedDelegate {
                        ready: ready.clone(),
                        old: DelayedItem {
                            title: "old",
                            render_count: render_count.clone(),
                        },
                        new: DelayedItem {
                            title: "new",
                            render_count: render_count.clone(),
                        },
                    },
                    None,
                    window,
                    cx,
                )
            })
        });
        let list = cx.update(|_, cx| state.read(cx).state.list.clone());

        cx.update(|window, cx| {
            list.update(cx, |list, cx| list.set_query("new", window, cx));
        });
        cx.run_until_parked();

        cx.update(|_, cx| {
            assert!(ready.get());
            let list = list.read(cx);
            assert_eq!(list.delegate().delegate.items_count(0), 1);
            assert_eq!(
                list.delegate()
                    .delegate
                    .item(IndexPath::new(0))
                    .unwrap()
                    .title(),
                "new"
            );
        });
        cx.draw(point(px(0.), px(0.)), size(px(400.), px(400.)), |_, _| {
            List::new(&list).into_any_element()
        });
        assert!(render_count.get() > 0);
    }

    #[gpui::test]
    fn legacy_group_search_preserves_original_query(cx: &mut TestAppContext) {
        cx.update(crate::init);
        let cx = cx.add_empty_window();
        let state = cx.update(|window, cx| {
            cx.new(|cx| {
                SelectState::new(
                    SearchableVec::new(vec![
                        SelectGroup::new("Other").item(CaseSensitiveItem("Rust")),
                    ]),
                    None,
                    window,
                    cx,
                )
            })
        });
        let list = cx.update(|_, cx| state.read(cx).state.list.clone());

        cx.update(|window, cx| {
            list.update(cx, |list, cx| list.set_query("R", window, cx));
        });
        cx.run_until_parked();

        cx.update(|_, cx| {
            let list = list.read(cx);
            assert_eq!(list.delegate().delegate.sections_count(cx), 1);
            assert_eq!(list.delegate().delegate.items_count(0), 1);
            assert_eq!(
                list.delegate()
                    .delegate
                    .item(IndexPath::new(0))
                    .unwrap()
                    .0
                    .0,
                "Rust"
            );
        });
    }

    #[test]
    fn test_select_menu_placement() {
        let trigger = |top| Bounds {
            origin: point(px(40.), px(top)),
            size: gpui::size(px(120.), px(32.)),
        };

        let top = select_menu_placement(trigger(8.), px(120.), px(600.), px(8.), px(6.));
        assert_eq!(top.anchor, Anchor::TopLeft);
        assert_eq!(top.position, point(px(40.), px(46.)));

        let middle = select_menu_placement(trigger(240.), px(120.), px(600.), px(8.), px(6.));
        assert_eq!(middle.anchor, Anchor::TopLeft);
        assert_eq!(middle.position, point(px(40.), px(278.)));

        let bottom = select_menu_placement(trigger(520.), px(120.), px(600.), px(8.), px(6.));
        assert_eq!(bottom.anchor, Anchor::BottomLeft);
        assert_eq!(bottom.position, point(px(40.), px(514.)));

        let margin = select_menu_placement(trigger(430.), px(130.), px(600.), px(8.), px(6.));
        assert_eq!(margin.anchor, Anchor::BottomLeft);
    }

    #[gpui::test]
    fn test_select_hover_moves_active_row_without_committing(cx: &mut TestAppContext) {
        let (window, mut cx) = new_select_state(cx);
        let state = window.root(&mut cx).unwrap();
        let list = state.read_with(&cx, |state, _| state.state.list.clone());

        list.update_in(&mut cx, |list, window, cx| {
            list.select_item_on_hover(IndexPath::default().row(0), window, cx);
            let active = list
                .delegate_mut()
                .render_item(IndexPath::default().row(0), window, cx)
                .unwrap();
            assert!(!active.is_checked());
            let committed = list
                .delegate_mut()
                .render_item(IndexPath::default().row(1), window, cx)
                .unwrap();
            assert!(committed.is_checked());
            assert_eq!(list.selected_index(), Some(IndexPath::default().row(0)));
        });

        state.read_with(&cx, |state, cx| {
            assert_eq!(state.selected_index(cx), Some(IndexPath::default().row(0)));
            assert_eq!(
                state.state.selection().first().map(|(ix, _)| *ix),
                Some(IndexPath::default().row(1))
            );
            assert_eq!(state.selected_value(), Some(&"Beta"));
        });
    }

    #[gpui::test]
    fn test_select_blur_restores_committed_selection(cx: &mut TestAppContext) {
        let (window, mut cx) = new_select_state(cx);
        let state = window.root(&mut cx).unwrap();

        state.update_in(&mut cx, |state, window, cx| {
            state.state.open = true;
            state.state.list.update(cx, |list, cx| {
                list._set_selected_index(Some(IndexPath::default().row(0)), window, cx);
            });
            state.on_blur(window, cx);
        });

        state.read_with(&cx, |state, cx| {
            assert_eq!(
                state.state.selection().first().map(|(ix, _)| *ix),
                Some(IndexPath::default().row(1))
            );
            assert_eq!(state.selected_index(cx), Some(IndexPath::default().row(1)));
            assert!(!state.state.open);
        });
    }

    #[gpui::test]
    fn test_select_confirm_commits_transient_selection(cx: &mut TestAppContext) {
        let (window, mut cx) = new_select_state(cx);
        let state = window.root(&mut cx).unwrap();

        state.update_in(&mut cx, |state, window, cx| {
            state.state.open = true;
            state.state.list.update(cx, |list, cx| {
                let selected_index = Some(IndexPath::default().row(2));
                list._set_selected_index(selected_index, window, cx);
                list.delegate_mut().confirm(false, window, cx);
            });
        });
        cx.run_until_parked();

        state.read_with(&cx, |state, cx| {
            assert_eq!(
                state.state.selection().first().map(|(ix, _)| *ix),
                Some(IndexPath::default().row(2))
            );
            assert_eq!(state.selected_index(cx), Some(IndexPath::default().row(2)));
            assert_eq!(state.selected_value(), Some(&"Gamma"));
            assert!(!state.state.open);
        });
    }

    #[gpui::test]
    fn test_select_escape_closes_once_then_propagates(cx: &mut TestAppContext) {
        let parent_cancels = Rc::new(Cell::new(0));
        let window = cx.update(|cx| {
            crate::init(cx);
            cx.open_window(Default::default(), {
                let parent_cancels = parent_cancels.clone();
                move |window, cx| {
                    let state = cx.new(|cx| {
                        SelectState::new(
                            vec!["Alpha", "Beta", "Gamma"],
                            Some(IndexPath::default()),
                            window,
                            cx,
                        )
                    });
                    cx.new(|_| SelectEscapeHarness {
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

        state.update_in(&mut cx, |state, window, cx| {
            state.state.open = true;
            state.state.list.update(cx, |list, cx| {
                list._set_selected_index(Some(IndexPath::default().row(2)), window, cx);
            });
            state.state.list.focus_handle(cx).focus(window, cx);
            cx.notify();
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));

        cx.dispatch_action(Cancel);
        cx.run_until_parked();
        state.read_with(&cx, |state, cx| {
            assert!(!state.state.open);
            assert_eq!(state.selected_index(cx), Some(IndexPath::default().row(0)));
        });
        cx.update(|window, cx| {
            assert!(state.read(cx).state.focus_handle.is_focused(window));
        });
        assert_eq!(parent_cancels.get(), 0);

        cx.update(|window, cx| window.draw(cx).clear(cx));
        cx.dispatch_action(Cancel);
        cx.run_until_parked();
        assert_eq!(parent_cancels.get(), 1);
    }

    #[gpui::test]
    fn test_select_custom_empty_builder_renders_repeatedly(cx: &mut TestAppContext) {
        let render_count = Rc::new(Cell::new(0));
        let window = cx.update(|cx| {
            crate::init(cx);
            let render_count = render_count.clone();
            cx.open_window(Default::default(), move |window, cx| {
                let state =
                    cx.new(|cx| SelectState::new(Vec::<&'static str>::new(), None, window, cx));
                cx.new(|_| SelectEmptyHarness {
                    state,
                    render_count,
                })
            })
            .unwrap()
        });
        let mut cx = VisualTestContext::from_window(window.into(), cx);
        let harness = window.root(&mut cx).unwrap();
        let state = harness.read_with(&cx, |harness, _| harness.state.clone());
        state.read_with(&cx, |state, _| assert!(state.state.empty.is_some()));

        state.update_in(&mut cx, |state, _, cx| {
            state.state.open = true;
            cx.notify();
        });

        cx.update(|window, cx| window.draw(cx).clear(cx));
        let first_render_count = render_count.get();
        cx.update(|window, cx| window.draw(cx).clear(cx));

        assert!(first_render_count > 0);
        assert!(render_count.get() > first_render_count);
    }

    #[gpui::test]
    fn legacy_empty_element_falls_back_after_first_render(cx: &mut TestAppContext) {
        let window = cx.update(|cx| {
            crate::init(cx);
            cx.open_window(Default::default(), |window, cx| {
                let state =
                    cx.new(|cx| SelectState::new(Vec::<&'static str>::new(), None, window, cx));
                let empty = Select::new(&state)
                    .empty(
                        div()
                            .debug_selector(|| "select-empty-legacy".into())
                            .child("No Data"),
                    )
                    .options
                    .empty;
                assert!(empty.is_some());
                let first = empty.as_ref().unwrap()(window, cx);
                assert!(first.is_some());
                drop(first);
                let second = empty.as_ref().unwrap()(window, cx);
                assert!(second.is_none());
                state.update(cx, |state, _| state.state.empty = empty);
                cx.new(|_| SelectLegacyEmptyHarness { state })
            })
            .unwrap()
        });
        let mut cx = VisualTestContext::from_window(window.into(), cx);
        let harness = window.root(&mut cx).unwrap();
        let state = harness.read_with(&cx, |harness, _| harness.state.clone());

        state.update_in(&mut cx, |state, _, cx| {
            state.state.open = true;
            cx.notify();
        });

        cx.update(|window, cx| window.draw(cx).clear(cx));
        assert!(cx.debug_bounds("select-empty-legacy").is_none());
        assert!(cx.debug_bounds("searchable-list-empty-default").is_some());

        cx.update(|window, cx| window.draw(cx).clear(cx));
        assert!(cx.debug_bounds("select-empty-legacy").is_none());
        assert!(cx.debug_bounds("searchable-list-empty-default").is_some());
    }
}
