---
title: Breadcrumb
description: A navigation trail that shows the current location in a hierarchy.
summary: "A navigation trail that shows the current location in a hierarchy."
---

# Breadcrumb

`Breadcrumb` shows a path through an application's hierarchy. It inserts a
chevron between each item and styles the last item as the current location.

## Import

```rust
use neutron_components::breadcrumb::{Breadcrumb, BreadcrumbItem};
```

## Basic usage

Pass strings to `child` or `children` for simple, non-interactive items:

```rust
Breadcrumb::new()
    .children(["Home", "Documents", "Projects"])
```

## Clickable items

Use `BreadcrumbItem` when an item needs a click handler. The callback receives
the click event, window, and app context.

```rust
Breadcrumb::new()
    .child(BreadcrumbItem::new("Home").on_click(|_, _, _| {
        // Navigate to the home view.
    }))
    .child(BreadcrumbItem::new("Projects").on_click(|_, _, _| {
        // Navigate to the projects view.
    }))
    .child("Current project")
```

Disable an item with `disabled(true)`. Disabled items keep their label but do
not receive click handlers.

```rust
Breadcrumb::new()
    .child(BreadcrumbItem::new("Workspace").disabled(true))
    .child("Project")
```

## API reference

### Breadcrumb

| Method | Description |
| --- | --- |
| `new()` | Create an empty breadcrumb. |
| `child(item)` | Add one string or `BreadcrumbItem`. |
| `children(items)` | Add several strings or `BreadcrumbItem` values. |

### BreadcrumbItem

| Method | Description |
| --- | --- |
| `new(label)` | Create an item with a label. |
| `on_click(handler)` | Run a callback when an enabled item is clicked. |
| `disabled(bool)` | Disable or enable the item. |

Both types implement GPUI's `Styled` trait, so you can apply standard layout
and text styles to the breadcrumb or an item.

## Layout

`Breadcrumb` uses a horizontal flex layout with a small gap. Place it in a
container that provides the width and alignment for your navigation header.

[Breadcrumb]: https://docs.rs/neutron-components/latest/neutron_components/breadcrumb/struct.Breadcrumb.html
[BreadcrumbItem]: https://docs.rs/neutron-components/latest/neutron_components/breadcrumb/struct.BreadcrumbItem.html
