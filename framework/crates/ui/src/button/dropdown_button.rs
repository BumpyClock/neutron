use std::sync::Arc;

use gpui::{
    App, Context, Corner, Corners, Edges, ElementId, InteractiveElement as _, IntoElement,
    ParentElement, RenderOnce, SharedString, StyleRefinement, Styled, Window, div,
    prelude::FluentBuilder,
};

use crate::{
    ActiveTheme, Disableable, Icon, IconName, Selectable, Sizable, Size, StyledExt as _,
    menu::{DropdownMenu, PopupMenu},
};

use super::{Button, ButtonRounded, ButtonVariant, ButtonVariants};

#[derive(IntoElement)]
pub struct DropdownButton {
    id: ElementId,
    trigger_id: ElementId,
    style: StyleRefinement,
    button: Option<Button>,
    icon: Option<Icon>,
    tooltip: Option<SharedString>,
    menu:
        Option<Box<dyn Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static>>,
    selected: bool,
    disabled: bool,
    // The button props
    bordered: bool,
    compact: bool,
    outline: bool,
    loading: bool,
    variant: ButtonVariant,
    size: Size,
    rounded: ButtonRounded,
    anchor: Corner,
}

impl DropdownButton {
    /// Create a new DropdownButton.
    pub fn new(id: impl Into<ElementId>) -> Self {
        let id = id.into();
        Self {
            trigger_id: ElementId::NamedChild(Arc::new(id.clone()), "menu-trigger".into()),
            id,
            style: StyleRefinement::default(),
            button: None,
            icon: None,
            tooltip: None,
            menu: None,
            selected: false,
            disabled: false,
            bordered: true,
            compact: false,
            outline: false,
            loading: false,
            variant: ButtonVariant::default(),
            size: Size::default(),
            rounded: ButtonRounded::default(),
            anchor: Corner::TopRight,
        }
    }

    /// Set the left button of the dropdown button.
    pub fn button(mut self, button: Button) -> Self {
        self.button = Some(button);
        self
    }

    /// Set the icon for the menu trigger.
    ///
    /// If no left button is set, the dropdown renders as a single icon button.
    pub fn icon(mut self, icon: impl Into<Icon>) -> Self {
        self.icon = Some(icon.into());
        self
    }

    /// Set the tooltip for the menu trigger.
    ///
    /// Icon-only dropdowns should always provide a tooltip.
    pub fn tooltip(mut self, tooltip: impl Into<SharedString>) -> Self {
        self.tooltip = Some(tooltip.into());
        self
    }

    /// Set the dropdown menu of the button.
    pub fn dropdown_menu(
        mut self,
        menu: impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static,
    ) -> Self {
        self.menu = Some(Box::new(menu));
        self
    }

    /// Set the dropdown menu of the button with anchor corner.
    pub fn dropdown_menu_with_anchor(
        mut self,
        anchor: impl Into<Corner>,
        menu: impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static,
    ) -> Self {
        self.menu = Some(Box::new(menu));
        self.anchor = anchor.into();
        self
    }

    /// Set the rounded style of the button.
    pub fn rounded(mut self, rounded: impl Into<ButtonRounded>) -> Self {
        self.rounded = rounded.into();
        self
    }

    /// Set whether the dropdown buttons have borders.
    ///
    /// Borderless split buttons render as two independently rounded controls.
    pub fn bordered(mut self, bordered: bool) -> Self {
        self.bordered = bordered;
        self
    }

    /// Set the button to compact style.
    ///
    /// See also: [`Button::compact`]
    pub fn compact(mut self) -> Self {
        self.compact = true;
        self
    }

    /// Set the button to outline style.
    ///
    /// See also: [`Button::outline`]
    pub fn outline(mut self) -> Self {
        self.outline = true;
        self
    }

    /// Set the button to loading state.
    pub fn loading(mut self, loading: bool) -> Self {
        self.loading = loading;
        self
    }
}

impl Disableable for DropdownButton {
    fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
}

impl Styled for DropdownButton {
    fn style(&mut self) -> &mut gpui::StyleRefinement {
        &mut self.style
    }
}

impl Sizable for DropdownButton {
    fn with_size(mut self, size: impl Into<Size>) -> Self {
        self.size = size.into();
        self
    }
}

impl ButtonVariants for DropdownButton {
    fn with_variant(mut self, variant: ButtonVariant) -> Self {
        self.variant = variant;
        self
    }
}

