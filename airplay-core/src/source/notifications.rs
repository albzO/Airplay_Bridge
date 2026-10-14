//! Source GUI work never runs on the capture thread. No disk writes or event history here.
use super::*;
use std::{collections::VecDeque, sync::Condvar};

const CONTROL_CAPACITY: usize = 64;

#[derive(Default)]
struct Pending {
    level: Option<Value>,
    controls: VecDeque<Value>,
    terminal: Option<Value>,
    closed: bool,
}

#[derive(Clone, Default)]
pub(super) struct Sender(Arc<(Mutex<Pending>, Condvar)>);

impl Sender {
    pub(super) fn level(&self, event: Value) {
        let (pending, wake) = &*self.0;
        let mut pending = pending.lock().unwrap();
        if !pending.closed {
            pending.level = Some(event);
            wake.notify_one();
        }
    }

    // Controls are never overwritten by telemetry. Overflow is a terminal Source fault,
    // not a silent drop; the separate terminal slot remains available even when full.
    pub(super) fn control(&self, event: Value) -> std::result::Result<(), String> {
        let (pending, wake) = &*self.0;
        let mut pending = pending.lock().unwrap();
        if pending.closed || pending.controls.len() == CONTROL_CAPACITY {
            return Err("[SOURCE_NOTIFY_FAILED] 采集通知队列已满或关闭，已停止采集".into());
        }
        pending.controls.push_back(event);
        wake.notify_one();
        Ok(())
    }

    pub(super) fn finish(&self, event: Value) {
        let (pending, wake) = &*self.0;
        let mut pending = pending.lock().unwrap();
        if !pending.closed {
            // No queued nonzero level may follow the final zero/error notification.
            pending.level = None;
            pending.terminal = Some(event);
            pending.closed = true;
            wake.notify_one();
        }
    }
}

pub(super) struct Worker {
    sender: Sender,
    thread: Option<thread::JoinHandle<()>>,
}

impl Worker {
    pub(super) fn start(emit: Arc<dyn Fn(Value) + Send + Sync>) -> Self {
        let sender = Sender::default();
        let receiver = sender.clone();
        let thread = thread::spawn(move || {
            loop {
                let event = {
                    let (pending, wake) = &*receiver.0;
                    let mut pending = pending.lock().unwrap();
                    loop {
                        if let Some(event) = pending.controls.pop_front() {
                            break event;
                        }
                        if let Some(event) = pending.terminal.take() {
                            break event;
                        }
                        if pending.closed {
                            return;
                        }
                        if let Some(event) = pending.level.take() {
                            break event;
                        }
                        pending = wake.wait(pending).unwrap();
                    }
                };
                // Release the queue lock before invoking potentially slow application code.
                emit(event);
            }
        });
        Self {
            sender,
            thread: Some(thread),
        }
    }

    pub(super) fn sender(&self) -> Sender {
        self.sender.clone()
    }

    pub(super) fn join_until(&mut self, deadline: Instant) -> std::result::Result<(), String> {
        crate::shutdown::join_until(&mut self.thread, deadline, false).map(|_| ())
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        let (pending, wake) = &*self.sender.0;
        let mut pending = pending.lock().unwrap();
        pending.closed = true;
        pending.level = None;
        wake.notify_one();
    }
}

#[cfg(test)]
#[path = "../../../test/core/unit/source/notifications.rs"]
mod tests;
