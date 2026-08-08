use anyhow::Context as _;
use calloop::{
    EventLoop, PostAction,
    channel::Event,
    timer::{TimeoutAction, Timer},
};
use gpui_util::ResultExt;
use parking_lot::{Condvar, Mutex};

use std::{
    mem::MaybeUninit,
    sync::{Arc, mpsc},
    thread,
    time::Duration,
};

use gpui::{
    PlatformDispatcher, Priority, PriorityQueueReceiver, PriorityQueueSender, RunnableVariant,
    profiler,
};

const MAX_CHANNEL_EVENTS: usize = 1024;

pub(crate) struct CalloopSender<T> {
    sender: mpsc::Sender<T>,
    ping: calloop::ping::Ping,
}

impl<T> Clone for CalloopSender<T> {
    fn clone(&self) -> Self {
        Self {
            sender: self.sender.clone(),
            ping: self.ping.clone(),
        }
    }
}

impl<T> CalloopSender<T> {
    pub(crate) fn send(&self, item: T) -> Result<(), mpsc::SendError<T>> {
        self.sender.send(item).map(|()| self.ping.ping())
    }
}

impl<T> Drop for CalloopSender<T> {
    fn drop(&mut self) {
        self.ping.ping();
    }
}

pub(crate) struct CalloopReceiver<T> {
    receiver: mpsc::Receiver<T>,
    source: calloop::ping::PingSource,
    ping: calloop::ping::Ping,
}

// SAFETY: A receiver can only move before it is inserted into an event loop, where it becomes
// pinned to that loop's thread.
unsafe impl<T: Send> Send for CalloopReceiver<T> {}

pub(crate) fn try_calloop_channel<T>() -> anyhow::Result<(CalloopSender<T>, CalloopReceiver<T>)> {
    let (ping, source) =
        calloop::ping::make_ping().context("failed to create calloop ping source")?;
    let (sender, receiver) = mpsc::channel();

    Ok((
        CalloopSender {
            sender,
            ping: ping.clone(),
        },
        CalloopReceiver {
            receiver,
            source,
            ping,
        },
    ))
}

impl<T> calloop::EventSource for CalloopReceiver<T> {
    type Event = Event<T>;
    type Metadata = ();
    type Ret = ();
    type Error = ChannelError;

    fn process_events<F>(
        &mut self,
        readiness: calloop::Readiness,
        token: calloop::Token,
        mut callback: F,
    ) -> Result<calloop::PostAction, Self::Error>
    where
        F: FnMut(Self::Event, &mut Self::Metadata) -> Self::Ret,
    {
        let mut clear_readiness = false;
        let mut disconnected = false;

        let action = self
            .source
            .process_events(readiness, token, |(), &mut ()| {
                for _ in 0..MAX_CHANNEL_EVENTS {
                    match self.receiver.try_recv() {
                        Ok(item) => callback(Event::Msg(item), &mut ()),
                        Err(mpsc::TryRecvError::Empty) => {
                            clear_readiness = true;
                            break;
                        }
                        Err(mpsc::TryRecvError::Disconnected) => {
                            callback(Event::Closed, &mut ());
                            disconnected = true;
                            break;
                        }
                    }
                }
            })
            .map_err(ChannelError)?;

        if disconnected {
            Ok(PostAction::Remove)
        } else if clear_readiness {
            Ok(action)
        } else {
            self.ping.ping();
            Ok(PostAction::Continue)
        }
    }

    fn register(
        &mut self,
        poll: &mut calloop::Poll,
        token_factory: &mut calloop::TokenFactory,
    ) -> calloop::Result<()> {
        self.source.register(poll, token_factory)
    }

    fn reregister(
        &mut self,
        poll: &mut calloop::Poll,
        token_factory: &mut calloop::TokenFactory,
    ) -> calloop::Result<()> {
        self.source.reregister(poll, token_factory)
    }

