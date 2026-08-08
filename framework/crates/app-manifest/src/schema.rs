//! Owned manifest schema and const-friendly borrowed codegen types.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::ManifestError;

/// Application identity read from `[package.metadata.gpui-app]`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct AppIdentity {
    pub app_id: String,
    pub display_name: String,
    pub data_namespace: String,
    #[serde(default)]
    pub binary_name: Option<String>,
    #[serde(default)]
    pub org: Option<String>,
    #[serde(default)]
    pub publisher: Option<String>,
    #[serde(default)]
    pub url_schemes: Vec<String>,
    #[serde(default)]
    pub categories: Vec<String>,
    #[serde(default)]
    pub macos: Option<MacosIdentity>,
    #[serde(default)]
    pub linux: Option<LinuxIdentity>,
    #[serde(default)]
    pub windows: Option<WindowsIdentity>,
    #[serde(default)]
    pub legacy_ids: Vec<String>,
    #[serde(default)]
    pub min_os: Option<MinOsVersions>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AppIdentityWire {
    app_id: String,
    display_name: String,
    data_namespace: Option<String>,
    #[serde(default)]
    binary_name: Option<String>,
    #[serde(default)]
    org: Option<String>,
    #[serde(default)]
    publisher: Option<String>,
    #[serde(default)]
    url_schemes: Vec<String>,
    #[serde(default)]
    categories: Vec<String>,
    #[serde(default)]
    macos: Option<MacosIdentity>,
    #[serde(default)]
    linux: Option<LinuxIdentity>,
    #[serde(default)]
    windows: Option<WindowsIdentity>,
    #[serde(default)]
    legacy_ids: Vec<String>,
    #[serde(default)]
    min_os: Option<MinOsVersions>,
}

impl<'de> Deserialize<'de> for AppIdentity {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = AppIdentityWire::deserialize(deserializer)?;
        let data_namespace = wire.data_namespace.unwrap_or_else(|| {
            wire.app_id
                .rsplit('.')
                .next()
                .unwrap_or(&wire.app_id)
                .to_owned()
        });
        Ok(Self {
            app_id: wire.app_id,
            display_name: wire.display_name,
            data_namespace,
            binary_name: wire.binary_name,
            org: wire.org,
            publisher: wire.publisher,
            url_schemes: wire.url_schemes,
            categories: wire.categories,
            macos: wire.macos,
            linux: wire.linux,
            windows: wire.windows,
            legacy_ids: wire.legacy_ids,
            min_os: wire.min_os,
        })
    }
}

/// macOS-specific identity metadata.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MacosIdentity {
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub entitlements: Vec<String>,
    #[serde(default)]
    pub usage_strings: BTreeMap<String, String>,
}

/// Linux-specific identity metadata.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LinuxIdentity {
    #[serde(default)]
    pub categories: Vec<String>,
    #[serde(default)]
    pub desktop_keywords: Vec<String>,
}

/// Windows-specific identity metadata.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WindowsIdentity {
    #[serde(default)]
    pub publisher: Option<String>,
}

/// Per-platform minimum OS version strings.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MinOsVersions {
    #[serde(default)]
    pub macos: Option<String>,
    #[serde(default)]
    pub windows: Option<String>,
    #[serde(default)]
    pub linux: Option<String>,
}

impl AppIdentity {
    /// Validates identity fields with portable, target-independent rules.
    pub fn validate(&self) -> Result<(), ManifestError> {
        validate_app_id("app_id", &self.app_id)?;
        if self.display_name.trim().is_empty() {
            return Err(ManifestError::EmptyDisplayName {
                value: self.display_name.clone(),
            });
        }
        // An omitted namespace is derived from `app_id` (never empty). An
        // explicit empty value would otherwise pass here and only fail later in
        // `AppShell`, so reject it at the manifest boundary.
        if self.data_namespace.trim().is_empty() {
            return Err(ManifestError::EmptyDataNamespace {
                value: self.data_namespace.clone(),
            });
        }
        for scheme in &self.url_schemes {
            if !is_valid_url_scheme(scheme) {
                return Err(ManifestError::InvalidUrlScheme {
                    value: scheme.clone(),
                });
            }
        }
        for (index, legacy_id) in self.legacy_ids.iter().enumerate() {
            validate_app_id(&format!("legacy_ids[{index}]"), legacy_id)?;
        }
        Ok(())
    }

    /// Returns the explicit data namespace, or the final `app_id` segment.
    ///
    /// The fallback is never derived from `display_name`. Once an app ships,
    /// persist this value explicitly and never change it, including when `app_id`
    /// changes.
    #[must_use]
    pub fn resolved_data_namespace(&self) -> &str {
        &self.data_namespace
    }
}

