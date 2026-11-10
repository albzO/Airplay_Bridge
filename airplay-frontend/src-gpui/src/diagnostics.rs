//! 诊断展示与设备累计统计，独立于串流控制和日志文件。
//! Diagnostic presentation and device totals, independent of streaming and log files.
use airplay_desktop::{DataMode, Settings};
use homepod_test::{discovery::Device, privacy::Redactor};
use serde::Deserialize;
use serde_json::Value;
use std::{collections::VecDeque, net::Ipv4Addr};

pub const LOG_LIMIT: usize = 300;
pub const SUMMARY_LIMIT: usize = 120;
const TEXT_LIMIT: usize = 4096;
const DEVICE_LIMIT: usize = 128;
const REPORT_LIMIT: usize = 1024 * 1024;
const COUNTERS: [&str; 6] = [
    "sent",
    "send_dropped",
    "sync_dropped",
    "rtx_requested",
    "rtx_resent",
    "rtx_expired",
];

fn bounded(mut text: String) -> String {
    if text.encode_utf16().count() > TEXT_LIMIT {
        let suffix = "…[截断]";
        let limit = TEXT_LIMIT - suffix.encode_utf16().count();
        let mut units = 0;
        let end = text.char_indices().find_map(|(offset, ch)| {
            units += ch.len_utf16();
            (units > limit).then_some(offset)
        });
        if let Some(end) = end {
            text.truncate(end);
        }
        text.push_str(suffix);
    }
    text
}

fn tail<T>(queue: &mut VecDeque<T>, item: T, limit: usize) {
    if queue.len() == limit {
        queue.pop_front();
    }
    queue.push_back(item);
}

fn number(value: &Value, decimals: usize) -> String {
    value
        .as_f64()
        .map_or_else(|| "—".into(), |n| format!("{n:.decimals$}"))
}

fn count(value: &Value) -> String {
    value.as_u64().map_or_else(|| "—".into(), |n| n.to_string())
}

fn safe(text: &str) -> String {
    bounded(Redactor::default().text(text))
}

fn file_name(value: &Value) -> Option<String> {
    let path = value.as_str()?;
    let name = path.rsplit(['/', '\\']).next()?;
    (!name.is_empty()).then(|| bounded(name.to_owned()))
}

#[derive(Default, Deserialize)]
pub struct Controller {
    pub correction_ppm: Option<f64>,
}

#[derive(Default, Deserialize)]
pub struct Telemetry {
    pub elapsed_seconds: Option<f64>,
    pub capture_frames: Option<u64>,
    pub output_frames: Option<u64>,
    pub water_ms: Option<f64>,
    pub lead_ms: Option<f64>,
    pub input_rate: Option<u32>,
    #[serde(default)]
    pub controller: Controller,
}

pub struct Report {
    pub text: String,
    pub discontinuities: Option<u64>,
    pub timestamp_errors: Option<u64>,
    pub conversion: Option<String>,
}

impl Report {
    fn decode(value: &Value) -> Self {
        let safe = Redactor::default().value(value);
        let compact = safe.to_string();
        if compact.len() > REPORT_LIMIT {
            return Self {
                text: "报告超过 1 MiB 显示上限。".into(),
                discontinuities: None,
                timestamp_errors: None,
                conversion: None,
            };
        }
        let pretty = serde_json::to_string_pretty(&safe).unwrap_or_default();
        Self {
            text: if pretty.len() <= REPORT_LIMIT {
                pretty
            } else {
                compact
            },
            discontinuities: safe["capture"]["discontinuities"].as_u64(),
            timestamp_errors: safe["capture"]["timestamp_errors"].as_u64(),
            conversion: (!safe["conversion"].is_null())
                .then(|| bounded(safe["conversion"].to_string())),
        }
    }
}

pub struct PacketStats {
    host: Ipv4Addr,
    values: [Option<u64>; COUNTERS.len()],
}

pub struct DeviceStats {
    host: Ipv4Addr,
    pub name: String,
    pub totals: [Option<u64>; COUNTERS.len()],
    previous: [Option<u64>; COUNTERS.len()],
}

pub enum Update {
    Log(String),
    Summary(String),
    Packet(PacketStats),
    LogPath {
        log: Option<String>,
        pipeline: Option<String>,
        detailed: bool,
    },
    CapturePath {
        capture: Option<String>,
        pipeline: Option<String>,
    },
    Clock(String),
    Auth(String),
    Telemetry(Telemetry),
    Report(Report),
}

