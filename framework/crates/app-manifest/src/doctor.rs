//! Read-only verification of app identity in existing packaging artifacts.
//!
//! Artifact parsing is deliberately targeted for v1: plist and XML readers only
//! recognize simple string keys/quoted attributes, desktop files use line-based
//! `key=value` matching, and Flatpak JSON uses quoted top-level identity keys.
//! This is not a general parser for any of those formats.

use std::{
    fs,
    path::{Path, PathBuf},
};

use gpui_component_manifest::schema::AppIdentity;

/// Outcome of one identity comparison.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Ok,
    Mismatch,
    Missing,
}

impl Status {
    /// Stable display and JSON representation.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "OK",
            Self::Mismatch => "MISMATCH",
            Self::Missing => "MISSING",
        }
    }
}

/// One row in a doctor report.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Check {
    pub field: String,
    pub manifest_value: String,
    pub artifact_value: String,
    pub status: Status,
}

/// Complete verification result.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Report {
    pub checks: Vec<Check>,
}

impl Report {
    /// Missing artifact fields fail verification, just like differing values.
    pub fn has_failures(&self) -> bool {
        self.checks.iter().any(|check| check.status != Status::Ok)
    }
}

/// Errors that prevent verification from completing.
///
/// The two heavy payloads (`ManifestError`, `toml::de::Error`) are boxed so the
/// error stays small: it rides in every `Result<_, DoctorError>` on the success
/// path, and an unboxed variant pushes the type past clippy's
/// `result_large_err` threshold (caught only by CI's newer toolchain). The const
/// assertion below guards against a regression.
#[derive(Debug, thiserror::Error)]
pub enum DoctorError {
    #[error(transparent)]
    Manifest(Box<gpui_component_manifest::ManifestError>),
    #[error("failed to read {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse packaging metadata in {path}: {source}")]
    Toml {
        path: PathBuf,
        #[source]
        source: Box<toml::de::Error>,
    },
}

const _: () = assert!(std::mem::size_of::<DoctorError>() <= 64);

/// Verifies identity metadata against recognized artifacts below `root`.
///
/// # Errors
///
/// Returns an error when the canonical manifest, a directory, or a recognized
/// artifact cannot be read, or when the root Cargo manifest is malformed.
pub fn verify(root: &Path) -> Result<Report, DoctorError> {
    let cargo_path = root.join("Cargo.toml");
    let identity = gpui_component_manifest::parse::parse_identity(&cargo_path)
        .map_err(|e| DoctorError::Manifest(Box::new(e)))?;
    let cargo_source = read(&cargo_path)?;
    let cargo: toml::Value = toml::from_str(&cargo_source).map_err(|source| DoctorError::Toml {
        path: cargo_path,
        source: Box::new(source),
    })?;
    let mut report = Report::default();
    verify_bundle(&cargo, &identity, &mut report);

    let mut files = Vec::new();
    collect_files(root, root, &mut files)?;
    files.sort();
    // Accumulate the contents of every candidate entitlements file so each
    // declared entitlement key can be verified against the union below — a
    // matching filename alone (possibly empty or carrying unrelated keys) must
    // not pass a required entitlement.
    let mut entitlement_source = String::new();
    for path in files {
        // Check-field names are stable identifiers, not I/O paths: always use
        // forward slashes so reports (and tests) are byte-identical on Windows.
        let relative = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .display()
            .to_string()
            .replace('\\', "/");
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default();
        let lower = name.to_ascii_lowercase();
        if name == "Info.plist" {
            verify_info(&relative, &read(&path)?, &identity, &mut report);
        } else if lower.ends_with("entitlements.plist") {
            entitlement_source.push_str(&read(&path)?);
            entitlement_source.push('\n');
        } else if path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("desktop"))
        {
            verify_desktop(&relative, &read(&path)?, &identity, &mut report);
        } else if name.eq_ignore_ascii_case("AppxManifest.xml") {
            verify_msix(&relative, &read(&path)?, &identity, &mut report);
        } else if path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("wxs"))
        {
            verify_wix(&relative, &read(&path)?, &identity, &mut report);
        } else if path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("json"))
        {
            let source = read(&path)?;
            let value = json_string(&source, "app-id").or_else(|| json_string(&source, "id"));
            if is_flatpak_candidate(&path, &identity.app_id, &source, value.is_some()) {
                compare(
                    &mut report,
                    format!("{relative}:id"),
                    &identity.app_id,
                    value,
                );
            }
        }
    }
    // Verify every declared entitlement key is actually present, not merely that
    // some entitlements file exists. A missing file leaves `entitlement_source`
    // empty, so each required key is reported `Missing`.
    if let Some(macos) = identity.macos.as_ref() {
        for key in &macos.entitlements {
            presence(
                &mut report,
                format!("entitlements.plist:{key}"),
                plist_has_key(&entitlement_source, key),
            );
        }
    }
    Ok(report)
}

