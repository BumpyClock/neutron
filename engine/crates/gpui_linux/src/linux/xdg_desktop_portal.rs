//! Provides a [calloop] event source from [XDG Desktop Portal] events
//!
//! This module uses the [ashpd] crate

use std::{future::Future, mem, sync::Arc};

use anyhow::Context as _;
use ashpd::desktop::settings::{ColorScheme, Settings};
use calloop::channel::Event as CalloopEvent;
use calloop::{EventSource, Poll, PostAction, Readiness, Token, TokenFactory};
use futures::future::{AbortHandle, AbortRegistration, Abortable};
use parking_lot::Mutex;
use smol::stream::StreamExt;

use crate::linux::{CalloopReceiver, CalloopSender, try_calloop_channel};
use gpui::{BackgroundExecutor, WindowAppearance};

pub enum Event {
    WindowAppearance(WindowAppearance),
    #[cfg_attr(feature = "x11", allow(dead_code))]
    CursorTheme(String),
    #[cfg_attr(feature = "x11", allow(dead_code))]
    CursorSize(u32),
}

#[derive(Clone, Default)]
struct XDPTaskRegistry {
    state: Arc<Mutex<XDPTaskRegistryState>>,
}

#[derive(Default)]
struct XDPTaskRegistryState {
    cancelled: bool,
    abort_handles: Vec<AbortHandle>,
}

impl XDPTaskRegistry {
    fn register(&self) -> Option<AbortRegistration> {
        let (abort_handle, registration) = AbortHandle::new_pair();
        let mut state = self.state.lock();
        if state.cancelled {
            return None;
        }
        state.abort_handles.push(abort_handle);
        Some(registration)
    }

    fn spawn<F>(&self, executor: &BackgroundExecutor, task: F)
    where
        F: Future<Output = anyhow::Result<()>> + Send + 'static,
    {
        let Some(registration) = self.register() else {
            return;
        };

        executor
            .spawn(async move {
                if let Ok(Err(error)) = Abortable::new(task, registration).await {
                    log::debug!("XDG desktop portal task failed: {error:#}");
                }
            })
            .detach();
    }

    fn cancel(&self) {
        let abort_handles = {
            let mut state = self.state.lock();
            if state.cancelled {
                return;
            }
            state.cancelled = true;
            mem::take(&mut state.abort_handles)
        };

        for abort_handle in abort_handles {
            abort_handle.abort();
        }
    }
}

pub struct XDPEventSource {
    channel: CalloopReceiver<Event>,
    tasks: XDPTaskRegistry,
}

pub(crate) struct XDPEventSourceStarter {
    sender: CalloopSender<Event>,
    tasks: XDPTaskRegistry,
}

impl XDPEventSource {
    pub(crate) fn new() -> anyhow::Result<(Self, XDPEventSourceStarter)> {
        let (sender, channel) =
            try_calloop_channel().context("failed to create XDG desktop portal event channel")?;

        let tasks = XDPTaskRegistry::default();
        Ok((
            Self {
                channel,
                tasks: tasks.clone(),
            },
            XDPEventSourceStarter { sender, tasks },
        ))
    }
}

impl XDPEventSourceStarter {
    pub(crate) fn start(self, executor: &BackgroundExecutor) {
        let background = executor.clone();
        let sender = self.sender;
        let tasks = self.tasks;
        let subscription_tasks = tasks.clone();

        tasks.spawn(executor, async move {
            let settings = Settings::new().await?;

            if let Ok(initial_appearance) = settings.color_scheme().await {
                sender.send(Event::WindowAppearance(
                    window_appearance_from_color_scheme(initial_appearance),
                ))?;
            }
            if let Ok(initial_theme) = settings
                .read::<String>("org.gnome.desktop.interface", "cursor-theme")
                .await
            {
                sender.send(Event::CursorTheme(initial_theme))?;
            }

            // If u32 is used here, it throws invalid type error
            if let Ok(initial_size) = settings
                .read::<i32>("org.gnome.desktop.interface", "cursor-size")
                .await
            {
                sender.send(Event::CursorSize(initial_size as u32))?;
            }

            if let Ok(mut cursor_theme_changed) = settings
                .receive_setting_changed_with_args("org.gnome.desktop.interface", "cursor-theme")
                .await
            {
                let sender = sender.clone();
                subscription_tasks.spawn(&background, async move {
                    while let Some(theme) = cursor_theme_changed.next().await {
                        let theme = theme?;
                        sender.send(Event::CursorTheme(theme))?;
                    }
                    anyhow::Ok(())
                });
            }

            if let Ok(mut cursor_size_changed) = settings
                .receive_setting_changed_with_args::<i32>(
                    "org.gnome.desktop.interface",
                    "cursor-size",
                )
                .await
            {
                let sender = sender.clone();
                subscription_tasks.spawn(&background, async move {
                    while let Some(size) = cursor_size_changed.next().await {
                        let size = size?;
                        sender.send(Event::CursorSize(size as u32))?;
                    }
                    anyhow::Ok(())
                });
            }

            let mut appearance_changed = settings.receive_color_scheme_changed().await?;
            while let Some(scheme) = appearance_changed.next().await {
                sender.send(Event::WindowAppearance(
                    window_appearance_from_color_scheme(scheme),
                ))?;
            }

            anyhow::Ok(())
        });
    }
}

