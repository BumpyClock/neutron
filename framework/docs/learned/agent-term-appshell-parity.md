---
title: "Agent Term AppShell Parity Audit"
summary: "Pinned host-capability ledger for evaluating an Agent Term migration without silent feature loss."
read_when: "changing AppShell host capabilities or claiming Agent Term boilerplate reduction or migration parity"
---

# Agent Term AppShell Parity Audit

Baseline: BumpyClock/agent-term commit
[`c533566b80014c42e831ff28369f74bf53ee2049`](https://github.com/BumpyClock/agent-term/tree/c533566b80014c42e831ff28369f74bf53ee2049).

Agent Term currently uses Tauri 2 with a React/WebView frontend. Replacing its
host with AppShell is not a bootstrap-only refactor: it requires a native GPUI UI
rewrite and explicit replacements for IPC, packaging, updater, sidecar bundling,
and platform window effects. Until those gates pass, Agent Term remains an audit
benchmark and keeps its current host.

Evidence tiers:

- `conformance-proven`: an in-repo AppShell test/example exercises the contract.
- `adopter-proven`: Agent Term source proves the product requirement, not an
  AppShell replacement.
- `unproven`: an API may exist, but replacement behavior has no sufficient
  conformance or adopter evidence.

| Capability | AppShell status | Agent Term migration status | Owner | Evidence tier | Source reference | Migration consequence |
|---|---|---|---|---|---|---|
| Identity, version, config directories | Compiled identity and `AppInfo` paths exist | Later migration | Shared manifest; app owns legacy paths | conformance-proven | [Agent Term config](https://github.com/BumpyClock/agent-term/blob/c533566b80014c42e831ff28369f74bf53ee2049/src-tauri/tauri.conf.json), [manifest schema](https://github.com/BumpyClock/gpui-component/blob/main/crates/app-manifest/src/schema.rs) | Preserve `com.adityasharma.agent-term-app` and its existing data namespace; migrate paths explicitly |
| Component init, assets, root window | Native AppShell conformance; ordered asset fallback | Blocked on native UI rewrite | Shell plus app assets/views | conformance-proven | [AppShell example](https://github.com/BumpyClock/gpui-component/blob/main/examples/app_shell/src/main.rs), [shell](https://github.com/BumpyClock/gpui-component/blob/main/crates/app/src/shell.rs) | HTML/JS bundles are not native `AssetSource` views |
| GUI-launch `PATH` repair | Explicit `EnvironmentPolicy::LoginShell` | Reusable only after AppShell becomes host | App selects policy | adopter-proven | [Agent Term startup](https://github.com/BumpyClock/agent-term/blob/c533566b80014c42e831ff28369f74bf53ee2049/src-tauri/src/lib.rs#L89), [shell policy](https://github.com/BumpyClock/gpui-component/blob/main/crates/app/src/shell.rs) | Run before app-created threads; no-op on Windows |
| Fallible path-aware services | Transactional `start` | Reusable after host rewrite | App service aggregate and runtime | conformance-proven | [Agent Term manager construction](https://github.com/BumpyClock/agent-term/blob/c533566b80014c42e831ff28369f74bf53ee2049/src-tauri/src/lib.rs#L120), [AppShell example](https://github.com/BumpyClock/gpui-component/blob/main/examples/app_shell/src/main.rs) | AppShell must not acquire a Tokio dependency |
| Standard menu and Settings action | `StandardMenus`, stable commands, conventional chords | Current frontend route must become native handler | Shell command; app settings view | conformance-proven | [Agent Term sidebar](https://github.com/BumpyClock/agent-term/blob/c533566b80014c42e831ff28369f74bf53ee2049/src/components/sidebar/Sidebar.tsx#L419), [standard commands](https://github.com/BumpyClock/gpui-component/blob/main/crates/app/src/commands/standard.rs) | Native rewrite wires `app.settings`; this does not change current Tauri code |
| Managed Rust services and Tokio | Startup/dispatch primitives only | Not migration-covered | App | adopter-proven | [Agent Term managers](https://github.com/BumpyClock/agent-term/blob/c533566b80014c42e831ff28369f74bf53ee2049/src-tauri/src/lib.rs#L120) | Install app globals/entities or pass services to native views |
| Setup and cleanup | Generic ordered lifecycle | Per-service adapters remain unproven | Shell sequencing plus app adapters | unproven | [Agent Term exit handling](https://github.com/BumpyClock/agent-term/blob/c533566b80014c42e831ff28369f74bf53ee2049/src-tauri/src/lib.rs#L245), [lifecycle](https://github.com/BumpyClock/gpui-component/blob/main/crates/app/src/lifecycle.rs) | Adapt and verify every service teardown; preserve bounded cleanup |
| Custom window chrome and effects | Partial native window hooks | Not migration-covered | App/window layer | unproven | [Agent Term window setup](https://github.com/BumpyClock/agent-term/blob/c533566b80014c42e831ff28369f74bf53ee2049/src-tauri/src/lib.rs#L187), [WindowSpec](https://github.com/BumpyClock/gpui-component/blob/main/crates/app/src/windows/spec.rs) | Mica, blur, decoration, and compositor parity need native OS validation |
| Foreground activation | Regular, Forced, and Passive policies | Host rewrite must select product policy | Shell policy | conformance-proven | [activation policy](https://github.com/BumpyClock/gpui-component/blob/main/crates/app/src/liveness.rs) | Preserve forced activation only where product behavior requires it |
| Diagnostics, panic hook, file logging | App-owned initialization seams and runtime error sink | Retain current behavior | App | adopter-proven | [Agent Term diagnostics](https://github.com/BumpyClock/agent-term/blob/c533566b80014c42e831ff28369f74bf53ee2049/src-tauri/crates/agentterm-shared/src/diagnostics.rs), [startup](https://github.com/BumpyClock/agent-term/blob/c533566b80014c42e831ff28369f74bf53ee2049/src-tauri/src/lib.rs#L94) | Do not silently replace sink, retention, redaction, or panic policy |
| Settings and layout migrations | New schema-versioned stores only | Explicit legacy conversion required | App | unproven | [Agent Term session storage](https://github.com/BumpyClock/agent-term/blob/c533566b80014c42e831ff28369f74bf53ee2049/src-tauri/src/session/storage.rs), [AppShell settings](https://github.com/BumpyClock/gpui-component/blob/main/crates/app/src/settings.rs) | Never repoint existing data without verified conversion and rollback |
| React/WebView and invoke IPC | Not covered | Blocked on native GPUI rewrite | Agent Term product | adopter-proven | [invoke host](https://github.com/BumpyClock/agent-term/blob/c533566b80014c42e831ff28369f74bf53ee2049/src-tauri/src/lib.rs#L138), [frontend dependencies](https://github.com/BumpyClock/agent-term/blob/c533566b80014c42e831ff28369f74bf53ee2049/package.json), [GPUI WebView status](https://github.com/BumpyClock/gpui-component/blob/main/crates/webview/README.md) | Embedding the current UI breaks Linux parity; replace every frontend consumer |
| PTY, MCP, search, and session services | Intentionally app-owned | Reuse after native rewrite | App | adopter-proven | [Agent Term command host](https://github.com/BumpyClock/agent-term/blob/c533566b80014c42e831ff28369f74bf53ee2049/src-tauri/src/lib.rs#L138) | These are product services, not shell features |
| Sidecar process and protocol | Intentionally app-owned | Keep supervision/protocol | App | adopter-proven | [Agent Term Rust workspace](https://github.com/BumpyClock/agent-term/blob/c533566b80014c42e831ff28369f74bf53ee2049/src-tauri/Cargo.toml) | Native host may spawn it directly |
| Sidecar bundling | Not covered | Later packaging capability | Packaging tool plus app config | adopter-proven | [Tauri external binary](https://github.com/BumpyClock/agent-term/blob/c533566b80014c42e831ff28369f74bf53ee2049/src-tauri/tauri.conf.json#L35) | Preserve target naming, executable permissions, placement, and signing |
| Signed updater | Not covered | Later updater engine | Platform engine plus app policy/key | adopter-proven | [updater config](https://github.com/BumpyClock/agent-term/blob/c533566b80014c42e831ff28369f74bf53ee2049/src-tauri/tauri.conf.json#L46) | Keep Tauri updater until download, verification, install, restart, and failure behavior match |
| Bundle, installer, signing, notarization, release | Manifest verifies metadata only | Later packaging work | Platform tooling plus app workflow | adopter-proven | [bundle config](https://github.com/BumpyClock/agent-term/blob/c533566b80014c42e831ff28369f74bf53ee2049/src-tauri/tauri.conf.json#L32), [release workflow](https://github.com/BumpyClock/agent-term/blob/c533566b80014c42e831ff28369f74bf53ee2049/.github/workflows/release.yml) | Blocks any Electron/Tauri replacement claim |
| Single instance, deep links, file associations | Not covered cross-platform | Later platform work | Shell/platform plus packaging | unproven | [AppShell capabilities](https://github.com/BumpyClock/gpui-component/blob/main/crates/app/src/capabilities.rs) | Requires registration, second-instance forwarding, and launch-delivery proof on all platforms |
| Windows GUI subsystem attribute | Impossible in a library | App/scaffold-owned | App/scaffold | adopter-proven | [Agent Term crate root](https://github.com/BumpyClock/agent-term/blob/c533566b80014c42e831ff28369f74bf53ee2049/src-tauri/src/main.rs) | Future scaffold templates the crate attribute |

The configured Agent Term updater public key is nonempty. Authenticity and
signature validity were not verified during this audit; do not disable it based
on the stale “zeroed key” statement in the older platform plan.

## Replacement gate

An Agent Term migration may claim reduced boilerplate with feature parity only
after every row is either:

1. `conformance-proven` and exercised by the native Agent Term host,
2. retained as an explicitly app-owned implementation with adopter evidence, or
3. intentionally removed through a product decision.

Compile success or an AppShell example alone is not Agent Term migration evidence.
