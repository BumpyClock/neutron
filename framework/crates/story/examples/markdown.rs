use gpui::{prelude::FluentBuilder as _, *};
use neutron_components::{
    ActiveTheme as _, IconName, Sizable as _,
    button::{Button, ButtonVariants as _},
    clipboard::Clipboard,
    h_flex,
    highlighter::Language,
    input::{Input, InputEvent, InputState, TabSize},
    resizable::{h_resizable, resizable_panel},
    text::markdown,
};
use neutron_components_app::prelude::*;
use neutron_components_app::{
    AppDeclaration, Command, CommandBinding, CommandId, Menu, MenuBar, MenuKey, Surface, SurfaceKey,
};
use neutron_story::{
    default_example_window_size, example_failure, example_http_client_module, example_theme_source,
    focus_example, story_preferences_key, story_preferences_module, with_example_window_defaults,
};

neutron_components_app::include_identity!();

actions!(story_markdown_example, [OpenFile]);

const OPEN_FILE_ID: CommandId = CommandId::new("story-markdown-example.open-file");

pub struct Example {
    input_state: Entity<InputState>,
    _subscriptions: Vec<Subscription>,
}

const EXAMPLE: &str = include_str!("./fixtures/test.md");

impl Example {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input_state = cx.new(|cx| {
            InputState::new(window, cx)
                .code_editor(Language::Markdown)
                .line_number(true)
                .tab_size(TabSize {
                    tab_size: 2,
                    ..Default::default()
                })
                .searchable(true)
                .placeholder("Enter your Markdown here...")
                .default_value(EXAMPLE)
        });

        let _subscriptions = vec![cx.subscribe(&input_state, |_, _, _: &InputEvent, _| {})];

        Self {
            input_state,
            _subscriptions,
        }
    }

    fn on_action_open(&mut self, _: &OpenFile, window: &mut Window, cx: &mut Context<Self>) {
        let path = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: true,
            multiple: false,
            prompt: Some("Select a Markdown file".into()),
        });

        let input_state = self.input_state.clone();
        cx.spawn_in(window, async move |_, window| {
            let path = path.await.ok()?.ok()??.iter().next()?.clone();

            let content = std::fs::read_to_string(&path).ok()?;

            window
                .update(|window, cx| {
                    _ = input_state.update(cx, |this, cx| {
                        this.set_value(content, window, cx);
                    });
                })
                .ok();

            Some(())
        })
        .detach();
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
        div()
            .id("editor")
            .size_full()
            .on_action(cx.listener(Self::on_action_open))
            .child(
                h_resizable("container")
                    .child(
                        resizable_panel().child(
                            div()
                                .id("source")
                                .size_full()
                                .font_family(cx.theme().mono_font_family.clone())
                                .text_size(cx.theme().mono_font_size)
                                .child(
                                    Input::new(&self.input_state)
                                        .h_full()
                                        .p_0()
                                        .border_0()
                                        .focus_bordered(false),
                                ),
                        ),
                    )
                    .child(
                        resizable_panel().child(
                            markdown(self.input_state.read(cx).value())
                                .code_block_actions(|code_block, _window, _cx| {
                                    let code = code_block.code();
                                    let lang = code_block.lang();

                                    h_flex()
                                        .gap_1()
                                        .child(Clipboard::new("copy").value(code.clone()))
                                        .when_some(lang, |this, lang| {
                                            // Only show run terminal button for certain languages
                                            if lang.as_ref() == "rust" || lang.as_ref() == "python"
                                            {
                                                this.child(
                                                    Button::new("run-terminal")
                                                        .icon(IconName::SquareTerminal)
                                                        .ghost()
                                                        .xsmall()
                                                        .on_click(move |_, _, _cx| {
                                                            println!(
                                                                "Running {} code: {}",
                                                                lang, code
                                                            );
                                                        }),
                                                )
                                            } else {
                                                this
                                            }
                                        })
                                })
                                .flex_none()
                                .p_5()
                                .scrollable(true)
                                .selectable(true),
                        ),
                    ),
            )
    }
}

/// The window-scoped Open command: `cmd-o` on macOS, `ctrl-o` elsewhere,
/// matching the deleted global `Open` keybinding exactly. Dispatched to
/// whichever view is focused; `Example::on_action_open` handles it once focus
/// has moved inside its own subtree (e.g. after the user clicks the editor).
fn open_file_command() -> Command<OpenFile> {
    Command::window(OPEN_FILE_ID, OpenFile)
        .label("Open…")
        .binding(CommandBinding::platform("cmd-o", "ctrl-o"))
}

fn primary_surface() -> Surface<Example, ()> {
    with_example_window_defaults(
        Surface::new(SurfaceKey::primary(), Example::view)
            .title("Markdown Editor")
            .after_open(focus_example::<Example>),
        default_example_window_size(),
    )
}

/// The `markdown` example's `DesktopApp` declaration. Zero-sized: `AppShell`
/// never creates or retains an application object.
struct MarkdownExampleApp;

impl DesktopApp for MarkdownExampleApp {
    fn declaration() -> AppDeclaration {
        AppDeclaration::new(APP_IDENTITY)
            .initial_activation(InitialActivation::Forced)
            .theme(example_theme_source())
            .settings_store::<neutron_story::StoryUiPreferences>(story_preferences_key())
            .setup(example_http_client_module())
            .setup(story_preferences_module())
            .command(open_file_command())
            .menu_bar(
                MenuBar::standard().contribute(Menu::keyed(MenuKey::FILE).command(OPEN_FILE_ID)),
            )
            .primary_surface(primary_surface())
    }
}

fn main() -> std::process::ExitCode {
    match AppShell::run::<MarkdownExampleApp>() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => example_failure("markdown example", error),
    }
}
