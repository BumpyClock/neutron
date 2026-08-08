// Reaches `include_identity!` through the `manifest-reexport` shim rather than
// naming `gpui_component_manifest` directly — proving the re-export path compiles
// with the manifest crate present only as a build dependency (see Cargo.toml).
manifest_reexport::include_identity!();

fn main() {
    assert_eq!(APP_IDENTITY.app_id, "com.example.downstreamfixture");
    assert_eq!(APP_IDENTITY.version, env!("CARGO_PKG_VERSION"));
    println!(
        "IDENTITY_OK {} {}",
        APP_IDENTITY.app_id, APP_IDENTITY.version
    );
}
