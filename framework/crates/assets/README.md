# GPUI Component Assets

The default assets bundle for [GPUI Component](https://github.com/longbridge/gpui-component).

This crate bundles component-owned assets such as icons and the surface noise texture at `surface/NoiseAsset_256.png`.

If your application has its own `AssetSource`, compose it with the bundled fallback:

```rust
let app = gpui_platform::application().with_assets(gpui_component_assets::chain(
    AppAssets,
    gpui_component_assets::Assets,
));
```

## License

Apache-2.0
