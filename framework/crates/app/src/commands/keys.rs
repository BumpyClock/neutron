//! Validated menu identities for the typed declaration model (issue #11).
//!
//! [`MenuKey`] names one top-level menu; [`MenuSectionKey`] names one reserved
//! slot inside a menu that a runtime provider fills. Both are `&'static str`
//! newtypes so they stay cheap to copy and compare, but — unlike the bare
//! string constants the registry uses — they can only be built from text
//! that passed [`validate_key`].
//!
//! Framework constants are built through the unchecked constructor so they stay
//! usable in `const` position; [`tests::framework_constants_are_valid`] proves
//! every one of them would also pass the runtime validator.

use std::fmt;

/// Why a menu or section key is unusable.
///
/// Static because every rejection is a fixed structural rule, not formatted
/// input.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum KeyFault {
    /// The key is empty.
    Empty,
    /// The key has leading or trailing whitespace.
    Untrimmed,
    /// The key contains a control character.
    ControlCharacter,
}

impl KeyFault {
    /// A stable phrase for diagnostics, e.g. "is empty".
    pub const fn reason(self) -> &'static str {
        match self {
            Self::Empty => "is empty",
            Self::Untrimmed => "has leading or trailing whitespace",
            Self::ControlCharacter => "contains a control character",
        }
    }
}

impl fmt::Display for KeyFault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.reason())
    }
}

/// The single validation rule shared by both key kinds: non-empty, trimmed, and
/// free of control characters, so a key is always safe to render in a menu and
/// to compare as an identity.
fn validate_key(raw: &str) -> Result<(), KeyFault> {
    if raw.is_empty() {
        return Err(KeyFault::Empty);
    }
    if raw.trim() != raw {
        return Err(KeyFault::Untrimmed);
    }
    if raw.chars().any(char::is_control) {
        return Err(KeyFault::ControlCharacter);
    }
    Ok(())
}

/// A validated top-level menu identity.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct MenuKey(&'static str);

impl MenuKey {
    /// The application menu. Rendered with the app display name on macOS.
    pub const APP: Self = Self("App");
    /// The File menu (the application block's home on Windows and Linux).
    pub const FILE: Self = Self("File");
    /// The Edit menu.
    pub const EDIT: Self = Self("Edit");
    /// The View menu (Windows and Linux Appearance host).
    pub const VIEW: Self = Self("View");
    /// The Window menu.
    pub const WINDOW: Self = Self("Window");
    /// The Help menu (Windows and Linux About host).
    pub const HELP: Self = Self("Help");
    /// The pseudo-menu projected into the macOS dock menu.
    pub const DOCK: Self = Self("Dock");

    /// Every framework-owned key, in the order used for diagnostics and tests.
    pub const FRAMEWORK: [Self; 7] = [
        Self::APP,
        Self::FILE,
        Self::EDIT,
        Self::VIEW,
        Self::WINDOW,
        Self::HELP,
        Self::DOCK,
    ];

    /// Build an application-owned menu key.
    ///
    /// # Errors
    ///
    /// Returns the first [`KeyFault`] the text violates.
    pub fn new(raw: &'static str) -> Result<Self, KeyFault> {
        validate_key(raw).map(|()| Self(raw))
    }

    /// The underlying text.
    pub const fn as_str(self) -> &'static str {
        self.0
    }

    /// Whether this is the dock projection key rather than a menu-bar menu.
    pub const fn is_dock(self) -> bool {
        // `PartialEq` is not const; compare the identity strings by pointer-free
        // byte equality instead.
        const_str_eq(self.0, MenuKey::DOCK.0)
    }
}

impl fmt::Display for MenuKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

/// A validated identity for a reserved menu section filled by a provider.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct MenuSectionKey(&'static str);

impl MenuSectionKey {
    /// The Appearance/Theme section fed by the theme service.
    pub const THEME: Self = Self("appearance");

    /// Every framework-owned section key.
    pub const FRAMEWORK: [Self; 1] = [Self::THEME];

    /// Build an application-owned section key.
    ///
    /// # Errors
    ///
    /// Returns the first [`KeyFault`] the text violates.
    pub fn new(raw: &'static str) -> Result<Self, KeyFault> {
        validate_key(raw).map(|()| Self(raw))
    }

    /// The underlying text, which is also the registry's section slot.
    pub const fn as_str(self) -> &'static str {
        self.0
    }
}

impl fmt::Display for MenuSectionKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

/// `str` equality usable from `const fn` (`PartialEq` is not const).
const fn const_str_eq(left: &str, right: &str) -> bool {
    let (left, right) = (left.as_bytes(), right.as_bytes());
    if left.len() != right.len() {
        return false;
    }
    let mut index = 0;
    while index < left.len() {
        if left[index] != right[index] {
            return false;
        }
        index += 1;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn framework_constants_are_valid() {
        for key in MenuKey::FRAMEWORK {
            MenuKey::new(key.as_str()).expect("framework menu key passes validation");
        }
        for key in MenuSectionKey::FRAMEWORK {
            MenuSectionKey::new(key.as_str()).expect("framework section key passes validation");
        }
    }

    #[test]
    fn invalid_text_is_rejected_with_its_reason() {
        assert_eq!(MenuKey::new(""), Err(KeyFault::Empty));
        assert_eq!(MenuKey::new(" Tools"), Err(KeyFault::Untrimmed));
        assert_eq!(MenuKey::new("Tools "), Err(KeyFault::Untrimmed));
        assert_eq!(MenuKey::new("To\nols"), Err(KeyFault::ControlCharacter));
        assert_eq!(MenuSectionKey::new(""), Err(KeyFault::Empty));
        assert_eq!(
            MenuSectionKey::new("locale\t"),
            Err(KeyFault::Untrimmed),
            "a trailing tab is both untrimmed and control; the trim rule is \
             checked first, so it is the reported reason",
        );
        assert_eq!(
            MenuSectionKey::new("loc\u{7}ale"),
            Err(KeyFault::ControlCharacter),
        );
    }

    #[test]
    fn valid_application_keys_round_trip() {
        assert_eq!(
            MenuKey::new("Tools").expect("valid").as_str(),
            "Tools",
            "an application key keeps its exact text",
        );
        assert_eq!(
            MenuSectionKey::new("story.locale").expect("valid").as_str(),
            "story.locale",
        );
    }

    #[test]
    fn dock_is_distinguishable_from_menu_bar_keys() {
        assert!(MenuKey::DOCK.is_dock());
        assert!(!MenuKey::APP.is_dock());
        assert!(
            !MenuKey::new("Dockyard").expect("valid").is_dock(),
            "a prefix match is not the dock key",
        );
    }
}
