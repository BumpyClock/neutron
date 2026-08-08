use gpui::{
    App, AppContext as _, Context, Entity, FocusHandle, Focusable, InteractiveElement as _,
    IntoElement, KeyBinding, ParentElement as _, Render, Styled as _, Window, div,
    prelude::FluentBuilder as _,
};
use gpui_component::{
    ActiveTheme as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    kbd::Kbd,
    label::Label,
    v_flex,
};
use gpui_component_app::{
    Capability, PlatformCapabilities,
    commands::{About, OpenSettings},
    liveness::{ExitPolicy, InitialActivation},
};

use crate::section;

const CONTEXT: &str = "app-shell-story";

pub fn init(cx: &mut App) {
    cx.bind_keys([
        #[cfg(target_os = "macos")]
        KeyBinding::new("cmd-,", OpenSettings, Some(CONTEXT)),
        #[cfg(not(target_os = "macos"))]
        KeyBinding::new("ctrl-,", OpenSettings, Some(CONTEXT)),
    ]);
}

#[derive(Clone, Copy, Default)]
enum RuntimeState {
    #[default]
    Ready,
    Settings,
    About,
    StartupFailure,
}

impl RuntimeState {
    fn title(self) -> &'static str {
        match self {
            Self::Ready => "Ready",
            Self::Settings => "OpenSettings handled",
            Self::About => "About handled",
            Self::StartupFailure => "Startup failure reported",
        }
    }

    fn description(self) -> &'static str {
        match self {
            Self::Ready => {
                "AppShell starts application code after its platform services are ready."
            }
            Self::Settings => "An app callback owns the Settings window or panel.",
            Self::About => "An app callback owns the About surface.",
            Self::StartupFailure => {
                "AppShell reports startup errors and exits nonzero; this gallery only previews the state."
            }
        }
    }
}

pub struct AppShellStory {
    focus_handle: FocusHandle,
    runtime_state: RuntimeState,
    activation: InitialActivation,
    exit_policy: ExitPolicy,
    capabilities: PlatformCapabilities,
}

impl super::Story for AppShellStory {
    fn title() -> &'static str {
        "App Shell"
    }

    fn description() -> &'static str {
        "Cross-platform AppShell startup, menus, settings actions, liveness, and capability reporting."
    }

    fn new_view(window: &mut Window, cx: &mut App) -> Entity<impl Render> {
        Self::view(window, cx)
    }
}

impl AppShellStory {
    pub fn view(window: &mut Window, cx: &mut App) -> Entity<Self> {
        cx.new(|cx| Self::new(window, cx))
    }

    fn new(_: &mut Window, cx: &mut Context<Self>) -> Self {
        Self {
            focus_handle: cx.focus_handle(),
            runtime_state: RuntimeState::Ready,
            activation: InitialActivation::Regular,
            exit_policy: ExitPolicy::WhenIdle,
            capabilities: PlatformCapabilities::detect(),
        }
    }

    fn show_settings(&mut self, _: &OpenSettings, _: &mut Window, cx: &mut Context<Self>) {
        self.runtime_state = RuntimeState::Settings;
        cx.notify();
    }

    fn show_about(&mut self, _: &About, _: &mut Window, cx: &mut Context<Self>) {
        self.runtime_state = RuntimeState::About;
        cx.notify();
    }

    fn set_runtime_state(&mut self, runtime_state: RuntimeState, cx: &mut Context<Self>) {
        self.runtime_state = runtime_state;
        cx.notify();
    }

    fn cycle_activation(&mut self, cx: &mut Context<Self>) {
        self.activation = match self.activation {
            InitialActivation::Regular => InitialActivation::Forced,
            InitialActivation::Forced => InitialActivation::Passive,
            InitialActivation::Passive => InitialActivation::Regular,
            _ => InitialActivation::Regular,
        };
        cx.notify();
    }

    fn toggle_exit_policy(&mut self, cx: &mut Context<Self>) {
        self.exit_policy = match self.exit_policy {
            ExitPolicy::WhenIdle => ExitPolicy::Explicit,
            ExitPolicy::Explicit => ExitPolicy::WhenIdle,
        };
        cx.notify();
    }
}

impl Focusable for AppShellStory {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for AppShellStory {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let runtime_state = self.runtime_state;
        let activation = self.activation;
        let exit_policy = self.exit_policy;
        let capabilities = self.capabilities;

