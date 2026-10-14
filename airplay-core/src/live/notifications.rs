//! One bounded GUI dispatcher per session; producers never invoke application callbacks.
use super::{GuiControl, GuiEmitter};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, VecDeque},
    io::{self, Write},
    sync::{Arc, Condvar, Mutex, atomic::Ordering},
    thread::{self, JoinHandle},
    time::Instant,
};

const CONTROLS: usize = 128;
const LOGS: usize = 64;
const SNAPSHOTS: usize = 16;
const EVENT_BYTES: usize = 64 * 1024;
type Entry = (u64, Value);

#[derive(Default)]
struct Pending {
    sequence: u64,
    controls: VecDeque<Entry>,
    logs: VecDeque<Entry>,
    snapshots: BTreeMap<String, Entry>,
    report: Option<Entry>,
    closed: bool,
    sealed: bool,
    error: Option<String>,
    dropped_logs: u64,
}

#[derive(Clone)]
struct Sender {
    pending: Arc<(Mutex<Pending>, Condvar)>,
    control: Arc<GuiControl>,
}

// Count serialized bytes without allocating a second copy of every event.
struct Limit(usize);
impl Write for Limit {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > self.0 {
            return Err(io::Error::other("GUI event exceeds its byte budget"));
        }
        self.0 -= bytes.len();
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

enum Class {
    Control,
    Log,
    Snapshot(String),
    Report,
}
fn classify(event: &Value) -> Class {
    match event["kind"].as_str() {
        Some("report") => Class::Report,
        Some("telemetry") => Class::Snapshot("telemetry".into()),
        Some("capture_diagnostic") if event["error"].is_null() => {
            Class::Snapshot("capture_diagnostic".into())
        }
        Some("native") => {
            let line = event["line"].as_str().unwrap_or_default();
            if event["is_fault"] == true {
                return Class::Control;
            }
            if let Some(probe) = line.split_once("[PROBE] ").map(|(_, tail)| tail) {
                let marker = probe.split_whitespace().next().unwrap_or_default();
                if matches!(marker, "PACKET_STATS" | "EVENTS_STATS" | "AUDIO_STATS") {
                    let host = probe
                        .split_whitespace()
                        .find_map(|field| field.strip_prefix("host="))
                        .unwrap_or_default();
                    return Class::Snapshot(format!("{marker}:{host}"));
                }
                // All other protocol markers (including auth and volume) retain FIFO delivery.
                return Class::Control;
            }
            Class::Log
        }
        _ => Class::Control,
    }
}

impl Sender {
    fn fail(&self, message: &str) {
        let (queue, wake) = &*self.pending;
        let mut queue = queue.lock().unwrap();
        queue
            .error
            .get_or_insert_with(|| format!("[STREAM_NOTIFY_FAILED] {message}"));
        wake.notify_one();
        drop(queue);
        self.control.request_stop();
    }

    fn send(&self, mut event: Value) {
        let budget = if event["kind"] == "report" {
            crate::log_store::REPORT_BYTES as usize
        } else {
            EVENT_BYTES
        };
        if serde_json::to_writer(Limit(budget), &event).is_err() {
            self.fail("会话通知超过大小限制，已请求停止串流");
            return;
        }
        let class = classify(&event);
        let (queue, wake) = &*self.pending;
        let mut queue = queue.lock().unwrap();
        if queue.closed || queue.sealed {
            return;
        }
        queue.sequence += 1;
        let sequence = queue.sequence;
        let overflow = match class {
            Class::Control if queue.controls.len() < CONTROLS => {
                queue.controls.push_back((sequence, event));
                false
            }
            Class::Log => {
                if queue.logs.len() < LOGS {
                    queue.logs.push_back((sequence, event));
                } else {
                    queue.dropped_logs = queue.dropped_logs.saturating_add(1);
                }
                false
            }
            Class::Snapshot(key)
                if queue.snapshots.contains_key(&key) || queue.snapshots.len() < SNAPSHOTS =>
            {
                queue.snapshots.insert(key, (sequence, event));
                false
            }
            Class::Report => {
                event["report"]["notification_status"] = json!({
                    "error":queue.error,"dropped_display_logs":queue.dropped_logs,
                });
                if serde_json::to_writer(Limit(budget), &event).is_err() {
                    drop(queue);
                    self.fail("最终报告通知超过大小限制，已请求停止串流");
                    return;
                }
                // Seal at the report barrier: no late telemetry can follow the final report.
                queue.report = Some((sequence, event));
                queue.sealed = true;
                false
            }
            _ => true,
        };
        wake.notify_one();
        drop(queue);
        if overflow {
            self.fail("会话控制通知或统计槽已满，已请求停止串流");
        }
    }

