This is a BumpyClock fork of Neutron Components by Longbridge. Neutron preserves local components and
contracts while it selectively adapts compatible upstream behavior.

## Upstream sync status

See [UPSTREAM.md](UPSTREAM.md) for exact Longbridge source identities, accepted adaptations,
Neutron conflicts, exclusions, and validation limits.

# Neutron Components

[Neutron repository](https://github.com/BumpyClock/neutron)

UI components for building fantastic desktop applications using [GPUI](https://gpui.rs).

## Features

- **Richness**: 60+ cross-platform desktop UI components.
- **Native**: Inspired by macOS and Windows controls, combined with shadcn/ui design for a modern experience.
- **Ease of Use**: Stateless `RenderOnce` components, simple and user-friendly.
- **Customizable**: Built-in `Theme` and `ThemeColor`, supporting multi-theme and variable-based configurations.
- **Versatile**: Supports sizes like `xs`, `sm`, `md`, and `lg`.
- **Flexible Layout**: Dock layout for panel arrangements, resizing, and freeform (Tiles) layouts.
- **High Performance**: Virtualized Table and List components for smooth large-data rendering.
- **Content Rendering**: Native support for Markdown and simple HTML.
- **Charting**: Built-in charts for visualizing your data.
- **Editor**: High performance code editor (support up to 200K lines) with LSP (diagnostics, completion, hover, etc).
- **Syntax Highlighting**: Syntax highlighting for editor and markdown components using Tree Sitter.


## Installation

The current framework stack is not ready for crates.io consumption. Use the
single Neutron workspace for local development. Do not add an independent
engine checkout, Git revision, or mutable branch dependency.

```toml
neutron-components = { path = "framework/crates/ui", version = "=0.7.0" }
neutron-components-assets = { path = "framework/crates/assets", version = "=0.7.0" }
```

The `0.7.0` source tree is unreleased. See [compatibility status](docs/COMPATIBILITY.md)
and the [release guide](RELEASING.md) before preparing a release. See [testing
and CI](TESTING.md) for validation levels and native-runtime limits.

### AppShell (experimental)

Native applications can use `neutron-components-app` to centralize identity, paths,
component initialization, startup/shutdown, settings, managed windows, standard
desktop menus, and platform capability reporting:

```rs
AppShell::builder(APP_IDENTITY)
    .assets(neutron_components_assets::Assets)
    .standard_menus(
        StandardMenus::new()
            .on_settings(open_settings)
            .on_about(open_about),
    )
    .start(|_, cx| {
        WindowManager::open(cx, WindowSpec::new("main"), |_, cx| {
            cx.new(|_| MainView)
        })?;
        Ok(())
    })
    .run()
```

See [Building an Application](docs/docs/app-shell.md) for identity codegen,
desktop-menu opt-in, automatic Windows/Linux menu-bar placement, persistent
settings, platform evidence, and current limitations. Managed normal windows
receive in-window menu bars on Windows/Linux. macOS uses the global native menu.
Raw windows remain app-owned.

### Manual GPUI bootstrap

For hosts that need direct GPUI types, depend on `neutron-components-app` from the
same root workspace and use its re-exports. This prevents a second GPUI type
identity from entering the application.

```toml
neutron-components-app = { path = "framework/crates/app", version = "=0.7.0" }
```

```rs
use neutron_components_app::gpui::*;
use neutron_components_app::gpui_platform;
use neutron_components::{button::*, *};

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

fn main() {
    let app = gpui_platform::application().with_assets(neutron_components_assets::Assets);

    app.run(move |cx| {
        // This must be called before using any Neutron Components features.
        neutron_components::init(cx);

        cx.spawn(async move |cx| {
            cx.open_window(WindowOptions::default(), |window, cx| {
                let view = cx.new(|_| HelloWorld);
                // This first level on the window, should be a Root.
                cx.new(|cx| Root::new(view, window, cx))
            })?;

            Ok::<_, anyhow::Error>(())
        })
        .detach();
    });
}
```

### Icons

Neutron Components has an `Icon` element, but it does not include SVG files by default.

The example uses [Lucide](https://lucide.dev) icons, but you can use any icons you like. Just name the SVG files as defined in [IconName](https://github.com/BumpyClock/neutron/blob/main/framework/crates/ui/src/icon.rs#L86). You can add any icons you need to your project.

The `neutron-components-assets` crate also bundles library-owned non-icon assets such as `surface/NoiseAsset_256.png`. If your app has its own `AssetSource`, compose it with `neutron_components_assets::chain(app_assets, neutron_components_assets::Assets)` so app assets win first and Neutron Components assets remain available as fallback.

## Development

We have a gallery of applications built with Neutron Components.

```bash
cargo run
```

More examples can be found in the `examples` directory. You can run them with `cargo run --example <example_name>`.

Check out [CONTRIBUTING.md](CONTRIBUTING.md) for more details.

## Compare to others

| Features              | Neutron Components                 | [Iced]             | [egui]                | [Qt 6]                                            |
| --------------------- | ------------------------------ | ------------------ | --------------------- | ------------------------------------------------- |
| Language              | Rust                           | Rust               | Rust                  | C++/QML                                           |
| Core Render           | GPUI                           | wgpu               | wgpu                  | QT                                                |
| License               | Apache 2.0                     | MIT                | MIT/Apache 2.0        | [Commercial/LGPL](https://www.qt.io/qt-licensing) |
| Min Binary Size [^1]  | 12MB                           | 11MB               | 5M                    | 20MB [^2]                                         |
| Cross-Platform        | Yes                            | Yes                | Yes                   | Yes                                               |
| Documentation         | Simple                         | Simple             | Simple                | Good                                              |
| Web                   | No                             | Yes                | Yes                   | Yes                                               |
| UI Style              | Modern                         | Basic              | Basic                 | Basic                                             |
| CJK Support           | Yes                            | Yes                | Bad                   | Yes                                               |
| Chart                 | Yes                            | No                 | No                    | Yes                                               |
| Table (Large dataset) | Yes<br>(Virtual Rows, Columns) | No                 | Yes<br>(Virtual Rows) | Yes<br>(Virtual Rows, Columns)                    |
| Table Column Resize   | Yes                            | No                 | Yes                   | Yes                                               |
| Text base             | Rope                           | [COSMIC Text] [^3] | trait TextBuffer [^4] | [QTextDocument]                                   |
| CodeEditor            | Simple                         | Simple             | Simple                | Basic API                                         |
| Dock Layout           | Yes                            | Yes                | Yes                   | Yes                                               |
| Syntax Highlight      | [Tree Sitter]                  | [Syntect]          | [Syntect]             | [QSyntaxHighlighter]                              |
| Markdown Rendering    | Yes                            | Yes                | Basic                 | No                                                |
| Markdown mix HTML     | Yes                            | No                 | No                    | No                                                |
| HTML Rendering        | Basic                          | No                 | No                    | Basic                                             |
| Text Selection        | TextView                       | No                 | Any Label             | Yes                                               |
| Custom Theme          | Yes                            | Yes                | Yes                   | Yes                                               |
| Built Themes          | Yes                            | No                 | No                    | No                                                |
| I18n                  | Yes                            | Yes                | Yes                   | Yes                                               |

> Please submit an issue or PR if any mistakes or outdated are found.

[Iced]: https://github.com/iced-rs/iced
[egui]: https://github.com/emilk/egui
[QT 6]: https://www.qt.io/product/qt6
[Tree Sitter]: https://tree-sitter.github.io/tree-sitter/
[Syntect]: https://github.com/trishume/syntect
[QSyntaxHighlighter]: https://doc.qt.io/qt-6/qsyntaxhighlighter.html
[QTextDocument]: https://doc.qt.io/qt-6/qtextdocument.html
[COSMIC Text]: https://github.com/pop-os/cosmic-text

[^1]: Release builds by use simple hello world example.

[^2]: [Reducing Binary Size of Qt Applications](https://www.qt.io/blog/reducing-binary-size-of-qt-applications-part-3-more-platforms)

[^3]: Iced Editor: <https://github.com/iced-rs/iced/blob/db5a1f6353b9f8520c4f9633d1cdc90242c2afe1/graphics/src/text/editor.rs#L65-L68>

[^4]: egui TextBuffer: <https://github.com/emilk/egui/blob/0a81372cfd3a4deda640acdecbbaf24bf78bb6a2/crates/egui/src/widgets/text_edit/text_buffer.rs#L20>

## License

Apache-2.0

- UI design based on [shadcn/ui](https://ui.shadcn.com).
- Icons from [Lucide](https://lucide.dev).