pub fn decode(payload: &Value, detailed: bool) -> Vec<Update> {
    let entry = match payload["kind"].as_str() {
        Some("log_path") => Update::LogPath {
            log: file_name(&payload["path"]),
            pipeline: file_name(&payload["pipeline_path"]),
            detailed: payload["detailed_logs"].as_bool().unwrap_or(detailed),
        },
        Some("diagnostic_path") => Update::CapturePath {
            capture: file_name(&payload["path"]),
            pipeline: file_name(&payload["pipeline_path"]),
        },
        Some("telemetry") => match Telemetry::deserialize(payload) {
            Ok(snapshot) => Update::Telemetry(snapshot),
            Err(_) => return Vec::new(),
        },
        Some("report") if payload["report"].is_object() => {
            Update::Report(Report::decode(&payload["report"]))
        }
        Some("capture_diagnostic") => {
            let capture = &payload["capture"];
            let mut summary = format!(
                "{} s · 采集 {} 帧 · 待发送 {} ms · 水位 {} ms · 校正 {} ppm · 设备位置 {} · 包 {} · 时间戳 {} · 不连续 {} · 时间戳错误 {}",
                number(&payload["elapsed_seconds"], 1),
                count(&payload["capture_frames"]),
                number(&payload["pending_pcm_ms"], 1),
                number(&payload["water_ms"], 1),
                number(&payload["correction_ppm"], 1),
                count(&capture["device_position"]),
                count(&capture["packets"]),
                count(&capture["packet_qpc_100ns"]),
                count(&capture["discontinuities"]),
                count(&capture["timestamp_errors"]),
            );
            if let Some(error) = payload["error"].as_str() {
                summary.push_str(&format!(" · 错误：{}", safe(error)));
            }
            Update::Summary(bounded(summary))
        }
        Some("diagnostic_end") => Update::Summary(bounded(format!(
            "诊断结束：{}",
            Redactor::default().value(&payload["status"])
        ))),
        Some("native") => return native_updates(payload, detailed),
        Some("finished") => {
            let Some(error) = payload["safe_error"]
                .as_str()
                .or_else(|| payload["error"].as_str())
            else {
                return Vec::new();
            };
            Update::Log(safe(error))
        }
        _ => return Vec::new(),
    };
    vec![entry]
}

fn native_updates(payload: &Value, detailed: bool) -> Vec<Update> {
    let mut updates = Vec::new();
    if (detailed || payload["is_fault"] == true)
        && let Some(line) = payload["safe_line"].as_str()
    {
        updates.push(Update::Log(bounded(line.to_owned())));
    }
    if let Some(probe) = payload["line"]
        .as_str()
        .and_then(|line| line.split_once("[PROBE] ").map(|(_, probe)| probe))
    {
        let mut fields = probe.split_whitespace();
        let marker = fields.next();
        let field = |key| {
            fields
                .clone()
                .filter_map(|part| part.split_once('='))
                .find_map(|(name, value)| (name == key).then_some(value))
        };
        match marker {
            Some("PACKET_STATS") => {
                if let Some(host) = field("host").and_then(|host| host.parse().ok()) {
                    let values =
                        COUNTERS.map(|key| field(key).and_then(|value| value.parse().ok()));
                    if values.iter().any(Option::is_some) {
                        updates.push(Update::Packet(PacketStats { host, values }));
                    }
                }
            }
            Some("AUTH_METHOD") => {
                if let Some(value) = field("value") {
                    updates.push(Update::Auth(safe(value)));
                }
            }
            Some("TIMING") => {
                if let Some(value) = field("value") {
                    updates.push(Update::Clock(safe(value)));
                }
            }
            _ => {}
        }
    }
    updates
}

#[derive(Default)]
pub struct Batch {
    session: Option<u64>,
    logs: VecDeque<Update>,
    summaries: VecDeque<Update>,
    packets: VecDeque<Update>,
    log_path: Option<Update>,
    capture_path: Option<Update>,
    clock: Option<Update>,
    auth: Option<Update>,
    telemetry: Option<Update>,
    report: Option<Update>,
}

impl Batch {
    pub fn push(&mut self, id: u64, update: Update) {
        // 会话编号递增。旧事件不能挤占当前批次的日志或报告。
        // Session IDs increase. Late events cannot evict the current batch's logs or report.
        if self.session.is_some_and(|current| id < current) {
            return;
        }
        if self.session != Some(id) {
            *self = Self {
                session: Some(id),
                ..Self::default()
            };
        }
        match update {
            Update::Log(_) => tail(&mut self.logs, update, LOG_LIMIT),
            Update::Summary(_) => tail(&mut self.summaries, update, SUMMARY_LIMIT),
            Update::Packet(_) => tail(&mut self.packets, update, DEVICE_LIMIT),
            Update::LogPath { .. } => self.log_path = Some(update),
            Update::CapturePath { .. } => self.capture_path = Some(update),
            Update::Clock(_) => self.clock = Some(update),
            Update::Auth(_) => self.auth = Some(update),
            Update::Telemetry(_) => self.telemetry = Some(update),
            Update::Report(_) => self.report = Some(update),
        }
    }

