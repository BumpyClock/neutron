pub(crate) struct TrivialActivationHandler {
    pub(crate) callback: Box<dyn Fn() -> Option<accesskit::TreeUpdate> + Send + 'static>,
}

impl accesskit::ActivationHandler for TrivialActivationHandler {
    fn request_initial_tree(&mut self) -> Option<accesskit::TreeUpdate> {
        (self.callback)()
    }
}

pub(crate) struct TrivialActionHandler {
    pub(crate) callback: Box<dyn Fn(accesskit::ActionRequest) + Send + 'static>,
}

impl accesskit::ActionHandler for TrivialActionHandler {
    fn do_action(&mut self, request: accesskit::ActionRequest) {
        (self.callback)(request);
    }
}

pub(crate) struct TrivialDeactivationHandler {
    pub(crate) callback: Box<dyn Fn() + Send + 'static>,
}

impl accesskit::DeactivationHandler for TrivialDeactivationHandler {
    fn deactivate_accessibility(&mut self) {
        (self.callback)();
    }
}
