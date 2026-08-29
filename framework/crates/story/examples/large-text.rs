use gpui::*;
use neutron_components::{
    ActiveTheme, Selectable, Sizable, WindowExt,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{self, Input, InputEvent, InputState, TabSize},
    v_flex,
};
use neutron_components_app::prelude::*;
use neutron_components_app::{AppDeclaration, Surface, SurfaceKey};
use neutron_story::{
    default_example_window_size, example_failure, example_http_client_module, example_theme_source,
    focus_example, story_preferences_key, story_preferences_module, with_example_window_defaults,
};

neutron_components_app::include_identity!();

pub struct Example {
    editor: Entity<InputState>,
    go_to_line_state: Entity<InputState>,
    soft_wrap: bool,
    _subscribes: Vec<Subscription>,
}

impl Example {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        // 10K lines
        let text = "这是一个中文演示段落，用于展示更多的 [Markdown GFM] 内容。您可以在此尝试使用使用**粗体**、*斜体*和`代码`等样式。これは日本語のデモ段落です。Markdown の多言語サポートを示すためのテキストが含まれています。例えば、、**ボールド**、_イタリック_、および`コード`のスタイルなどを試すことができます。\n".repeat(10000);

        let editor = cx.new(|cx| {
            InputState::new(window, cx)
                .multi_line(true)
                .tab_size(TabSize {
                    tab_size: 4,
                    hard_tabs: false,
                })
                .soft_wrap(true)
                .placeholder("Enter your code here...")
                .default_value(text)
        });
        let go_to_line_state = cx.new(|cx| InputState::new(window, cx));

        let _subscribes = vec![cx.subscribe(&editor, |_, _, _: &InputEvent, cx| {
            cx.notify();
        })];

        Self {
            editor,
            go_to_line_state,
            soft_wrap: false,
            _subscribes,
        }
    }

    fn view(_args: &(), window: &mut Window, cx: &mut App) -> Entity<Self> {
        cx.new(|cx| Self::new(window, cx))
    }

    fn go_to_line(&mut self, _: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        let editor = self.editor.clone();
        let input_state = self.go_to_line_state.clone();

        window.open_dialog(cx, move |dialog, window, cx| {
            input_state.update(cx, |state, cx| {
                let position = editor.read(cx).cursor_position();
                state.set_placeholder(
                    format!("{}:{}", position.line, position.character),
                    window,
                    cx,
                );
                state.focus(window, cx);
            });

            dialog
                .title("Go to line")
                .child(Input::new(&input_state))
                .confirm()
                .on_ok({
                    let editor = editor.clone();
                    let input_state = input_state.clone();
                    move |_, window, cx| {
                        let query = input_state.read(cx).value();
                        let mut parts = query
                            .split(':')
                            .map(|s| s.trim().parse::<usize>().ok())
                            .collect::<Vec<_>>()
                            .into_iter();
                        let Some(line) = parts.next().and_then(|l| l) else {
                            return false;
                        };
                        let line = line.saturating_sub(1);
                        let column = parts.next().and_then(|c| c).unwrap_or(1).saturating_sub(1);

                        editor.update(cx, |state, cx| {
                            state.set_cursor_position(
                                input::Position::new(line as u32, column as u32),
                                window,
                                cx,
                            );
                        });

                        true
                    }
                })
        });
    }

    fn toggle_soft_wrap(&mut self, _: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.soft_wrap = !self.soft_wrap;
        self.editor.update(cx, |state, cx| {
            state.set_soft_wrap(self.soft_wrap, window, cx);
        });
        cx.notify();
    }
}

impl Focusable for Example {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.editor.focus_handle(cx)
    }
}

impl Render for Example {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex().size_full().child(
            v_flex()
                .id("source")
                .w_full()
                .flex_1()
                .child(
                    Input::new(&self.editor)
                        .bordered(false)
                        .h_full()
                        .focus_bordered(false),
                )
                .child(
                    h_flex()
                        .justify_between()
                        .text_sm()
                        .bg(cx.theme().secondary)
                        .py_1p5()
                        .px_4()
                        .border_t_1()
                        .border_color(cx.theme().border)
                        .text_color(cx.theme().muted_foreground)
                        .child(h_flex().gap_3().child({
                            Button::new("soft-wrap")
                                .ghost()
                                .xsmall()
                                .label("Soft Wrap")
                                .selected(self.soft_wrap)
                                .on_click(cx.listener(Self::toggle_soft_wrap))
                        }))
                        .child({
                            let loc = self.editor.read(cx).cursor_position();
                            let cursor = self.editor.read(cx).cursor();

                            Button::new("line-column")
                                .ghost()
                                .xsmall()
                                .label(format!("{}:{} ({} c)", loc.line, loc.character, cursor))
                                .on_click(cx.listener(Self::go_to_line))
                        }),
                ),
        )
    }
}

fn primary_surface() -> Surface<Example, ()> {
    with_example_window_defaults(
        Surface::new(SurfaceKey::primary(), Example::view)
            .title("Large Text Editor")
            .after_open(focus_example::<Example>),
        default_example_window_size(),
    )
}

/// The `large-text` example's `DesktopApp` declaration. Zero-sized: `AppShell`
/// never creates or retains an application object.
struct LargeTextExampleApp;

impl DesktopApp for LargeTextExampleApp {
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
    match AppShell::run::<LargeTextExampleApp>() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => example_failure("large-text example", error),
    }
}
