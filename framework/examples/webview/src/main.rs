use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::{
    ActiveTheme as _, Root, h_flex,
    input::{Input, InputEvent, InputState},
    v_flex,
};
use gpui_wry::WebView;

pub struct Example {
    focus_handle: FocusHandle,
    webview: Option<Entity<WebView>>,
    address_input: Entity<InputState>,
    status_message: Option<SharedString>,
}

impl Example {
    fn build_webview(window: &mut Window, cx: &mut App) -> anyhow::Result<WebView> {
        let builder = wry::WebViewBuilder::new();
        #[cfg(any(debug_assertions, feature = "inspector"))]
        let builder = builder.with_devtools(true);

        #[cfg(not(any(
            target_os = "windows",
            target_os = "macos",
            target_os = "ios",
            target_os = "android"
        )))]
        let webview = {
            use gtk::prelude::*;
            use wry::WebViewBuilderExtUnix;

            let fixed = gtk::Fixed::builder().build();
            fixed.show_all();
            builder
                .build_gtk(&fixed)
                .map_err(|err| anyhow::anyhow!("failed to create GTK webview: {err}"))?
        };
        #[cfg(any(
            target_os = "windows",
            target_os = "macos",
            target_os = "ios",
            target_os = "android"
        ))]
        let webview = {
            use raw_window_handle::HasWindowHandle;

            let window_handle = window.window_handle().map_err(|err| {
                anyhow::anyhow!("failed to access the native window handle: {err}")
            })?;
            builder
                .build_as_child(&window_handle)
                .map_err(|err| anyhow::anyhow!("failed to create child webview: {err}"))?
        };

        Ok(WebView::new(webview, window, cx))
    }

    fn set_status(&mut self, message: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.status_message = Some(message.into());
        cx.notify();
    }

    fn clear_status(&mut self, cx: &mut Context<Self>) {
        if self.status_message.take().is_some() {
            cx.notify();
        }
    }

    pub fn new(window: &mut Window, cx: &mut App) -> Entity<Self> {
        let (webview, mut status_message) = match Self::build_webview(window, cx) {
            Ok(webview) => (Some(cx.new(|_| webview)), None),
            Err(err) => (None, Some(SharedString::from(err.to_string()))),
        };

        let address_input = cx.new(|cx| {
            InputState::new(window, cx).default_value("https://longbridge.github.io/gpui-component")
        });

        let url = address_input.read(cx).value();
        if let Some(webview) = webview.as_ref() {
            if let Err(err) = webview.update(cx, |view, _| view.load_url(&url)) {
                status_message = Some(err.to_string().into());
            }
        }

        cx.new(|cx| {
            let this = Self {
                focus_handle: cx.focus_handle(),
                webview,
                address_input: address_input.clone(),
                status_message,
            };

            cx.subscribe(
                &address_input,
                |this: &mut Self, input, event: &InputEvent, cx| match event {
                    InputEvent::PressEnter { .. } => {
                        let url = input.read(cx).value();
                        let Some(webview) = this.webview.as_ref() else {
                            this.set_status("WebView is unavailable because setup failed.", cx);
                            return;
                        };

                        match webview.update(cx, |view, _| view.load_url(&url)) {
                            Ok(()) => this.clear_status(cx),
                            Err(err) => this.set_status(err.to_string(), cx),
                        }
                    }
                    _ => {}
                },
            )
            .detach();

            this
        })
    }

    pub fn hide(&self, _: &mut Window, cx: &mut App) {
        if let Some(webview) = self.webview.as_ref() {
            let _ = webview.update(cx, |webview, _| webview.hide());
        }
    }

    #[allow(unused)]
    fn go_back(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        let Some(webview) = self.webview.as_ref() else {
            self.set_status("WebView is unavailable because setup failed.", cx);
            return;
        };

        match webview.update(cx, |webview, _| webview.back()) {
            Ok(()) => self.clear_status(cx),
            Err(err) => self.set_status(err.to_string(), cx),
        }
    }
}

impl Focusable for Example {
    fn focus_handle(&self, _cx: &gpui::App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for Example {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let status_message = self.status_message.clone().or_else(|| {
            self.webview
                .as_ref()
                .and_then(|webview| webview.read(cx).last_error().cloned())
        });

        v_flex()
            .p_2()
            .gap_3()
            .size_full()
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(Input::new(&self.address_input)),
            )
            .when_some(status_message, |this, message| {
                this.child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().danger_foreground)
                        .child(message),
                )
            })
            .child(
                div()
                    .flex_1()
                    .border_1()
                    .h(gpui::px(400.))
                    .border_color(cx.theme().border)
                    .child(match self.webview.clone() {
                        Some(webview) => div().size_full().child(webview).into_any_element(),
                        None => div()
                            .size_full()
                            .items_center()
                            .justify_center()
                            .text_color(cx.theme().muted_foreground)
                            .child("WebView setup failed.")
                            .into_any_element(),
                    }),
            )
    }
}

fn main() {
    // Required this for Windows to render the WebView.
    #[cfg(target_os = "windows")]
    unsafe {
        std::env::set_var("GPUI_DISABLE_DIRECT_COMPOSITION", "true");
    }

    gpui_platform::application().run(move |cx| {
        // This must be called before using any GPUI Component features.
        gpui_component::init(cx);

        cx.spawn(async move |cx| {
            cx.open_window(WindowOptions::default(), |window, cx| {
                let view = Example::new(window, cx);
                cx.new(|cx| Root::new(view, window, cx))
            })?;

            Ok::<_, anyhow::Error>(())
        })
        .detach();
    });
}
