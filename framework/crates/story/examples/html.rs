use gpui::*;
use neutron_components::{
    ActiveTheme as _,
    highlighter::Language,
    input::{Input, InputState, TabSize},
    resizable::h_resizable,
    text::html,
};
use neutron_components_app::prelude::*;
use neutron_components_app::{AppDeclaration, Surface, SurfaceKey};
use neutron_story::{
    default_example_window_size, example_failure, example_http_client_module, example_theme_source,
    focus_example, story_preferences_key, story_preferences_module, with_example_window_defaults,
};

neutron_components_app::include_identity!();

pub struct Example {
    input_state: Entity<InputState>,
    _subscribe: Subscription,
}

const EXAMPLE: &str = include_str!("./fixtures/test.html");

impl Example {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input_state = cx.new(|cx| {
            InputState::new(window, cx)
                .code_editor(Language::Html)
                .tab_size(TabSize {
                    tab_size: 4,
                    hard_tabs: false,
                })
                .default_value(EXAMPLE)
                .placeholder("Enter your HTML here...")
        });

        let _subscribe = cx.subscribe(
            &input_state,
            |_, _, _: &neutron_components::input::InputEvent, cx| {
                cx.notify();
            },
        );

        Self {
            input_state,
            _subscribe,
        }
    }

    fn view(_args: &(), window: &mut Window, cx: &mut App) -> Entity<Self> {
        cx.new(|cx| Self::new(window, cx))
    }
}

impl Focusable for Example {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.input_state.focus_handle(cx)
    }
}

impl Render for Example {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        h_resizable("container")
            .child(
                div()
                    .id("source")
                    .size_full()
                    .font_family(cx.theme().mono_font_family.clone())
                    .text_size(cx.theme().mono_font_size)
                    .child(
                        Input::new(&self.input_state)
                            .h_full()
                            .appearance(false)
                            .focus_bordered(false),
                    )
                    .into_any(),
            )
            .child(
                html(self.input_state.read(cx).value())
                    .p_5()
                    .scrollable(true)
                    .selectable(true)
                    .into_any(),
            )
    }
}

/// This example's primary surface: the deleted `create_new_window`'s default
/// 1600x1200 (clamped to 85% of the display) and 480x320 minimum, matching
/// every other unsized example and the main gallery.
fn primary_surface() -> Surface<Example, ()> {
    with_example_window_defaults(
        Surface::new(SurfaceKey::primary(), Example::view)
            .title("HTML Render (native)")
            .after_open(focus_example::<Example>),
        default_example_window_size(),
    )
}

/// The `html` example's `DesktopApp` declaration. Zero-sized: `AppShell` never
/// creates or retains an application object.
struct HtmlExampleApp;

impl DesktopApp for HtmlExampleApp {
    fn declaration() -> AppDeclaration {
        AppDeclaration::new(APP_IDENTITY)
            .initial_activation(InitialActivation::Forced)
            .theme(example_theme_source())
            .settings_store::<neutron_story::StoryUiPreferences>(story_preferences_key())
            .setup(example_http_client_module())
            .setup(story_preferences_module())
            .primary_surface(primary_surface())
    }
}

fn main() -> std::process::ExitCode {
    match AppShell::run::<HtmlExampleApp>() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => example_failure("html example", error),
    }
}
