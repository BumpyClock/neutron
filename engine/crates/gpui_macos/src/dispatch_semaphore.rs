//! A thin wrapper around `dispatch_semaphore_t` for in-flight frame throttling.
//!
//! Kept in its own file so cbindgen (which scans `metal_renderer.rs` to
//! generate the Metal shader header `scene.h`) does not emit the libdispatch
//! FFI declarations into the shader header.

use std::{ffi::c_void, sync::Arc};

#[link(name = "System", kind = "dylib")]
unsafe extern "C" {
    fn dispatch_semaphore_create(value: i64) -> *mut c_void;
    fn dispatch_semaphore_wait(dsema: *mut c_void, timeout: u64) -> i64;
    fn dispatch_semaphore_signal(dsema: *mut c_void) -> i64;
    fn dispatch_release(object: *mut c_void);
}

/// `DISPATCH_TIME_FOREVER`
const DISPATCH_TIME_FOREVER: u64 = !0;

/// A thread-safe wrapper around `dispatch_semaphore_t`.
///
/// Used to limit the number of frames the CPU can submit ahead of the GPU,
/// preventing the Metal driver from pinning GPU memory for many queued
/// command buffers. This follows Apple's recommended in-flight frame
/// throttling pattern.
///
/// See: <https://developer.apple.com/library/archive/documentation/3DDrawing/Conceptual/MTLBestPracticesGuide/TripleBuffering.html>
#[derive(Clone)]
pub(crate) struct FrameSemaphore {
    inner: Arc<DispatchSemaphore>,
}

struct DispatchSemaphore {
    inner: *mut c_void,
}

// dispatch_semaphore_t is thread-safe and reference-counted by libdispatch.
unsafe impl Send for DispatchSemaphore {}
unsafe impl Sync for DispatchSemaphore {}

impl FrameSemaphore {
    pub(crate) fn new(count: i64) -> Self {
        let inner = unsafe { dispatch_semaphore_create(count) };
        assert!(!inner.is_null(), "dispatch_semaphore_create failed");
        Self {
            inner: Arc::new(DispatchSemaphore { inner }),
        }
    }

    /// Block until a slot is available.
    pub(crate) fn wait(&self) {
        unsafe {
            dispatch_semaphore_wait(self.inner.inner, DISPATCH_TIME_FOREVER);
        }
    }

    /// Signal that a frame has completed.
    pub(crate) fn signal(&self) {
        unsafe {
            dispatch_semaphore_signal(self.inner.inner);
        }
    }
}

impl Drop for DispatchSemaphore {
    fn drop(&mut self) {
        unsafe {
            dispatch_release(self.inner);
        }
    }
}
