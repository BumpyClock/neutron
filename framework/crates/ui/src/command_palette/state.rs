//! State management for the Command Palette.

use super::matcher::{FuzzyMatcherWrapper, NucleoMatcher};
use super::provider::CommandPaletteProvider;
use super::reveal_query_delay;
use super::types::{
    CommandMatcher, CommandMatcherKind, CommandPaletteConfig, CommandPaletteItem, MatchedItem,
};
use crate::global_state::GlobalState;
use gpui::{Context, EventEmitter, Task, Window};
use smol::Timer;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// Events emitted by the Command Palette.
#[derive(Clone)]
pub enum CommandPaletteEvent {
    /// An item was selected (user pressed Enter or clicked).
    Selected {
        /// The selected item.
        item: CommandPaletteItem,
    },
    /// The palette was dismissed (user pressed Escape or clicked outside).
    Dismissed,
}

/// Internal state of the Command Palette.
pub struct CommandPaletteState {
    /// The current configuration.
    pub config: CommandPaletteConfig,
    /// The provider for items.
    pub provider: Arc<dyn CommandPaletteProvider>,
    /// The current query string.
    pub query: String,
    /// The list of matched items (sorted by score).
    pub matched_items: Vec<MatchedItem>,
    /// The number of matched items from the static provider.
    pub matched_static_len: usize,
    /// The currently selected index.
    pub selected_index: Option<usize>,
    /// The matcher implementation.
    matcher: Box<dyn CommandMatcher + Send + Sync>,
    /// Query ID for tracking stale results.
    query_id: Arc<AtomicU64>,
    /// Timestamp after which async results can load.
    reveal_deadline: Option<Instant>,
    /// Static items from the provider.
    static_items: Vec<CommandPaletteItem>,
    /// Async items from the provider (keyed by id).
    async_items: HashMap<String, CommandPaletteItem>,
    /// The current async query task.
    _query_task: Task<()>,
}

impl EventEmitter<CommandPaletteEvent> for CommandPaletteState {}

impl CommandPaletteState {
    /// Create a new command palette state.
    pub fn new(
        config: CommandPaletteConfig,
        provider: Arc<dyn CommandPaletteProvider>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        // Create the matcher based on config
        let matcher: Box<dyn CommandMatcher + Send + Sync> = match &config.matcher {
            CommandMatcherKind::Nucleo => Box::new(NucleoMatcher::new()),
            CommandMatcherKind::FuzzyMatcher => Box::new(FuzzyMatcherWrapper::new()),
            CommandMatcherKind::Custom(m) => {
                // Wrap the custom matcher
                Box::new(CustomMatcherWrapper(m.clone()))
            }
        };

        // Get static items
        let static_items = provider.items(cx);

        let reveal_deadline = if GlobalState::global(cx).reduced_motion() {
            None
        } else {
            Some(Instant::now() + reveal_query_delay(cx))
        };

        let mut state = Self {
            config,
            provider,
            query: String::new(),
            matched_items: Vec::new(),
            matched_static_len: 0,
            selected_index: None,
            matcher,
            query_id: Arc::new(AtomicU64::new(0)),
            reveal_deadline,
            static_items,
            async_items: HashMap::new(),
            _query_task: Task::ready(()),
        };

        // Initial matching with empty query
        state.update_matches(window, cx);

        state
    }

    /// Set the query string and update matches.
    pub fn set_query(&mut self, query: String, window: &mut Window, cx: &mut Context<Self>) {
        if self.query == query {
            return;
        }

        self.query = query;

        // Increment query ID to invalidate stale results
        let current_query_id = self.query_id.fetch_add(1, Ordering::SeqCst) + 1;

        // Clear async items so stale results don't appear while awaiting fresh results
        self.async_items.clear();

        // Update matches immediately with static items
        self.update_matches(window, cx);

        if self.query.len() < 2 {
            self._query_task = Task::ready(());
            return;
        }

        let query_delay = self
            .reveal_deadline
            .and_then(|deadline| deadline.checked_duration_since(Instant::now()))
            .unwrap_or(Duration::ZERO);

        // Start async query
        let provider = self.provider.clone();
        let query_id = self.query_id.clone();

        self._query_task = cx.spawn_in(window, async move |this, window| {
            if !query_delay.is_zero() {
                Timer::after(query_delay).await;
            }

            if query_id.load(Ordering::SeqCst) != current_query_id {
                return;
            }

            let task = this.update_in(window, |this, _, cx| provider.query(&this.query, cx));

            let Ok(task) = task else {
                return;
            };

            let async_items = task.await;

            // Check if this query is still current
            if query_id.load(Ordering::SeqCst) != current_query_id {
                return;
            }

            _ = this.update_in(window, |this, window, cx| {
                // Merge async items
                this.async_items.clear();
                for item in async_items {
                    this.async_items.insert(item.id.to_string(), item);
                }
                this.update_matches(window, cx);
            });
        });

        cx.notify();
    }