impl Selectable for DropdownButton {
    fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    fn is_selected(&self) -> bool {
        self.selected
    }
}

impl RenderOnce for DropdownButton {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let grouped = self.button.is_some() && self.menu.is_some();
        let joined = grouped && self.bordered && !self.variant.is_ghost();
        let button_corners = if joined {
            Corners {
                top_left: true,
                top_right: false,
                bottom_left: true,
                bottom_right: false,
            }
        } else {
            Corners::all(true)
        };
        let trigger_corners = if joined {
            Corners {
                top_left: false,
                top_right: true,
                bottom_left: false,
                bottom_right: true,
            }
        } else {
            Corners::all(true)
        };
        let button_edges = Edges::all(self.bordered);
        let trigger_edges = if joined {
            Edges {
                left: false,
                top: true,
                right: true,
                bottom: true,
            }
        } else {
            Edges::all(self.bordered)
        };
        let trigger_disabled = self.disabled || (grouped && self.loading);
        let trigger_focus_background = cx.theme().selection;
        let trigger_focus_foreground = cx.theme().foreground;
        let trigger_size = if grouped {
            match self.size {
                Size::XSmall => Size::XSmall,
                Size::Small => Size::Small,
                Size::Medium | Size::Large => Size::Medium,
                Size::Size(size) => Size::Size(size),
            }
        } else {
            self.size
        };
        let icon = self
            .icon
            .unwrap_or_else(|| Icon::new(IconName::ChevronDown));

        div()
            .id(self.id)
            .h_flex()
            .when(grouped && !joined, |this| this.gap_1())
            .refine_style(&self.style)
            .when_some(self.button, |this, button| {
                this.child(
                    button
                        .rounded(self.rounded)
                        .border_corners(button_corners)
                        .border_edges(button_edges)
                        .loading(self.loading)
                        .selected(self.selected)
                        .disabled(self.disabled || self.loading)
                        .when(self.compact, |this| this.compact())
                        .when(self.outline, |this| this.outline())
                        .with_size(self.size)
                        .with_variant(self.variant),
                )
            })
            .when_some(self.menu, |this, menu| {
                this.child(
                    Button::new(self.trigger_id)
                        .icon(icon)
                        .rounded(self.rounded)
                        .border_edges(trigger_edges)
                        .border_corners(trigger_corners)
                        .selected(self.selected)
                        .disabled(trigger_disabled)
                        .loading(!grouped && self.loading)
                        .when(self.compact, |this| this.compact())
                        .when(self.outline, |this| this.outline())
                        .when_some(self.tooltip, |this, tooltip| this.tooltip(tooltip))
                        .with_size(trigger_size)
                        .when(grouped, |this| match self.size {
                            Size::XSmall => this.w_5().h_5(),
                            Size::Small => this.w_5().h_6(),
                            Size::Medium | Size::Large => this.w_6().h_8(),
                            Size::Size(size) => this.size(size),
                        })
                        .with_variant(self.variant)
                        .focus_visible(move |this| {
                            this.bg(trigger_focus_background)
                                .border_color(trigger_focus_background)
                                .text_color(trigger_focus_foreground)
                        })
                        .dropdown_menu_with_anchor(self.anchor, menu),
                )
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::Corner;

    #[gpui::test]
    fn test_dropdown_button_builder(_cx: &mut gpui::TestAppContext) {
        let button = Button::new("inner").label("Action");
        let dropdown = DropdownButton::new("complex-dropdown")
            .button(button)
            .primary()
            .outline()
            .large()
            .compact()
            .bordered(false)
            .icon(IconName::Ellipsis)
            .tooltip("More actions")
            .loading(false)
            .disabled(false)
            .selected(false)
            .rounded(ButtonRounded::Medium)
            .dropdown_menu_with_anchor(Corner::BottomLeft, |menu, _, _| menu);

        assert!(dropdown.button.is_some());
        assert_eq!(dropdown.variant, ButtonVariant::Primary);
        assert!(dropdown.outline);
        assert!(!dropdown.bordered);
        assert!(dropdown.icon.is_some());
        assert!(dropdown.tooltip.is_some());
        assert_eq!(dropdown.size, Size::Large);
        assert!(dropdown.compact);
        assert!(!dropdown.loading);
        assert!(!dropdown.disabled);
        assert!(!dropdown.selected);
        assert!(matches!(dropdown.rounded, ButtonRounded::Medium));
        assert!(dropdown.menu.is_some());
        assert_eq!(dropdown.anchor, Corner::BottomLeft);
    }
}
