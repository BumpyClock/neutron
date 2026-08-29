---
title: "Home"
summary: "Project home page for Neutron Components with install guidance and component highlights."
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

## Install Neutron Components

Add the following to your `Cargo.toml`:

```toml-vue
neutron-components-app = "{{ VERSION }}"
neutron-components-assets = "{{ VERSION }}"

[build-dependencies]
neutron-components-manifest = "{{ VERSION }}"
```

## Hello World

After declaring application metadata and the identity `build.rs` from
[Building an Application](./docs/app-shell.md), implement `DesktopApp` and run
it:

```rs
use neutron_components_app::gpui::*;
use neutron_components_app::prelude::*;
use neutron_components_app::{AppDeclaration, Surface, SurfaceKey};
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

fn build_hello(_args: &(), _window: &mut Window, cx: &mut App) -> Entity<HelloWorld> {
    cx.new(|_| HelloWorld)
}

struct HelloApp;

impl DesktopApp for HelloApp {
    fn declaration() -> AppDeclaration {
        AppDeclaration::new(APP_IDENTITY).primary_surface(Surface::new(
            SurfaceKey::<HelloWorld>::primary(),
            build_hello,
        ))
    }
}

fn main() -> Result<(), AppShellError> {
    AppShell::run::<HelloApp>()
}
```

Run the program with the following command:

```sh
$ cargo run
```
