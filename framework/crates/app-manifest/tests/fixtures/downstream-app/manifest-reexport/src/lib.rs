//! Minimal re-export shim mirroring `neutron-components-app`'s re-export of the
//! identity macro. Its only job is to prove that reaching `include_identity!`
//! through a re-export needs no direct `neutron-components-manifest` dependency in
//! the calling crate: macro `$crate` still resolves to the defining manifest
//! crate, transitively available via this shim.
pub use neutron_components_manifest::include_identity;
