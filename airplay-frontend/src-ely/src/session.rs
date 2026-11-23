//! 播放页会话状态，只有 Engine 返回的编号可以接收事件。
//! Player session state, keyed by the ID returned by Engine.
use crate::{
    inbox::StreamUpdate,
    volume::{Volume, VolumeReport},
};

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum Phase {
    #[default]
    Idle,
    Connecting,
    Preparing,
    Password,
    Verifying,
    Ready,
    Playing,
    Stopping,
    Stopped,
    Failed,
}

#[derive(Default)]
pub struct Session {
    pub id: Option<u64>,
    pub phase: Phase,
    pub prompt: Option<String>,
    pub error: Option<String>,
    pub volume: Volume,
    last_password_host: Option<String>,
}

impl Session {
    pub fn start(&mut self, id: u64) {
        *self = Self {
            id: Some(id),
            phase: Phase::Connecting,
            ..Default::default()
        };
    }

    pub fn stop(&mut self) {
        self.phase = Phase::Stopping;
        self.prompt = None;
    }

    pub fn password_sent(&mut self, host: String) {
        self.last_password_host = Some(host);
        self.prompt = None;
        self.phase = Phase::Verifying;
    }

    pub fn accept(&mut self, id: u64, event: StreamUpdate) -> bool {
        if self.id != Some(id) {
            return false;
        }
        match event {
            StreamUpdate::Finished { error, code } => {
                let rejected = code.as_deref() == Some("PASSWORD_REJECTED");
                self.prompt = if rejected && self.phase != Phase::Stopping {
                    self.last_password_host.take()
                } else {
                    None
                };
                self.phase = if error.is_some() {
                    Phase::Failed
                } else {
                    Phase::Stopped
                };
                self.id = None;
                if error.is_some() {
                    self.error = error;
                }
            }
            StreamUpdate::Notice(error) => self.error = Some(error),
            _ if self.phase == Phase::Stopping => return false,
            StreamUpdate::Preparing => self.phase = Phase::Preparing,
            StreamUpdate::Password(host) => {
                self.prompt = Some(host);
                self.phase = Phase::Password;
            }
            StreamUpdate::Ready => {
                self.prompt = None;
                self.phase = Phase::Ready;
            }
            StreamUpdate::Playing => self.phase = Phase::Playing,
            StreamUpdate::Volume(VolumeReport::Level(level)) => self.volume.confirm(level),
            StreamUpdate::Volume(VolumeReport::Rejected) => {}
        }
        true
    }
}

#[cfg(test)]
#[path = "../../../test/frontend/desktop/gpui_session.rs"]
mod tests;
