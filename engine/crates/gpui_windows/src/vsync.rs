use std::{
    os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use gpui_util::ResultExt;
use windows::{
    Win32::{
        Foundation::{
            FARPROC, FreeLibrary, HANDLE, HMODULE, HWND, WAIT_FAILED, WAIT_OBJECT_0, WAIT_TIMEOUT,
        },
        Graphics::Dwm::{DWM_TIMING_INFO, DwmGetCompositionTimingInfo},
        System::{
            LibraryLoader::{GetProcAddress, LoadLibraryW},
            Performance::QueryPerformanceFrequency,
            Threading::{CreateEventW, SetEvent, WaitForSingleObject},
        },
    },
    core::{s, w},
};

static QPC_TICKS_PER_SECOND: std::sync::LazyLock<u64> = std::sync::LazyLock::new(|| {
    let mut frequency = 0;
    // On systems that run Windows XP or later, the function will always succeed and
    // will thus never return zero.
    unsafe { QueryPerformanceFrequency(&mut frequency).unwrap() };
    frequency as u64
});

const VSYNC_INTERVAL_THRESHOLD: Duration = Duration::from_millis(1);
const DEFAULT_VSYNC_INTERVAL: Duration = Duration::from_micros(16_666); // ~60Hz
const MAX_VSYNC_INTERVAL: Duration = Duration::from_millis(100);
const COMPOSITOR_CLOCK_TIMEOUT_MS: u32 = 100;

type WaitForCompositorClock =
    unsafe extern "system" fn(count: u32, handles: *const HANDLE, timeout_ms: u32) -> u32;

struct CompositorClock {
    library: HMODULE,
    wait: WaitForCompositorClock,
}

impl CompositorClock {
    fn load() -> Option<Self> {
        // `DCompositionWaitForCompositorClock` is available on Windows 11 and later. Resolve it
        // dynamically so Windows 10 keeps the bounded timer fallback instead of failing to load.
        let library = unsafe { LoadLibraryW(w!("dcomp.dll")).ok()? };
        let procedure: FARPROC =
            unsafe { GetProcAddress(library, s!("DCompositionWaitForCompositorClock")) };
        let Some(procedure) = procedure else {
            unsafe { FreeLibrary(library).ok() };
            return None;
        };
        // SAFETY: the symbol has the documented `DCompositionWaitForCompositorClock` ABI and the
        // loaded module remains owned by `CompositorClock` for the function pointer's lifetime.
        let wait = unsafe {
            std::mem::transmute::<unsafe extern "system" fn() -> isize, WaitForCompositorClock>(
                procedure,
            )
        };
        Some(Self { library, wait })
    }
}

impl Drop for CompositorClock {
    fn drop(&mut self) {
        unsafe { FreeLibrary(self.library).ok() };
    }
}

pub(crate) struct VSyncCancellation {
    cancelled: AtomicBool,
    wake_event: OwnedHandle,
}

impl VSyncCancellation {
    pub(crate) fn new() -> Result<Self> {
        let wake_event = unsafe {
            CreateEventW(
                /*lpeventattributes*/ None, /*bmanualreset*/ true,
                /*binitialstate*/ false, /*lpname*/ None,
            )
        }
        .context("creating VSync cancellation event")?;
        // SAFETY: `CreateEventW` returned a fresh owned handle. `OwnedHandle` closes it once.
        let wake_event = unsafe { OwnedHandle::from_raw_handle(wake_event.0) };
        Ok(Self {
            cancelled: AtomicBool::new(false),
            wake_event,
        })
    }

    pub(crate) fn cancel(&self) {
        if !self.cancelled.swap(true, Ordering::AcqRel)
            && let Err(error) = unsafe { SetEvent(self.event()) }
        {
            log::error!("failed to signal VSync cancellation event: {error}");
        }
    }

    pub(crate) fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }

    pub(crate) fn wait_for(&self, timeout: Duration) -> VSyncWait {
        let timeout_ms = u32::try_from(timeout.as_millis().max(1)).unwrap_or(u32::MAX - 1);
        let wait_result = unsafe { WaitForSingleObject(self.event(), timeout_ms) };
        if wait_result == WAIT_OBJECT_0 {
            VSyncWait::Cancelled
        } else {
            if wait_result != WAIT_TIMEOUT {
                let detail = if wait_result == WAIT_FAILED {
                    windows::core::Error::from_win32().to_string()
                } else {
                    format!("unexpected result {wait_result:?}")
                };
                log::error!("VSync cancellation wait failed: {detail}");
                std::thread::sleep(timeout.min(MAX_VSYNC_INTERVAL));
            }
            if self.is_cancelled() {
                VSyncWait::Cancelled
            } else {
                VSyncWait::Tick
            }
        }
    }

    fn event(&self) -> HANDLE {
        HANDLE(self.wake_event.as_raw_handle())
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub(crate) enum VSyncWait {
    Tick,
    Cancelled,
}

pub(crate) struct VSyncProvider {
    interval: Duration,
    compositor_clock: Option<CompositorClock>,
}

impl VSyncProvider {
    pub(crate) fn new() -> Self {
        let interval = get_dwm_interval()
            .context("Failed to get DWM interval")
            .log_err()
            .unwrap_or(DEFAULT_VSYNC_INTERVAL)
            .clamp(VSYNC_INTERVAL_THRESHOLD, MAX_VSYNC_INTERVAL);
        Self {
            interval,
            compositor_clock: CompositorClock::load(),
        }
    }

    pub(crate) fn wait_for_vsync(&mut self, cancellation: &VSyncCancellation) -> VSyncWait {
        if cancellation.is_cancelled() {
            return VSyncWait::Cancelled;
        }

        if let Some(clock) = &self.compositor_clock {
            let start = Instant::now();
            let wait_result = unsafe {
                (clock.wait)(
                    /*count*/ 0,
                    /*handles*/ std::ptr::null(),
                    COMPOSITOR_CLOCK_TIMEOUT_MS,
                )
            };
            if cancellation.is_cancelled() {
                return VSyncWait::Cancelled;
            }
            if wait_result == 0 && start.elapsed() >= VSYNC_INTERVAL_THRESHOLD {
                return VSyncWait::Tick;
            }
            if wait_result != 0 && wait_result != WAIT_TIMEOUT.0 {
                log::error!("compositor clock wait failed with result {wait_result:#x}");
                self.compositor_clock = None;
            }
        }

        self.wait_for_interval(cancellation)
    }

    fn wait_for_interval(&self, cancellation: &VSyncCancellation) -> VSyncWait {
        cancellation.wait_for(self.interval)
    }
}

fn get_dwm_interval() -> Result<Duration> {
    let mut timing_info = DWM_TIMING_INFO {
        cbSize: std::mem::size_of::<DWM_TIMING_INFO>() as u32,
        ..Default::default()
    };
    unsafe { DwmGetCompositionTimingInfo(HWND::default(), &mut timing_info) }?;
    let interval = retrieve_duration(timing_info.qpcRefreshPeriod, *QPC_TICKS_PER_SECOND);
    // Check for interval values that are impossibly low. A 29 microsecond
    // interval was seen (from a qpcRefreshPeriod of 60).
    if interval < VSYNC_INTERVAL_THRESHOLD {
        Ok(retrieve_duration(
            timing_info.rateRefresh.uiDenominator as u64,
            timing_info.rateRefresh.uiNumerator as u64,
        ))
    } else {
        Ok(interval)
    }
}

#[inline]
fn retrieve_duration(counts: u64, ticks_per_second: u64) -> Duration {
    let ticks_per_microsecond = ticks_per_second / 1_000_000;
    Duration::from_micros(counts / ticks_per_microsecond)
}
