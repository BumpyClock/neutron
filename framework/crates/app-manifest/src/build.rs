//! Code generation called by a consuming application's own `build.rs`.

use std::{fmt::Write as _, path::PathBuf};

use crate::{
    ManifestError,
    schema::{AppIdentity, LinuxIdentity, MacosIdentity, MinOsVersions, WindowsIdentity},
    versions::{MsixTarget, derive_cfbundle_short_version, derive_msix_version},
};

/// Reads the consuming app's manifest and writes `gpui_app_identity.rs` to its
/// `OUT_DIR`.
///
/// # Errors
///
/// Returns a contextual error when required Cargo environment is absent, the
/// manifest is invalid, a packaging version cannot be derived, or output fails.
pub fn emit_identity() -> Result<(), ManifestError> {
    let manifest_dir = required_env("CARGO_MANIFEST_DIR")?;
    let version = required_env("CARGO_PKG_VERSION")?;
    let _package_name = required_env("CARGO_PKG_NAME")?;
    let out_dir = required_env("OUT_DIR")?;

    println!("cargo:rerun-if-changed=Cargo.toml");
    println!("cargo:rerun-if-env-changed=GPUI_APP_BUILD_NUMBER");

    let identity = crate::parse::parse_identity(&PathBuf::from(manifest_dir).join("Cargo.toml"))?;
    let cfbundle_short_version = derive_cfbundle_short_version(&version)?;
    let build_number = optional_build_number()?;
    // Sideload target: the generated identity carries the build number in the
    // fourth component. Store submissions reserve and overwrite that component,
    // so a Store-specific version is derived at packaging time, not here.
    let msix_version = derive_msix_version(&version, MsixTarget::Sideload, build_number)?;
    let source = generate_source(&identity, &version, &cfbundle_short_version, &msix_version);
    let output_path = PathBuf::from(out_dir).join("gpui_app_identity.rs");
    std::fs::write(&output_path, source).map_err(|source| ManifestError::WriteGenerated {
        path: output_path,
        source,
    })
}

fn required_env(name: &'static str) -> Result<String, ManifestError> {
    std::env::var(name).map_err(|source| ManifestError::MissingEnvironment { name, source })
}

fn optional_build_number() -> Result<Option<u32>, ManifestError> {
    match std::env::var("GPUI_APP_BUILD_NUMBER") {
        Ok(value) => value
            .parse()
            .map(Some)
            .map_err(|source| ManifestError::InvalidBuildNumber { value, source }),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(source) => Err(ManifestError::MissingEnvironment {
            name: "GPUI_APP_BUILD_NUMBER",
            source,
        }),
    }
}

fn generate_source(
    identity: &AppIdentity,
    version: &str,
    cfbundle_short_version: &str,
    msix_version: &str,
) -> String {
    // Types are named UNQUALIFIED here. `include_identity!` includes this file
    // inside a module that brings the schema types into scope via `$crate`, so
    // the identifiers resolve to `gpui-component-manifest` even when the macro is
    // reached through a re-export — letting apps depend on it only as a
    // build-dependency. Do not reintroduce `::gpui_component_manifest::` paths:
    // an absolute path would force every consumer back onto a direct runtime dep.
    let mut source = String::new();
    writeln!(
        source,
        "pub static APP_IDENTITY: IdentityRef = IdentityRef {{"
    )
    .expect("writing generated source to a String cannot fail");
    write_field(&mut source, "app_id", &format!("{:?}", identity.app_id));
    write_field(
        &mut source,
        "display_name",
        &format!("{:?}", identity.display_name),
    );
    write_field(
        &mut source,
        "data_namespace",
        &format!("{:?}", identity.resolved_data_namespace()),
    );
    write_field(
        &mut source,
        "binary_name",
        &format!("{:?}", identity.binary_name),
    );
    write_field(&mut source, "org", &format!("{:?}", identity.org));
    write_field(
        &mut source,
        "publisher",
        &format!("{:?}", identity.publisher),
    );
    write_field(
        &mut source,
        "url_schemes",
        &format!("&{:?}", identity.url_schemes),
    );
    write_field(
        &mut source,
        "categories",
        &format!("&{:?}", identity.categories),
    );
    write_field(&mut source, "macos", &macos_source(identity.macos.as_ref()));
    write_field(&mut source, "linux", &linux_source(identity.linux.as_ref()));
    write_field(
        &mut source,
        "windows",
        &windows_source(identity.windows.as_ref()),
    );
    write_field(
        &mut source,
        "legacy_ids",
        &format!("&{:?}", identity.legacy_ids),
    );
    write_field(
        &mut source,
        "min_os",
        &min_os_source(identity.min_os.as_ref()),
    );
    write_field(&mut source, "version", &format!("{version:?}"));
    write_field(
        &mut source,
        "cfbundle_short_version",
        &format!("{cfbundle_short_version:?}"),
    );
    write_field(&mut source, "msix_version", &format!("{msix_version:?}"));
    source.push_str("};\n");
    source
}

