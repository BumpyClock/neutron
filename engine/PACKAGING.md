# GPUI package map and release gates

`cargo run --locked -p xtask -- publish-plan` derives publication order from normal and
build path dependencies. It reports local package versions, registry status,
metadata readiness, and private prerequisites. It never publishes.

## Candidate engine graph

The workspace dependency keys below are not registry claims. `registry` is the
name currently represented by the manifest; `provisional` is an unapproved,
fork-specific candidate recorded in `fork.toml`.

| Workspace dependency key | Rust import | Registry package identity | Version | Provisional candidate |
| --- | --- | --- | --- | --- |
| `gpui` | `gpui` | `bumpyclock-gpui` | `0.1.0 (selected-unpublished)` | — |
| `gpui_platform` | `gpui_platform` | `gpui_platform` | `0.1.0` | `bumpyclock-gpui-platform` |
| `gpui_linux` | `gpui_linux` | `gpui_linux` | `0.1.0` | `bumpyclock-gpui-linux` |
| `gpui_macos` | `gpui_macos` | `gpui_macos` | `0.1.0` | `bumpyclock-gpui-macos` |
| `gpui_windows` | `gpui_windows` | `gpui_windows` | `0.1.0` | `bumpyclock-gpui-windows` |
| `gpui_wgpu` | `gpui_wgpu` | `gpui_wgpu` | `0.1.0` | `bumpyclock-gpui-wgpu` |
| `gpui_web` | `gpui_web` | `gpui_web` | `0.1.0` | `bumpyclock-gpui-web` |
| `gpui_tokio` | `gpui_tokio` | `gpui_tokio` | `0.1.0` | `bumpyclock-gpui-tokio` |
| `gpui_macros` | `gpui_macros` | `gpui_macros` | `0.1.0 (conflict)` | `bumpyclock-gpui-macros` |
| `gpui_shared_string` | `gpui_shared_string` | `gpui_shared_string` | `0.1.0` | `bumpyclock-gpui-shared-string` |
| `gpui_util` | `gpui_util` | `gpui_util` | `0.1.0 (conflict)` | `bumpyclock-gpui-util` |
| `collections` | `collections` | `collections` | `0.1.0` | `bumpyclock-gpui-collections` |
| `http_client` | `http_client` | `http_client` | `0.1.0 (conflict)` | `bumpyclock-gpui-http-client` |
| `media` | `media` | `media` | `0.1.0 (conflict)` | `bumpyclock-gpui-media` |
| `util` | `util` | `util` | `0.1.0 (conflict)` | `bumpyclock-gpui-util` |
| `util_macros` | `util_macros` | `util_macros` | `0.1.0` | `bumpyclock-gpui-util-macros` |
| `refineable` | `refineable` | `refineable` | `0.1.0` | `bumpyclock-gpui-refineable` |
| `derive_refineable` | `derive_refineable` | `derive_refineable` | `0.1.0` | `bumpyclock-gpui-derive-refineable` |
| `scheduler` | `scheduler` | `scheduler` | `0.1.0 (conflict)` | `bumpyclock-gpui-scheduler` |
| `sum_tree` | `sum_tree` | `sum_tree` | `0.1.0` | `bumpyclock-gpui-sum-tree` |
| `zlog` | `zlog` | `zlog` | `0.1.0 (conflict)` | `bumpyclock-gpui-zlog` |
| `ztracing` | `ztracing` | `ztracing` | `0.1.0` | `bumpyclock-gpui-ztracing` |
| `ztracing_macro` | `ztracing_macro` | `ztracing_macro` | `0.1.0` | `bumpyclock-gpui-ztracing-macro` |
| `perf` | `perf` | `perf` | `0.1.0 (conflict)` | `bumpyclock-gpui-perf` |

`bumpyclock-gpui@0.1.0` is the owner-selected package identity for this fork, but it
has not been published or reserved on crates.io. The selected-unpublished status is
intentional and is not a registry ownership claim. `gpui-macros@0.2.2`,
`gpui_util@0.2.2`, and `zlog@0.1.0` already exist with external metadata; they must
not be treated as equivalent to this fork. Their package identities remain deferred.

The 2026-07-28 crates.io index audit (`cargo info --registry crates-io`) also
found existing `media@0.1.0`, `util@0.1.0`, `scheduler@0.1.0`, and `perf@0.0.2` packages owned
by unrelated projects. The remaining `@0.1.0` names in this table were not found
at that exact version. “Unavailable” is evidence at audit time, not a reservation
or ownership claim; recheck immediately before an authorized release.

## Artifact and patch gates

`cargo package --list -p <package>` is safe and validates the source file list.
`cargo package --no-verify` can inspect normalized manifests, but full packaging
cannot resolve this graph until prerequisite registry packages exist. Root
`[patch.crates-io]` entries (`async-task`, `calloop`, and `windows-capture`) are
also source-only overrides; `calloop` is pinned to lock revision
`eb6b4fd17b9af5ecc226546bdd04185391b3e265`. Packaged manifests drop them. The release checker
flags affected publishable packages instead of claiming source/registry
equivalence.

Run:

```sh
cargo run --locked -p xtask -- fork validate
cargo run --locked -p xtask -- publish-plan
cargo run --locked -p xtask -- release-check
cargo run --locked -p xtask -- release-check --require-registry
```

Both release checks must remain blocked until deferred package identities, GPL tracing
licensing, root patch equivalents, and prerequisite publication are resolved. The
selected `bumpyclock-gpui` identity is not itself a collision, but remains unpublished.
