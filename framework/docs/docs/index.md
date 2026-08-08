---
title: Introduction
description: Rust GUI components for building fantastic cross-platform desktop application by using GPUI.
summary: "Rust GUI components for building fantastic cross-platform desktop applications using GPUI."
---

# GPUI Component Introduction

GPUI Component is a Rust UI component library for building fantastic desktop applications using [GPUI](https://gpui.rs).

GPUI Component is a comprehensive UI component library for building fantastic desktop applications using [GPUI](https://gpui.rs). It provides 60+ cross-platform components with modern design, theming support, and high performance.

## Features

- **Richness**: 60+ cross-platform desktop UI components
- **Native**: Inspired by macOS and Windows controls, combined with shadcn/ui design
- **Ease of Use**: Stateless `RenderOnce` components, simple and user-friendly
- **Customizable**: Built-in `Theme` and `ThemeColor`, supporting multi-theme
- **Versatile**: Supports sizes like `xs`, `sm`, `md`, and `lg`
- **Flexible Layout**: Dock layout for panel arrangements, resizing, and freeform (Tiles) layouts
- **High Performance**: Virtualized Table and List components for smooth large-data rendering
- **Content Rendering**: Native support for Markdown and simple HTML
- **Charting**: Built-in charts for visualization
- **Editor**: High performance code editor with LSP support
- **Syntax Highlighting**: Using Tree Sitter

## Quick Example

For a native application, start with the experimental AppShell layer:

```toml
[dependencies]
gpui-component-app = { git = "https://github.com/BumpyClock/gpui-component", tag = "v0.6.0" }
gpui-component-assets = { git = "https://github.com/BumpyClock/gpui-component", tag = "v0.6.0" }

[build-dependencies]
gpui-component-manifest = { git = "https://github.com/BumpyClock/gpui-component", tag = "v0.6.0" }
```

The framework tag selects its GPUI revision. Do not add a separate GPUI
dependency; see [Installation](./installation.md) for release status.

Declare `[package.metadata.gpui-app]` and the two-line identity `build.rs`
described in [Building an Application](./app-shell.md), then create the window:

```rust
use gpui_component_app::gpui::*;
use gpui_component_app::prelude::*;
use gpui_component_app::{StandardMenus, WindowManager};
use gpui_component_app::ui::{button::*, *};

gpui_component_app::include_identity!();

pub struct HelloWorld;
impl Render for HelloWorld {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .v_flex()
            .gap_2()
            .size_full()
            .items_center()
            .justify_center()
            .child("Hello, World!")
            .child(
                Button::new("ok")
                    .primary()
                    .label("Let's Go!")
                    .on_click(|_, _, _| println!("Clicked!")),
            )
    }
}

fn main() -> Result<(), AppShellError> {
    AppShell::builder(APP_IDENTITY)
        .assets(gpui_component_assets::Assets)
        .standard_menus(StandardMenus::new())
        .start(|_, cx| {
            WindowManager::open(cx, WindowSpec::new("main"), |_, cx| {
                cx.new(|_| HelloWorld)
            })?;
            Ok(())
        })
        .run()
}
```

AppShell initializes GPUI Component, wraps managed windows in `Root`, and wires
desktop lifecycle. See [Building an Application](./app-shell.md) for standard
Settings/About controls and Windows/Linux menu-bar placement.

## Community & Support

- [GitHub Repository](https://github.com/BumpyClock/gpui-component)
- [Issue Tracker](https://github.com/BumpyClock/gpui-component/issues)
- [Contributing Guide](https://github.com/BumpyClock/gpui-component/blob/main/CONTRIBUTING.md)

## License

Apache-2.0
