use gpui::{
    App, AppContext, Context, Entity, FocusHandle, Focusable, InteractiveElement as _, IntoElement,
    ParentElement, Render, SharedString, Styled, Task, Window, div,
};
use smol::Timer;
use std::sync::Arc;
use std::time::Duration;

use gpui_component::{
    ActiveTheme, IconName,
    button::Button,
    command_palette::{
        CommandPalette, CommandPaletteConfig, CommandPaletteEvent, CommandPaletteItem,
        CommandPaletteProvider, StaticProvider,
    },
    h_flex, v_flex,
};

use crate::section;

pub struct CommandPaletteStory {
    focus_handle: FocusHandle,
    last_selected: Option<SharedString>,
}

impl super::Story for CommandPaletteStory {
    fn title() -> &'static str {
        "CommandPalette"
    }

    fn description() -> &'static str {
        "A modal command palette with fuzzy search and keyboard navigation"
    }

    fn new_view(window: &mut Window, cx: &mut App) -> Entity<impl Render> {
        Self::view(window, cx)
    }
}

impl CommandPaletteStory {
    pub fn view(window: &mut Window, cx: &mut App) -> Entity<Self> {
        cx.new(|cx| Self::new(window, cx))
    }

    fn new(_window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self {
            focus_handle: cx.focus_handle(),
            last_selected: None,
        }
    }

    fn static_items() -> Vec<CommandPaletteItem> {
        #[cfg(target_os = "macos")]
        const PRIMARY_MODIFIER: &str = "cmd";
        #[cfg(not(target_os = "macos"))]
        const PRIMARY_MODIFIER: &str = "ctrl";

        let shortcut = |key: &str| format!("{PRIMARY_MODIFIER}-{key}");

        vec![
            CommandPaletteItem::new("file.new", "New File")
                .category("File")
                .icon(IconName::Plus)
                .shortcut(shortcut("n"))
                .keyword("create"),
            CommandPaletteItem::new("file.open", "Open File")
                .category("File")
                .icon(IconName::FolderOpen)
                .shortcut(shortcut("o"))
                .keyword("browse"),
            CommandPaletteItem::new("file.save", "Save File")
                .category("File")
                .icon(IconName::File)
                .shortcut(shortcut("s")),
            CommandPaletteItem::new("file.save-all", "Save All")
                .category("File")
                .icon(IconName::File)
                .shortcut(shortcut("shift-s")),
            CommandPaletteItem::new("edit.undo", "Undo")
                .category("Edit")
                .icon(IconName::Undo)
                .shortcut(shortcut("z")),
            CommandPaletteItem::new("edit.redo", "Redo")
                .category("Edit")
                .icon(IconName::Redo)
                .shortcut(shortcut("shift-z")),
            CommandPaletteItem::new("edit.copy", "Copy")
                .category("Edit")
                .icon(IconName::Copy)
                .shortcut(shortcut("c")),
            CommandPaletteItem::new("search.find", "Find")
                .category("Search")
                .icon(IconName::Search)
                .shortcut(shortcut("f"))
                .keyword("locate"),
            CommandPaletteItem::new("search.replace", "Replace")
                .category("Search")
                .icon(IconName::Replace)
                .shortcut(shortcut("r")),
            CommandPaletteItem::new("view.terminal", "Toggle Terminal")
                .category("View")
                .icon(IconName::SquareTerminal)
                .shortcut(shortcut("`")),
            CommandPaletteItem::new("view.sidebar", "Toggle Sidebar")
                .category("View")
                .icon(IconName::PanelLeft)
                .shortcut(shortcut("b")),
        ]
    }

    fn show_static_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let provider = Arc::new(StaticProvider::new(Self::static_items()));
        let handle = CommandPalette::open(window, cx, provider);

        cx.subscribe(&handle.state(), move |this, _state, event, cx| {
            if let CommandPaletteEvent::Selected { item } = event {
                this.last_selected = Some(item.title.clone());
                cx.notify();
            }
        })
        .detach();
    }

    fn show_loading_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let provider = Arc::new(StaticProvider::new(Self::static_items()));
        let config = CommandPaletteConfig {
            status_provider: Some(Arc::new(|_| Some("Indexing workspace…".into()))),
            ..Default::default()
        };
        CommandPalette::open_with_config(window, cx, provider, config);
    }

    fn show_empty_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let provider = Arc::new(StaticProvider::new(Vec::new()));
        CommandPalette::open(window, cx, provider);
    }

    fn show_async_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let provider = Arc::new(AsyncDemoProvider::new());
        let handle = CommandPalette::open(window, cx, provider);

        cx.subscribe(&handle.state(), move |this, _state, event, cx| {
            if let CommandPaletteEvent::Selected { item } = event {
                this.last_selected = Some(item.title.clone());
                cx.notify();
            }
        })
        .detach();
    }

    fn show_custom_config_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let items = vec![
            CommandPaletteItem::new("action-1", "Action One")
                .category("Actions")
                .icon(IconName::Star),
            CommandPaletteItem::new("action-2", "Action Two")
                .category("Actions")
                .icon(IconName::Star),
            CommandPaletteItem::new("action-3", "Action Three")
                .category("Actions")
                .icon(IconName::Star),
        ];

        let provider = Arc::new(StaticProvider::new(items));

        let custom_config = CommandPaletteConfig {
            placeholder: "Search actions...".into(),
            width: 700.0,
            max_height: 300.0,
            max_results: 10,
            show_footer: true,
            show_categories_inline: true,
            ..Default::default()
        };

        let handle = CommandPalette::open_with_config(window, cx, provider, custom_config);

        cx.subscribe(&handle.state(), move |this, _state, event, cx| {
            if let CommandPaletteEvent::Selected { item } = event {
                this.last_selected = Some(item.title.clone());
                cx.notify();
            }
        })
        .detach();
    }
}

