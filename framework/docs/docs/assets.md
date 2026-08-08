---
title: Icons & Assets
summary: "How to register GPUI Component bundled assets and compose custom app asset sources."
order: -4
---

# Icons & Assets

The [IconName] and [Icon] APIs rely on your app's registered [`AssetSource`](https://docs.rs/gpui/latest/gpui/trait.AssetSource.html). GPUI Component also ships a small bundled asset crate for library-owned resources.

The main `gpui-component` crate still does **not** embed icon SVGs directly, which keeps the core crate lean.

Those assets live in [gpui-component-assets], which now packages both:

- bundled icons under `icons/...`
- library-owned non-icon assets under stable names such as `surface/NoiseAsset_256.png`

## Use bundled component assets

If your application does not have its own assets, register the bundled source directly.

Add the crate:

```toml-vue
[dependencies]
gpui-component = "{{ VERSION }}"
gpui-component-assets = "{{ VERSION }}"
```

Then register it with GPUI:

```rs
use gpui_component_assets::Assets;

let app = gpui_platform::application().with_assets(Assets);
```

This is enough for bundled icons and GPUI Component-owned assets such as the surface noise texture.

Continue [Use the icons](#use-the-icons) section to see how to use the icons in your application.

## Compose app assets with bundled component assets

If your app has its own assets, do not replace `gpui_component_assets::Assets`. Compose your asset source with it so app paths resolve first and bundled GPUI Component paths still work.

This is the recommended downstream integration pattern.

With AppShell, register sources in precedence order:

```rs
AppShell::builder(APP_IDENTITY)
    .assets(AppAssets)
    .assets(gpui_component_assets::Assets)
    // ...
```

Loading uses first-hit-wins semantics. An earlier source error is remembered but
does not prevent a later source from resolving the path. If no source resolves
it, AppShell returns the first remembered error; it returns `Ok(None)` only when
every source reports a clean miss.

For a raw GPUI host, use the two-source helper:

```rs
use gpui::*;
use gpui_component::{v_flex, IconName, Root};
use gpui_component_assets::{Assets as ComponentAssets, chain};
use rust_embed::RustEmbed;
use std::borrow::Cow;

/// An asset source that loads assets from the `./assets` folder.
#[derive(RustEmbed)]
#[folder = "./assets"]
#[include = "icons/**/*.svg"]
pub struct AppAssets;

impl AssetSource for AppAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if path.is_empty() {
            return Ok(None);
        }

        Ok(Self::get(path).map(|file| file.data))
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        Ok(Self::iter()
            .filter_map(|p| p.starts_with(path).then(|| p.into()))
            .collect())
    }
}

fn main() {
    let app = gpui_platform::application().with_assets(chain(AppAssets, ComponentAssets));

    app.run(move |cx| {
        // We must initialize gpui_component before using it.
        gpui_component::init(cx);

        cx.spawn(async move |cx| {
            cx.open_window(WindowOptions::default(), |window, cx| {
                let view = cx.new(|_| Example);
                // The first level on the window must be Root.
                cx.new(|cx| Root::new(view, window, cx))
            })?;

            Ok::<_, anyhow::Error>(())
        })
        .detach();
    });
}
```

With this setup:

- `icons/...` can be app-owned and override bundled assets if you want
- GPUI Component assets like `surface/NoiseAsset_256.png` still load from the fallback bundle
- no bare-path aliases are needed; library assets stay namespaced

## Build your own icon set

If you want to ship a smaller icon subset, copy only the SVGs you need into your app's `assets/icons/` directory and keep the composed fallback above.

The [assets] folder in source code contains the bundled icons and other packaged component assets.

## Use the icons

Now we can use the icons in our application:

```rs
pub struct Example;

impl Render for Example {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .gap_2()
            .size_full()
            .items_center()
            .justify_center()
            .text_center()
            .child(IconName::Inbox)
            .child(IconName::Bot)
    }
}
```

## Resources

- [Lucide Icons](https://lucide.dev/) - The icon set used in GPUI Component is based on the open-source Lucide Icons library, which provides a wide range of customizable SVG icons.

[rust-embed]: https://docs.rs/rust-embed/latest/rust_embed/
[IconName]: https://docs.rs/gpui_component/latest/gpui_component/icon/enum.IconName.html
[Icon]: https://docs.rs/gpui_component/latest/gpui_component/icon/struct.Icon.html
[assets]: https://github.com/BumpyClock/gpui-component/tree/main/crates/assets/assets/
[gpui-component-assets]: https://crates.io/crates/gpui-component-assets