    fn unregister(&mut self, poll: &mut calloop::Poll) -> calloop::Result<()> {
        self.source.unregister(poll)
    }
}

struct TimerAfter {
    duration: Duration,
    runnable: RunnableVariant,
}

struct TimerWorker {
    timer_sender: CalloopSender<TimerAfter>,
    shutdown_sender: CalloopSender<()>,
    thread: Option<thread::JoinHandle<()>>,
}

impl TimerWorker {
    fn new() -> anyhow::Result<Self> {
        let (timer_sender, timer_receiver) =
            try_calloop_channel().context("failed to create timer request channel")?;
        let (shutdown_sender, shutdown_receiver) =
            try_calloop_channel().context("failed to create timer shutdown channel")?;
        let (ready_sender, ready_receiver) = mpsc::sync_channel(1);

        // The event loop is thread-bound, so wait until its sources are registered before
        // reporting successful construction.
        let thread = thread::Builder::new()
            .name("Timer".to_owned())
            .spawn(
                move || match Self::initialize_event_loop(timer_receiver, shutdown_receiver) {
                    Ok(mut event_loop) => {
                        let _ = ready_sender.send(Ok(()));
                        event_loop.run(None, &mut (), |_| {}).log_err();
                    }
                    Err(error) => {
                        let _ = ready_sender.send(Err(error));
                    }
                },
            )
            .context("failed to spawn timer worker")?;

        match ready_receiver
            .recv()
            .context("timer worker exited before initialization")?
        {
            Ok(()) => Ok(Self {
                timer_sender,
                shutdown_sender,
                thread: Some(thread),
            }),
            Err(error) => {
                let _ = thread.join();
                Err(error)
            }
        }
    }

    fn initialize_event_loop(
        timer_receiver: CalloopReceiver<TimerAfter>,
        shutdown_receiver: CalloopReceiver<()>,
    ) -> anyhow::Result<EventLoop<'static, ()>> {
        let event_loop = EventLoop::try_new().context("failed to create timer event loop")?;
        let handle = event_loop.handle();
        let timer_handle = handle.clone();
        let timer_signal = event_loop.get_signal();

        handle
            .insert_source(timer_receiver, move |event, _, _| match event {
                Event::Msg(timer) => {
                    let mut runnable = Some(timer.runnable);
                    if let Err(error) = timer_handle.insert_source(
                        Timer::from_duration(timer.duration),
                        move |_, _, _| {
                            if let Some(runnable) = runnable.take() {
                                let location = runnable.metadata().location;
                                let spawned = runnable.metadata().spawned;
                                profiler::update_running_task(spawned, location);
                                runnable.run();
                                profiler::save_task_timing();
                            }
                            TimeoutAction::Drop
                        },
                    ) {
                        let error = calloop::Error::from(error);
                        log::error!("failed to register delayed task timer: {error}");
                    }
                }
                Event::Closed => timer_signal.stop(),
            })
            .map_err(calloop::Error::from)
            .context("failed to register timer request source")?;

        let shutdown_signal = event_loop.get_signal();
        handle
            .insert_source(shutdown_receiver, move |_, _, _| shutdown_signal.stop())
            .map_err(calloop::Error::from)
            .context("failed to register timer shutdown source")?;

        Ok(event_loop)
    }

    fn send(&self, timer: TimerAfter) -> Result<(), mpsc::SendError<TimerAfter>> {
        self.timer_sender.send(timer)
    }

    fn stop(&self) {
        self.shutdown_sender.send(()).ok();
    }

    fn join(&mut self) {
        self.stop();
        let Some(handle) = self.thread.take() else {
            return;
        };
        if handle.thread().id() == thread::current().id() {
            return;
        }
        if handle.join().is_err() {
            log::error!("timer worker panicked during shutdown");
        }
    }
}

impl Drop for TimerWorker {
    fn drop(&mut self) {
        self.join();
    }
}

