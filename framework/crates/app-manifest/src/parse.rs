//! Cargo manifest identity parsing.

use std::path::Path;

use crate::{ManifestError, schema::AppIdentity};

/// Reads and validates `[package.metadata.gpui-app]` from a Cargo manifest.
///
/// # Errors
///
/// Returns a contextual error for file I/O, malformed TOML, missing tables,
/// schema deserialization, or identity validation failures.
pub fn parse_identity(cargo_toml_path: &Path) -> Result<AppIdentity, ManifestError> {
    let source =
        std::fs::read_to_string(cargo_toml_path).map_err(|source| ManifestError::ReadManifest {
            path: cargo_toml_path.to_owned(),
            source,
        })?;
    let document: toml::Value =
        toml::from_str(&source).map_err(|source| ManifestError::ParseToml {
            path: cargo_toml_path.to_owned(),
            source,
        })?;
    let package = document
        .get("package")
        .and_then(toml::Value::as_table)
        .ok_or_else(|| ManifestError::MissingPackage {
            path: cargo_toml_path.to_owned(),
        })?;
    let metadata = package.get("metadata").and_then(toml::Value::as_table);
    let identity = metadata.and_then(|table| table.get("gpui-app"));
    let Some(identity) = identity else {
        let suggestion = if metadata.is_some_and(|table| table.contains_key("gpui_app")) {
            "; found `gpui_app`; rename it to `gpui-app`"
        } else {
            ""
        };
        return Err(ManifestError::MissingIdentity {
            path: cargo_toml_path.to_owned(),
            suggestion,
        });
    };
    let identity: AppIdentity =
        identity
            .clone()
            .try_into()
            .map_err(|source| ManifestError::InvalidIdentityTable {
                path: cargo_toml_path.to_owned(),
                source,
            })?;
    identity.validate()?;
    Ok(identity)
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use tempfile::NamedTempFile;

    use super::*;

    fn parse(source: &str) -> Result<AppIdentity, ManifestError> {
        let mut file = NamedTempFile::new().expect("create manifest");
        file.write_all(source.as_bytes()).expect("write manifest");
        parse_identity(file.path())
    }

    #[test]
    fn parses_valid_minimal_identity() {
        let identity = parse(
            r#"
                [package]
                name = "example"
                [package.metadata.gpui-app]
                app_id = "com.example.app"
                display_name = "Example"
            "#,
        )
        .expect("valid identity");
        assert_eq!(identity.app_id, "com.example.app");
        assert_eq!(identity.display_name, "Example");
        assert_eq!(identity.resolved_data_namespace(), "app");
    }

    #[test]
    fn reports_missing_identity_table() {
        let error = parse("[package]\nname = \"example\"\n")
            .expect_err("identity metadata should be required");
        assert!(error.to_string().contains("[package.metadata.gpui-app]"));
    }

    #[test]
    fn suggests_hyphenated_key_for_underscore_typo() {
        let error = parse(
            "[package]\nname = \"example\"\n[package.metadata.gpui_app]\napp_id = \"com.example.app\"\ndisplay_name = \"Example\"\n",
        )
        .expect_err("underscore spelling should not parse");
        let message = error.to_string();
        assert!(message.contains("gpui_app"));
        assert!(message.contains("gpui-app"));
    }

    #[test]
    fn reports_malformed_toml() {
        let error = parse("[package\nname = ???").expect_err("malformed TOML");
        assert!(error.to_string().contains("failed to parse Cargo manifest"));
    }

    #[test]
    fn reports_missing_package_table() {
        let error = parse("title = \"not a package\"\n").expect_err("missing package");
        assert!(error.to_string().contains("missing the `[package]` table"));
    }

    #[test]
    fn rejects_version_in_identity_metadata() {
        let error = parse(
            "[package]\nname = \"example\"\n[package.metadata.gpui-app]\napp_id = \"com.example.app\"\ndisplay_name = \"Example\"\nversion = \"1.0.0\"\n",
        )
        .expect_err("version has a canonical Cargo source");
        assert!(error.to_string().contains("unknown field `version`"));
    }
}