impl Drop for XDPEventSource {
    fn drop(&mut self) {
        self.tasks.cancel();
    }
}

impl EventSource for XDPEventSource {
    type Event = Event;
    type Metadata = ();
    type Ret = ();
    type Error = anyhow::Error;

    fn process_events<F>(
        &mut self,
        readiness: Readiness,
        token: Token,
        mut callback: F,
    ) -> Result<PostAction, Self::Error>
    where
        F: FnMut(Self::Event, &mut Self::Metadata) -> Self::Ret,
    {
        self.channel.process_events(readiness, token, |evt, _| {
            if let CalloopEvent::Msg(msg) = evt {
                (callback)(msg, &mut ())
            }
        })?;

        Ok(PostAction::Continue)
    }

    fn register(
        &mut self,
        poll: &mut Poll,
        token_factory: &mut TokenFactory,
    ) -> calloop::Result<()> {
        self.channel.register(poll, token_factory)?;

        Ok(())
    }

    fn reregister(
        &mut self,
        poll: &mut Poll,
        token_factory: &mut TokenFactory,
    ) -> calloop::Result<()> {
        self.channel.reregister(poll, token_factory)?;

        Ok(())
    }

    fn unregister(&mut self, poll: &mut Poll) -> calloop::Result<()> {
        self.channel.unregister(poll)?;

        Ok(())
    }
}

fn window_appearance_from_color_scheme(cs: ColorScheme) -> WindowAppearance {
    match cs {
        ColorScheme::PreferDark => WindowAppearance::Dark,
        ColorScheme::PreferLight => WindowAppearance::Light,
        ColorScheme::NoPreference => WindowAppearance::Light,
    }
}

#[cfg(test)]
mod tests {
    use std::{sync::mpsc, time::Duration};

    use super::*;
    use crate::linux::{HeadlessClient, LinuxClient};

    struct DropSignal(mpsc::Sender<()>);

    impl Drop for DropSignal {
        fn drop(&mut self) {
            self.0.send(()).unwrap();
        }
    }

    #[test]
    fn dropping_event_source_aborts_running_outer_and_subscription_tasks() {
        let client = HeadlessClient::new().unwrap();
        let executor = client.with_common(|common| common.background_executor.clone());
        let (source, _starter) = XDPEventSource::new().unwrap();
        let (task_started_tx, task_started_rx) = mpsc::channel();
        let (task_dropped_tx, task_dropped_rx) = mpsc::channel();

        for _ in 0..3 {
            let task_started_tx = task_started_tx.clone();
            let task_dropped_tx = task_dropped_tx.clone();
            source.tasks.spawn(&executor, async move {
                let _drop_signal = DropSignal(task_dropped_tx);
                task_started_tx.send(()).unwrap();
                std::future::pending::<()>().await;
                anyhow::Ok(())
            });
        }
        drop(task_started_tx);
        drop(task_dropped_tx);

        for _ in 0..3 {
            task_started_rx
                .recv_timeout(Duration::from_secs(1))
                .unwrap();
        }
        drop(source);

        for _ in 0..3 {
            task_dropped_rx
                .recv_timeout(Duration::from_secs(1))
                .unwrap();
        }
    }

    #[test]
    fn source_drop_aborts_or_rejects_concurrent_task_registration() {
        let (source, _starter) = XDPEventSource::new().unwrap();
        let tasks = source.tasks.clone();
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let register_barrier = barrier.clone();
        let registration = std::thread::spawn(move || {
            register_barrier.wait();
            tasks.register()
        });

        barrier.wait();
        drop(source);

        if let Some(registration) = registration.join().unwrap() {
            let result = smol::block_on(Abortable::new(std::future::pending::<()>(), registration));
            assert!(result.is_err());
        }
    }
}
