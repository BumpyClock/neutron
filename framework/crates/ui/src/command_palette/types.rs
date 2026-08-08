//! Types for the Command Palette component.

use gpui::SharedString;
use std::any::Any;
use std::sync::Arc;

/// Configuration for the Command Palette.
#[derive(Clone)]
pub struct CommandPaletteConfig {
    /// The keyboard shortcut to open the palette. Default: "cmd-p" on macOS, "ctrl-p" elsewhere.
    /// Set to None to disable the default keybinding.
    pub shortcut: Option<SharedString>,
    /// The matcher implementation to use. Default: Nucleo.
    pub matcher: CommandMatcherKind,
    /// Maximum number of results to display. Default: 50.
    pub max_results: usize,
    /// Placeholder text for the search input.
    pub placeholder: SharedString,
    /// Width of the palette in pixels. Default: 560.0.
    pub width: f32,
    /// Maximum height of the palette in pixels. Default: 400.0.
    pub max_height: f32,
    /// Whether to show the footer with keyboard hints. Default: true.
    pub show_footer: bool,
    /// Whether to show category inline with item. Default: true.
    pub show_categories_inline: bool,
    /// Optional title for the commands section when a query is present. Default: "Commands".
    pub commands_section_title: Option<SharedString>,
    /// Optional title for the results section when a query is present. Default: "Search Results".
    pub results_section_title: Option<SharedString>,
    /// Optional status provider for footer text (e.g. indexing status).
    pub status_provider: Option<Arc<dyn Fn(&str) -> Option<SharedString> + Send + Sync>>,
}

impl Default for CommandPaletteConfig {
    fn default() -> Self {
        #[cfg(target_os = "macos")]
        let shortcut = Some("cmd-p".into());
        #[cfg(not(target_os = "macos"))]
        let shortcut = Some("ctrl-p".into());

        Self {
            shortcut,
            matcher: CommandMatcherKind::Nucleo,
            max_results: 50,
            placeholder: "Type a command...".into(),
            width: 560.0,
            max_height: 400.0,
            show_footer: true,
            show_categories_inline: true,
            commands_section_title: Some("Commands".into()),
            results_section_title: Some("Search Results".into()),
            status_provider: None,
        }
    }
}

/// The type of matcher to use for fuzzy matching.
#[derive(Clone, Default)]
pub enum CommandMatcherKind {
    /// Use nucleo for async-friendly fuzzy matching (default).
    #[default]
    Nucleo,
    /// Use fuzzy-matcher (SkimMatcherV2) for matching.
    FuzzyMatcher,
    /// Use a custom matcher implementation.
    Custom(Arc<dyn CommandMatcher + Send + Sync>),
}

/// An item that can be displayed in the command palette.
#[derive(Clone)]
pub struct CommandPaletteItem {
    /// Unique identifier for the item.
    pub id: SharedString,
    /// The main title of the item.
    pub title: SharedString,
    /// Optional subtitle/description.
    pub subtitle: Option<SharedString>,
    /// Category for grouping/display.
    pub category: SharedString,
    /// Optional icon to display.
    pub icon: Option<crate::IconName>,
    /// Optional keyboard shortcut to display.
    pub shortcut: Option<SharedString>,
    /// Additional keywords for matching.
    pub keywords: Vec<SharedString>,
    /// Whether the item is disabled.
    pub disabled: bool,
    /// Optional payload for custom data.
    pub payload: Option<Arc<dyn Any + Send + Sync>>,
}

impl CommandPaletteItem {
    /// Create a new command palette item with the required fields.
    pub fn new(id: impl Into<SharedString>, title: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            subtitle: None,
            category: "".into(),
            icon: None,
            shortcut: None,
            keywords: Vec::new(),
            disabled: false,
            payload: None,
        }
    }

    /// Set the subtitle.
    pub fn subtitle(mut self, subtitle: impl Into<SharedString>) -> Self {
        self.subtitle = Some(subtitle.into());
        self
    }

    /// Set the category.
    pub fn category(mut self, category: impl Into<SharedString>) -> Self {
        self.category = category.into();
        self
    }

    /// Set the icon.
    pub fn icon(mut self, icon: crate::IconName) -> Self {
        self.icon = Some(icon);
        self
    }

    /// Set the keyboard shortcut.
    pub fn shortcut(mut self, shortcut: impl Into<SharedString>) -> Self {
        self.shortcut = Some(shortcut.into());
        self
    }

    /// Add a keyword for matching.
    pub fn keyword(mut self, keyword: impl Into<SharedString>) -> Self {
        self.keywords.push(keyword.into());
        self
    }

    /// Add multiple keywords for matching.
    pub fn keywords(mut self, keywords: impl IntoIterator<Item = impl Into<SharedString>>) -> Self {
        self.keywords.extend(keywords.into_iter().map(Into::into));
        self
    }

    /// Set whether the item is disabled.
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Set a payload.
    pub fn payload<T: Any + Send + Sync>(mut self, payload: T) -> Self {
        self.payload = Some(Arc::new(payload));
        self
    }
}

/// A match result from the command matcher.
#[derive(Clone, Debug, Default)]
pub struct CommandPaletteMatch {
    /// The match score (higher is better).
    pub score: i64,
    /// Highlight ranges in the title (start, end).
    pub title_ranges: Vec<(usize, usize)>,
    /// Highlight ranges in the subtitle (start, end).
    pub subtitle_ranges: Vec<(usize, usize)>,
}

impl CommandPaletteMatch {
    /// Create a new match result with the given score.
    pub fn new(score: i64) -> Self {
        Self {
            score,
            title_ranges: Vec::new(),
            subtitle_ranges: Vec::new(),
        }
    }

    /// Set the title highlight ranges.
    pub fn with_title_ranges(mut self, ranges: Vec<(usize, usize)>) -> Self {
        self.title_ranges = ranges;
        self
    }

    /// Set the subtitle highlight ranges.
    pub fn with_subtitle_ranges(mut self, ranges: Vec<(usize, usize)>) -> Self {
        self.subtitle_ranges = ranges;
        self
    }
}

/// Trait for implementing custom matchers.
pub trait CommandMatcher {
    /// Match an item against a query, returning match info if it matches.
    fn match_item(&self, query: &str, item: &CommandPaletteItem) -> Option<CommandPaletteMatch>;
}

/// A matched item with its match information.
#[derive(Clone)]
pub struct MatchedItem {
    /// The original item.
    pub item: CommandPaletteItem,
    /// Match information including score and highlight ranges.
    pub match_info: CommandPaletteMatch,
}

impl MatchedItem {
    pub fn new(item: CommandPaletteItem, match_info: CommandPaletteMatch) -> Self {
        Self { item, match_info }
    }
}
