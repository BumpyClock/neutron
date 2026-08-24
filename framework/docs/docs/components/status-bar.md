---
title: StatusBar
description: A horizontal status bar with left, center, and right regions, usually placed at the bottom of a window or pane.
---

# StatusBar

StatusBar is a horizontal bar split into `left`, `center`, and `right` regions. It is usually placed at the bottom of a window or pane to show contextual information and quick actions.

## Import

```rust
use neutron_components::status_bar::StatusBar;
```

## Regions

Pass any `impl IntoElement` to a region. `left` and `right` pin items to each end. `child` and `children` add to the center. The center is centered with both pinned ends, end-aligned with only `left`, and start-aligned otherwise.

- Pass a plain string for a non-interactive label.
- Use a ghost, xsmall `Button` for a clickable item.
- Use `Divider::vertical()` for a separator.
- Pass custom layouts directly.

## Usage

### Labels

```rust
StatusBar::new()
    .left("Ready")
    .child("README.md")
    .right("UTF-8")
```

### Buttons

```rust
StatusBar::new()
    .left(
        Button::new("branch")
            .ghost()
            .xsmall()
            .icon(IconName::GitHub)
            .label("main")
            .on_click(|_, window, cx| {
                window.push_notification("Switch branch", cx);
            }),
    )
    .right(
        Button::new("go-to-line")
            .ghost()
            .xsmall()
            .label("Ln 1, Col 1")
            .tooltip("Go to Line/Column"),
    )
```

### Dividers and custom elements

```rust
StatusBar::new()
    .left(Button::new("branch").ghost().xsmall().label("main"))
    .left(Divider::vertical().h_3())
    .left(
        h_flex()
            .items_center()
            .gap_1()
            .child(Icon::new(IconName::CircleCheck).xsmall())
            .child("0 problems"),
    )
    .child(Progress::new("indexing").value(60.).w_24())
```

### Custom styling

`StatusBar` implements `Styled`, so style methods override its defaults.

```rust
StatusBar::new()
    .bg(cx.theme().secondary)
    .border_color(cx.theme().border)
    .py_2()
    .left("Ready")
```

## API Reference

| Method | Description |
| --- | --- |
| `new()` | Create an empty status bar. |
| `left(child)` | Append an element to the left region. |
| `right(child)` | Append an element to the right region. |
| `child(c)` / `children(cs)` | Add elements to the center region. |

StatusBar implements `Styled`, so standard style methods such as `bg`, `border_color`, and `py` override defaults.

## Notes

- Use plain text for read-only items to avoid button hover effects.
- Use ghost xsmall buttons for clickable items.
- Background and border use `status_bar.background` and `status_bar.border` theme tokens.
- Missing status-bar tokens fall back to title-bar tokens.
