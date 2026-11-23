//! 接收端确认的音量，以及静音前最近一次确认的非零音量。
//! Receiver-confirmed volume and the last confirmed nonzero level for unmute.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum VolumeReport {
    Level(f32),
    Rejected,
}

impl VolumeReport {
    pub fn from_native(line: &str) -> Option<Self> {
        let mut parts = line.split_once("[PROBE] ")?.1.split_whitespace();
        let marker = parts.next()?;
        let field = |prefix| parts.clone().find_map(|part| part.strip_prefix(prefix));
        match marker {
            "VOLUME_CURRENT" => {}
            "VOLUME_APPLIED" if field("http=")? != "200" => return Some(Self::Rejected),
            "VOLUME_APPLIED" => {}
            _ => return None,
        }
        let level = if let Some(percent) = field("percent=") {
            percent.parse::<f32>().ok()?
        } else {
            let db = field("db=")?.parse::<f64>().ok()?;
            if !db.is_finite() {
                return None;
            }
            if db <= -144. {
                0.
            } else {
                ((db + 30.) / 0.3).clamp(0., 100.) as f32
            }
        };
        (0. ..=100.).contains(&level).then_some(Self::Level(level))
    }
}

#[derive(Default)]
pub struct Volume {
    pub confirmed: Option<f32>,
    last_audible: Option<f32>,
}

impl Volume {
    pub fn confirm(&mut self, level: f32) {
        self.confirmed = Some(level);
        if level > 0. {
            self.last_audible = Some(level);
        }
    }

    pub fn mute_target(&self) -> f32 {
        if self.confirmed == Some(0.) {
            self.last_audible.unwrap_or(50.)
        } else {
            0.
        }
    }
}

#[cfg(test)]
#[path = "../../../test/frontend/desktop/gpui_volume.rs"]
mod tests;
