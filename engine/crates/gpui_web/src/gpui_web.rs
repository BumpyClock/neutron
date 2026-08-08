#![cfg(target_family = "wasm")]
//! Web GPUI platform.
//!
//! The default feature set enables multithreaded WebAssembly. Build default
//! `gpui_web` with nightly Rust, atomics/shared-memory flags, and COOP/COEP
//! headers. Use `--no-default-features` for single-threaded stable wasm builds.

mod dispatcher;
mod display;
mod events;
mod http_client;
mod keyboard;
mod logging;
mod platform;
mod window;

pub use dispatcher::WebDispatcher;
pub use display::WebDisplay;
pub use http_client::FetchHttpClient;
pub use keyboard::WebKeyboardLayout;
pub use logging::init_logging;
pub use platform::WebPlatform;
pub use window::WebWindow;
