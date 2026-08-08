# gpui_platform

Platform selection facade for the standalone GPUI framework.

Use `try_current_platform`, `try_application`, and `try_headless` when platform construction
failures must be handled by the caller. Reusable hosts should propagate these errors; executable
frontends may translate them to an `ExitCode`. The legacy `current_platform`, `application`, and
`headless` functions remain available for compatibility and panic when construction fails. The
`background_executor` convenience helper is also infallible; callers that need to handle startup
errors can obtain an executor from `try_headless()` instead.

Native and headless event loops are one-shot and return after orderly shutdown. Web launch is
asynchronous and cannot block its browser event loop; `run_embedded` instead gives lifecycle
ownership to the caller. Headless Web construction is unsupported. Windows session-end messages
(`WM_QUERYENDSESSION` / `WM_ENDSESSION`) are not integrated into the orderly quit path.

Renderer software selection is strict: Windows requires D3D11 WARP, Linux WGPU requires a Vulkan
CPU/software adapter, and macOS returns an error rather than silently selecting Metal. These
selection rules provide adapter evidence only; native window/display handles and presentation
status are separate contracts.

This package is part of the standalone [GPUI](https://github.com/BumpyClock/gpui) workspace.
