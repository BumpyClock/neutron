#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum VSyncShutdownAction {
    None,
    CancelWorker,
    CancelWorkerAndPostQuit,
    WaitForWorker,
    PostQuit,
    Exit,
}

#[derive(Clone, Copy, Default)]
enum VSyncShutdownState {
    #[default]
    NotStarted,
    Running,
    StopRequested,
    DirectShutdown,
    AwaitingAcknowledgement(usize),
    Stopped,
    QuitPosted,
    Exited,
}

#[derive(Default)]
pub(crate) struct VSyncShutdown {
    state: VSyncShutdownState,
    next_acknowledgement: usize,
}

impl VSyncShutdown {
    pub(crate) fn worker_started(&mut self) {
        debug_assert!(matches!(self.state, VSyncShutdownState::NotStarted));
        self.state = VSyncShutdownState::Running;
    }

    /// Advances an ordinary quit request and returns the one action the caller must perform.
    pub(crate) fn request_quit(&mut self) -> VSyncShutdownAction {
        match self.state {
            VSyncShutdownState::NotStarted | VSyncShutdownState::Stopped => {
                self.state = VSyncShutdownState::QuitPosted;
                VSyncShutdownAction::PostQuit
            }
            VSyncShutdownState::Running => {
                self.state = VSyncShutdownState::StopRequested;
                VSyncShutdownAction::CancelWorkerAndPostQuit
            }
            VSyncShutdownState::StopRequested
            | VSyncShutdownState::DirectShutdown
            | VSyncShutdownState::AwaitingAcknowledgement(_)
            | VSyncShutdownState::QuitPosted
            | VSyncShutdownState::Exited => VSyncShutdownAction::None,
        }
    }

    /// Converts a message-loop error into a terminal path that never needs another queued message.
    pub(crate) fn request_terminal_shutdown(&mut self) -> VSyncShutdownAction {
        match self.state {
            VSyncShutdownState::NotStarted
            | VSyncShutdownState::Stopped
            | VSyncShutdownState::QuitPosted => {
                self.state = VSyncShutdownState::Exited;
                VSyncShutdownAction::Exit
            }
            VSyncShutdownState::Running => {
                self.state = VSyncShutdownState::DirectShutdown;
                VSyncShutdownAction::CancelWorker
            }
            VSyncShutdownState::StopRequested | VSyncShutdownState::AwaitingAcknowledgement(_) => {
                self.state = VSyncShutdownState::DirectShutdown;
                VSyncShutdownAction::WaitForWorker
            }
            VSyncShutdownState::DirectShutdown | VSyncShutdownState::Exited => {
                VSyncShutdownAction::None
            }
        }
    }

    /// Returns the acknowledgement that the worker must send to the UI thread.
    pub(crate) fn worker_stopped(&mut self) -> Option<usize> {
        match self.state {
            VSyncShutdownState::Running => {
                self.state = VSyncShutdownState::Stopped;
                None
            }
            VSyncShutdownState::StopRequested => {
                let acknowledgement = self.next_acknowledgement();
                self.state = VSyncShutdownState::AwaitingAcknowledgement(acknowledgement);
                Some(acknowledgement)
            }
            VSyncShutdownState::NotStarted
            | VSyncShutdownState::DirectShutdown
            | VSyncShutdownState::AwaitingAcknowledgement(_)
            | VSyncShutdownState::Stopped
            | VSyncShutdownState::QuitPosted
            | VSyncShutdownState::Exited => None,
        }
    }

    /// Accepts only the ordinary acknowledgement that precedes the queued `WM_QUIT`.
    pub(crate) fn acknowledge_worker_stop(
        &mut self,
        acknowledgement: usize,
    ) -> VSyncShutdownAction {
        if matches!(self.state, VSyncShutdownState::AwaitingAcknowledgement(expected) if expected == acknowledgement)
        {
            self.state = VSyncShutdownState::Stopped;
            VSyncShutdownAction::None
        } else {
            VSyncShutdownAction::None
        }
    }

    /// Completes a direct quit path after the UI thread has joined the worker.
    pub(crate) fn complete_direct_shutdown(&mut self) -> VSyncShutdownAction {
        self.finish_direct_shutdown()
    }

    /// Completes a direct quit path after its deadline while detached worker state stays owned.
    pub(crate) fn abandon_direct_shutdown(&mut self) -> VSyncShutdownAction {
        self.finish_direct_shutdown()
    }

    /// Advances a retrieved `WM_QUIT`, consuming the terminal exit exactly once.
    pub(crate) fn should_exit_after_quit_message(&mut self) -> VSyncShutdownAction {
        match self.state {
            VSyncShutdownState::NotStarted
            | VSyncShutdownState::Stopped
            | VSyncShutdownState::QuitPosted => {
                self.state = VSyncShutdownState::Exited;
                VSyncShutdownAction::Exit
            }
            VSyncShutdownState::Running => {
                self.state = VSyncShutdownState::DirectShutdown;
                VSyncShutdownAction::CancelWorker
            }
            VSyncShutdownState::StopRequested | VSyncShutdownState::AwaitingAcknowledgement(_) => {
                self.state = VSyncShutdownState::DirectShutdown;
                VSyncShutdownAction::WaitForWorker
            }
            VSyncShutdownState::DirectShutdown | VSyncShutdownState::Exited => {
                VSyncShutdownAction::None
            }
        }
    }

    fn finish_direct_shutdown(&mut self) -> VSyncShutdownAction {
        if matches!(self.state, VSyncShutdownState::DirectShutdown) {
            self.state = VSyncShutdownState::Exited;
            VSyncShutdownAction::Exit
        } else {
            VSyncShutdownAction::None
        }
    }

    fn next_acknowledgement(&mut self) -> usize {
        self.next_acknowledgement = self.next_acknowledgement.wrapping_add(1);
        if self.next_acknowledgement == 0 {
            self.next_acknowledgement = 1;
        }
        self.next_acknowledgement
    }
}
