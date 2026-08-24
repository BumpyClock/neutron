---
title: Introduction
description: Rust GUI components for building fantastic cross-platform desktop application by using GPUI.
summary: "Rust GUI components for building fantastic cross-platform desktop applications using GPUI."
---

# Neutron Components Introduction

Neutron Components is a Rust UI component library for building fantastic desktop applications using [GPUI](https://gpui.rs).

Neutron Components is a comprehensive UI component library for building fantastic desktop applications using [GPUI](https://gpui.rs). It provides 60+ cross-platform components with modern design, theming support, and high performance.

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
neutron-components-app = { path = "framework/crates/app", version = "=0.7.0" }
neutron-components-assets = { path = "framework/crates/assets", version = "=0.7.0" }

[build-dependencies]
neutron-components-manifest = { path = "framework/crates/app-manifest", version = "=0.7.0" }
```

The root workspace selects its exact engine path dependencies. Do not add a
separate engine dependency; see [Installation](./installation.md) for release status.

Declare `[package.metadata.gpui-app]` and the two-line identity `build.rs`
described in [Building an Application](./app-shell.md), then create the window:

```rust
use neutron_components_app::gpui::*;
use neutron_components_app::prelude::*;
use neutron_components_app::{StandardMenus, WindowManager};
use neutron_components_app::ui::{button::*, *};

neutron_components_app::include_identity!();

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
        .assets(neutron_components_assets::Assets)
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

AppShell initializes Neutron Components, wraps managed windows in `Root`, and wires
desktop lifecycle. See [Building an Application](./app-shell.md) for standard
Settings/About controls and Windows/Linux menu-bar placement.

## Community & Support

- [GitHub Repository](https://github.com/BumpyClock/neutron)
- [Issue Tracker](https://github.com/BumpyClock/neutron/issues)
- [Contributing Guide](https://github.com/BumpyClock/neutron/blob/main/framework/CONTRIBUTING.md)

## License

Apache-2.0