fn read(path: &Path) -> Result<String, DoctorError> {
    fs::read_to_string(path).map_err(|source| DoctorError::Read {
        path: path.to_owned(),
        source,
    })
}

fn collect_files(root: &Path, path: &Path, files: &mut Vec<PathBuf>) -> Result<(), DoctorError> {
    let entries = fs::read_dir(path).map_err(|source| DoctorError::Read {
        path: path.to_owned(),
        source,
    })?;
    for entry in entries {
        let entry = entry.map_err(|source| DoctorError::Read {
            path: path.to_owned(),
            source,
        })?;
        let file_type = entry.file_type().map_err(|source| DoctorError::Read {
            path: entry.path(),
            source,
        })?;
        if file_type.is_dir() {
            let name = entry.file_name();
            if name != ".git" && name != "target" {
                collect_files(root, &entry.path(), files)?;
            }
        } else if file_type.is_file() && entry.path() != root.join("Cargo.toml") {
            files.push(entry.path());
        }
    }
    Ok(())
}

fn verify_bundle(cargo: &toml::Value, identity: &AppIdentity, report: &mut Report) {
    let Some(bundle) = cargo
        .get("package")
        .and_then(|v| v.get("metadata"))
        .and_then(|v| v.get("bundle"))
    else {
        return;
    };
    compare(
        report,
        "Cargo.toml:bundle.identifier".to_owned(),
        &identity.app_id,
        bundle.get("identifier").and_then(toml::Value::as_str),
    );
    compare(
        report,
        "Cargo.toml:bundle.name".to_owned(),
        &identity.display_name,
        bundle.get("name").and_then(toml::Value::as_str),
    );
    let icon = bundle.get("icon").filter(|value| match value {
        toml::Value::String(v) => !v.trim().is_empty(),
        toml::Value::Array(values) => {
            !values.is_empty()
                && values
                    .iter()
                    .all(|value| value.as_str().is_some_and(|value| !value.trim().is_empty()))
        }
        _ => false,
    });
    report.checks.push(Check {
        field: "Cargo.toml:bundle.icon".to_owned(),
        manifest_value: "present".to_owned(),
        artifact_value: icon.map(toml_value).unwrap_or_else(|| "-".to_owned()),
        status: if icon.is_some() {
            Status::Ok
        } else {
            Status::Missing
        },
    });
}

fn verify_info(path: &str, source: &str, identity: &AppIdentity, report: &mut Report) {
    compare(
        report,
        format!("{path}:CFBundleIdentifier"),
        &identity.app_id,
        plist_string(source, "CFBundleIdentifier"),
    );
    compare(
        report,
        format!("{path}:CFBundleName"),
        &identity.display_name,
        plist_string(source, "CFBundleName"),
    );
    if let Some(macos) = &identity.macos {
        for (key, expected) in &macos.usage_strings {
            presence_string(
                report,
                format!("{path}:{key}"),
                expected,
                plist_string(source, key),
            );
        }
    }
}

