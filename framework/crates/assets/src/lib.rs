use anyhow::anyhow;
use gpui::{AssetSource, Result, SharedString};
use rust_embed::RustEmbed;
use std::{borrow::Cow, collections::HashSet};

/// Embed application assets for GPUI Component.
///
/// This bundle includes GPUI Component-owned assets such as icon SVGs and non-icon
/// resources like the surface noise texture at `surface/NoiseAsset_256.png`.
///
/// ```
/// use gpui_component_assets::{Assets, chain};
///
/// # fn main() -> gpui::Result<()> {
/// let _app = gpui_platform::try_headless()?.with_assets(chain(MyAssets, Assets));
///
/// # struct MyAssets;
/// # impl gpui::AssetSource for MyAssets {
/// #     fn load(&self, _path: &str) -> gpui::Result<Option<std::borrow::Cow<'static, [u8]>>> {
/// #         Ok(None)
/// #     }
/// #     fn list(&self, _path: &str) -> gpui::Result<Vec<gpui::SharedString>> {
/// #         Ok(Vec::new())
/// #     }
/// # }
/// # Ok(())
/// # }
/// ```
#[derive(RustEmbed)]
#[folder = "assets"]
pub struct Assets;

/// A composed [`AssetSource`] that tries the primary source before a fallback source.
pub struct AssetSourceChain<P, F> {
    primary: P,
    fallback: F,
}

impl<P, F> AssetSourceChain<P, F> {
    /// Create a composed asset source that checks `primary` first, then `fallback`.
    pub fn new(primary: P, fallback: F) -> Self {
        Self { primary, fallback }
    }
}

/// Compose two asset sources with primary-first lookup semantics.
#[must_use]
pub fn chain<P, F>(primary: P, fallback: F) -> AssetSourceChain<P, F> {
    AssetSourceChain::new(primary, fallback)
}

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if path.is_empty() {
            return Ok(None);
        }

        Self::get(path)
            .map(|f| Some(f.data))
            .ok_or_else(|| anyhow!("could not find asset at path \"{path}\""))
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        Ok(Self::iter()
            .filter_map(|p| p.starts_with(path).then(|| p.into()))
            .collect())
    }
}

impl<P, F> AssetSource for AssetSourceChain<P, F>
where
    P: AssetSource,
    F: AssetSource,
{
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        match self.primary.load(path) {
            Ok(Some(asset)) => Ok(Some(asset)),
            Ok(None) => self.fallback.load(path),
            Err(primary_err) => match self.fallback.load(path) {
                Ok(Some(asset)) => Ok(Some(asset)),
                Ok(None) | Err(_) => Err(primary_err),
            },
        }
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut assets = self.primary.list(path)?;
        let mut seen = HashSet::with_capacity(assets.len());

        assets.retain(|asset| seen.insert(asset.clone()));

        for asset in self.fallback.list(path)? {
            if seen.insert(asset.clone()) {
                assets.push(asset);
            }
        }

        Ok(assets)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{borrow::Cow, collections::HashMap};

    const SURFACE_NOISE_ASSET_PATH: &str = "surface/NoiseAsset_256.png";

    #[derive(Default)]
    struct TestAssetSource {
        assets: HashMap<&'static str, &'static [u8]>,
        miss_is_error: bool,
    }

    impl TestAssetSource {
        fn with_assets(assets: [(&'static str, &'static [u8]); 1]) -> Self {
            Self {
                assets: HashMap::from(assets),
                miss_is_error: true,
            }
        }
    }

    impl AssetSource for TestAssetSource {
        fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
            match self.assets.get(path) {
                Some(bytes) => Ok(Some(Cow::Borrowed(*bytes))),
                None if self.miss_is_error => {
                    Err(anyhow!("could not find asset at path \"{path}\""))
                }
                None => Ok(None),
            }
        }

        fn list(&self, path: &str) -> Result<Vec<SharedString>> {
            Ok(self
                .assets
                .keys()
                .filter(|asset| asset.starts_with(path))
                .map(|asset| (*asset).into())
                .collect())
        }
    }

    #[test]
    fn loads_surface_noise_from_bundled_assets() {
        let asset = Assets
            .load(SURFACE_NOISE_ASSET_PATH)
            .expect("surface asset lookup should succeed")
            .expect("surface asset should be bundled");

        assert!(!asset.is_empty());
    }

    #[test]
    fn chained_asset_sources_prefer_primary_then_fallback_to_bundled_assets() {
        let chained = chain(
            TestAssetSource::with_assets([("icons/inbox.svg", b"app-owned-inbox")]),
            Assets,
        );

        let primary_asset = chained
            .load("icons/inbox.svg")
            .expect("primary asset lookup should succeed")
            .expect("primary asset should exist");
        assert_eq!(primary_asset.as_ref(), b"app-owned-inbox");

        let fallback_asset = chained
            .load(SURFACE_NOISE_ASSET_PATH)
            .expect("fallback asset lookup should succeed")
            .expect("fallback asset should exist");
        let bundled_asset = Assets
            .load(SURFACE_NOISE_ASSET_PATH)
            .expect("bundled asset lookup should succeed")
            .expect("bundled asset should exist");
        assert_eq!(fallback_asset.as_ref(), bundled_asset.as_ref());
    }

    #[test]
    fn bundled_assets_do_not_require_or_expose_path_aliases() {
        assert!(Assets.load("NoiseAsset_256.png").is_err());

        let listed_assets = Assets.list("").expect("asset listing should succeed");
        assert!(
            listed_assets
                .iter()
                .any(|asset| asset == SURFACE_NOISE_ASSET_PATH)
        );
        assert!(
            !listed_assets
                .iter()
                .any(|asset| asset == "NoiseAsset_256.png")
        );
    }
}
