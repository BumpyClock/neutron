//! Minimal re-export shim mirroring `gpui-component-app`'s re-export of the
//! identity macro. Its only job is to prove that reaching `include_identity!`
//! through a re-export needs no direct `gpui-component-manifest` dependency in
//! the calling crate: macro `$crate` still resolves to the defining manifest
//! crate, transitively available via this shim.
pub use gpui_component_manifest::include_identity;
