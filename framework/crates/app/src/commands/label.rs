//! Static and runtime-derived presentation text (issue #11).
//!
//! A label is either fixed at declaration time or derived from the current
//! application state each time menus are (re)projected. Derived labels are
//! non-capturing `fn` pointers, not closures: a declaration is a pure value, so
//! it must not own runtime state.

use gpui::{App, SharedString};

/// The shared representation behind [`CommandLabel`] and [`MenuLabel`]. Kept
/// private so the two label kinds stay distinct types at every call site.
#[derive(Clone)]
enum Text {
    Static(SharedString),
    Derived(fn(&App) -> SharedString),
}

impl Text {
    fn resolve(&self, cx: &App) -> SharedString {
        match self {
            Self::Static(text) => text.clone(),
            Self::Derived(f) => f(cx),
        }
    }

    fn static_text(&self) -> Option<&SharedString> {
        match self {
            Self::Static(text) => Some(text),
            Self::Derived(_) => None,
        }
    }

    fn derived(&self) -> Option<fn(&App) -> SharedString> {
        match self {
            Self::Static(_) => None,
            Self::Derived(f) => Some(*f),
        }
    }
}

impl std::fmt::Debug for Text {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Static(text) => f.debug_tuple("Static").field(text).finish(),
            Self::Derived(_) => f.write_str("Derived(..)"),
        }
    }
}

/// The label shown for a command in every surface that projects it.
#[derive(Clone, Debug)]
pub struct CommandLabel(Text);

impl CommandLabel {
    /// A label fixed at declaration time.
    pub fn text(text: impl Into<SharedString>) -> Self {
        Self(Text::Static(text.into()))
    }

    /// A label recomputed from application state on every projection.
    pub fn derived(f: fn(&App) -> SharedString) -> Self {
        Self(Text::Derived(f))
    }

    /// Resolve the label against the current application state.
    #[allow(dead_code)] // Exercised only by unit tests; no production caller yet.
    pub(crate) fn resolve(&self, cx: &App) -> SharedString {
        self.0.resolve(cx)
    }

    /// The fixed text, when this label is static.
    pub(crate) fn static_text(&self) -> Option<&SharedString> {
        self.0.static_text()
    }

    /// The deriving function, when this label is dynamic.
    pub(crate) fn derived_fn(&self) -> Option<fn(&App) -> SharedString> {
        self.0.derived()
    }
}

impl<T: Into<SharedString>> From<T> for CommandLabel {
    fn from(text: T) -> Self {
        Self::text(text)
    }
}

/// The title shown for a top-level menu.
#[derive(Clone, Debug)]
pub struct MenuLabel(Text);

impl MenuLabel {
    /// A title fixed at declaration time.
    pub fn text(text: impl Into<SharedString>) -> Self {
        Self(Text::Static(text.into()))
    }

    /// A title recomputed from application state on every projection. The
    /// standard application menu uses this to render the app display name.
    pub fn derived(f: fn(&App) -> SharedString) -> Self {
        Self(Text::Derived(f))
    }

    /// Resolve the title against the current application state.
    pub(crate) fn resolve(&self, cx: &App) -> SharedString {
        self.0.resolve(cx)
    }

    /// The fixed text, when this title is static.
    #[allow(dead_code)] // Exercised only by unit tests; no production caller yet.
    pub(crate) fn static_text(&self) -> Option<&SharedString> {
        self.0.static_text()
    }
}

impl<T: Into<SharedString>> From<T> for MenuLabel {
    fn from(text: T) -> Self {
        Self::text(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app_derived(_: &App) -> SharedString {
        "derived".into()
    }

    #[test]
    fn static_and_derived_labels_are_distinguishable_without_an_app() {
        let fixed = CommandLabel::from("Copy");
        let dynamic = CommandLabel::derived(app_derived);

        assert_eq!(fixed.static_text().map(SharedString::as_ref), Some("Copy"));
        assert!(fixed.derived_fn().is_none());
        assert!(dynamic.static_text().is_none());
        assert!(dynamic.derived_fn().is_some());
        assert_eq!(
            MenuLabel::from("Edit")
                .static_text()
                .map(SharedString::as_ref),
            Some("Edit"),
        );
        assert!(MenuLabel::derived(app_derived).static_text().is_none());
    }

    #[gpui::test]
    fn labels_resolve_against_live_application_state(cx: &mut gpui::TestAppContext) {
        fn window_count(cx: &App) -> SharedString {
            format!("Windows: {}", cx.windows().len()).into()
        }

        cx.update(|cx| {
            assert_eq!(CommandLabel::from("Copy").resolve(cx).as_ref(), "Copy");
            assert_eq!(
                CommandLabel::derived(window_count).resolve(cx).as_ref(),
                "Windows: 0",
            );
            assert_eq!(
                MenuLabel::derived(window_count).resolve(cx).as_ref(),
                "Windows: 0",
            );
        });
    }
}
