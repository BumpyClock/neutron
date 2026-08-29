//! The framework's default About surface.
//!
//! Installed by convention for every declaration that does not replace or
//! disable it, so a desktop application has a working About window before it
//! writes any UI code.
//!
//! Content is deliberately narrow: display name, version, an optional
//! publisher, and the application ID as secondary selectable text for support
//! and bug reports. Storage namespaces, URL schemes, package internals, and
//! platform manifest details are the framework's business, not the user's, and
//! never appear here.

use gpui::{
    App, AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render, SharedString,
    Styled as _, Window, px, size,
};
use neutron_components::{ActiveTheme as _, text::TextView, v_flex};

use crate::declaration::{DeclaredSurface, Surface, SurfaceKey, SurfaceRole};
use crate::handles::AppShellExt as _;
use crate::windows::WindowSize;

/// The framework's default About content.
pub(crate) struct AboutWindow {
    pub(super) display_name: SharedString,
    pub(super) version: SharedString,
    pub(super) publisher: Option<SharedString>,
    pub(super) app_id: SharedString,
}

impl AboutWindow {
    /// Read the compiled-in identity the shell already resolved.
    ///
    /// Snapshotted rather than read per frame: the identity is immutable for
    /// the whole process, so a snapshot cannot drift.
    fn from_identity(cx: &App) -> Self {
        let identity = cx.app_info().identity();
        Self {
            display_name: SharedString::new_static(identity.display_name),
            version: SharedString::new_static(identity.version),
            publisher: identity.publisher.map(SharedString::new_static),
            app_id: SharedString::new_static(identity.app_id),
        }
    }
}

impl Render for AboutWindow {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .gap_2()
            .p_6()
            .justify_center()
            .items_center()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(
                v_flex()
                    .items_center()
                    .child(self.display_name.clone())
                    .child(
                        gpui::div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(format!("Version {}", self.version)),
                    ),
            )
            .children(self.publisher.clone().map(|publisher| {
                gpui::div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(publisher)
            }))
            // Selectable so a user filing a bug report can copy the identifier
            // the framework namespaces their data under.
            .child(
                TextView::markdown("about-app-id", self.app_id.clone())
                    .selectable(true)
                    .text_color(cx.theme().muted_foreground)
                    .text_xs(),
            )
    }
}

/// Build the default About content from the resolved identity.
fn build(_: &(), _: &mut Window, cx: &mut App) -> Entity<AboutWindow> {
    let about = AboutWindow::from_identity(cx);
    cx.new(|_| about)
}

/// The framework's default About surface, already erased in its role.
///
/// Singleton and chrome-free follow from [`SurfaceRole::About`] rather than
/// from explicit options here, so the default About obeys exactly the same role
/// policy an application-supplied one does. Root composition is likewise the
/// window manager's job, not this surface's.
pub(crate) fn default_about_surface() -> DeclaredSurface {
    DeclaredSurface::erase(
        Surface::new(SurfaceKey::<AboutWindow>::about(), build)
            .size(WindowSize::Fixed(size(px(360.), px(220.)))),
        SurfaceRole::About,
    )
}
