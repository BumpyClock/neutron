//! Provider trait for command palette items.

use super::types::CommandPaletteItem;
use gpui::{App, Task};

/// Trait for providing items to the command palette.
///
/// Implementors can provide static items via `items()` and/or
/// async items via `query()`.
pub trait CommandPaletteProvider: Send + Sync {
    /// Return static items. These are immediately available and filtered locally.
    ///
    /// Default implementation returns an empty list.
    fn items(&self, _cx: &App) -> Vec<CommandPaletteItem> {
        Vec::new()
    }

    /// Query for items asynchronously.
    ///
    /// This is called whenever the query changes. The returned items
    /// will be merged with static items (async items override static
    /// items with the same id).
    ///
    /// Default implementation returns an empty list.
    fn query(&self, _query: &str, _cx: &App) -> Task<Vec<CommandPaletteItem>> {
        Task::ready(Vec::new())
    }
}

/// A simple provider that holds a static list of items.
pub struct StaticProvider {
    items: Vec<CommandPaletteItem>,
}

impl StaticProvider {
    pub fn new(items: Vec<CommandPaletteItem>) -> Self {
        Self { items }
    }
}

impl CommandPaletteProvider for StaticProvider {
    fn items(&self, _cx: &App) -> Vec<CommandPaletteItem> {
        self.items.clone()
    }
}
