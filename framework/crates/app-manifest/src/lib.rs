//! App identity schema, validation, and app-local build-time code generation.

pub mod build;
mod error;
pub mod parse;
pub mod schema;
pub mod versions;

pub use error::ManifestError;

/// Includes identity source generated for the consuming application, defining a
/// `pub static APP_IDENTITY` at the call site.
///
/// Call this from the consuming application's own crate after its `build.rs` has
/// called [`build::emit_identity`]. Do not call it from a library crate: `OUT_DIR`
/// must belong to the application. `gpui-component-app` re-exports this macro for
/// ergonomic access.
///
/// # No direct manifest dependency required
///
/// The generated source names its schema types unqualified (`IdentityRef`, …).
/// This macro includes that source inside a private module that imports those
/// types with `use $crate::schema::*`. `$crate` resolves to the crate that
/// *defined* the macro — `gpui-component-manifest` — by def-id rather than the
/// caller's extern prelude, so it works even when the macro is reached through a
/// re-export (e.g. `gpui_component_app::include_identity!`). Consequently an app
/// that calls this via `gpui-component-app` needs `gpui-component-manifest` only
/// as a `[build-dependencies]` entry (for [`build::emit_identity`]) — never as a
/// runtime dependency.
#[macro_export]
macro_rules! include_identity {
    () => {
        // The private module scopes the `use` so the generated file's unqualified
        // type names resolve via `$crate`; `pub use` then lifts `APP_IDENTITY`
        // back to the call site. `unused_imports` is allowed because an identity
        // without a given platform section never references that section's type.
        #[doc(hidden)]
        mod __gpui_app_identity {
            #[allow(unused_imports)]
            use $crate::schema::{
                IdentityRef, LinuxIdentityRef, MacosIdentityRef, MinOsVersionsRef,
                WindowsIdentityRef,
            };
            include!(concat!(env!("OUT_DIR"), "/gpui_app_identity.rs"));
        }
        pub use __gpui_app_identity::APP_IDENTITY;
    };
}
