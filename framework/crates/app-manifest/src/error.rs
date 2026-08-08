use std::path::PathBuf;

/// Errors returned while validating, parsing, or generating app identity data.
#[derive(Debug, thiserror::Error)]
pub enum ManifestError {
    #[error(
        "invalid `{field}` value {value:?}: expected at least two dot-separated segments, each starting with an ASCII letter and containing only ASCII letters, digits, or hyphens (underscores are rejected because `app_id` becomes the macOS CFBundleIdentifier, which permits only alphanumerics, hyphens, and periods)"
    )]
    InvalidAppId { field: String, value: String },

    #[error(
        "invalid `display_name` value {value:?}: expected a non-empty name after trimming whitespace"
    )]
    EmptyDisplayName { value: String },

    #[error(
        "invalid `data_namespace` value {value:?}: expected a non-empty namespace after trimming whitespace (omit the field to derive it from `app_id`)"
    )]
    EmptyDataNamespace { value: String },

    #[error(
        "invalid `url_schemes` value {value:?}: expected a lowercase ASCII scheme starting with a letter and containing only letters, digits, '+', '-', or '.'"
    )]
    InvalidUrlScheme { value: String },

    #[error("invalid semantic version {value:?}: {reason}")]
    InvalidSemver { value: String, reason: String },

    #[error(
        "invalid MSIX version component `{field}` value {value}: expected an integer from 0 through 65535"
    )]
    InvalidMsixComponent { field: &'static str, value: u64 },

    #[error(
        "invalid Microsoft Store MSIX version {value:?}: the first component (major) must be nonzero for Store packages"
    )]
    MsixStoreZeroMajor { value: String },

    #[error("failed to read Cargo manifest at {path}: {source}")]
    ReadManifest {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to parse Cargo manifest at {path}: {source}")]
    ParseToml {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },

    #[error("Cargo manifest at {path} is missing the `[package]` table")]
    MissingPackage { path: PathBuf },

    #[error("Cargo manifest at {path} is missing `[package.metadata.gpui-app]`{suggestion}")]
    MissingIdentity {
        path: PathBuf,
        suggestion: &'static str,
    },

    #[error("invalid `[package.metadata.gpui-app]` in {path}: {source}")]
    InvalidIdentityTable {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },

    #[error("required build environment variable `{name}` is unavailable: {source}")]
    MissingEnvironment {
        name: &'static str,
        #[source]
        source: std::env::VarError,
    },

    #[error(
        "invalid build environment variable `GPUI_APP_BUILD_NUMBER` value {value:?}: expected an unsigned 32-bit integer: {source}"
    )]
    InvalidBuildNumber {
        value: String,
        #[source]
        source: std::num::ParseIntError,
    },

    #[error("failed to write generated app identity to {path}: {source}")]
    WriteGenerated {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}
