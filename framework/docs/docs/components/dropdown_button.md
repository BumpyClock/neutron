---
title: DropdownButton
description: Combines an optional action button with a dropdown menu trigger.
summary: "Combines an optional action button with a dropdown menu trigger."
---

# DropdownButton

A [DropdownButton] combines an optional action button with a menu trigger. In split mode, the left button performs the primary action while the right button opens the menu. Without a left button, it renders as a single icon menu trigger.

The component supports [Button] variants, [Sizable] sizes, borderless styling, custom trigger icons, loading, disabled, and selected states.

The split control uses one outer silhouette and one 1 px seam. Its menu trigger stays
narrower than the primary action while retaining the action's height. Ghost and
explicitly borderless modes use a stable 4 px gap so selection never shifts layout.

## Import

```rust
use gpui_component::{
    button::{Button, DropdownButton},
    menu::DropdownMenu as _,
};
```

## Usage

```rust
DropdownButton::new("dropdown")
    .button(Button::new("action").label("Click Me"))
    .dropdown_menu(|menu, _, _| {
        menu.menu("Option 1", Box::new(MyAction))
            .menu("Option 2", Box::new(MyAction))
            .separator()
            .menu("Option 3", Box::new(MyAction))
    })
```

### Variants

Like [Button], DropdownButton supports button variants.

```rust
DropdownButton::new("dropdown")
    .primary()
    .button(Button::new("action").label("Primary"))
    .dropdown_menu(|menu, _, _| {
        menu.menu("Option 1", Box::new(MyAction))
    })
```

### Borderless

Use `bordered(false)` to remove borders. Split controls get independent rounding and spacing so hover and focus states remain clear. Combine it with the ghost variant for a flat toolbar treatment.

```rust
DropdownButton::new("view-options")
    .ghost()
    .bordered(false)
    .button(Button::new("apply-view").label("Apply view"))
    .dropdown_menu(|menu, _, _| {
        menu.menu("Compact", Box::new(CompactView))
            .menu("Comfortable", Box::new(ComfortableView))
    })
```

### Icon only

Omit `button(...)` to render one icon trigger. Provide a tooltip so the icon has a discoverable and accessible name.

```rust
use gpui_component::IconName;

DropdownButton::new("more-actions")
    .ghost()
    .bordered(false)
    .icon(IconName::Ellipsis)
    .tooltip("More actions")
    .dropdown_menu(|menu, _, _| {
        menu.menu("Duplicate", Box::new(Duplicate))
            .menu("Archive", Box::new(Archive))
    })
```

When `icon(...)` is used with `button(...)`, it replaces the split button's default chevron.

### Full trigger

Use a [Button] with `dropdown_caret(true)` when the entire label and caret
should open the menu. Do not model this as a split `DropdownButton` with an
unused primary action.

```rust
Button::new("view-options")
    .ghost()
    .label("View options")
    .dropdown_caret(true)
    .dropdown_menu(|menu, _, _| {
        menu.menu("Compact", Box::new(CompactView))
            .menu("Comfortable", Box::new(ComfortableView))
    })
```

### With custom anchor

```rust
use gpui::Corner;

DropdownButton::new("dropdown")
    .button(Button::new("action").label("Click Me"))
    .dropdown_menu_with_anchor(Corner::BottomRight, |menu, _, _| {
        menu.menu("Option 1", Box::new(MyAction))
    })
```

## Accessibility

- Split mode exposes two focus stops: primary action, then menu trigger.
- Each split segment gets its own visible focus ring so keyboard users can tell which action will run.
- Loading and disabled split controls remove both segments from interaction; loading remains visible on the primary action.
- Icon-only triggers should always include `tooltip(...)`.

## Motion

DropdownButton keeps its geometry stable across default, selected, loading, and
disabled states. Menu enter and exit motion belongs to the shared [Popover] and
[PopupMenu] layers rather than the trigger, so split controls do not stack a second
animation on top. The shared layers honor the application's reduced-motion setting.

## Platform support

Bordered, borderless, split, and icon-only modes use shared GPUI primitives and behave consistently on macOS, Windows, and Linux.

[Button]: https://docs.rs/gpui-component/latest/gpui_component/button/struct.Button.html
[DropdownButton]: https://docs.rs/gpui-component/latest/gpui_component/button/struct.DropdownButton.html
[Popover]: https://docs.rs/gpui-component/latest/gpui_component/popover/struct.Popover.html
[PopupMenu]: https://docs.rs/gpui-component/latest/gpui_component/menu/struct.PopupMenu.html
[Sizable]: https://docs.rs/gpui-component/latest/gpui_component/trait.Sizable.html
