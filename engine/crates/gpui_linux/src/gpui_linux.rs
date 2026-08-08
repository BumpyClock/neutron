#![cfg(any(target_os = "linux", target_os = "freebsd"))]
mod linux;

pub use linux::{
    current_platform, current_platform_with_startup_activation_token,
    take_startup_activation_token_from_environment, try_current_platform,
    try_current_platform_with_startup_activation_token,
};
