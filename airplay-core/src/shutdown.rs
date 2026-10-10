//! One stop budget shared by cleanup stages; cancellation is a request, not proof of completion.
use std::{
    os::windows::io::AsRawHandle,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use windows::Win32::{Foundation::HANDLE, System::IO::CancelSynchronousIo};

pub const STOP_TIMEOUT: Duration = Duration::from_secs(15);

/// The first stop request sets the deadline. Later requests may shorten, never renew it.
#[derive(Clone, Default)]
pub struct Budget(Arc<Mutex<Option<Instant>>>);
impl Budget {
    pub fn begin(&self) -> Instant {
        self.begin_until(Instant::now() + STOP_TIMEOUT)
    }
    pub fn begin_until(&self, deadline: Instant) -> Instant {
        let mut first = self.0.lock().unwrap();
        let end = first.map_or(deadline, |end| end.min(deadline));
        *first = Some(end);
        end
    }
    pub fn current(&self) -> Option<Instant> {
        *self.0.lock().unwrap()
    }
}

/// Only join completed threads. Keep an unfinished handle so the owner can retry or report it.
/// Repeat cancellation to cover the race between checking the flag and entering synchronous I/O.
pub fn join_until<T>(
    worker: &mut Option<JoinHandle<T>>,
    deadline: Instant,
    cancel_io: bool,
) -> Result<Option<T>, String> {
    let Some(handle) = worker.as_ref() else {
        return Ok(None);
    };
    while !handle.is_finished() {
        if cancel_io {
            cancel(handle);
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err("[STOP_TIMEOUT] 等待线程退出超过统一停止期限".into());
        }
        thread::sleep(remaining.min(Duration::from_millis(5)));
    }
    worker
        .take()
        .unwrap()
        .join()
        .map(Some)
        .map_err(|_| "收尾线程异常退出".into())
}

pub fn cancel<T>(handle: &JoinHandle<T>) {
    // ERROR_NOT_FOUND is harmless: no synchronous request was in flight at this instant.
    let _ = unsafe { CancelSynchronousIo(HANDLE(handle.as_raw_handle())) };
}

/// Bound best-effort disk/notification work, including its thread count after timeouts.
/// Rust cannot terminate an arbitrary callback or a stalled filesystem operation safely.
pub fn run_until<T: Send + 'static>(
    deadline: Instant,
    work: impl FnOnce() -> T + Send + 'static,
) -> Result<T, String> {
    static ACTIVE: AtomicUsize = AtomicUsize::new(0);
    if Instant::now() >= deadline {
        return Err("[STOP_TIMEOUT] 收尾期限已到".into());
    }
    ACTIVE
        .try_update(Ordering::AcqRel, Ordering::Acquire, |n| {
            (n < 8).then_some(n + 1)
        })
        .map_err(|_| "[STOP_TIMEOUT] 收尾后台任务已达上限".to_owned())?;
    struct Slot;
    impl Drop for Slot {
        fn drop(&mut self) {
            ACTIVE.fetch_sub(1, Ordering::Release);
        }
    }
    let slot = Slot;
    let (tx, rx) = mpsc::sync_channel(1);
    thread::Builder::new()
        .name("airplay-cleanup".into())
        .spawn(move || {
            let _slot = slot;
            let _ = tx.send(work());
        })
        .map_err(|e| e.to_string())?;
    rx.recv_timeout(deadline.saturating_duration_since(Instant::now()))
        .map_err(|e| match e {
            mpsc::RecvTimeoutError::Timeout => "[STOP_TIMEOUT] 收尾任务超过统一停止期限".into(),
            mpsc::RecvTimeoutError::Disconnected => "收尾任务异常退出".into(),
        })
}

#[cfg(test)]
#[path = "../../test/core/unit/shutdown.rs"]
mod tests;
