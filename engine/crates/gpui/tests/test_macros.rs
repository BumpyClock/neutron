#![cfg(feature = "test-support")]

use gpui::{AppContext as _, TestAppContext};

#[gpui::test]
fn synchronous_wrapper_provides_app_context(cx: &mut TestAppContext) {
    let entity = cx.new(|_| 42);
    assert_eq!(entity.read_with(cx, |value, _| *value), 42);
}

#[gpui::test]
async fn asynchronous_wrapper_drives_executor(cx: &mut TestAppContext) {
    let task = cx.executor().spawn(async { 42 });
    assert_eq!(task.await, 42);
}
