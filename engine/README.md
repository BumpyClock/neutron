# GPUI

A standalone extraction of [Zed's](https://github.com/zed-industries/zed) GPU-accelerated UI framework.

GPUI is a hybrid immediate and retained mode, GPU accelerated, UI framework for Rust.

## Getting Started

Add GPUI as a git dependency:

```toml
[dependencies]
gpui = { package = "bumpyclock-gpui", git = "https://github.com/BumpyClock/gpui", rev = "<full-40-character-commit-sha>" }
```

Replace the placeholder with an immutable commit containing the renamed `bumpyclock-gpui`
package; do not depend on `main`.

See `crates/gpui/examples/` for example applications.

## Building

```sh
cargo check -p bumpyclock-gpui
cargo test -p bumpyclock-gpui --features test-support
cargo build --example hello_world
```

## Application lifecycle

GPUI library code performs orderly shutdown and returns control to its host; it does not terminate
the process. Executables remain responsible for choosing an exit code or restart policy after
`Application::run` returns.

| Host | `Application::run` ownership and return |
| --- | --- |
| Native and headless | Blocks in the platform loop, gives `on_app_quit` futures up to 100 ms, then returns normally. Native platform loops are one-shot. |
| Web | Returns after scheduling asynchronous launch. GPUI retains the application on the browser thread until shutdown; quit observers and the 100 ms timeout finish asynchronously because the browser event loop cannot be blocked. |
| Embedded | `run_embedded` returns an `ApplicationHandle`; the embedder owns that handle and drives the host run loop. Dropping it before an asynchronously scheduled launch cancels that launch. |

A quit requested during launch or later through `App::quit` wakes a blocking native/headless loop.
Cross-thread callers should submit the request through `MainThreadPoster`. The first quit closes
poster admission, drops queued/deferred effects, invokes the platform quit callback and shutdown
observers once, and starts the observer window. Repeated quit requests are no-ops, and posters
obtained before or after shutdown reject new submissions.

`QuitMode` controls window-driven shutdown:

- `Explicit` remains alive with zero windows until `App::quit` or an implemented OS quit request.
- `LastWindowClosed` quits after the final window closes.
- `Default` is `Explicit` on macOS and `LastWindowClosed` elsewhere.
- On macOS, an AppKit termination request enters the same orderly quit path and cancels immediate
  process termination while GPUI stops and wakes its owned run loop. Windows session-end messages
  (`WM_QUERYENDSESSION` / `WM_ENDSESSION`) remain unsupported and do not enter GPUI's orderly
  shutdown path.

Headless Web construction is unsupported and is reported by the fallible `gpui_platform::try_headless`
and `try_current_platform` APIs. The legacy infallible constructor wrappers remain for compatibility
and panic when platform initialization fails.

## Renderer and native evidence

`RendererSelection::{Default, Software}` controls adapter policy through
`GPUI_RENDERER=default|software`. Software mode is strict: Windows selects D3D11 WARP;
WGPU-backed Linux selects only a Vulkan CPU/software adapter; macOS returns an error because Metal
has no software adapter. An exact lavapipe claim also requires a constrained Vulkan ICD and matching
adapter-name evidence.

`Window::renderer_info()` reports structured selection, renderer, backend, adapter name, and adapter
type. `PresentationEvidence::ApiSubmitted` means only that a presentation API call returned; WGPU
uses it after `SurfaceTexture::present()`. `PresentationEvidence::BackendAccepted` means a backend
accepted or scheduled work; DXGI `Present` and backend-specific Metal status still do not prove
scanout. Software-GPU evidence is not hardware-GPU evidence.

Native conformance consumers should pair `HasWindowHandle` with `HasDisplayHandle` and serialize
only pointer-free handle kinds. Matching window/display families prove native construction, not
presentation. A real presentation claim requires separate renderer and first-presentation evidence.

## Upstream Sync

GPUI is a selective semantic fork of [Zed](https://github.com/zed-industries/zed), not a
contiguous merge. Exact provenance and maintained divergence clusters live in
[`fork.toml`](fork.toml), the machine-readable source of truth. See [UPSTREAM.md](UPSTREAM.md) for
sync policy, invariants, exclusions, verification evidence, and the next-sync procedure.

## Platform validation

The repository currently proves source builds and backend-focused tests. Native runtime behavior
remains session- and hardware-dependent: backend tests are not full application/window conformance,
Linux compile tests are not live X11 or Wayland compositor runs, and WebAssembly is compile-only.
Downstream Stage 1 native jobs may add stronger retained evidence, but configured jobs are not passed
runs. See CI and [UPSTREAM.md](UPSTREAM.md) before treating a platform as production-supported.

| Target | CI evidence | Maturity |
| --- | --- | --- |
| macOS | Native backend tests on macOS runner; no full app/window presentation claim | preview |
| Windows | Native backend tests plus shader compile on Windows runner; no full app/window presentation claim | preview |
| Linux X11 | Backend compile and tests; no live X11 display/presentation | experimental |
| Linux Wayland | Backend compile and tests; no live Wayland compositor/presentation | experimental |
| WebAssembly | Nightly compile-only checks | compile-only |

## License

The extracted GPUI crates generally declare Apache-2.0; `zlog`, `ztracing`, and
`ztracing_macro` declare GPL-3.0-or-later. See crate manifests and [LICENSE-AUDIT.md](LICENSE-AUDIT.md)
for the unresolved combined-publication review.