/// Demo provider that combines static items with async search
struct AsyncDemoProvider {
    static_items: Vec<CommandPaletteItem>,
}

impl AsyncDemoProvider {
    fn new() -> Self {
        Self {
            static_items: vec![
                CommandPaletteItem::new("static.help", "Help")
                    .category("Static")
                    .icon(IconName::Info)
                    .shortcut("F1"),
                CommandPaletteItem::new("static.settings", "Settings")
                    .category("Static")
                    .icon(IconName::Settings)
                    .shortcut(if cfg!(target_os = "macos") {
                        "cmd-,"
                    } else {
                        "ctrl-,"
                    }),
                CommandPaletteItem::new("static.about", "About")
                    .category("Static")
                    .icon(IconName::Info),
            ],
        }
    }
}

impl CommandPaletteProvider for AsyncDemoProvider {
    fn items(&self, _cx: &App) -> Vec<CommandPaletteItem> {
        self.static_items.clone()
    }

    fn query(&self, query: &str, cx: &App) -> Task<Vec<CommandPaletteItem>> {
        if query.is_empty() {
            return Task::ready(Vec::new());
        }

        let query = query.to_lowercase();

        cx.background_spawn(async move {
            // Simulate async search delay
            Timer::after(Duration::from_millis(200)).await;

            // Simulate searching files/resources
            let mut results = Vec::new();

            if query.contains("file") || query.contains("doc") {
                results.push(
                    CommandPaletteItem::new("async.file1", "document.txt")
                        .category("Files")
                        .subtitle("~/Documents/")
                        .icon(IconName::File),
                );
                results.push(
                    CommandPaletteItem::new("async.file2", "notes.md")
                        .category("Files")
                        .subtitle("~/Documents/")
                        .icon(IconName::File),
                );
            }

            if query.contains("folder") || query.contains("src") {
                results.push(
                    CommandPaletteItem::new("async.folder1", "src/")
                        .category("Folders")
                        .subtitle("Project folder")
                        .icon(IconName::Folder),
                );
            }

            // Generic async result for any query
            results.push(
                CommandPaletteItem::new(
                    format!("async.result.{}", query),
                    format!("Async result for: {}", query),
                )
                .category("Async Search")
                .icon(IconName::Search)
                .subtitle("Dynamically loaded"),
            );

            results
        })
    }
}

impl Focusable for CommandPaletteStory {
    fn focus_handle(&self, _cx: &gpui::App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for CommandPaletteStory {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("command-palette-story")
            .track_focus(&self.focus_handle)
            .size_full()
            .child(
                v_flex()
                    .gap_6()
                    .child(
                        section("Visual Review")
                            .child(
                                "Replay deterministic states for screenshots and motion recording.",
                            )
                            .child(
                                h_flex()
                                    .gap_2()
                                    .flex_wrap()
                                    .child(
                                        Button::new("show-static")
                                            .outline()
                                            .label("Replay Open Motion")
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.show_static_palette(window, cx)
                                            })),
                                    )
                                    .child(
                                        Button::new("show-loading")
                                            .outline()
                                            .label("Review Loading State")
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.show_loading_palette(window, cx)
                                            })),
                                    )
                                    .child(
                                        Button::new("show-empty")
                                            .outline()
                                            .label("Review Empty State")
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.show_empty_palette(window, cx)
                                            })),
                                    ),
                            ),
                    )
                    .child(
                        section("Async Provider")
                            .child(
                                "Command palette with static items plus async search. \
                                Try typing 'file', 'folder', or 'src' to see async results.",
                            )
                            .child(
                                Button::new("show-async")
                                    .outline()
                                    .label("Open Async Palette")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.show_async_palette(window, cx)
                                    })),
                            ),
                    )
                    .child(
                        section("Custom Configuration")
                            .child("Command palette with custom width, height, and placeholder.")
                            .child(
                                Button::new("show-custom")
                                    .outline()
                                    .label("Open Custom Config Palette")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.show_custom_config_palette(window, cx)
                                    })),
                            ),
                    )
                    .child(
                        section("Last Selected").child(
                            h_flex()
                                .gap_2()
                                .child(
                                    div()
                                        .text_color(cx.theme().muted_foreground)
                                        .child("Last selected item:"),
                                )
                                .child(
                                    div().font_weight(gpui::FontWeight::SEMIBOLD).child(
                                        self.last_selected
                                            .as_ref()
                                            .map(|s| s.to_string())
                                            .unwrap_or_else(|| "None".to_string()),
                                    ),
                                ),
                        ),
                    )
                    .child(
                        section("Features").child(
                            v_flex()
                                .gap_2()
                                .child("Fuzzy search with highlighted matches")
                                .child("Keyboard navigation (Up/Down, Enter, Escape)")
                                .child("Categories and icons")
                                .child("Keyboard shortcuts display")
                                .child("Static and async item providers")
                                .child("Customizable appearance"),
                        ),
                    )
                    .child(
                        section("Keyboard Shortcuts").child(
                            v_flex()
                                .gap_2()
                                .child("Up/Down: Navigate items")
                                .child("Enter: Select item")
                                .child("Escape: Close palette"),
                        ),
                    ),
            )
    }
}