    /// Update the matched items based on the current query.
    fn update_matches(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        if self.query.len() == 1 {
            self.matched_static_len = 0;
            self.matched_items.clear();
            self.selected_index = None;
            cx.notify();
            return;
        }
        let mut static_items: Vec<CommandPaletteItem> = self.static_items.clone();
        let mut async_only_items: Vec<CommandPaletteItem> = Vec::new();

        let static_ids: std::collections::HashSet<_> =
            static_items.iter().map(|i| i.id.to_string()).collect();

        for (id, item) in &self.async_items {
            if static_ids.contains(id) {
                if let Some(pos) = static_items
                    .iter()
                    .position(|i| i.id.as_ref() == id.as_str())
                {
                    static_items[pos] = item.clone();
                }
            } else {
                async_only_items.push(item.clone());
            }
        }

        let mut matched_static: Vec<MatchedItem> = static_items
            .into_iter()
            .filter_map(|item| {
                self.matcher
                    .match_item(&self.query, &item)
                    .map(|match_info| MatchedItem::new(item, match_info))
            })
            .collect();

        let mut matched_async: Vec<MatchedItem> = async_only_items
            .into_iter()
            .filter_map(|item| {
                self.matcher
                    .match_item(&self.query, &item)
                    .map(|match_info| MatchedItem::new(item, match_info))
            })
            .collect();

        if !self.query.is_empty() {
            matched_static.sort_by(|a, b| {
                b.match_info
                    .score
                    .cmp(&a.match_info.score)
                    .then_with(|| a.item.title.cmp(&b.item.title))
            });
            matched_async.sort_by(|a, b| {
                b.match_info
                    .score
                    .cmp(&a.match_info.score)
                    .then_with(|| a.item.title.cmp(&b.item.title))
            });
        }

        let max_results = self.config.max_results;
        let total_len = matched_static.len() + matched_async.len();
        let total_limit = total_len.min(max_results);
        let static_limit = matched_static.len().min(total_limit);
        let async_limit = total_limit.saturating_sub(static_limit);

        matched_static.truncate(static_limit);
        matched_async.truncate(async_limit);

        self.matched_static_len = matched_static.len();
        self.matched_items.clear();
        self.matched_items.extend(matched_static);
        self.matched_items.extend(matched_async);

        // Reset selection to first item if available
        self.selected_index = if self.matched_items.is_empty() {
            None
        } else {
            Some(0)
        };

        cx.notify();
    }

    /// Move selection up.
    pub fn select_prev(&mut self, cx: &mut Context<Self>) {
        if self.matched_items.is_empty() {
            return;
        }

        self.selected_index = Some(match self.selected_index {
            Some(0) | None => self.matched_items.len() - 1,
            Some(i) => i - 1,
        });

        cx.notify();
    }

    /// Move selection down.
    pub fn select_next(&mut self, cx: &mut Context<Self>) {
        if self.matched_items.is_empty() {
            return;
        }

        self.selected_index = Some(match self.selected_index {
            None => 0,
            Some(i) if i >= self.matched_items.len() - 1 => 0,
            Some(i) => i + 1,
        });

        cx.notify();
    }

    /// Confirm the current selection.
    pub fn confirm(&mut self, cx: &mut Context<Self>) {
        if let Some(index) = self.selected_index {
            if let Some(matched) = self.matched_items.get(index) {
                if !matched.item.disabled {
                    cx.emit(CommandPaletteEvent::Selected {
                        item: matched.item.clone(),
                    });
                }
            }
        }
    }

    /// Dismiss the palette.
    pub fn dismiss(&mut self, cx: &mut Context<Self>) {
        cx.emit(CommandPaletteEvent::Dismissed);
    }

    /// Select an item at a specific index.
    pub fn select_index(&mut self, index: usize, cx: &mut Context<Self>) {
        if index < self.matched_items.len() {
            self.selected_index = Some(index);
            cx.notify();
        }
    }

    /// Get the currently selected item.
    pub fn selected_item(&self) -> Option<&MatchedItem> {
        self.selected_index.and_then(|i| self.matched_items.get(i))
    }
}

/// Wrapper to implement CommandMatcher for Arc<dyn CommandMatcher>.
struct CustomMatcherWrapper(Arc<dyn CommandMatcher + Send + Sync>);

impl CommandMatcher for CustomMatcherWrapper {
    fn match_item(
        &self,
        query: &str,
        item: &CommandPaletteItem,
    ) -> Option<super::types::CommandPaletteMatch> {
        self.0.match_item(query, item)
    }
}