    fn entries(self) -> impl Iterator<Item = Update> {
        [
            self.log_path,
            self.capture_path,
            self.clock,
            self.auth,
            self.telemetry,
            self.report,
        ]
        .into_iter()
        .flatten()
        .chain(self.logs)
        .chain(self.summaries)
        .chain(self.packets)
    }
}

pub struct Diagnostics {
    mode: DataMode,
    pub detailed: bool,
    pub extra_capture: bool,
    pub logs: VecDeque<String>,
    pub summaries: VecDeque<String>,
    pub log_path: String,
    pub pipeline_path: Option<String>,
    pub capture_path: Option<String>,
    pub clock: Option<String>,
    pub auth: Option<String>,
    pub telemetry: Telemetry,
    pub report: Option<Report>,
    pub devices: VecDeque<DeviceStats>,
}

impl Default for Diagnostics {
    fn default() -> Self {
        Self {
            mode: DataMode::Installed,
            detailed: false,
            extra_capture: false,
            logs: VecDeque::new(),
            summaries: VecDeque::new(),
            log_path: "%APPDATA%/AirPlay Hub/logs".into(),
            pipeline_path: None,
            capture_path: None,
            clock: None,
            auth: None,
            telemetry: Telemetry::default(),
            report: None,
            devices: VecDeque::new(),
        }
    }
}

impl Diagnostics {
    fn directory(&self) -> &'static str {
        match self.mode {
            DataMode::Portable => "[程序目录]/data/logs",
            DataMode::Installed => "%APPDATA%/AirPlay Hub/logs",
        }
    }

    pub fn set_mode(&mut self, mode: DataMode) {
        self.mode = mode;
        self.log_path = self.directory().into();
    }

    fn display_path(&self, name: Option<String>) -> Option<String> {
        name.map(|name| format!("{}/{name}", self.directory()))
    }

    pub fn begin(&mut self, settings: &Settings) {
        self.detailed = settings.detailed_logs;
        self.extra_capture = settings.capture_diagnostics;
        self.logs.clear();
        self.summaries.clear();
        self.pipeline_path = None;
        self.capture_path = None;
        self.clock = None;
        self.auth = None;
        self.telemetry = Telemetry::default();
        self.report = None;
        for device in &mut self.devices {
            device.previous = [None; COUNTERS.len()];
        }
    }

    pub fn source_warning(&mut self, warning: &str) {
        tail(
            &mut self.logs,
            bounded(format!("[WARN] {}", safe(warning))),
            LOG_LIMIT,
        );
    }

    pub fn accept(&mut self, batch: Batch, session: u64, devices: &[Device]) {
        if batch.session != Some(session) {
            return;
        }
        for update in batch.entries() {
            match update {
                Update::Log(line) => tail(&mut self.logs, line, LOG_LIMIT),
                Update::Summary(line) => tail(&mut self.summaries, line, SUMMARY_LIMIT),
                Update::Packet(stats) => self.record_stats(stats, devices),
                Update::LogPath {
                    log,
                    pipeline,
                    detailed,
                } => {
                    self.log_path = self
                        .display_path(log)
                        .unwrap_or_else(|| self.directory().into());
                    self.pipeline_path = self.display_path(pipeline);
                    self.detailed = detailed;
                }
                Update::CapturePath { capture, pipeline } => {
                    self.capture_path = self.display_path(capture);
                    self.pipeline_path = self.display_path(pipeline);
                }
                Update::Clock(value) => self.clock = Some(value),
                Update::Auth(value) => self.auth = Some(value),
                Update::Telemetry(snapshot) => self.telemetry = snapshot,
                Update::Report(report) => self.report = Some(report),
            }
        }
    }

    fn record_stats(&mut self, stats: PacketStats, devices: &[Device]) {
        let name = devices
            .iter()
            .find(|device| device.addresses.contains(&stats.host))
            .map(|device| bounded(device.name.clone()));
        let index = if let Some(index) = self
            .devices
            .iter()
            .position(|device| device.host == stats.host)
        {
            index
        } else {
            tail(
                &mut self.devices,
                DeviceStats {
                    host: stats.host,
                    name: "未知接收端".into(),
                    totals: [None; COUNTERS.len()],
                    previous: [None; COUNTERS.len()],
                },
                DEVICE_LIMIT,
            );
            self.devices.len() - 1
        };
        let device = &mut self.devices[index];
        if let Some(name) = name {
            device.name = name;
        }
        for (index, value) in stats.values.into_iter().enumerate() {
            if let Some(value) = value {
                let delta = value.saturating_sub(device.previous[index].unwrap_or(0));
                device.totals[index] =
                    Some(device.totals[index].unwrap_or(0).saturating_add(delta));
                device.previous[index] = Some(value);
            }
        }
    }
}

#[cfg(test)]
#[path = "../../../test/frontend/desktop/gpui_diagnostics.rs"]
mod tests;
