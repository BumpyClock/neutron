//! Deterministic platform packaging version derivation.

use crate::ManifestError;

/// Derives Apple's `CFBundleShortVersionString` from canonical SemVer.
///
/// Apple requires `CFBundleShortVersionString` to use exactly three dotted
/// integers. SemVer prerelease and build metadata are therefore removed.
///
/// # Errors
///
/// Returns [`ManifestError::InvalidSemver`] when `semver` is malformed.
pub fn derive_cfbundle_short_version(semver: &str) -> Result<String, ManifestError> {
    let core = parse_semver(semver)?;
    Ok(format!("{}.{}.{}", core.major, core.minor, core.patch))
}

/// The packaging target that determines the MSIX version-component rules.
///
/// The two targets cannot share one version string: the Store requires a
/// nonzero first component and reserves the fourth, while sideloaded packages
/// are free to carry a build number in the fourth component.
///
/// See the MSIX app package requirements:
/// <https://learn.microsoft.com/en-us/windows/apps/publish/publish-your-app/msix/app-package-requirements>
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MsixTarget {
    /// Microsoft Store submission. The Store requires a nonzero first component
    /// (major) and reserves the fourth component, which must be zero.
    Store,
    /// Sideloaded or otherwise non-Store MSIX. The fourth component may carry a
    /// build number.
    Sideload,
}

