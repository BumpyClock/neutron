---
title: AlertDialog
description: A modal dialog for important content that requires a response.
---

# AlertDialog

`AlertDialog` is an imperative modal dialog for important content. It uses the
existing `Root` dialog stack. It preserves focus restoration, callback vetoes,
and reduced-motion close behavior.

Alert dialogs do not close from an overlay click or a close button by default.
Their default action buttons are centered. They expose the accessibility role
`AlertDialog`.

## Import

```rust
use neutron_components::{WindowExt as _, dialog::DialogButtonProps};
```

## Basic alert

```rust
window.open_alert_dialog(cx, |alert, _, _| {
    alert
        .title("Changes saved")
        .description("Your settings are available in every open window.")
        .on_ok(|_, window, cx| {
            window.push_notification("Alert confirmed", cx);
            true
        })
});
```

## Confirmation

Use `confirm()` or `show_cancel(true)` to add a Cancel button.

```rust
window.open_alert_dialog(cx, |alert, _, _| {
    alert
        .confirm()
        .title("Delete this project?")
        .description("This action cannot be undone.")
        .button_props(
            DialogButtonProps::default()
                .ok_text("Delete")
                .cancel_text("Keep project")
        )
        .on_ok(|_, _, _| true)
        .on_cancel(|_, _, _| true)
});
```

Return `false` from `on_ok` or `on_cancel` to keep the dialog open. The
`on_close` callback runs only after a successful close.

## Custom content

Use `icon`, `title`, `description`, and child elements for content. Use
`footer` when default centered buttons do not fit the workflow.

```rust
window.open_alert_dialog(cx, |alert, _, _| {
    alert
        .icon(Icon::new(IconName::TriangleAlert))
        .title("Network access needed")
        .description("Allow this application to contact the configured service?")
        .show_cancel(true)
        .child("This choice applies to the current workspace.")
});
```

`AlertDialog` is imperative. Use `WindowExt::open_alert_dialog` to open it.
The trigger/content API from newer Longbridge releases requires a Dialog
refactor that does not preserve Neutron's deferred-close contract.

## Options

| Method | Effect |
| --- | --- |
| `confirm()` | Show Cancel and OK buttons. |
| `show_cancel(bool)` | Set Cancel button visibility. |
| `icon(element)` | Add an icon above the title. |
| `title(element)` | Set heading content. |
| `description(element)` | Set muted description content. |
| `button_props(props)` | Set default button labels and variants. |
| `footer(...)` | Replace default centered actions. |
| `overlay_closable(bool)` | Allow overlay cancellation. |
| `close_button(bool)` | Show a close button. |
| `keyboard(bool)` | Allow Escape cancellation. |
| `on_ok`, `on_cancel`, `on_close` | Set action callbacks. |