struct WorkerStartGate {
    state: Mutex<WorkerStartState>,
    changed: Condvar,
}

struct WorkerStartState {
    released: bool,
    cancelled: bool,
}

impl WorkerStartGate {
    fn new() -> Self {
        Self {
            state: Mutex::new(WorkerStartState {
                released: false,
                cancelled: false,
            }),
            changed: Condvar::new(),
        }
    }

    fn wait(&self) -> bool {
        let mut state = self.state.lock();
        while !state.released && !state.cancelled {
            self.changed.wait(&mut state);
        }
        state.released
    }

    fn release(&self) {
        let mut state = self.state.lock();
        state.released = true;
        self.changed.notify_all();
    }

    fn cancel(&self) {
        let mut state = self.state.lock();
        state.cancelled = true;
        self.changed.notify_all();
    }
}

pub(crate) struct LinuxDispatcher {
    main_sender: PriorityQueueCalloopSender<RunnableVariant>,
    timer_worker: TimerWorker,
    background_sender: PriorityQueueSender<RunnableVariant>,
    _background_threads: Vec<thread::JoinHandle<()>>,
    main_thread_id: thread::ThreadId,
}

const MIN_THREADS: usize = 2;

impl LinuxDispatcher {
    pub fn new(main_sender: PriorityQueueCalloopSender<RunnableVariant>) -> anyhow::Result<Self> {
        let timer_worker = TimerWorker::new()?;
        let (background_sender, background_receiver) = PriorityQueueReceiver::new();
        let thread_count =
            std::thread::available_parallelism().map_or(MIN_THREADS, |i| i.get().max(MIN_THREADS));
        let start_gate = Arc::new(WorkerStartGate::new());
        let mut background_threads = Vec::with_capacity(thread_count);

        for i in 0..thread_count {
            let receiver: PriorityQueueReceiver<RunnableVariant> = background_receiver.clone();
            let worker_start_gate = Arc::clone(&start_gate);
            let thread = thread::Builder::new()
                .name(format!("Worker-{i}"))
                .spawn(move || {
                    if !worker_start_gate.wait() {
                        return;
                    }
                    for runnable in receiver.iter() {
                        let location = runnable.metadata().location;
                        let spawned = runnable.metadata().spawned;
                        profiler::update_running_task(spawned, location);
                        runnable.run();
                        profiler::save_task_timing();
                    }
                });

            match thread {
                Ok(thread) => background_threads.push(thread),
                Err(error) => {
                    start_gate.cancel();
                    for thread in background_threads {
                        if thread.join().is_err() {
                            log::error!("background worker panicked during startup cleanup");
                        }
                    }
                    return Err(error)
                        .context(format!("failed to spawn Linux background worker {i}"));
                }
            }
        }

        start_gate.release();

        Ok(Self {
            main_sender,
            timer_worker,
            background_sender,
            _background_threads: background_threads,
            main_thread_id: thread::current().id(),
        })
    }

    pub(crate) fn stop_timer(&self) {
        self.timer_worker.stop();
    }
}

impl Drop for LinuxDispatcher {
    fn drop(&mut self) {
        self.stop_timer();
    }
}

impl PlatformDispatcher for LinuxDispatcher {
    fn is_main_thread(&self) -> bool {
        thread::current().id() == self.main_thread_id
    }

    fn dispatch(&self, runnable: RunnableVariant, priority: Priority) {
        self.background_sender
            .send(priority, runnable)
            .unwrap_or_else(|_| panic!("blocking sender returned without value"));
    }

    fn dispatch_on_main_thread(&self, runnable: RunnableVariant, priority: Priority) {
        self.main_sender
            .send(priority, runnable)
            .unwrap_or_else(|runnable| {
                // NOTE: Runnable may wrap a Future that is !Send.
                //
                // This is usually safe because we only poll it on the main thread.
                // However if the send fails, we know that:
                // 1. main_receiver has been dropped (which implies the app is shutting down)
                // 2. we are on a background thread.
                // It is not safe to drop something !Send on the wrong thread, and
                // the app will exit soon anyway, so we must forget the runnable.
                std::mem::forget(runnable);
            });
    }

