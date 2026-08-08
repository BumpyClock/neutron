---
title: "Home"
summary: "Project home page for GPUI Component with install guidance and component highlights."
layout: home
---

<script setup>
import Index from './index.vue'
</script>

<Index />

## Simple and Intuitive API

Get started with just a few lines of code. Stateless components
make it easy to build complex UIs.

```rs
Button::new("ok")
    .primary()
    .label("Click Me")
    .on_click(|_, _, _| println!("Button clicked!"))
```

## Install GPUI Component

Add the following to your `Cargo.toml`:

```toml-vue
gpui-component-app = "{{ VERSION }}"
gpui-component-assets = "{{ VERSION }}"

[build-dependencies]
gpui-component-manifest = "{{ VERSION }}"
```

## Hello World

After declaring application metadata and the identity `build.rs` from
[Building an Application](./docs/app-shell.md), `src/main.rs` stays focused on
product UI:

```rs
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

Run the program with the following command:

```sh
$ cargo run
```
