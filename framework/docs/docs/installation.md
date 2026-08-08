---
title: Installation
summary: "How to install gpui-component and configure project dependencies."
order: -1
---

# Installation

Use one immutable GPUI Component release. The framework source selects its
matching GPUI revision; applications must not select GPUI independently.

The latest framework tag is `v0.6.0`. The `0.7.0` source tree is not released,
and crates.io publication remains blocked on GPUI fork packages. See the
[compatibility matrix](../COMPATIBILITY.md) for current evidence and blockers.

## System Requirements

We can development application on macOS, Windows or Linux.

### macOS

- macOS 15 or later
- Xcode command line tools

## Windows

- Windows 10 or later

There have a bootstrap script to help install the required toolchain and dependencies.

You can run the script in PowerShell:

```ps
.\script\install-window.ps1
```

## Linux

Run `./script/bootstrap` to install system dependencies.

## Rust and Cargo

Make sure Rust and Cargo are installed.

- Rust 1.90 or later
- Cargo (comes with Rust)

Add GPUI Component from its immutable framework tag:

```toml
[dependencies]
gpui-component = { git = "https://github.com/BumpyClock/gpui-component", tag = "v0.6.0" }
gpui-component-assets = { git = "https://github.com/BumpyClock/gpui-component", tag = "v0.6.0" }
```

For AppShell, add `gpui-component-app` and
`gpui-component-manifest` from the same tag as shown in
[Getting Started](./getting-started.md). Do not add `gpui`,
`gpui_platform`, or a GPUI Git revision directly.
