---
title: Divider
description: A horizontal or vertical rule with solid or dashed styles and optional labels.
summary: "A horizontal or vertical rule with solid or dashed styles and optional labels."
---

# Divider

`Divider` separates content with a horizontal or vertical line. It can use a
solid or dashed line, a custom color, and an optional centered label.

## Import

```rust
use neutron_components::divider::Divider;
```

## Horizontal divider

Use `horizontal()` inside a container that provides its width:

```rust
v_flex()
    .gap_4()
    .child(Divider::horizontal())
    .child(Divider::horizontal().label("More options"))
```

## Vertical divider

Use `vertical()` inside a container that provides its height:

```rust
h_flex()
    .h(px(96.))
    .gap_4()
    .child("Details")
    .child(Divider::vertical())
    .child("Activity")
```

Import `px` from GPUI for the explicit height in this example:

```rust
use gpui::px;
```

## Dashed and custom colors

Use the dashed constructors or call `dashed()` on an existing divider. The
default line color is `theme.border`.

```rust
Divider::horizontal_dashed()
Divider::vertical_dashed().label("Optional")
Divider::horizontal().color(cx.theme().primary)
```

## API reference

| Method | Description |
| --- | --- |
| `horizontal()` | Create a horizontal solid divider. |
| `vertical()` | Create a vertical solid divider. |
| `horizontal_dashed()` | Create a horizontal dashed divider. |
| `vertical_dashed()` | Create a vertical dashed divider. |
| `dashed()` | Change a divider to dashed style. |
| `label(text)` | Add a label over the line. |
| `color(color)` | Set the line color. |

`Divider` implements GPUI's `Styled` trait. Use the parent layout for its
available width or height, then refine spacing and alignment as needed.

[Divider]: https://docs.rs/neutron-components/latest/neutron_components/divider/struct.Divider.html