        v_flex()
            .id("app-shell-story")
            .track_focus(&self.focus_handle)
            .key_context(CONTEXT)
            .on_action(cx.listener(Self::show_settings))
            .on_action(cx.listener(Self::show_about))
            .gap_6()
            .child(
                section("Standard desktop menus")
                    .sub_title("Native on macOS; render AppMenuBar in-window on Windows and Linux")
                    .child(menu_preview(cx)),
            )
            .child(
                section("Settings and About actions")
                    .sub_title("Uses AppShell's real OpenSettings and About action types")
                    .child(
                        h_flex()
                            .gap_3()
                            .child(
                                Button::new("app-shell-settings")
                                    .primary()
                                    .label("Open Settings")
                                    .on_click(|_, _, cx| cx.dispatch_action(&OpenSettings)),
                            )
                            .child(settings_shortcut())
                            .child(
                                Button::new("app-shell-about")
                                    .outline()
                                    .label("About")
                                    .on_click(|_, _, cx| cx.dispatch_action(&About)),
                            ),
                    )
                    .child(runtime_card(runtime_state, cx)),
            )
            .child(
                section("Launch and liveness")
                    .sub_title("Builder options shown here are previews; Gallery does not start a nested Application")
                    .child(
                        Button::new("app-shell-activation")
                            .outline()
                            .label(format!("Initial activation: {activation:?}"))
                            .on_click(cx.listener(|this, _, _, cx| this.cycle_activation(cx))),
                    )
                    .child(
                        Button::new("app-shell-exit-policy")
                            .outline()
                            .label(format!("Exit policy: {exit_policy:?}"))
                            .on_click(cx.listener(|this, _, _, cx| this.toggle_exit_policy(cx))),
                    ),
            )
            .child(
                section("Runtime error state")
                    .sub_title("Fatal startup errors win over a deferred quit request")
                    .child(
                        Button::new("app-shell-startup-failure")
                            .danger()
                            .label("Preview startup failure")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.set_runtime_state(RuntimeState::StartupFailure, cx);
                            })),
                    )
                    .child(
                        Button::new("app-shell-reset-runtime")
                            .outline()
                            .label("Reset preview")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.set_runtime_state(RuntimeState::Ready, cx);
                            })),
                    ),
            )
            .child(
                section("Platform capabilities")
                    .sub_title("Current target detection from PlatformCapabilities::detect()")
                    .child(capability_card("Overlay surface", capabilities.overlay_surface, cx))
                    .child(capability_card("Tray", capabilities.tray, cx))
                    .child(capability_card("Dock menu / jump list", capabilities.dock_menu, cx))
                    .child(capability_card("URL schemes", capabilities.url_schemes, cx))
                    .child(capability_card(
                        "Precise window positioning",
                        capabilities.precise_window_positioning,
                        cx,
                    )),
            )
    }
}

fn menu_preview(cx: &Context<AppShellStory>) -> impl IntoElement {
    #[cfg(target_os = "macos")]
    let menus = ["App", "Edit", "Window"];
    #[cfg(target_os = "windows")]
    let menus = ["File", "Edit", "View", "Window", "Help"];
    #[cfg(target_os = "linux")]
    let menus = ["File", "Edit", "View", "Window", "Help"];

    h_flex()
        .gap_1()
        .p_2()
        .rounded(cx.theme().radius)
        .bg(cx.theme().muted)
        .children(menus.into_iter().map(|menu| {
            div()
                .px_2()
                .py_1()
                .rounded(cx.theme().radius)
                .hover(|this| this.bg(cx.theme().accent))
                .child(menu)
        }))
}

fn settings_shortcut() -> Kbd {
    #[cfg(target_os = "macos")]
    return Kbd::new(gpui::Keystroke::parse("cmd-,").expect("valid settings shortcut"));
    #[cfg(not(target_os = "macos"))]
    Kbd::new(gpui::Keystroke::parse("ctrl-,").expect("valid settings shortcut"))
}

fn runtime_card(runtime_state: RuntimeState, cx: &Context<AppShellStory>) -> impl IntoElement {
    v_flex()
        .w_full()
        .gap_1()
        .p_3()
        .rounded(cx.theme().radius)
        .border_1()
        .border_color(cx.theme().border)
        .child(Label::new(runtime_state.title()).text_sm())
        .child(
            div()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(runtime_state.description()),
        )
}

fn capability_card(
    label: &'static str,
    capability: Capability,
    cx: &Context<AppShellStory>,
) -> impl IntoElement {
    let (state, reason, color) = match capability {
        Capability::Supported => ("Supported", None, cx.theme().green),
        Capability::Unsupported { reason } => ("Unsupported", Some(reason), cx.theme().red),
    };

    v_flex()
        .gap_1()
        .p_3()
        .rounded(cx.theme().radius)
        .border_1()
        .border_color(cx.theme().border)
        .child(
            h_flex()
                .justify_between()
                .w_full()
                .child(Label::new(label).text_sm())
                .child(div().text_sm().text_color(color).child(state)),
        )
        .when_some(reason, |this, reason| {
            this.child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(reason),
            )
        })
}
