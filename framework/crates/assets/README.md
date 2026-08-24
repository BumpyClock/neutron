# Neutron Components Assets

The default assets bundle for [Neutron Components](https://github.com/BumpyClock/neutron/tree/main/framework).

This crate bundles component-owned assets such as icons and the surface noise texture at `surface/NoiseAsset_256.png`.

If your application has its own `AssetSource`, compose it with the bundled fallback:

```rust
let app = gpui_platform::application().with_assets(neutron_components_assets::chain(
    AppAssets,
    neutron_components_assets::Assets,
));
```

## License

Apache-2.0