/// Derives the four-part numeric MSIX package version for the given `target`.
///
/// For [`MsixTarget::Store`], the fourth component is always zero (the Store
/// reserves and overwrites it) and a zero `major` is rejected, because Store
/// packages require a nonzero first component; `build` is ignored. For
/// [`MsixTarget::Sideload`], `build` fills the fourth component and defaults to
/// zero.
///
/// See the MSIX app package requirements:
/// <https://learn.microsoft.com/en-us/windows/apps/publish/publish-your-app/msix/app-package-requirements>
///
/// # Errors
///
/// Returns [`ManifestError::MsixStoreZeroMajor`] when `target` is
/// [`MsixTarget::Store`] and `major` is zero, [`ManifestError::InvalidSemver`]
/// for malformed SemVer, or [`ManifestError::InvalidMsixComponent`] for any
/// component above 65535.
pub fn derive_msix_version(
    semver: &str,
    target: MsixTarget,
    build: Option<u32>,
) -> Result<String, ManifestError> {
    let core = parse_semver(semver)?;
    let build = match target {
        MsixTarget::Store => {
            if core.major == 0 {
                return Err(ManifestError::MsixStoreZeroMajor {
                    value: semver.to_owned(),
                });
            }
            0
        }
        MsixTarget::Sideload => u64::from(build.unwrap_or(0)),
    };
    for (field, value) in [
        ("major", core.major),
        ("minor", core.minor),
        ("patch", core.patch),
        ("build", build),
    ] {
        if value > u64::from(u16::MAX) {
            return Err(ManifestError::InvalidMsixComponent { field, value });
        }
    }
    Ok(format!(
        "{}.{}.{}.{}",
        core.major, core.minor, core.patch, build
    ))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SemverCore {
    major: u64,
    minor: u64,
    patch: u64,
}

fn parse_semver(value: &str) -> Result<SemverCore, ManifestError> {
    let (without_build, build) = split_build(value)?;
    if let Some(build) = build {
        validate_identifiers(value, build, false, "build metadata")?;
    }
    let (core, prerelease) = without_build
        .split_once('-')
        .map_or((without_build, None), |(core, suffix)| (core, Some(suffix)));
    if let Some(prerelease) = prerelease {
        validate_identifiers(value, prerelease, true, "prerelease")?;
    }

    let mut parts = core.split('.');
    let major = parse_core_part(value, parts.next(), "major")?;
    let minor = parse_core_part(value, parts.next(), "minor")?;
    let patch = parse_core_part(value, parts.next(), "patch")?;
    if parts.next().is_some() {
        return invalid_semver(value, "expected exactly MAJOR.MINOR.PATCH");
    }
    Ok(SemverCore {
        major,
        minor,
        patch,
    })
}

fn split_build(value: &str) -> Result<(&str, Option<&str>), ManifestError> {
    let Some((head, tail)) = value.split_once('+') else {
        return Ok((value, None));
    };
    if tail.contains('+') {
        return invalid_semver(value, "multiple '+' separators");
    }
    Ok((head, Some(tail)))
}

fn parse_core_part(input: &str, part: Option<&str>, field: &str) -> Result<u64, ManifestError> {
    let Some(part) = part else {
        return invalid_semver(input, "expected exactly MAJOR.MINOR.PATCH");
    };
    if part.is_empty() || !part.bytes().all(|byte| byte.is_ascii_digit()) {
        return invalid_semver(input, &format!("{field} must be an unsigned integer"));
    }
    if part.len() > 1 && part.starts_with('0') {
        return invalid_semver(input, &format!("{field} must not contain leading zeros"));
    }
    part.parse().map_err(|error| ManifestError::InvalidSemver {
        value: input.to_owned(),
        reason: format!("{field} is outside the supported integer range: {error}"),
    })
}

fn validate_identifiers(
    input: &str,
    suffix: &str,
    reject_numeric_leading_zero: bool,
    field: &str,
) -> Result<(), ManifestError> {
    for identifier in suffix.split('.') {
        let valid_chars = !identifier.is_empty()
            && identifier
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-');
        let invalid_numeric_zero = reject_numeric_leading_zero
            && identifier.len() > 1
            && identifier.bytes().all(|byte| byte.is_ascii_digit())
            && identifier.starts_with('0');
        if !valid_chars || invalid_numeric_zero {
            return invalid_semver(input, &format!("invalid {field} identifier {identifier:?}"));
        }
    }
    Ok(())
}

fn invalid_semver<T>(value: &str, reason: &str) -> Result<T, ManifestError> {
    Err(ManifestError::InvalidSemver {
        value: value.to_owned(),
        reason: reason.to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derives_platform_versions_from_valid_semver() {
        for value in [
            "1.2.3",
            "1.2.3-beta.1",
            "1.2.3+abc",
            "1.2.3-beta.1+abc123",
            "1.2.3-alpha-beta",
        ] {
            assert_eq!(derive_cfbundle_short_version(value).unwrap(), "1.2.3");
            assert_eq!(
                derive_msix_version(value, MsixTarget::Sideload, None).unwrap(),
                "1.2.3.0"
            );
            assert_eq!(
                derive_msix_version(value, MsixTarget::Store, None).unwrap(),
                "1.2.3.0"
            );
        }
        assert_eq!(
            derive_msix_version("1.2.3", MsixTarget::Sideload, Some(42)).unwrap(),
            "1.2.3.42"
        );
    }

    #[test]
    fn store_reserves_fourth_component_and_rejects_zero_major() {
        // The Store overwrites the fourth component, so `build` is ignored.
        assert_eq!(
            derive_msix_version("1.2.3", MsixTarget::Store, Some(42)).unwrap(),
            "1.2.3.0"
        );
        // The Store requires a nonzero first component; sideload allows it.
        assert!(matches!(
            derive_msix_version("0.4.2", MsixTarget::Store, None),
            Err(ManifestError::MsixStoreZeroMajor { .. })
        ));
        assert_eq!(
            derive_msix_version("0.4.2", MsixTarget::Sideload, Some(7)).unwrap(),
            "0.4.2.7"
        );
    }

    #[test]
    fn accepts_max_msix_component_and_rejects_overflow() {
        // 65535 is the largest value each MSIX component may hold.
        assert_eq!(
            derive_msix_version("1.2.65535", MsixTarget::Sideload, None).unwrap(),
            "1.2.65535.0"
        );
        // 65536 overflows the patch component.
        assert!(matches!(
            derive_msix_version("1.2.65536", MsixTarget::Sideload, None),
            Err(ManifestError::InvalidMsixComponent {
                field: "patch",
                value: 65536
            })
        ));
        // The sideload build component is validated on the same boundary.
        assert_eq!(
            derive_msix_version("1.2.3", MsixTarget::Sideload, Some(65535)).unwrap(),
            "1.2.3.65535"
        );
        assert!(matches!(
            derive_msix_version("1.2.3", MsixTarget::Sideload, Some(65536)),
            Err(ManifestError::InvalidMsixComponent {
                field: "build",
                value: 65536
            })
        ));
    }

    #[test]
    fn rejects_malformed_semver_without_panicking() {
        for value in ["1.2", "not-a-version", "01.2.3", "1.2.3-beta..1"] {
            assert!(derive_cfbundle_short_version(value).is_err());
            assert!(derive_msix_version(value, MsixTarget::Sideload, None).is_err());
        }
    }
}
