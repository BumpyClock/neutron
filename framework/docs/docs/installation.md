---
title: Installation
summary: "How to install gpui-component and configure project dependencies."
order: -1
---

# Installation

Use the root Neutron workspace. Framework packages resolve engine crates from
`engine/` with exact versions; applications must not select a separate engine
revision.

The `0.7.0` source tree is not released, and crates.io publication remains
blocked on engine fork packages. See the [compatibility matrix](../COMPATIBILITY.md)
for current evidence and blockers.

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

Run `../../script/bootstrap` to fetch workspace and documentation dependencies.
Install Linux system dependencies with `framework/script/install-linux.sh`.

## Rust and Cargo

Make sure Rust and Cargo are installed.

- Rust 1.90 or later
- Cargo (comes with Rust)

Add GPUI Component from the root workspace:

```toml
[dependencies]
gpui-component = { path = "framework/crates/ui", version = "=0.7.0" }
gpui-component-assets = { path = "framework/crates/assets", version = "=0.7.0" }
```

For AppShell, add `gpui-component-app` and `gpui-component-manifest` from the
same root workspace as shown in [Getting Started](./getting-started.md). Do not
add a separate `gpui`, `gpui_platform`, or engine Git revision.
