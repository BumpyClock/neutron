use gpui::*;
use neutron_components::{
    button::Button,
    h_flex,
    text::{TextView, TextViewState},
    v_flex,
};
use neutron_components_app::prelude::*;
use neutron_components_app::{AppDeclaration, Surface, SurfaceKey};
use neutron_story::{
    example_failure, example_http_client_module, example_theme_source, focus_example,
    story_preferences_key, story_preferences_module, with_example_window_defaults,
};

neutron_components_app::include_identity!();

pub struct Example {
    focus_handle: FocusHandle,
    markdown_state: Entity<TextViewState>,
    tx: smol::channel::Sender<String>,
    scroll_handle: ScrollHandle,
    _task: Task<()>,
    _update_task: Task<()>,
}

const EXAMPLE: &str = include_str!("./fixtures/test.md");

impl Example {
    pub fn new(_: &mut Window, cx: &mut Context<Self>) -> Self {
        let markdown_state =
            cx.new(|cx| TextViewState::markdown("# Streaming Markdown Parse\n\n", cx));
        let scroll_handle = ScrollHandle::new();

        let (tx, rx) = smol::channel::unbounded::<String>();
        let _task = cx.spawn({
            let scroll_handle = scroll_handle.clone();
            let weak_state = markdown_state.downgrade();
            async move |_, cx| {
                while let Ok(chunk) = rx.recv().await {
                    _ = weak_state.update(cx, |state, cx| {
                        // Push the new chunk to the markdown state,
                        // it will reparse and re-render automatically.
                        state.push_str(&chunk, cx);
                        scroll_handle.scroll_to_bottom();
                    });
                }
            }
        });

        Self {
            focus_handle: cx.focus_handle(),
            markdown_state,
            scroll_handle,
            tx,
            _task,
            _update_task: Task::ready(()),
        }
    }

    fn view(_args: &(), window: &mut Window, cx: &mut App) -> Entity<Self> {
        cx.new(|cx| Self::new(window, cx))
    }

    /// Simulate streaming by updating markdown state in chunks
    /// 50ms for a iteration, every time adding about 5 - 20 characters
    /// This is just for demonstration; in a real app, you'd stream from a source.
    fn replay(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let tx = self.tx.clone();
        let mut current = 0;
        self.markdown_state.update(cx, |state, cx| {
            state.set_text("", cx);
        });

        self._update_task = cx.background_executor().spawn(async move {
            let chars: Vec<char> = EXAMPLE.chars().collect();
            while current < chars.len() {
                let chunk_size = (5 + rand::random::<usize>() % 15).min(chars.len() - current);
                let chunk: String = chars[current..current + chunk_size].iter().collect();
                _ = tx.try_send(chunk);
                current += chunk_size;
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
        });
    }
}

impl Focusable for Example {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for Example {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .id("example")
            .track_focus(&self.focus_handle)
            .size_full()
            .p_4()
            .gap_4()
            .child(
                h_flex()
                    .w_full()
                    .child(
                        Button::new("replay")
                            .outline()
                            .label("Replay")
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.replay(window, cx);
                            })),
                    ),
            )
            .child(
                div()
                    .id("contents")
                    .flex_1()
                    .w_full()
                    .track_scroll(&self.scroll_handle)
                    .overflow_y_scroll()
                    .size_full()
                    .child(TextView::new(&self.markdown_state).selectable(true)),
            )
    }
}

fn primary_surface() -> Surface<Example, ()> {
    with_example_window_defaults(
        Surface::new(SurfaceKey::primary(), Example::view)
            .title("Stream Markdown")
            .after_open(focus_example::<Example>),
        size(px(600.), px(800.)),
    )
}

/// The `stream_markdown` example's `DesktopApp` declaration. Zero-sized:
/// `AppShell` never creates or retains an application object.
struct StreamMarkdownExampleApp;

impl DesktopApp for StreamMarkdownExampleApp {
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
    match AppShell::run::<StreamMarkdownExampleApp>() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => example_failure("stream_markdown example", error),
    }
}