fn verify_desktop(path: &str, source: &str, identity: &AppIdentity, report: &mut Report) {
    compare(
        report,
        format!("{path}:Name"),
        &identity.display_name,
        desktop_value(source, "Name"),
    );
    if let Some(binary) = identity.binary_name.as_deref() {
        compare(
            report,
            format!("{path}:Exec"),
            binary,
            desktop_value(source, "Exec").map(exec_program),
        );
        compare(
            report,
            format!("{path}:StartupWMClass"),
            binary,
            desktop_value(source, "StartupWMClass"),
        );
    } else {
        presence_value(
            report,
            format!("{path}:Exec"),
            desktop_value(source, "Exec"),
        );
        presence_value(
            report,
            format!("{path}:StartupWMClass"),
            desktop_value(source, "StartupWMClass"),
        );
    }
    presence_value(
        report,
        format!("{path}:Icon"),
        desktop_value(source, "Icon"),
    );
}

fn verify_msix(path: &str, source: &str, identity: &AppIdentity, report: &mut Report) {
    compare(
        report,
        format!("{path}:Identity.Name"),
        &identity.app_id,
        xml_attribute(source, "Identity", "Name"),
    );
}

fn verify_wix(path: &str, source: &str, _identity: &AppIdentity, report: &mut Report) {
    presence(
        report,
        format!("{path}:GUID"),
        has_assignment(source, "Guid"),
    );
    presence(
        report,
        format!("{path}:Name"),
        has_assignment(source, "Name"),
    );
}

fn compare(report: &mut Report, field: String, expected: &str, actual: Option<&str>) {
    let status = match actual {
        None => Status::Missing,
        Some(value) if value == expected => Status::Ok,
        Some(_) => Status::Mismatch,
    };
    report.checks.push(Check {
        field,
        manifest_value: expected.to_owned(),
        artifact_value: actual.unwrap_or("-").to_owned(),
        status,
    });
}

fn presence(report: &mut Report, field: String, found: bool) {
    report.checks.push(Check {
        field,
        manifest_value: "present".to_owned(),
        artifact_value: if found { "present" } else { "-" }.to_owned(),
        status: if found { Status::Ok } else { Status::Missing },
    });
}

fn presence_value(report: &mut Report, field: String, actual: Option<&str>) {
    report.checks.push(Check {
        field,
        manifest_value: "present".to_owned(),
        artifact_value: actual.unwrap_or("-").to_owned(),
        status: if actual.is_some_and(|v| !v.trim().is_empty()) {
            Status::Ok
        } else {
            Status::Missing
        },
    });
}

fn presence_string(report: &mut Report, field: String, expected: &str, actual: Option<&str>) {
    report.checks.push(Check {
        field,
        manifest_value: expected.to_owned(),
        artifact_value: actual.unwrap_or("-").to_owned(),
        status: if actual.is_some_and(|value| !value.trim().is_empty()) {
            Status::Ok
        } else {
            Status::Missing
        },
    });
}

fn toml_value(value: &toml::Value) -> String {
    match value {
        toml::Value::String(v) => v.clone(),
        toml::Value::Array(v) => v.iter().map(toml_value).collect::<Vec<_>>().join(", "),
        _ => value.to_string(),
    }
}

fn plist_string<'a>(source: &'a str, key: &str) -> Option<&'a str> {
    let after_key = source.split_once(&format!("<key>{key}</key>"))?.1;
    let after_open = after_key.split_once("<string>")?.1;
    Some(after_open.split_once("</string>")?.0.trim())
}

fn desktop_value<'a>(source: &'a str, key: &str) -> Option<&'a str> {
    // Only the `[Desktop Entry]` group defines the main launcher fields. Keys in
    // `[Desktop Action ...]` groups must not satisfy a missing launcher field.
    let mut in_desktop_entry = false;
    for line in source.lines().map(str::trim) {
        if line.starts_with('[') && line.ends_with(']') {
            in_desktop_entry = line == "[Desktop Entry]";
            continue;
        }
        if !in_desktop_entry || line.starts_with('#') {
            continue;
        }
        if let Some(value) = line
            .strip_prefix(key)
            .and_then(|rest| rest.strip_prefix('='))
        {
            return Some(value.trim());
        }
    }
    None
}

fn plist_has_key(source: &str, key: &str) -> bool {
    source.contains(&format!("<key>{key}</key>"))
}

