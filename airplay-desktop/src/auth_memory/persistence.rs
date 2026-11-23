//! At most two policy writes per session. GUI callbacks only register an index or drain errors.
use super::password_host;
use homepod_test::live::GuiControl;
use serde_json::Value;
use std::{
    collections::VecDeque,
    io,
    sync::{Arc, Condvar, Mutex, atomic::Ordering},
    thread::{self, JoinHandle},
    time::Instant,
};

#[derive(Default)]
struct Pending {
    requests: VecDeque<usize>,
    seen: u8,
    closed: bool,
    errors: Vec<String>,
}
#[derive(Clone)]
pub struct Sender {
    targets: Arc<Vec<(String, String)>>,
    pending: Arc<(Mutex<Pending>, Condvar)>,
    control: Arc<GuiControl>,
}
impl Sender {
    pub fn observe(&self, event: &Value) {
        if self.control.stop.load(Ordering::Acquire) {
            return;
        }
        let Some(host) = password_host(event) else {
            return;
        };
        let Some(index) = self.targets.iter().position(|(address, _)| address == host) else {
            return;
        };
        let (pending, wake) = &*self.pending;
        let mut pending = pending.lock().unwrap();
        if !pending.closed && pending.seen & (1 << index) == 0 {
            pending.seen |= 1 << index;
            pending.requests.push_back(index);
            wake.notify_one();
        }
    }
    pub fn take_errors(&self) -> Vec<String> {
        std::mem::take(&mut self.pending.0.lock().unwrap().errors)
    }
    fn error(&self, error: String) {
        // One attempted save per allowed target; even panic/deadline errors stay bounded.
        let bounded: String = error.chars().take(2048).collect();
        let mut pending = self.pending.0.lock().unwrap();
        if pending.errors.len() < 2 {
            pending.errors.push(bounded);
        }
    }
    fn close(&self) {
        let (pending, wake) = &*self.pending;
        pending.lock().unwrap().closed = true;
        wake.notify_one();
    }
}

pub struct Worker {
    sender: Sender,
    thread: Option<JoinHandle<()>>,
}
impl Worker {
    pub fn start(
        targets: Vec<(String, String)>,
        control: Arc<GuiControl>,
        mut save: impl FnMut(&str) -> Result<(), String> + Send + 'static,
    ) -> io::Result<Self> {
        if targets.is_empty()
            || targets.len() > 2
            || targets.iter().any(|(host, key)| {
                host.is_empty() || host.len() > 128 || key.is_empty() || key.len() > 4096
            })
        {
            return Err(io::Error::other("认证策略目标数量或身份长度无效"));
        }
        let sender = Sender {
            targets: Arc::new(targets),
            pending: Arc::default(),
            control,
        };
        let receiver = sender.clone();
        let thread = thread::Builder::new()
            .name("airplay-auth-policy".into())
            .spawn(move || {
                loop {
                    let request = {
                        let (pending, wake) = &*receiver.pending;
                        let mut pending = pending.lock().unwrap();
                        loop {
                            if let Some(request) = pending.requests.pop_front() {
                                break request;
                            }
                            if pending.closed {
                                return;
                            }
                            pending = wake.wait(pending).unwrap();
                        }
                    };
                    // Never begin another disk write after the shared stop deadline expires.
                    // A write already inside the OS can only finish normally; its session stays reserved.
                    if receiver
                        .control
                        .shutdown
                        .current()
                        .is_some_and(|deadline| Instant::now() >= deadline)
                    {
                        receiver.error("认证策略未保存：停止期限已到".into());
                        continue;
                    }
                    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        save(&receiver.targets[request].1)
                    }))
                    .unwrap_or_else(|_| Err("认证策略后台保存异常".into()));
                    if let Err(error) = outcome {
                        receiver.error(error);
                    }
                }
            })?;
        Ok(Self {
            sender,
            thread: Some(thread),
        })
    }
    pub fn sender(&self) -> Sender {
        self.sender.clone()
    }
    pub fn unfinished(&self) -> bool {
        self.thread.is_some()
    }
    pub fn finish_until(&mut self, deadline: Instant) -> Result<(), String> {
        self.sender.close();
        let deadline = self
            .sender
            .control
            .shutdown
            .current()
            .map_or(deadline, |end| end.min(deadline));
        let outcome =
            homepod_test::shutdown::join_until(&mut self.thread, deadline, false).map(|_| ());
        if outcome.is_err() && self.unfinished() {
            self.sender
                .control
                .cleanup_complete
                .store(false, Ordering::Release);
        }
        outcome
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.sender.close();
    }
}

#[cfg(test)]
#[path = "../../../test/frontend/desktop/auth_persistence.rs"]
mod tests;
