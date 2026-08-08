# Welcome to GPUI

GPUI is a hybrid immediate and retained mode, GPU accelerated, UI framework
for Rust, designed to support a wide variety of applications.

## Getting Started

GPUI is still pre-1.0 and may make breaking changes between versions. Native backends exist for macOS, Windows, Linux X11, and Linux Wayland, but their validation maturity differs; Web is a separate asynchronous host contract. Use the repository-pinned Rust toolchain and consult the root platform-evidence matrix before making support claims. While the fork facade is unpublished, add the following Git dependency to your `Cargo.toml` (the dependency key remains `gpui`):

```toml
gpui = { package = "bumpyclock-gpui", git = "https://github.com/BumpyClock/gpui", rev = "<full-40-character-commit-sha>" }
```

Replace the placeholder with an immutable commit containing the renamed `bumpyclock-gpui`
package; do not depend on `main`.

- [Ownership and data flow](_ownership_and_data_flow)
- [Accessibility](_accessibility)

Everything in GPUI starts with an `Application`. You can create one with `Application::new()`, and kick off your application by passing a callback to `Application::run()`. Inside this callback, you can create a new window with `App::open_window()`, and register your first root view. See [gpui.rs](https://www.gpui.rs/) for a complete example.

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

### Dependencies

GPUI has various system dependencies that it needs in order to work.

#### macOS

On macOS, GPUI uses Metal for rendering. In order to use Metal, you need to do the following:

- Install [Xcode](https://apps.apple.com/us/app/xcode/id497799835?mt=12) from the macOS App Store, or from the [Apple Developer](https://developer.apple.com/download/all/) website. Note this requires a developer account.

> Ensure you launch Xcode after installing, and install the macOS components, which is the default option.

- Install [Xcode command line tools](https://developer.apple.com/xcode/resources/)

  ```sh
  xcode-select --install
  ```

- Ensure that the Xcode command line tools are using your newly installed copy of Xcode:

  ```sh
  sudo xcode-select --switch /Applications/Xcode.app/Contents/Developer
  ```

## Renderer conformance

`RendererSelection::{Default, Software}` is selected with `GPUI_RENDERER=default|software`. Software mode is strict: Windows selects D3D11 WARP; WGPU-backed native platforms require a Vulkan CPU/software adapter such as lavapipe. A software request fails if the platform cannot configure that adapter; it never falls back to hardware or GL. Exact lavapipe evidence also requires a constrained ICD and matching adapter name. Metal has no software adapter, so macOS rejects this selection.

Use `Window::renderer_info()` for structured renderer and adapter evidence. `Window::observe_first_presentation()` resolves once with `PresentationEvidence`: `backend_accepted` means the renderer backend accepted or scheduled the presentation; `api_submitted` means only that the native presentation API returned. WGPU reports `api_submitted` after `SurfaceTexture::present()` returns because public WGPU does not expose per-surface backend status. Successful DXGI `Present` and backend-specific Metal status do not prove scanout. Software-GPU evidence does not prove hardware-GPU execution.

`Window` exposes both `HasWindowHandle` and `HasDisplayHandle`. Pointer-free native conformance should record matching handle kinds, not raw values or addresses. Window/display construction evidence is separate from renderer selection and presentation evidence; constructing a window does not prove presentation.

## The Big Picture

GPUI offers three different [registers](<https://en.wikipedia.org/wiki/Register_(sociolinguistics)>) depending on your needs:

- State management and communication with `Entity`'s. Whenever you need to store application state that communicates between different parts of your application, you'll want to use GPUI's entities. Entities are owned by GPUI and are only accessible through an owned smart pointer similar to an `Rc`. See the `app::context` module for more information.

- High level, declarative UI with views. All UI in GPUI starts with a view. A view is simply an `Entity` that can be rendered, by implementing the `Render` trait. At the start of each frame, GPUI will call this render method on the root view of a given window. Views build a tree of `elements`, lay them out and style them with a tailwind-style API, and then give them to GPUI to turn into pixels. See the `div` element for an all purpose swiss-army knife of rendering.

- Low level, imperative UI with Elements. Elements are the building blocks of UI in GPUI, and they provide a nice wrapper around an imperative API that provides as much flexibility and control as you need. Elements have total control over how they and their child elements are rendered and can be used for making efficient views into large lists, implement custom layouting for a code editor, and anything else you can think of. See the `element` module for more information.

Each of these registers has one or more corresponding contexts that can be accessed from all GPUI services. This context is your main interface to GPUI, and is used extensively throughout the framework.

## Other Resources

In addition to the systems above, GPUI provides a range of smaller services that are useful for building complex applications:

- Actions are user-defined structs that are used for converting keystrokes into logical operations in your UI. Use this for implementing keyboard shortcuts, such as cmd-q. See the `action` module for more information.

- Platform services, such as `quit the app` or `open a URL` are available as methods on the `app::App`.

- An async executor that is integrated with the platform's event loop. See the `executor` module for more information.,

- The `[gpui::test]` macro provides a convenient way to write tests for your GPUI applications. Tests also have their own kind of context, a `TestAppContext` which provides ways of simulating common platform input. See `app::test_context` and `test` modules for more details.

Currently, the best way to learn about these APIs is to read the Zed source code or drop a question in the [Zed Discord](https://zed.dev/community-links). We're working on improving the documentation, creating more examples, and will be publishing more guides to GPUI on our [blog](https://zed.dev/blog).
