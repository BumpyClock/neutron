---
title: "Root View"
summary: "How Root wraps each GPUI window and enables sheets, dialogs, notifications, and keyboard navigation."
order: -7
---

# Root View

[Root] must be the first view in a window. It owns sheets, dialogs, notifications, and tab navigation for that window.

If Root is not first, `Root::update` panics and those overlays never render.

:::tip
`neutron-components-app::WindowManager::open` and `open_singleton` wrap content in
`Root` automatically. Use the manual pattern below only with raw GPUI bootstrap;
`open_raw` intentionally opts out.
:::

```rs
fn main() {
    let app = gpui_platform::application().with_assets(neutron_components_assets::Assets);

    app.run(move |cx| {
        // This must be called before using any Neutron Components features.
        neutron_components::init(cx);

        cx.spawn(async move |cx| {
            cx.open_window(WindowOptions::default(), |window, cx| {
                let view = cx.new(|_| Example);
                // This first level on the window, should be a Root.
                cx.new(|cx| Root::new(view, window, cx))
            })?;

            Ok::<_, anyhow::Error>(())
        })
        .detach();
    });
}
```

## Overlays

Root renders the sheet, dialog, and notification layers exactly once. It mounts them as a sibling of the content view.

Open overlays from the content view with `WindowExt` methods `open_sheet`, `open_dialog`, and `push_notification`. Do not paint those layers in the content view's `render` method.

GPUI dispatches an action by walking from the focused element through its ancestors only. A focused dialog or sheet does not deliver actions into the content view's `on_action` handlers, and the content view's actions do not leak into the overlay.

[Root]: https://docs.rs/neutron-components/latest/neutron_components/root/struct.Root.html
