use gpui::{
    AnyElement, App, ElementId, Entity, Focusable, InteractiveElement, IntoElement, Length,
    ParentElement, RenderOnce, Role, SharedString, StatefulInteractiveElement, StyleRefinement,
    Styled, Window, div, prelude::FluentBuilder, rems,
};

use crate::{
    Icon, Sizable, Size,
    searchable_list::{SearchableListDelegate, SearchableListItem},
};

use super::{CONTEXT, ComboboxState, ComboboxTriggerCtx};

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
            this.state.empty = empty;
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
