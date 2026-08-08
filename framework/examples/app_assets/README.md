## Asset composition in GPUI Component

The [IconName](https://github.com/longbridge/gpui-component/blob/6998708b817024c2ac0f1ea164d74ddfc024e124/crates/ui/src/icon.rs#L9) enum defines the icon filenames used by GPUI Component. The separate `gpui-component-assets` crate now also bundles library-owned non-icon assets, such as `surface/NoiseAsset_256.png`.

This example keeps app-specific icons in `./assets/icons`, but still composes them with `gpui_component_assets::Assets` so bundled component assets continue to resolve.

You can still ship your own icons. Put them under `icons/...` and register your asset source first so app assets win on conflicts.

You can download icon files from [Lucide](https://lucide.dev/) or use your own SVGs, as long as the filenames match the `IconName` values you use.

For example your assets folder:

```
app_root
  assets
    icons
      close.svg
      menu.svg
      ...
  src
    main.rs
  Cargo.toml
```

You also can just copy the svg files you want from the `assets/icons` folder in GPUI Component repo to your own assets folder.

## How to use

Define an app asset source with `rust-embed`, then compose it with the bundled GPUI Component assets.

```rs
use gpui::*;
use gpui_component_assets::{chain, Assets as ComponentAssets};
use rust_embed::RustEmbed;
use std::borrow::Cow;

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

    // ...
}
```

This gives you app assets first and GPUI Component assets second. No path aliases are required for bundled resources like `surface/NoiseAsset_256.png`; the library continues to own those namespaced paths.

## Use only the bundled component assets

If you do not have app-specific assets, use the bundled source directly.

```rs
let app = gpui_platform::application().with_assets(gpui_component_assets::Assets);
```

## Dependency

```toml
[dependencies]
gpui-component = "*"
gpui-component-assets = "*"
```