    fn close(&self) {
        let (pending, wake) = &*self.pending;
        pending.lock().unwrap().closed = true;
        wake.notify_one();
    }
}

impl Pending {
    fn pop(&mut self) -> Option<Value> {
        // A replaced snapshot gets a new sequence. This preserves ready-before-telemetry,
        // and drains all older accepted events before the report, even across queue classes.
        let mut next = self
            .controls
            .front()
            .map(|entry| (entry.0, 0, String::new()));
        for candidate in self
            .logs
            .front()
            .map(|entry| (entry.0, 1, String::new()))
            .into_iter()
            .chain(
                self.snapshots
                    .iter()
                    .map(|(key, entry)| (entry.0, 2, key.clone())),
            )
            .chain(
                self.report
                    .as_ref()
                    .map(|entry| (entry.0, 3, String::new())),
            )
        {
            if next.as_ref().is_none_or(|entry| candidate.0 < entry.0) {
                next = Some(candidate);
            }
        }
        match next? {
            (_, 0, _) => self.controls.pop_front().map(|entry| entry.1),
            (_, 1, _) => self.logs.pop_front().map(|entry| entry.1),
            (_, 2, key) => self.snapshots.remove(&key).map(|entry| entry.1),
            _ => self.report.take().map(|entry| entry.1),
        }
    }
}

pub(super) struct Worker {
    sender: Sender,
    thread: Option<JoinHandle<()>>,
}
impl Worker {
    pub(super) fn start(emit: GuiEmitter, control: Arc<GuiControl>) -> io::Result<Self> {
        let sender = Sender {
            pending: Arc::default(),
            control,
        };
        let receiver = sender.clone();
        let thread = thread::Builder::new()
            .name("airplay-notifications".into())
            .spawn(move || {
                loop {
                    let event = {
                        let (queue, wake) = &*receiver.pending;
                        let mut queue = queue.lock().unwrap();
                        loop {
                            if let Some(event) = queue.pop() {
                                break event;
                            }
                            if queue.closed {
                                return;
                            }
                            queue = wake.wait(queue).unwrap();
                        }
                    };
                    if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| emit(event)))
                        .is_err()
                    {
                        receiver.fail("界面通知回调异常，已请求停止串流");
                        return;
                    }
                }
            })?;
        Ok(Self {
            sender,
            thread: Some(thread),
        })
    }

    pub(super) fn emitter(&self) -> GuiEmitter {
        let sender = self.sender.clone();
        Arc::new(move |event| sender.send(event))
    }

    pub(super) fn finish_until(&mut self, deadline: Instant) -> Result<(), String> {
        self.sender.close();
        crate::shutdown::join_until(&mut self.thread, deadline, false)?;
        let queue = self.sender.pending.0.lock().unwrap();
        queue.error.clone().map_or(Ok(()), Err)
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.sender.close();
        if let Some(thread) = self.thread.take() {
            // A callback cannot be force-killed. Keep its handle in the reserved session.
            self.sender
                .control
                .cleanup_complete
                .store(false, Ordering::Release);
            *self.sender.control.notification_worker.lock().unwrap() = Some(thread);
        }
    }
}

#[cfg(test)]
#[path = "../../../test/core/unit/live/notifications.rs"]
mod tests;
