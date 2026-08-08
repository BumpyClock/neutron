//! Command Palette component for gpui-component.
//!
//! A modal command palette with fuzzy search, keyboard navigation,
//! and customizable appearance.
//!
//! # Example
//!
//! ```ignore
//! use gpui_component::command_palette::{CommandPalette, CommandPaletteConfig, CommandPaletteItem};
//! use gpui_component::command_palette::StaticProvider;
//!
//! // Initialize the command palette (call once at startup)
//! CommandPalette::init(cx, CommandPaletteConfig::default());
//!
//! // Create a provider with items
//! let items = vec![
//!     CommandPaletteItem::new("open", "Open File")
//!         .category("File")
//!         .shortcut("cmd-o"),
//!     CommandPaletteItem::new("save", "Save File")
//!         .category("File")
//!         .shortcut("cmd-s"),
//! ];
//! let provider = Arc::new(StaticProvider::new(items));
//!
//! // Open the palette
//! let handle = CommandPalette::open(window, cx, provider);
//!
//! // Close when done
//! handle.close(window, cx);
//! ```

mod matcher;
mod provider;
mod state;
mod types;
mod view;

pub use matcher::{FuzzyMatcherWrapper, NucleoMatcher};
pub use provider::{CommandPaletteProvider, StaticProvider};
pub use state::{CommandPaletteEvent, CommandPaletteState};
pub use types::{
    CommandMatcher, CommandMatcherKind, CommandPaletteConfig, CommandPaletteItem,
    CommandPaletteMatch, MatchedItem,
};

pub(crate) fn reveal_delay(cx: &App) -> std::time::Duration {
    std::time::Duration::from_millis(u64::from(cx.theme().motion.fade_duration_ms))
}

pub(crate) fn reveal_query_delay(cx: &App) -> std::time::Duration {
    reveal_delay(cx)
        + std::time::Duration::from_millis(u64::from(cx.theme().motion.enter_duration_ms))
}

use gpui::{App, AppContext as _, Entity, KeyBinding, ParentElement as _, Styled, Window, actions};
use std::sync::Arc;
use view::CommandPaletteView;

use crate::{ActiveTheme as _, WindowExt as _};

actions!(command_palette, [Open]);

/// Handle to an open command palette.
///
/// Use this to close the palette or query its state.
pub struct CommandPaletteHandle {
    /// Entity reference to the palette state.
    state: Entity<CommandPaletteState>,
}

impl CommandPaletteHandle {
    /// Get the state entity for subscribing to events.
    ///
    /// Use this to subscribe to `CommandPaletteEvent`:
    /// ```ignore
    /// let handle = CommandPalette::open(window, cx, provider);
    /// cx.subscribe(&handle.state(), |_, event, cx| {
    ///     match event {
    ///         CommandPaletteEvent::Selected { item } => { /* handle selection */ }
    ///         CommandPaletteEvent::Dismissed => { /* handle dismissal */ }
    ///     }
    /// });
    /// ```
    pub fn state(&self) -> &Entity<CommandPaletteState> {
        &self.state
    }

    /// Close the command palette.
    pub fn close(self, window: &mut Window, cx: &mut App) {
        window.close_dialog(cx);
    }
}

/// The Command Palette entry point.
pub struct CommandPalette;

impl CommandPalette {
    /// Initialize the command palette with the given configuration.
    ///
    /// This sets up the global keybinding (if configured) and should be
    /// called once at application startup.
    pub fn init(cx: &mut App, config: CommandPaletteConfig) {
        view::init(cx);

        // Register the Open action keybinding if configured
        if let Some(shortcut) = &config.shortcut {
            // Validate the shortcut parses, then use the original string
            if gpui::Keystroke::parse(shortcut).is_ok() {
                cx.bind_keys([KeyBinding::new(shortcut.as_ref(), Open, None)]);
            }
        }

        // Store config globally for default open behavior
        cx.set_global(GlobalCommandPaletteConfig(config));
    }

    /// Open the command palette with the given provider.
    ///
    /// Returns a handle that can be used to close the palette.
    pub fn open(
        window: &mut Window,
        cx: &mut App,
        provider: Arc<dyn CommandPaletteProvider>,
    ) -> CommandPaletteHandle {
        let config = cx
            .try_global::<GlobalCommandPaletteConfig>()
            .map(|g| g.0.clone())
            .unwrap_or_default();

        Self::open_with_config(window, cx, provider, config)
    }

    /// Open the command palette with custom configuration.
    pub fn open_with_config(
        window: &mut Window,
        cx: &mut App,
        provider: Arc<dyn CommandPaletteProvider>,
        config: CommandPaletteConfig,
    ) -> CommandPaletteHandle {
        let width = gpui::px(config.width);

        // Create the view entity
        let view: Entity<CommandPaletteView> =
            cx.new(|cx| CommandPaletteView::new(config.clone(), provider, window, cx));

        // Get the state entity from the view
        let state = view.read(cx).state.clone();

        // Open as a dialog
        window.open_dialog(cx, move |dialog, _window, _cx| {
            dialog
                .w(width)
                .min_h(gpui::px(0.))
                .overlay(true)
                .overlay_closable(true)
                .keyboard(true)
                .animate(false)
                // No dialog-chrome animation, but keep the dialog mounted
                // through the exit window so the palette can animate out.
                .defer_close(true)
                .appearance(false)
                .close_button(false)
                .p_0()
                .child(view.clone())
        });

        CommandPaletteHandle { state }
    }
}

/// Global storage for the default command palette configuration.
struct GlobalCommandPaletteConfig(CommandPaletteConfig);

impl gpui::Global for GlobalCommandPaletteConfig {}