fn write_field(source: &mut String, name: &str, value: &str) {
    writeln!(source, "    {name}: {value},")
        .expect("writing generated source to a String cannot fail");
}

fn macos_source(value: Option<&MacosIdentity>) -> String {
    value.map_or_else(
        || "None".to_owned(),
        |value| {
            let usage_strings: Vec<_> = value.usage_strings.iter().collect();
            format!(
                "Some(MacosIdentityRef {{ category: {:?}, entitlements: &{:?}, usage_strings: &{:?} }})",
                value.category, value.entitlements, usage_strings
            )
        },
    )
}

fn linux_source(value: Option<&LinuxIdentity>) -> String {
    value.map_or_else(
        || "None".to_owned(),
        |value| {
            format!(
                "Some(LinuxIdentityRef {{ categories: &{:?}, desktop_keywords: &{:?} }})",
                value.categories, value.desktop_keywords
            )
        },
    )
}

fn windows_source(value: Option<&WindowsIdentity>) -> String {
    value.map_or_else(
        || "None".to_owned(),
        |value| {
            format!(
                "Some(WindowsIdentityRef {{ publisher: {:?} }})",
                value.publisher
            )
        },
    )
}

fn min_os_source(value: Option<&MinOsVersions>) -> String {
    value.map_or_else(
        || "None".to_owned(),
        |value| {
            format!(
                "Some(MinOsVersionsRef {{ macos: {:?}, windows: {:?}, linux: {:?} }})",
                value.macos, value.windows, value.linux
            )
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_literals_escape_strings() {
        let identity = AppIdentity {
            app_id: "com.example.escaped".to_owned(),
            display_name: "Quoted \"Name\"\n".to_owned(),
            data_namespace: "escaped".to_owned(),
            binary_name: Some("bin\\name".to_owned()),
            org: None,
            publisher: None,
            url_schemes: vec!["escaped".to_owned()],
            categories: Vec::new(),
            macos: None,
            linux: None,
            windows: None,
            legacy_ids: Vec::new(),
            min_os: None,
        };
        let source = generate_source(&identity, "1.2.3", "1.2.3", "1.2.3.0");
        assert!(source.contains(r#"display_name: "Quoted \"Name\"\n""#));
        assert!(source.contains(r#"binary_name: Some("bin\\name")"#));
    }

    #[test]
    fn generated_source_names_schema_types_unqualified() {
        // `include_identity!` resolves these via `$crate`, so the generated file
        // must NOT hardcode `::gpui_component_manifest::` — that would force a
        // direct runtime dependency on every consumer (see the macro's rustdoc).
        let identity = AppIdentity {
            app_id: "com.example.macro".to_owned(),
            display_name: "Macro".to_owned(),
            data_namespace: "macro".to_owned(),
            binary_name: None,
            org: None,
            publisher: None,
            url_schemes: Vec::new(),
            categories: Vec::new(),
            macos: Some(MacosIdentity::default()),
            linux: None,
            windows: None,
            legacy_ids: Vec::new(),
            min_os: None,
        };
        let source = generate_source(&identity, "1.0.0", "1.0.0", "1.0.0.0");
        assert!(source.contains("APP_IDENTITY: IdentityRef = IdentityRef {"));
        assert!(source.contains("Some(MacosIdentityRef {"));
        assert!(
            !source.contains("::gpui_component_manifest"),
            "generated source must not hardcode an absolute manifest path:\n{source}"
        );
    }
}
