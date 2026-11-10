//! 将后台事件整理成播放页所需的更新，并合并高频快照。
//! Translate worker events for the player and coalesce high-frequency snapshots.
use airplay_desktop::{DesktopEvent, EventSink};
use async_channel::{Receiver, Sender};
use serde::Deserialize;
use serde_json::Value;
use std::{
    collections::VecDeque,
    mem,
    sync::{Arc, Mutex},
};

const CONTROL_CAPACITY: usize = 32;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceUpdate {
    pub endpoint: String,
    pub peaks: Option<[f32; 2]>,
    pub capture_enabled: Option<bool>,
    pub warning: Option<String>,
    pub error: Option<String>,
}

pub enum StreamUpdate {
    Preparing,
    Password(String),
    Ready,
    Playing,
    Notice(String),
    Finished {
        error: Option<String>,
        code: Option<String>,
    },
}

impl StreamUpdate {
    fn decode(payload: &Value) -> Option<Self> {
        let field = |name| payload[name].as_str().map(str::to_owned);
        Some(match payload["kind"].as_str()? {
            "preparing_source" => Self::Preparing,
            "password_required" => Self::Password(field("host")?),
            "native" if payload["line"].as_str()?.contains("[PROBE] PCM_READY") => Self::Ready,
            "telemetry" => Self::Playing,
            "auth_memory_error" | "auth_pipe_error" => Self::Notice(field("error")?),
            "finished" => Self::Finished {
                error: field("safe_error").or_else(|| field("error")),
                code: field("error_code"),
            },
            _ => return None,
        })
    }
}

#[derive(Default)]
pub struct Pending {
    pub source: Option<SourceUpdate>,
    pub controls: VecDeque<(u64, StreamUpdate)>,
    pub playing: Option<u64>,
    pub finished: Option<(u64, StreamUpdate)>,
    pub overflow: Option<u64>,
}

#[derive(Clone)]
pub struct Inbox {
    pending: Arc<Mutex<Pending>>,
    wake: Sender<()>,
}

impl Inbox {
    pub fn new() -> (Self, Receiver<()>) {
        let (wake, receive) = async_channel::bounded(1);
        (
            Self {
                pending: Arc::default(),
                wake,
            },
            receive,
        )
    }

    pub fn sink(&self) -> EventSink {
        let inbox = self.clone();
        Arc::new(move |event| inbox.push(event))
    }

    pub fn take(&self) -> Pending {
        mem::take(&mut *self.pending.lock().unwrap())
    }

    fn push(&self, event: DesktopEvent) {
        match event {
            DesktopEvent::SourceLevel(payload) => {
                let Ok(mut update) = serde_json::from_value::<SourceUpdate>(payload) else {
                    return;
                };
                let mut pending = self.pending.lock().unwrap();
                if let Some(previous) = pending
                    .source
                    .take()
                    .filter(|old| old.endpoint == update.endpoint)
                {
                    update.peaks = update.peaks.or(previous.peaks);
                    update.capture_enabled = update.capture_enabled.or(previous.capture_enabled);
                    update.warning = update.warning.or(previous.warning);
                    update.error = update.error.or(previous.error);
                }
                pending.source = Some(update);
            }
            DesktopEvent::Stream {
                session_id,
                payload,
            } => {
                let Some(update) = StreamUpdate::decode(&payload) else {
                    return;
                };
                let mut pending = self.pending.lock().unwrap();
                match update {
                    StreamUpdate::Playing => pending.playing = Some(session_id),
                    StreamUpdate::Finished { .. } => pending.finished = Some((session_id, update)),
                    _ if pending.controls.len() < CONTROL_CAPACITY => {
                        pending.controls.push_back((session_id, update))
                    }
                    _ => pending.overflow = Some(session_id),
                }
            }
        }
        // 一个待处理唤醒即可。锁释放后才通知，后台线程不等待界面取走事件。
        // One pending wake is enough. Notify after unlocking without waiting for the UI.
        let _ = self.wake.try_send(());
    }
}

#[cfg(test)]
#[path = "../../../test/frontend/desktop/gpui_inbox.rs"]
mod tests;