fn exec_program(value: &str) -> &str {
    let program = value.split_whitespace().next().unwrap_or_default();
    Path::new(program)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(program)
}

fn xml_attribute<'a>(source: &'a str, tag: &str, attribute: &str) -> Option<&'a str> {
    let tag_start = source.find(&format!("<{tag}"))?;
    let tag_source = source.get(tag_start..source.get(tag_start..)?.find('>')? + tag_start)?;
    attribute_value(tag_source, attribute)
}

fn json_string<'a>(source: &'a str, key: &str) -> Option<&'a str> {
    let bytes = source.as_bytes();
    let mut depth = 0_u32;
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'{' => depth += 1,
            b'}' => depth = depth.saturating_sub(1),
            b'"' => {
                let end = json_quote_end(bytes, index + 1)?;
                if depth == 1 && source.get(index + 1..end) == Some(key) {
                    let after_key = source.get(end + 1..)?.trim_start();
                    if let Some(after_colon) = after_key.strip_prefix(':') {
                        return quoted(after_colon.trim_start());
                    }
                }
                index = end;
            }
            _ => {}
        }
        index += 1;
    }
    None
}

fn json_quote_end(bytes: &[u8], mut index: usize) -> Option<usize> {
    let mut escaped = false;
    while index < bytes.len() {
        match bytes[index] {
            b'"' if !escaped => return Some(index),
            b'\\' => escaped = !escaped,
            _ => escaped = false,
        }
        index += 1;
    }
    None
}

fn is_flatpak_candidate(path: &Path, app_id: &str, source: &str, has_id: bool) -> bool {
    path.file_stem().and_then(|name| name.to_str()) == Some(app_id)
        || path.components().any(|part| {
            part.as_os_str()
                .to_str()
                .is_some_and(|part| part.to_ascii_lowercase().contains("flatpak"))
        })
        || has_id
            && ["runtime", "sdk", "command"]
                .iter()
                .any(|key| json_string(source, key).is_some())
}

fn has_assignment(source: &str, key: &str) -> bool {
    source.match_indices(key).any(|(index, _)| {
        let before = source
            .get(..index)
            .and_then(|value| value.chars().next_back());
        let after = source.get(index + key.len()..).map(str::trim_start);
        before.is_some_and(|value| value.is_ascii_whitespace())
            && after
                .and_then(|value| value.strip_prefix('='))
                .map(str::trim_start)
                .is_some_and(|value| quoted(value).is_some_and(|value| !value.is_empty()))
    })
}

fn attribute_value<'a>(source: &'a str, key: &str) -> Option<&'a str> {
    source.match_indices(key).find_map(|(index, _)| {
        let before = source.get(..index)?.chars().next_back()?;
        let after = source.get(index + key.len()..)?.trim_start();
        (before.is_ascii_whitespace() && after.starts_with('='))
            .then(|| quoted(after.strip_prefix('=')?.trim_start()))
            .flatten()
    })
}

fn quoted(source: &str) -> Option<&str> {
    let quote = source.chars().next()?;
    if quote != '\'' && quote != '"' {
        return None;
    }
    source.get(1..)?.split_once(quote).map(|(value, _)| value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_value_only_reads_desktop_entry_group() {
        let source = "\
[Desktop Entry]
Name=Real App

[Desktop Action new]
Name=New Window
Exec=/opt/app --new";
        assert_eq!(desktop_value(source, "Name"), Some("Real App"));
        // `Exec` exists only in the action group, so the launcher field is absent.
        assert_eq!(desktop_value(source, "Exec"), None);
    }

    #[test]
    fn desktop_value_skips_keys_before_first_group() {
        let source = "Name=Stray\n[Desktop Entry]\nName=Real";
        assert_eq!(desktop_value(source, "Name"), Some("Real"));
    }

    #[test]
    fn plist_has_key_matches_declared_entitlements() {
        let source = "<dict><key>com.apple.security.network.client</key><true/></dict>";
        assert!(plist_has_key(source, "com.apple.security.network.client"));
        assert!(!plist_has_key(source, "com.apple.security.network.server"));
        assert!(!plist_has_key("", "com.apple.security.network.client"));
    }
}
