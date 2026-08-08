---
title: Getting Started
description: Learn how to set up and use GPUI Component in your project
summary: "Learn how to set up and use GPUI Component in your project"
order: -2
---

# Getting Started

## Installation

Use the same immutable framework tag for every framework package. The framework
owns GPUI selection, so this application does not declare GPUI directly.

```toml
[dependencies]
gpui-component-app = { git = "https://github.com/BumpyClock/gpui-component", tag = "v0.6.0" }
gpui-component-assets = { git = "https://github.com/BumpyClock/gpui-component", tag = "v0.6.0" }
serde = { version = "1", features = ["derive"] }

[build-dependencies]
gpui-component-manifest = { git = "https://github.com/BumpyClock/gpui-component", tag = "v0.6.0" }

[package.metadata.gpui-app]
app_id = "com.example.hello"
display_name = "Hello"
categories = ["Development"]
```

`v0.6.0` is the latest immutable framework tag. The current `0.7.0` source
tree is not released, and registry consumption is blocked until exact GPUI fork
packages are published. See [Compatibility](../COMPATIBILITY.md).

## Quick Start

Add an app-local `build.rs`:

```rust
fn main() {
    gpui_component_manifest::build::emit_identity()
        .expect("invalid [package.metadata.gpui-app]");
}
```

Then build the native app:

```rust
use gpui_component_app::gpui::*;
use gpui_component_app::prelude::*;
use gpui_component_app::{StandardMenus, WindowManager};
use gpui_component_app::ui::{button::*, *};

gpui_component_app::include_identity!();

struct HelloWorld;

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

:::info
AppShell calls `gpui_component::init`, applies compiled identity, wraps managed
windows in `Root`, and sequences startup/shutdown. Your `start` callback owns app
services and initial windows.
:::

For standard Settings/About actions, persistent controls, native macOS menus,
Windows/Linux in-window menu bars, lifecycle, and platform limitations, continue
to [Building an Application](./app-shell.md).

## Manual bootstrap

Raw `gpui::Application` remains available for advanced hosts. In that mode the app
must call `gpui_component::init(cx)` before creating components and must place
`Root` at the first level of every managed window. See [Root View](./root.md).

## Basic Concepts

### Stateless Elements

GPUI Component uses stateless [RenderOnce] elements, making them simple and predictable. State management is handled at the view level, not in individual components.

The are all implemented [IntoElement] types.

For example:

```rs
struct MyView;

impl Render for MyView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .child(Button::new("btn").label("Click Me"))
            .child(Tag::secondary().child("Secondary"))
    }
}
```

### Stateful Components

There are some stateful components like `Dropdown`, `List`, and `Table` that manage their own internal state for convenience, these components implement the [Render] trait.

Those components to use are a bit different, we need create the [Entity] and hold it in the view struct.

```rs
struct MyView {
    input: Entity<InputState>,
}

impl MyView {
    fn new(window: &Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| InputState::new(window, cx).default_value("Hello 世界"));
        Self { input }
    }
}

impl Render for MyView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.input.clone()
    }
}
```

### Theming

All components support theming through the built-in `Theme` system:

```rust
use gpui_component::{ActiveTheme, Theme};

// Access theme colors in your components
cx.theme().primary
cx.theme().background
cx.theme().foreground
```

### Sizing

Most components support multiple sizes:

```rust
Button::new("btn").small()
Button::new("btn").medium() // default
Button::new("btn").large()
Button::new("btn").xsmall()
```

### Variants

Components offer different visual variants:

```rust
Button::new("btn").primary()
Button::new("btn").danger()
Button::new("btn").warning()
Button::new("btn").success()
Button::new("btn").ghost()
Button::new("btn").outline()
```

## Icons

:::info
Icons are not bundled with GPUI Component to keep the library lightweight.

Continue read [Icons & Assets](./assets.md) to learn how to add icons to your project.
:::

GPUI Component has an `Icon` element, but does not include SVG files by default.

The examples use [Lucide](https://lucide.dev) icons. You can use any icons you like by naming the SVG files as defined in `IconName`. Add the icons you need to your project.

```rust
use gpui_component::{Icon, IconName};

Icon::new(IconName::Check)
Icon::new(IconName::Search).small()
```

## Next Steps

Explore the component documentation to learn more about each component:

- [Button](./components/button) - Interactive button component
- [Input](./components/input) - Text input with validation
- [Dialog](./components/dialog) - Dialog and modal windows
- [Table](./components/table) - High-performance data tables
- [More components...](./components/index)

## Development

To run the component gallery:

```bash
cargo run
```

More examples can be found in the `examples` directory:

```bash
cargo run --example <example_name>
```

[RenderOnce]: https://docs.rs/gpui/latest/gpui/trait.RenderOnce.html
[IntoElement]: https://docs.rs/gpui/latest/gpui/trait.IntoElement.html
[Render]: https://docs.rs/gpui/latest/gpui/trait.Render.html