    fn dispatch_after(&self, duration: Duration, runnable: RunnableVariant) {
        if let Err(err) = self.timer_worker.send(TimerAfter { duration, runnable }) {
            std::mem::forget(err);
        }
    }

    fn spawn_realtime(&self, f: Box<dyn FnOnce() + Send>) {
        std::thread::spawn(move || {
            // SAFETY: always safe to call
            let thread_id = unsafe { libc::pthread_self() };

            let policy = libc::SCHED_FIFO;
            let sched_priority = 65;

            // SAFETY: all sched_param members are valid when initialized to zero.
            let mut sched_param =
                unsafe { MaybeUninit::<libc::sched_param>::zeroed().assume_init() };
            sched_param.sched_priority = sched_priority;
            // SAFETY: sched_param is a valid initialized structure
            let result = unsafe { libc::pthread_setschedparam(thread_id, policy, &sched_param) };
            if result != 0 {
                log::warn!("failed to set realtime thread priority");
            }

            f();
        });
    }
}

pub struct PriorityQueueCalloopSender<T> {
    sender: PriorityQueueSender<T>,
    ping: calloop::ping::Ping,
}

impl<T> PriorityQueueCalloopSender<T> {
    fn new(tx: PriorityQueueSender<T>, ping: calloop::ping::Ping) -> Self {
        Self { sender: tx, ping }
    }

    fn send(&self, priority: Priority, item: T) -> Result<(), gpui::queue::SendError<T>> {
        let res = self.sender.send(priority, item);
        if res.is_ok() {
            self.ping.ping();
        }
        res
    }
}

impl<T> Drop for PriorityQueueCalloopSender<T> {
    fn drop(&mut self) {
        self.ping.ping();
    }
}

pub struct PriorityQueueCalloopReceiver<T> {
    receiver: PriorityQueueReceiver<T>,
    source: calloop::ping::PingSource,
    ping: calloop::ping::Ping,
}

impl<T> PriorityQueueCalloopReceiver<T> {
    pub fn new() -> anyhow::Result<(PriorityQueueCalloopSender<T>, Self)> {
        let (ping, source) =
            calloop::ping::make_ping().context("failed to create calloop ping source")?;
        let (tx, rx) = PriorityQueueReceiver::new();

        Ok((
            PriorityQueueCalloopSender::new(tx, ping.clone()),
            Self {
                receiver: rx,
                source,
                ping,
            },
        ))
    }
}

#[derive(Debug)]
pub struct ChannelError(calloop::ping::PingError);

impl std::fmt::Display for ChannelError {
    #[cfg_attr(feature = "nightly_coverage", coverage(off))]
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.0, f)
    }
}

impl std::error::Error for ChannelError {
    #[cfg_attr(feature = "nightly_coverage", coverage(off))]
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.0)
    }
}

impl<T> calloop::EventSource for PriorityQueueCalloopReceiver<T> {
    type Event = Event<T>;
    type Metadata = ();
    type Ret = ();
    type Error = ChannelError;

    fn process_events<F>(
        &mut self,
        readiness: calloop::Readiness,
        token: calloop::Token,
        mut callback: F,
    ) -> Result<calloop::PostAction, Self::Error>
    where
        F: FnMut(Self::Event, &mut Self::Metadata) -> Self::Ret,
    {
        let mut clear_readiness = false;
        let mut disconnected = false;

        let action = self
            .source
            .process_events(readiness, token, |(), &mut ()| {
                let mut is_empty = true;

                let receiver = self.receiver.clone();
                for runnable in receiver.try_iter() {
                    match runnable {
                        Ok(r) => {
                            callback(Event::Msg(r), &mut ());
                            is_empty = false;
                        }
                        Err(_) => {
                            disconnected = true;
                        }
                    }
                }

                if disconnected {
                    callback(Event::Closed, &mut ());
                }

                if is_empty {
                    clear_readiness = true;
                }
            })
            .map_err(ChannelError)?;

        if disconnected {
            Ok(PostAction::Remove)
        } else if clear_readiness {
            Ok(action)
        } else {
            // Re-notify the ping source so we can try again.
            self.ping.ping();
            Ok(PostAction::Continue)
        }
    }

