---
title: Command Palette
description: A searchable command surface with keyboard navigation and static or asynchronous providers.
summary: "A searchable command surface with keyboard navigation and static or asynchronous providers."
---

# Command Palette

`CommandPalette` opens a modal search surface for commands and other actions.
It supports fuzzy matching, categories, icons, shortcuts, disabled items, and
providers that load results asynchronously.

## Import

```rust
use std::sync::Arc;

use neutron_components::command_palette::{
    CommandPalette, CommandPaletteConfig, CommandPaletteEvent, CommandPaletteItem,
    StaticProvider,
};
```

## Initialize

Initialize the palette once after component setup. The default binding is
`cmd-p` on macOS and `ctrl-p` on other platforms.

```rust
CommandPalette::init(cx, CommandPaletteConfig::default());
```

Set `CommandPaletteConfig::shortcut` to `None` to disable the binding.

## Open a static palette

Create a `StaticProvider` with the commands that the palette should show.
Subscribe to the handle's state to receive selection and dismissal events.

```rust
let provider = Arc::new(StaticProvider::new(vec![
    CommandPaletteItem::new("file.open", "Open File")
        .category("File")
        .shortcut("ctrl-o")
        .keyword("browse"),
    CommandPaletteItem::new("file.save", "Save File")
        .category("File")
        .shortcut("ctrl-s"),
]));

let handle = CommandPalette::open(window, cx, provider);
cx.subscribe(&handle.state(), |_, _, event, _| {
    if let CommandPaletteEvent::Selected { item } = event {
        println!("selected {}", item.id);
    }
})
.detach();
```

`CommandPaletteHandle::close` closes the current dialog. The method consumes
the handle.

## Configure the palette

Use `open_with_config` for a palette-specific configuration:

```rust
let config = CommandPaletteConfig {
    placeholder: "Search actions...".into(),
    max_results: 20,
    width: 640.0,
    max_height: 480.0,
    show_footer: true,
    show_categories_inline: true,
    ..Default::default()
};

CommandPalette::open_with_config(window, cx, provider, config);
```

| Field | Default | Description |
| --- | --- | --- |
| `shortcut` | `cmd-p` or `ctrl-p` | Global key binding. |
| `matcher` | `Nucleo` | Fuzzy matcher implementation. |
| `max_results` | `50` | Maximum results to display. |
| `placeholder` | `Type a command...` | Search field placeholder. |
| `width` | `560.0` | Palette width in pixels. |
| `max_height` | `400.0` | Maximum palette height in pixels. |
| `show_footer` | `true` | Show keyboard hints and status text. |
| `show_categories_inline` | `true` | Show item categories beside titles. |
| `commands_section_title` | `Commands` | Title for commands when a query exists. |
| `results_section_title` | `Search Results` | Title for search results when a query exists. |
| `status_provider` | `None` | Return status text for the footer. |

## Static and asynchronous results

Implement `CommandPaletteProvider` when results come from an application
service. Return immediate items from `items` and asynchronous results from
`query`. Results with an existing item ID replace that static item.

```rust
use gpui::{App, Task};

impl CommandPaletteProvider for MyProvider {
    fn items(&self, _cx: &App) -> Vec<CommandPaletteItem> {
        self.commands.clone()
    }

    fn query(&self, query: &str, cx: &App) -> Task<Vec<CommandPaletteItem>> {
        // Start an async lookup and return its task.
        let _ = (query, cx);
        Task::ready(Vec::new())
    }
}
```

Use `CommandPaletteItem` builders to set `subtitle`, `category`, `icon`,
`shortcut`, `keyword`, `keywords`, `disabled`, or an application payload.

## Keyboard behavior

| Key | Action |
| --- | --- |
| `Up` / `Down` | Move selection. |
| `Enter` | Select the highlighted item. |
| `Escape` | Dismiss the palette. |

The palette uses theme motion tokens. Reduced-motion mode removes its reveal
delay and transition animations.

[CommandPalette]: https://docs.rs/neutron-components/latest/neutron_components/command_palette/struct.CommandPalette.html
[CommandPaletteConfig]: https://docs.rs/neutron-components/latest/neutron_components/command_palette/struct.CommandPaletteConfig.html
[CommandPaletteItem]: https://docs.rs/neutron-components/latest/neutron_components/command_palette/struct.CommandPaletteItem.html
[CommandPaletteProvider]: https://docs.rs/neutron-components/latest/neutron_components/command_palette/trait.CommandPaletteProvider.html
