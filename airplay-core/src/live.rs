//! 实时串流桥接：采集 → 有状态重采样 → 有界 PCM 队列 → 原生后端管道。
//! GUI 订阅持续 Source；CLI 使用自身采集流程，二者共用转换和协议调度。
//! 输出为 44.1 kHz、16 位、立体声 PCM；帧数每秒为 44100，字节数每秒为 176400。
//! 后端的 QPC 播放计划作为时基，控制器按缓冲水位微调采样率，避免长期时钟漂移。
//! PCM 写管道、协议日志读取和有界诊断落盘各有独立线程；漂移摘要每 10 s 非阻塞入队。
//!
//! Live bridge: capture, stateful resampling, bounded PCM queue, then native backend pipe.
//! GUI subscribes to a persistent Source; CLI owns its capture path. Both share conversion
//! and protocol scheduling. Output is 44.1 kHz, 16-bit stereo: 44100 frames/176400 bytes per second.
//! The backend's QPC playback schedule is the timebase; water-level feedback corrects clock drift.
//! Pipe writing, protocol reading and bounded diagnostic writing use workers; drift summaries enqueue every 10 s.
//!
//! live.rs 只编排启动、采集与收尾；具体职责见同名目录中的内部模块。
//! live.rs orchestrates startup, capture and cleanup; private submodules own the implementation.
mod control;
pub(crate) mod diagnostics;
mod pipeline;
mod protocol;
mod protocol_log;
mod report;
mod transport;
use crate::audio_queue::{AUDIO_BUDGET_MS, BLOCK_LIMIT};
use crate::{capture, discovery::Device};
use control::{Control, STOP};
pub use control::{GuiContext, GuiControl, GuiEmitter};
use diagnostics::DetailLog;
use pipeline::{AudioPipeline, PipelineOptions};
use protocol::BackendReader;
use std::{
    error::Error,
    fs,
    path::Path,
    sync::{Mutex, atomic::Ordering},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use transport::{BYTES_PER_FRAME, BYTES_PER_MS, Backend, BackendOptions, PcmWriter};
type Result<T> = std::result::Result<T, Box<dyn Error>>;
/// CLI 单设备串流；seconds 为 0 时持续运行，latency_ms / buffer_ms 的单位为毫秒。
/// CLI streaming to one device; zero seconds runs indefinitely, latency_ms / buffer_ms use milliseconds.
pub fn run(
    root: &Path,
    name: &str,
    seconds: u64,
    latency_ms: u32,
    buffer_ms: u32,
    endpoint: Option<&str>,
) -> Result<()> {
    run_targets(
        root, name, None, seconds, latency_ms, buffer_ms, None, endpoint,
    )
}
/// CLI 立体声串流；两个名称必须对应同一 tsid 的不同设备，共享采集与播放计划。
/// CLI stereo streaming; distinct devices must share tsid, capture and the playback schedule.
pub fn run_stereo(
    root: &Path,
    left: &str,
    right: &str,
    seconds: u64,
    latency_ms: u32,
    buffer_ms: u32,
    endpoint: Option<&str>,
) -> Result<()> {
    run_targets(
        root,
        left,
        Some(right),
        seconds,
        latency_ms,
        buffer_ms,
        None,
        endpoint,
    )
}
/// GUI 串流复用现有 Source；停止本次会话只解除订阅，由桌面端管理来源生命周期。
/// GUI streaming reuses Source; stopping detaches this session while the desktop owns source lifetime.
pub fn run_gui(
    root: &Path,
    name: &str,
    peer: Option<&str>,
    latency_ms: u32,
    buffer_ms: u32,
    gui: GuiContext,
) -> Result<()> {
    run_targets(root, name, peer, 0, latency_ms, buffer_ms, Some(gui), None)
}
fn run_targets(
    root: &Path,
    name: &str,
    peer_name: Option<&str>,
    seconds: u64,
    latency_ms: u32,
    buffer_ms: u32,
    gui: Option<GuiContext>,
    endpoint: Option<&str>,
) -> Result<()> {
    // 在启动任何子进程前校验目标；后续协议与命令参数仍使用原始身份，日志独立脱敏。
    // Validate targets before spawning; protocol/arguments use original identities, logs use redacted copies.
    let devices: Vec<Device> = serde_json::from_slice(&fs::read(root.join("devices.json"))?)?;
    let matches: Vec<_> = devices.iter().filter(|d| d.name == name).collect();
    if matches.len() != 1 {
        return Err("设备名称不唯一或未发现，请先 discover".into());
    }
    let device = matches[0];
    let peer = if let Some(peer_name) = peer_name {
        let found: Vec<_> = devices.iter().filter(|d| d.name == peer_name).collect();
        if name == peer_name || found.len() != 1 {
            return Err("立体声需要两个不同且唯一的设备名称".into());
        }
        let other = found[0];
        let tsid = device.properties.get("tsid").filter(|s| !s.is_empty());
        if tsid.is_none() || tsid != other.properties.get("tsid") {
            return Err("两台设备没有相同的立体声组 tsid，请在家庭 App 核对并重新 discover".into());
        }
        Some(other)
    } else {
        None
    };
    let host = device
        .addresses
        .first()
        .ok_or("设备没有 IPv4 地址")?
        .to_string();
    let txt = device
        .properties
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join(" ");
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();
    let identity = format!("{stamp:016X}");
    let active_remote = ((stamp as u64 ^ (stamp as u64 >> 32)) as u32)
        .max(1)
        .to_string();
    fs::create_dir_all(root.join("logs"))?;
    let log_path = root.join("logs").join(format!("live-{stamp}.log"));
    let privacy = crate::privacy::Redactor::new(&devices);
    let detailed_logs = gui.as_ref().is_none_or(|g| g.detailed_logs);
    let log = crate::log_store::LogFile::create_limited(
        &log_path,
        if detailed_logs {
            crate::log_store::FILE_BYTES
        } else {
            256 * 1024
        },
    )?;
    let drift_log = if detailed_logs || gui.is_none() {
        Some(DetailLog::start_drift(
            detailed_logs
                .then_some(log_path.with_extension("drift.jsonl"))
                .as_deref(),
            gui.is_none(),
        )?)
    } else {
        None
    };
    let dacp = crate::volume::Dacp::start_with_logging(
        *device.addresses.first().unwrap(),
        &identity,
        &active_remote,
        &log_path.with_extension("volume.log"),
        !detailed_logs,
    )?;
    let peer_identity = format!("{:016X}", stamp + 1);
    let peer_remote = active_remote
        .parse::<u32>()?
        .wrapping_add(1)
        .max(1)
        .to_string();
    let peer_dacp = if let Some(other) = peer {
        Some(crate::volume::Dacp::start_with_logging(
            *other.addresses.first().ok_or("另一台没有 IPv4 地址")?,
            &peer_identity,
            &peer_remote,
            &log_path.with_extension("peer.volume.log"),
            !detailed_logs,
        )?)
    } else {
        None
    };
    if let Some(g) = &gui {
        let mut senders = g.control.volume.lock().unwrap();
        senders.push(dacp.command_sender());
        if let Some(peer) = &peer_dacp {
            senders.push(peer.command_sender());
        }
        (g.emit)(serde_json::json!({
            "kind": "log_path",
            "path": log_path,
            "detailed_logs": detailed_logs,
            "pipeline_path": if detailed_logs { Some(log_path.with_extension("pipeline.jsonl")) } else { None },
        }));
    }
    let peer_args = if let Some(other) = peer {
        vec![
            "--peer-host".to_owned(),
            other.addresses[0].to_string(),
            "--peer-port".to_owned(),
            other.port.to_string(),
            "--peer-name".to_owned(),
            other.name.clone(),
            "--peer-txt".to_owned(),
            other
                .properties
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect::<Vec<_>>()
                .join(" "),
            "--peer-identity".to_owned(),
            peer_identity,
            "--peer-active-remote".to_owned(),
            peer_remote,
            "--peer-volume-control-port".to_owned(),
            peer_dacp.as_ref().unwrap().control_port.to_string(),
        ]
    } else {
        Vec::new()
    };
    let stop = gui.as_ref().map_or(&STOP, |g| g.control.stop.as_ref());
    let _control = Control::install(gui.is_some())?;
    if seconds == 0 {
        println!(
            "建立持续流到 {name}（{host}:{}），持续运行直到 Ctrl+C。",
            device.port
        );
    } else {
        println!(
            "建立持续流到 {name}（{host}:{}），最多 {seconds} 秒。",
            device.port
        );
    }
    println!(
        "先自动认证；仅在设备要求密码时隐藏输入。连接完成后开始采集所选音频来源，Ctrl+C 正常停止。"
    );
    if let Some(other) = peer {
        println!(
            "立体声对：{name} + {}；若需要密码，只输入一次并用于两台，采集一次，共享 PTP 和播放起点。",
            other.name
        );
    }
    println!("日志：{}", log_path.display());
    // 先完成认证并收到 PCM_READY，再创建音频发送线程和订阅；避免把握手等待变成音频积压。
    // Authenticate and receive PCM_READY before starting PCM delivery/capture, avoiding a handshake backlog.
    let mut backend = Backend::spawn(BackendOptions {
        executable: gui
            .as_ref()
            .map_or_else(|| crate::backend::executable(root), |g| g.backend.clone()),
        gui: gui.as_ref(),
        host: &host,
        port: device.port,
        name,
        txt: &txt,
        identity: &identity,
        active_remote: &active_remote,
        volume_control_port: dacp.control_port,
        latency_ms,
        buffer_ms,
        detailed_logs,
        peer_args: &peer_args,
    })?;
    let mut reader = BackendReader::start(
        backend.stderr()?,
        log,
        privacy.clone(),
        detailed_logs,
        gui.as_ref().map(|g| g.emit.clone()),
    )?;
    let protocol = reader.state.clone();
    reader.wait_ready(&mut backend, stop)?;
    let (sender, writer) = PcmWriter::start(backend.stdin()?);
    let counters = writer.counters.clone();
    let progress = Mutex::new(capture::CaptureProgress::default());
    let capture_diagnostics = gui.as_ref().is_some_and(|g| g.capture_diagnostics);
    let diagnostic_path = log_path.with_extension("capture.jsonl");
    let mut packet_log = if capture_diagnostics {
        Some(DetailLog::start_capture(&diagnostic_path)?)
    } else {
        None
    };
    if let Some(g) = &gui {
        if capture_diagnostics {
            (g.emit)(serde_json::json!({
                "kind": "diagnostic_path",
                "path": diagnostic_path,
                "pipeline_path": log_path.with_extension("pipeline.jsonl"),
            }));
        }
    }
    let mut packet_record = |mut entry: serde_json::Value| {
        entry["pending_pcm_ms"] =
            serde_json::json!(counters.pending_bytes.load(Ordering::Relaxed) as f64 / BYTES_PER_MS);
        entry["pipe_written_frames"] =
            serde_json::json!(counters.written_bytes.load(Ordering::Relaxed) / BYTES_PER_FRAME);
        if let Some(log) = &packet_log {
            log.record(entry);
        }
    };
    let trace = if detailed_logs || capture_diagnostics {
        Some(DetailLog::start(
            &log_path.with_extension("pipeline.jsonl"),
        )?)
    } else {
        None
    };
    if let Some(trace) = &trace {
        trace.record(serde_json::json!({
            "kind": "session",
            "speakers_swapped": gui.as_ref().is_some_and(|g| g.control.speakers_swapped.load(Ordering::Relaxed)),
            "endpoint": gui.as_ref().map(|g|&g.endpoint),
            "mapping": gui.as_ref().map(|g| *g.control.mapping.lock().unwrap()),
            "output_rate": 44100,
            "queue_blocks_max": BLOCK_LIMIT,
            "queue_budget_ms": AUDIO_BUDGET_MS,
            "buffer_ms": buffer_ms,
            "latency_ms": latency_ms,
        }));
    }
    let mut audio = AudioPipeline::new(
        PipelineOptions {
            gui: gui.as_ref(),
            stereo: peer.is_some(),
            seed: stamp as u64,
            protocol: &protocol,
            progress: &progress,
            capture_diagnostics,
        },
        sender,
        counters.clone(),
        trace,
        drift_log,
    );
    let mut process_audio = |samples: &[f32], rate: u32| audio.process(samples, rate);
    let mut capture_result = if let Some(g) = &gui {
        g.source.consume(
            stop,
            &progress,
            if capture_diagnostics {
                Some(&mut packet_record)
            } else {
                None
            },
            &mut process_audio,
        )
    } else {
        capture::live_selected_diagnosed(
            root,
            seconds,
            stop,
            endpoint,
            None,
            (detailed_logs || capture_diagnostics).then_some(&progress),
            if capture_diagnostics {
                Some(&mut packet_record)
            } else {
                None
            },
            None,
            &mut process_audio,
        )
    };
    // 正常停止排出滤波器尾帧；发生采集/发送故障时停止继续产出，再关闭唯一发送端。
    // Drain filter tails on normal stop; after capture/delivery failure, stop producing and close the sole sender.
    if capture_result.is_ok() {
        if let Err(err) = audio.finish_audio() {
            capture_result = Err(err);
        }
    }
    audio.close_input();
    let captured_frames = audio.captured_frames();
    if let Some(log) = &packet_log {
        log.record(serde_json::json!({
            "kind": "capture_end",
            "capture_frames": captured_frames,
            "error": capture_result.as_ref().err().map(ToString::to_string),
            "stopped_by_user": stop.load(Ordering::Relaxed),
        }));
    }
    // EOF 让写线程排空已接受的 PCM，后端完成协议收尾；后端退出后再等待读写线程。
    // 超过 15 秒先终止并回收后端，再等待线程，防止等待仍未关闭的管道。
    // EOF drains accepted PCM and requests backend teardown; join workers after the backend exits.
    // After 15 seconds, kill/reap the backend before joining so open pipes cannot keep workers waiting.
    let closing = Instant::now();
    let status = loop {
        if let Some(status) = backend.try_wait()? {
            break status;
        }
        if closing.elapsed() > Duration::from_secs(15) {
            backend.kill()?;
            backend.wait()?;
            let _ = writer.join();
            reader.join();
            return Err("后端停止超过 15 秒，已终止；请查看日志".into());
        }
        thread::sleep(Duration::from_millis(20));
    };
    let written = writer.join()?;
    reader.join();
    // 后端先按自己的期限退出，再给所有诊断线程合计 250 ms；不让慢日志拖延回收后端。
    // Reap the backend first, then allow all diagnostics a shared 250 ms; slow logs cannot delay backend cleanup.
    audio.close_logs(&capture_result, stop.load(Ordering::Relaxed));
    if let Some(log) = &mut packet_log {
        log.close();
    }
    let log_deadline = Instant::now() + Duration::from_millis(250);
    let trace_status = audio.finish_trace(log_deadline);
    let drift_log_status = audio.finish_drift(log_deadline);
    let packet_log_status = packet_log.take().map(|log| log.finish_until(log_deadline));
    let protocol_log_status = reader.finish_log(log_deadline);
    if let Some(g) = &gui {
        if capture_diagnostics {
            (g.emit)(serde_json::json!({"kind":"diagnostic_end","status":packet_log_status}));
        }
    }
    let backend_error = protocol.failure.lock().unwrap().clone();
    let report = serde_json::json!({
        "device": name,
        "requested_seconds": seconds,
        "peer_device": peer.map(|d| &d.name),
        "shared_ptp": peer.is_some(),
        "speakers_swapped": gui.as_ref().is_some_and(|g| g.control.speakers_swapped.load(Ordering::Relaxed)),
        "member_packet_stats": *protocol.member_packets.lock().unwrap(),
        "capture": capture_result.as_ref().ok(),
        "capture_error": capture_result.as_ref().err().map(|e| e.to_string()),
        "conversion": audio.conversion_stats(),
        "pipe_bytes": written.as_ref().ok(),
        "pipe_error": written.as_ref().err().map(|e| crate::failure::describe(&format!("PCM 发送管道：{e}"), "PCM_PIPE_FAILED")),
        "queue_blocks_max": BLOCK_LIMIT,
        "queue_budget_ms": AUDIO_BUDGET_MS,
        "pcm_queue_peak_ms": counters.queue.peak_ms(),
        "pcm_buffers_created": counters.buffers_created.load(Ordering::Relaxed),
        "pcm_buffers_reused": counters.buffers_reused.load(Ordering::Relaxed),
        "clock": *protocol.clock.lock().unwrap(),
        "requested_latency_ms": latency_ms,
        "requested_buffer_ms": buffer_ms,
        "backend_exit": status.code(),
        "transport_ok": protocol.transport.load(Ordering::Relaxed),
        "backend_error": backend_error,
        "last_event_stats": *protocol.event.lock().unwrap(),
        "drift_correction": true,
        "drift_controller": audio.controller(),
        "last_packet_stats": *protocol.packet.lock().unwrap(),
        "last_volume_marker": *protocol.volume.lock().unwrap(),
        "volume_log": log_path.with_extension("volume.log"),
        "peer_volume_log": peer.map(|_| log_path.with_extension("peer.volume.log")),
        "delay_estimate_scope": "sender pipeline + protocol lead; excludes application/physical rendering/acoustic delay",
        "capture_log_mode": "rolling_4_segments_8MiB",
        "capture_diagnostics": capture_diagnostics,
        "capture_trace": if capture_diagnostics { Some(diagnostic_path) } else { None },
        "capture_trace_status": packet_log_status,
        "detailed_logs": detailed_logs,
        "pipeline_trace": if detailed_logs || capture_diagnostics { Some(log_path.with_extension("pipeline.jsonl")) } else { None },
        "pipeline_trace_status": trace_status,
        "drift_trace": if detailed_logs { Some(log_path.with_extension("drift.jsonl")) } else { None },
        "drift_trace_status": drift_log_status,
        "protocol_log_status": protocol_log_status,
    });
    let report_path = log_path.with_extension("json");
    let report = privacy.value(&report);
    let primary = report::session_result(
        backend_error,
        capture_result.map(|_| ()),
        written.map(|_| ()).map_err(|e| {
            crate::failure::describe(&format!("PCM 发送管道：{e}"), "PCM_PIPE_FAILED").into()
        }),
        status.success() && protocol.transport.load(Ordering::Relaxed),
        &log_path,
    );
    report::finish(&report_path, report, primary, gui.as_ref().map(|g| &g.emit))?;
    println!("持续流发送完成；请核对 HomePod 内容、连续性和停止是否正常。");
    Ok(())
}

#[cfg(test)]
#[path = "../../test/core/unit/live/hardware.rs"]
mod tests;