    fn register(
        &mut self,
        poll: &mut calloop::Poll,
        token_factory: &mut calloop::TokenFactory,
    ) -> calloop::Result<()> {
        self.source.register(poll, token_factory)
    }

    fn reregister(
        &mut self,
        poll: &mut calloop::Poll,
        token_factory: &mut calloop::TokenFactory,
    ) -> calloop::Result<()> {
        self.source.reregister(poll, token_factory)
    }

    fn unregister(&mut self, poll: &mut calloop::Poll) -> calloop::Result<()> {
        self.source.unregister(poll)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calloop_works() {
        let mut event_loop = calloop::EventLoop::try_new().unwrap();
        let handle = event_loop.handle();

        let (tx, rx) = PriorityQueueCalloopReceiver::new().unwrap();

        struct Data {
            got_msg: bool,
            got_closed: bool,
        }

        let mut data = Data {
            got_msg: false,
            got_closed: false,
        };

        let _channel_token = handle
            .insert_source(rx, move |evt, &mut (), data: &mut Data| match evt {
                Event::Msg(()) => {
                    data.got_msg = true;
                }

                Event::Closed => {
                    data.got_closed = true;
                }
            })
            .unwrap();

        // nothing is sent, nothing is received
        event_loop
            .dispatch(Some(::std::time::Duration::ZERO), &mut data)
            .unwrap();

        assert!(!data.got_msg);
        assert!(!data.got_closed);
        // a message is send

        tx.send(Priority::Medium, ()).unwrap();
        event_loop
            .dispatch(Some(::std::time::Duration::ZERO), &mut data)
            .unwrap();

        assert!(data.got_msg);
        assert!(!data.got_closed);

        // the sender is dropped
        drop(tx);
        event_loop
            .dispatch(Some(::std::time::Duration::ZERO), &mut data)
            .unwrap();

        assert!(data.got_msg);
        assert!(data.got_closed);
    }

    #[test]
    fn fallible_calloop_channel_delivers_messages() {
        let mut event_loop = EventLoop::try_new().unwrap();
        let (sender, receiver) = try_calloop_channel().unwrap();
        let _token = event_loop
            .handle()
            .insert_source(receiver, |event, _, received: &mut bool| {
                if matches!(event, Event::Msg(())) {
                    *received = true;
                }
            })
            .unwrap();

        sender.send(()).unwrap();
        let mut received = false;
        event_loop
            .dispatch(Some(std::time::Duration::ZERO), &mut received)
            .unwrap();

        assert!(received);
    }

    #[test]
    fn timer_worker_stops_after_shutdown() {
        let mut worker = TimerWorker::new().unwrap();
        worker.stop();
        worker.join();
    }
}

// running 1 test
// test linux::dispatcher::tests::tomato ... FAILED

// failures:

// ---- linux::dispatcher::tests::tomato stdout ----
// [crates/gpui/src/platform/linux/dispatcher.rs:262:9]
// returning 1 tasks to process
// [crates/gpui/src/platform/linux/dispatcher.rs:480:75] evt = Msg(
//     (),
// )
// returning 0 tasks to process

// thread 'linux::dispatcher::tests::tomato' (478301) panicked at crates/gpui/src/platform/linux/dispatcher.rs:515:9:
// assertion failed: data.got_closed
// note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