fn validate_app_id(field: &str, value: &str) -> Result<(), ManifestError> {
    // `app_id` becomes the macOS `CFBundleIdentifier`, which permits only
    // alphanumerics, hyphens, and periods. Underscores would pass a looser
    // grammar here but fail code signing and App Store validation, so they are
    // rejected up front.
    let segments: Vec<_> = value.split('.').collect();
    let valid = segments.len() >= 2
        && segments.iter().all(|segment| {
            segment.is_ascii()
                && segment
                    .as_bytes()
                    .first()
                    .is_some_and(u8::is_ascii_alphabetic)
                && segment
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        });
    if valid {
        Ok(())
    } else {
        Err(ManifestError::InvalidAppId {
            field: field.to_owned(),
            value: value.to_owned(),
        })
    }
}

fn is_valid_url_scheme(value: &str) -> bool {
    value.is_ascii()
        && value.as_bytes().first().is_some_and(u8::is_ascii_lowercase)
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'+' | b'-' | b'.')
        })
}

/// Const-friendly borrowed application identity emitted into a consuming app.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IdentityRef {
    pub app_id: &'static str,
    pub display_name: &'static str,
    pub data_namespace: &'static str,
    pub binary_name: Option<&'static str>,
    pub org: Option<&'static str>,
    pub publisher: Option<&'static str>,
    pub url_schemes: &'static [&'static str],
    pub categories: &'static [&'static str],
    pub macos: Option<MacosIdentityRef>,
    pub linux: Option<LinuxIdentityRef>,
    pub windows: Option<WindowsIdentityRef>,
    pub legacy_ids: &'static [&'static str],
    pub min_os: Option<MinOsVersionsRef>,
    pub version: &'static str,
    pub cfbundle_short_version: &'static str,
    pub msix_version: &'static str,
}

/// Borrowed macOS-specific identity metadata.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MacosIdentityRef {
    pub category: Option<&'static str>,
    pub entitlements: &'static [&'static str],
    pub usage_strings: &'static [(&'static str, &'static str)],
}

/// Borrowed Linux-specific identity metadata.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LinuxIdentityRef {
    pub categories: &'static [&'static str],
    pub desktop_keywords: &'static [&'static str],
}

/// Borrowed Windows-specific identity metadata.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WindowsIdentityRef {
    pub publisher: Option<&'static str>,
}

/// Borrowed per-platform minimum OS version strings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MinOsVersionsRef {
    pub macos: Option<&'static str>,
    pub windows: Option<&'static str>,
    pub linux: Option<&'static str>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity(app_id: &str, display_name: &str) -> AppIdentity {
        AppIdentity {
            app_id: app_id.to_owned(),
            display_name: display_name.to_owned(),
            data_namespace: app_id.rsplit('.').next().unwrap_or(app_id).to_owned(),
            binary_name: None,
            org: None,
            publisher: None,
            url_schemes: Vec::new(),
            categories: Vec::new(),
            macos: None,
            linux: None,
            windows: None,
            legacy_ids: Vec::new(),
            min_os: None,
        }
    }

    #[test]
    fn validates_app_id_and_display_name() {
        assert!(identity("com.example", "Example").validate().is_ok());
        assert!(identity("com.example-app", "Example").validate().is_ok());
        assert!(identity("example", "Example").validate().is_err());
        assert!(identity("com.exa$mple", "Example").validate().is_err());
        assert!(identity("com.example", "").validate().is_err());
        assert!(identity("com.example", "  \n").validate().is_err());
    }

    #[test]
    fn rejects_underscore_in_app_id() {
        // `app_id` becomes the macOS CFBundleIdentifier, which forbids `_`.
        let error = identity("com.example.my_app", "Example")
            .validate()
            .expect_err("underscore is invalid in CFBundleIdentifier");
        assert!(error.to_string().contains("CFBundleIdentifier"));
    }

    #[test]
    fn validates_url_schemes() {
        for scheme in ["example", "example+dev", "example-2.0"] {
            let mut value = identity("com.example", "Example");
            value.url_schemes.push(scheme.to_owned());
            assert!(value.validate().is_ok(), "scheme {scheme:?} should pass");
        }
        for scheme in ["Example", "example app", "example_app", "1example"] {
            let mut value = identity("com.example", "Example");
            value.url_schemes.push(scheme.to_owned());
            assert!(value.validate().is_err(), "scheme {scheme:?} should fail");
        }
    }

    #[test]
    fn rejects_explicit_empty_data_namespace() {
        let mut value = identity("com.example.app", "Example");
        assert!(value.validate().is_ok());
        value.data_namespace = String::new();
        assert!(value.validate().is_err());
        value.data_namespace = "  ".to_owned();
        assert!(value.validate().is_err());
    }

    #[test]
    fn resolves_stable_data_namespace() {
        let mut value = identity("com.example.my_app", "Mutable Name");
        assert_eq!(value.resolved_data_namespace(), "my_app");
        value.data_namespace = "stable-data".to_owned();
        assert_eq!(value.resolved_data_namespace(), "stable-data");
    }

    #[test]
    fn validates_legacy_ids_like_app_id() {
        let mut value = identity("com.example.current", "Example");
        value.legacy_ids.push("org.example.previous".to_owned());
        assert!(value.validate().is_ok());
        value.legacy_ids.push("invalid".to_owned());
        let error = value.validate().expect_err("single-segment legacy ID");
        assert!(error.to_string().contains("legacy_ids[1]"));
    }
}
