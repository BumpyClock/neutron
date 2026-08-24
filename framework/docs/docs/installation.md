---
title: Installation
summary: "How to install neutron-components and configure project dependencies."
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

- Rust 1.95.0 or later
- Cargo (comes with Rust)

The engine-independent `neutron-components-manifest`, `neutron-components-storage`,
`neutron-components-macros`, and `framework-xtask` packages retain Rust 1.90 as
their MSRV.

Add Neutron Components from the root workspace:

```toml
[dependencies]
neutron-components = { path = "framework/crates/ui", version = "=0.7.0" }
neutron-components-assets = { path = "framework/crates/assets", version = "=0.7.0" }
```

For AppShell, add `neutron-components-app` and `neutron-components-manifest` from the
same root workspace as shown in [Getting Started](./getting-started.md). Do not
add a separate `gpui`, `gpui_platform`, or engine Git revision.
