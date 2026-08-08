use gpui::PlatformKeyboardLayout;

pub struct WebKeyboardLayout;

impl WebKeyboardLayout {
    pub fn new() -> Self {
        WebKeyboardLayout
    }
}

impl PlatformKeyboardLayout for WebKeyboardLayout {
    fn id(&self) -> &str {
        "unknown"
    }

    fn name(&self) -> &str {
        "Unknown"
    }
}
