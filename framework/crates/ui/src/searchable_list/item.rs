use gpui::{
    AnyElement, App, ElementId, InteractiveElement as _, IntoElement, ParentElement, RenderOnce,
    Role, StatefulInteractiveElement as _, StyleRefinement, Styled, Window, prelude::FluentBuilder,
};

use crate::{
    ActiveTheme, Disableable, FlyoutTokens, Icon, IconName, Selectable, Sizable, Size, StyleSized,
    StyledExt, flyout_disabled_foreground, flyout_primary_foreground, h_flex,
};

/// A single row element used inside searchable-list dropdowns (Select, ComboBox, MultiComboBox).
///
/// - `selected` — controls the cursor-highlight background (the `List` overwrites this field via
///   `Selectable::selected` to match the keyboard cursor position).
/// - `checked` — controls the visibility of the trailing check icon; set by the adapter based on
///   the current selection state and NOT overwritten by the `List`.
#[derive(IntoElement)]
pub struct SearchableListItemElement {
    id: ElementId,
    size: Size,
    style: StyleRefinement,
    /// Cursor/highlight background (overridden by `List` to the keyboard cursor row).
    selected: bool,
    /// Whether the trailing check icon is shown.
    checked: bool,
    disabled: bool,
    children: Vec<AnyElement>,
    /// The icon drawn at the trailing edge when `checked` is `true`.
    check_icon: Option<Icon>,
}

impl SearchableListItemElement {
    pub fn new(ix: usize) -> Self {
        Self {
            id: ("searchable-list-item", ix).into(),
            size: Size::default(),
            style: StyleRefinement::default(),
            selected: false,
            checked: false,
            disabled: false,
            children: Vec::new(),
            check_icon: Some(Icon::new(IconName::Check)),
        }
    }

    /// Set whether the trailing check icon is visible.
    pub fn checked(mut self, checked: bool) -> Self {
        self.checked = checked;
        self
    }

    /// Override the default check icon.
    pub fn check_icon(mut self, icon: impl Into<Icon>) -> Self {
        self.check_icon = Some(icon.into());
        self
    }

    pub(crate) fn hide_check_icon(mut self) -> Self {
        self.check_icon = None;
        self
    }

    #[cfg(test)]
    pub(crate) fn is_checked(&self) -> bool {
        self.checked
    }
}

impl ParentElement for SearchableListItemElement {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl Disableable for SearchableListItemElement {
    fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
}

impl Selectable for SearchableListItemElement {
    fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    fn is_selected(&self) -> bool {
        self.selected
    }
}

impl Sizable for SearchableListItemElement {
    fn with_size(mut self, size: impl Into<Size>) -> Self {
        self.size = size.into();
        self
    }
}

impl Styled for SearchableListItemElement {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl RenderOnce for SearchableListItemElement {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let tokens = FlyoutTokens::sized(self.size, cx);

        h_flex()
            .id(self.id)
            .relative()
            .gap(tokens.icon_gap)
            .h(tokens.item_height)
            .px(tokens.item_padding_x)
            .rounded(tokens.item_radius)
            .text_size(tokens.label_size)
            .text_color(flyout_primary_foreground(cx))
            .items_center()
            .justify_between()
            .input_text_size(self.size)
            .list_size(self.size)
            .role(Role::ListBoxOption)
            // `selected` is the keyboard cursor state; `checked` is the committed selection.
            .aria_selected(self.checked)
            .aria_disabled(self.disabled)
            .refine_style(&self.style)
            .when(!self.disabled, |this| {
                this.when(!self.selected, |this| {
                    this.hover(|this| this.bg(cx.theme().accent.opacity(0.7)))
                })
            })
            .when(self.selected, |this| this.bg(cx.theme().accent))
            .when(self.disabled, |this| {
                this.cursor_not_allowed()
                    .text_color(flyout_disabled_foreground(cx))
            })
            .child(
                h_flex()
                    .w_full()
                    .items_center()
                    .justify_between()
                    .gap_x_1()
                    .child(h_flex().w_full().items_center().children(self.children))
                    .when_some(self.check_icon, |this, icon| {
                        this.child(
                            icon.xsmall()
                                .text_color(flyout_primary_foreground(cx))
                                .when(!self.checked, |this| this.invisible()),
                        )
                    }),
            )
    }
}
