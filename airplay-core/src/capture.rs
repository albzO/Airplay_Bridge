//! Windows WASAPI 共享模式采集，兼容录音输入及播放端点 loopback。
//! enumerate 读取端点和格式；实时采集将来源声道映射为交错 float32 立体声，
//! 后续重采样和网络发送见 convert/live，持续线程与预览生命周期见 source。
//! 所有 COM 和系统分配资源用 Drop 配对释放；取得音频包后必须 ReleaseBuffer，
//! 即使转换途中返回错误也不能跳过，否则设备缓冲区会被占住。
//!
//! Windows WASAPI shared-mode capture for recording inputs and playback loopback.
//! enumerate reads endpoints/formats; live capture maps source channels to interleaved
//! float32 stereo. See convert/live for resampling and transport, source for preview lifetime.
//! Drop pairs COM/system allocations with cleanup. Every acquired packet needs ReleaseBuffer,
//! including error paths, or the device buffer remains occupied.
mod clock;
mod decode;
mod diagnostics;
mod health;
mod metrics;
mod state;
mod timeline;
use decode::{decode_stereo, raw_packet_summary, sample};
use diagnostics::{PacketTrace, Trace};
pub(crate) use health::LoopbackStalled;
use health::{LoopbackHealth, PlaybackMonitor, check_loopback_health};
use metrics::PacketMetrics;
use serde::{Deserialize, Serialize};
use state::{Counters, PacketInfo, Progress};
use std::{
    error::Error,
    fs::{self, File},
    io::{Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use timeline::Timeline;
#[cfg(test)]
use timeline::{recoverable_gap, validate_packet_timestamp};
use windows::{
    Win32::{
        Foundation::{CloseHandle, HANDLE, PROPERTYKEY, WAIT_OBJECT_0, WAIT_TIMEOUT},
        Media::Audio::*,
        System::Com::{
            CLSCTX_ALL, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree,
            CoUninitialize, STGM_READ,
            StructuredStorage::{PROPVARIANT, PropVariantClear, PropVariantToStringAlloc},
        },
        System::Threading::{CreateEventW, WaitForSingleObject},
    },
    core::{GUID, PWSTR},
};

type Result<T> = std::result::Result<T, Box<dyn Error>>;
struct CaptureEvent(HANDLE);
impl CaptureEvent {
    fn new() -> Result<Self> {
        Ok(Self(unsafe { CreateEventW(None, false, false, None)? }))
    }
    fn wait(&self, timeout_ms: u32) -> Result<bool> {
        match unsafe { WaitForSingleObject(self.0, timeout_ms) } {
            WAIT_OBJECT_0 => Ok(true),
            WAIT_TIMEOUT => Ok(false),
            _ => Err(windows::core::Error::from_thread().into()),
        }
    }
}
impl Drop for CaptureEvent {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}
struct Com;
impl Com {
    fn open() -> Result<Self> {
        unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()? };
        Ok(Self)
    }
}
impl Drop for Com {
    fn drop(&mut self) {
        unsafe { CoUninitialize() };
    }
}
struct Wide(PWSTR);
impl Wide {
    fn text(&self) -> Result<String> {
        Ok(unsafe { self.0.to_string()? })
    }
}
impl Drop for Wide {
    fn drop(&mut self) {
        unsafe { CoTaskMemFree(Some(self.0.0.cast())) };
    }
}
struct Property(PROPVARIANT);
impl Drop for Property {
    fn drop(&mut self) {
        unsafe {
            let _ = PropVariantClear(&mut self.0);
        }
    }
}
struct Mix(*mut WAVEFORMATEX);
impl Drop for Mix {
    fn drop(&mut self) {
        unsafe { CoTaskMemFree(Some(self.0.cast())) };
    }
}
struct Running<'a>(&'a IAudioClient);
impl Drop for Running<'_> {
    fn drop(&mut self) {
        unsafe {
            let _ = self.0.Stop();
        }
    }
}
/// 音频包借用守卫。显式 release 先清空 frames，防止 Drop 重复释放；
/// 中途出错时 Drop 自动归还未释放包，不转移 WASAPI 提供的底层指针所有权。
/// Packet borrow guard: explicit release clears frames first to prevent a second release in Drop.
/// Drop returns unreleased packets on errors; ownership of WASAPI's raw pointer never transfers.
struct Packet<'a> {
    client: &'a IAudioCaptureClient,
    frames: u32,
}
impl Packet<'_> {
    fn release(mut self) -> Result<()> {
        let frames = std::mem::replace(&mut self.frames, 0);
        unsafe {
            self.client.ReleaseBuffer(frames)?;
        }
        Ok(())
    }
}
impl Drop for Packet<'_> {
    fn drop(&mut self) {
        if self.frames != 0 {
            unsafe {
                let _ = self.client.ReleaseBuffer(self.frames);
            }
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Input {
    pub id: String,
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub channels: Option<u16>,
    #[serde(default)]
    pub rate: Option<u32>,
    #[serde(default)]
    pub encoding: Option<String>,
    #[serde(default)]
    pub device_format: Option<Format>,
    #[serde(default)]
    pub mix_format: Option<Format>,
    #[serde(default = "recording_flow")]
    pub flow: String,
}
fn recording_flow() -> String {
    "recording".into()
}
fn property(device: &IMMDevice, pid: u32) -> Result<String> {
    let key = PROPERTYKEY {
        fmtid: GUID::from_u128(0xa45c254e_df1c_4efd_8020_67d146a850e0),
        pid,
    };
    let store = unsafe { device.OpenPropertyStore(STGM_READ)? };
    let value = Property(unsafe { store.GetValue(&key)? });
    let string = Wide(unsafe { PropVariantToStringAlloc(&value.0)? });
    string.text()
}
fn inputs(enumerator: &IMMDeviceEnumerator) -> Result<Vec<(Input, IMMDevice)>> {
    let mut result = Vec::new();
    for (direction, flow) in [(eCapture, "recording"), (eRender, "playback")] {
        let devices = unsafe { enumerator.EnumAudioEndpoints(direction, DEVICE_STATE_ACTIVE)? };
        for index in 0..unsafe { devices.GetCount()? } {
            let device = unsafe { devices.Item(index)? };
            let id = Wide(unsafe { device.GetId()? }).text()?;
            let description = property(&device, 2).unwrap_or_default();
            let name = property(&device, 14)
                .ok()
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| description.clone());
            let format = (|| -> Result<Format> {
                let client: IAudioClient = unsafe { device.Activate(CLSCTX_ALL, None)? };
                let mix = Mix(unsafe { client.GetMixFormat()? });
                if mix.0.is_null() {
                    return Err("空音频格式".into());
                }
                let blob = unsafe {
                    std::slice::from_raw_parts(mix.0.cast::<u8>(), 18 + (*mix.0).cbSize as usize)
                };
                parse_format(blob)
            })()
            .ok();
            let device_format = (|| -> Result<Format> {
                let store = unsafe { device.OpenPropertyStore(STGM_READ)? };
                let value = Property(unsafe { store.GetValue(&PKEY_AudioEngine_DeviceFormat)? });
                let value = unsafe { &value.0.Anonymous.Anonymous };
                if value.vt.0 != 65 {
                    return Err("设备格式不是 VT_BLOB".into());
                }
                let blob = unsafe { value.Anonymous.blob };
                if blob.pBlobData.is_null() || !(18..=4096).contains(&blob.cbSize) {
                    return Err("设备格式为空或长度无效".into());
                }
                parse_format(unsafe {
                    std::slice::from_raw_parts(blob.pBlobData, blob.cbSize as usize)
                })
            })()
            .ok();
            result.push((
                Input {
                    id,
                    name,
                    description,
                    channels: format.as_ref().map(|f| f.channels),
                    rate: format.as_ref().map(|f| f.rate),
                    encoding: format.as_ref().map(|f| f.encoding.clone()),
                    mix_format: format,
                    device_format,
                    flow: flow.into(),
                },
                device,
            ));
        }
    }
    Ok(result)
}
/// Return the Windows default endpoint, independent of vendor/display name.
pub fn default_endpoint(flow: &str) -> Result<String> {
    let _com = Com::open()?;
    let enumerator: IMMDeviceEnumerator =
        unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)? };
    let direction = match flow {
        "playback" => eRender,
        "recording" => eCapture,
        _ => return Err("未知音频来源类型".into()),
    };
    let device = unsafe { enumerator.GetDefaultAudioEndpoint(direction, eConsole)? };
    Wide(unsafe { device.GetId()? }).text()
}
pub fn list(root: &Path) -> Result<()> {
    let _com = Com::open()?;
    let enumerator: IMMDeviceEnumerator =
        unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)? };
    let devices = inputs(&enumerator)?;
    let records: Vec<_> = devices.into_iter().map(|(input, _)| input).collect();
    for input in &records {
        println!("{}\n  ID: {}", input.name, input.id);
    }
    fs::write(
        root.join("audio-inputs.json"),
        serde_json::to_vec_pretty(&records)?,
    )?;
    Ok(())
}
pub fn enumerate() -> Result<Vec<Input>> {
    let _com = Com::open()?;
    let enumerator: IMMDeviceEnumerator =
        unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)? };
    Ok(inputs(&enumerator)?
        .into_iter()
        .map(|(input, _)| input)
        .collect())
}

#[derive(Clone, Serialize, Deserialize)]
/// 原始设备格式。bits 是每采样存储位宽，valid_bits 是有效位数，
/// block_align 是一帧全部声道的字节数；不可把声道数或位宽直接当成帧数。
/// Device format: bits is storage width, valid_bits is meaningful precision, and block_align
/// counts bytes for all channels in one frame. Neither channel count nor bit width is a frame count.
pub struct Format {
    pub(crate) rate: u32,
    pub(crate) channels: u16,
    bits: u16,
    #[serde(default)]
    valid_bits: u16,
    block_align: u16,
    pub(crate) encoding: String,
    channel_mask: u32,
}
pub(crate) fn parse_format(blob: &[u8]) -> Result<Format> {
    if blob.len() < 18 {
        return Err("WASAPI 格式头不完整".into());
    }
    let u16_at = |i| u16::from_le_bytes(blob[i..i + 2].try_into().unwrap());
    let u32_at = |i| u32::from_le_bytes(blob[i..i + 4].try_into().unwrap());
    let tag = u16_at(0);
    let mut subtype = tag;
    let mut mask = 0;
    let mut valid_bits = u16_at(14);
    if tag == 0xfffe {
        if blob.len() < 40 || u16_at(16) < 22 {
            return Err("WAVEFORMATEXTENSIBLE 不完整".into());
        }
        mask = u32_at(20);
        valid_bits = u16_at(18);
        if valid_bits == 0 {
            valid_bits = u16_at(14);
        }
        let guid = unsafe { std::ptr::read_unaligned(blob[24..40].as_ptr().cast::<GUID>()) };
        subtype = if guid == GUID::from_u128(0x00000001_0000_0010_8000_00aa00389b71) {
            1
        } else if guid == GUID::from_u128(0x00000003_0000_0010_8000_00aa00389b71) {
            3
        } else {
            return Err(format!("不支持的音频子格式：{guid:?}").into());
        };
    }
    let bits = u16_at(14);
    let encoding = match (subtype, bits) {
        (3, 32) => "float32",
        (1, 8 | 16 | 24 | 32) => "pcm",
        _ => return Err(format!("暂不支持格式 tag={subtype} bits={bits}").into()),
    };
    let format = Format {
        rate: u32_at(4),
        channels: u16_at(2),
        bits,
        valid_bits,
        block_align: u16_at(12),
        encoding: encoding.into(),
        channel_mask: mask,
    };
    if format.rate == 0
        || format.channels == 0
        || format.block_align != format.channels * (bits / 8)
    {
        return Err("WASAPI 格式的采样率、声道或帧大小无效".into());
    }
    Ok(format)
}
fn db(value: f64) -> String {
    if value > 0.0 {
        format!("{:.1} dBFS", 20.0 * value.log10())
    } else {
        "-inf dBFS".into()
    }
}

fn open_source(
    root: &Path,
    endpoint: Option<&str>,
) -> Result<(
    Com,
    IAudioClient,
    IAudioCaptureClient,
    Format,
    Vec<u8>,
    Input,
)> {
    open_source_with_event(root, endpoint, None)
}
fn open_source_with_event(
    _root: &Path,
    endpoint: Option<&str>,
    event: Option<HANDLE>,
) -> Result<(
    Com,
    IAudioClient,
    IAudioCaptureClient,
    Format,
    Vec<u8>,
    Input,
)> {
    let _com = Com::open()?;
    let enumerator: IMMDeviceEnumerator =
        unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)? };
    let default_id = if endpoint.is_none() {
        Some(default_endpoint("playback")?)
    } else {
        None
    };
    let selected = endpoint.or(default_id.as_deref()).ok_or("未选择音频来源")?;
    let candidates: Vec<_> = inputs(&enumerator)?
        .into_iter()
        .filter(|(input, _)| input.id == selected)
        .collect();
    if candidates.len() != 1 {
        return Err(format!("找到 {} 个匹配的音频端点，请核对来源选择", candidates.len()).into());
    }
    let (input, device) = &candidates[0];
    let client: IAudioClient = unsafe { device.Activate(CLSCTX_ALL, None)? };
    let mix = Mix(unsafe { client.GetMixFormat()? });
    if mix.0.is_null() {
        return Err("GetMixFormat 返回空指针".into());
    }
    let extra = unsafe { (*mix.0).cbSize } as usize;
    let blob = unsafe { std::slice::from_raw_parts(mix.0.cast::<u8>(), 18 + extra) }.to_vec();
    let format = parse_format(&blob)?;
    println!("采集端点：{}\nID：{}", input.name, input.id);
    println!(
        "WASAPI 共享模式：{} Hz / {} 声道 / {} 位 {}，声道掩码 0x{:x}",
        format.rate, format.channels, format.bits, format.encoding, format.channel_mask
    );
    unsafe {
        let flags = if input.flow == "playback" {
            AUDCLNT_STREAMFLAGS_LOOPBACK
                | if event.is_some() {
                    AUDCLNT_STREAMFLAGS_EVENTCALLBACK
                } else {
                    0
                }
        } else {
            0
        };
        client.Initialize(AUDCLNT_SHAREMODE_SHARED, flags, 1_000_000, 0, mix.0, None)?;
        if input.flow == "playback" {
            if let Some(event) = event {
                client.SetEventHandle(event)?;
            }
        }
    }
    let capture: IAudioCaptureClient = unsafe { client.GetService()? };
    Ok((_com, client, capture, format, blob, input.clone()))
}

/// Capture callback runs after releasing the WASAPI buffer. It must never block
/// on the network; the live bridge uses try_send into a bounded queue.
pub fn live(
    root: &Path,
    seconds: u64,
    stop: &std::sync::atomic::AtomicBool,
    sink: impl FnMut(&[f32], u32) -> Result<()>,
) -> Result<serde_json::Value> {
    live_selected(root, seconds, stop, None, None, sink)
}
pub fn live_selected(
    root: &Path,
    seconds: u64,
    stop: &std::sync::atomic::AtomicBool,
    endpoint: Option<&str>,
    mapping: Option<&std::sync::Mutex<[usize; 2]>>,
    sink: impl FnMut(&[f32], u32) -> Result<()>,
) -> Result<serde_json::Value> {
    live_selected_traced(root, seconds, stop, endpoint, mapping, None, sink)
}

#[derive(Default, Clone, Copy, Serialize)]
pub struct CaptureProgress {
    signal_frames: u64,
    packet_peaks: [f32; 2],
    windows_endpoint_peak: Option<f32>,
    windows_endpoint_peak_age_ms: Option<f64>,
    windows_endpoint_muted: Option<bool>,
    windows_endpoint_volume: Option<f32>,
    pub(crate) loopback_suspect: bool,
    packets: u64,
    event_timeout_packets: u64,
    synthesized_silent_frames: u64,
    skipped_overlap_frames: u64,
    device_position: u64,
    packet_qpc_100ns: u64,
    packet_frames: u32,
    packet_flags: u32,
    pub discontinuities: u64,
    pub timestamp_errors: u64,
    pub repaired_gaps: u64,
    pub repaired_gap_frames: u64,
}

pub fn live_selected_traced(
    root: &Path,
    seconds: u64,
    stop: &std::sync::atomic::AtomicBool,
    endpoint: Option<&str>,
    mapping: Option<&std::sync::Mutex<[usize; 2]>>,
    progress: Option<&std::sync::Mutex<CaptureProgress>>,
    sink: impl FnMut(&[f32], u32) -> Result<()>,
) -> Result<serde_json::Value> {
    live_selected_diagnosed(
        root, seconds, stop, endpoint, mapping, progress, None, None, sink,
    )
}
pub fn live_selected_diagnosed(
    root: &Path,
    seconds: u64,
    stop: &std::sync::atomic::AtomicBool,
    endpoint: Option<&str>,
    mapping: Option<&std::sync::Mutex<[usize; 2]>>,
    progress: Option<&std::sync::Mutex<CaptureProgress>>,
    diagnostic: Option<&mut dyn FnMut(serde_json::Value)>,
    diagnostic_enabled: Option<&std::sync::atomic::AtomicBool>,
    mut sink: impl FnMut(&[f32], u32) -> Result<()>,
) -> Result<serde_json::Value> {
    // Created before the client so it stays valid until after client release.
    let capture_event = CaptureEvent::new()?;
    let (_com, client, capture, format, _blob, input) =
        open_source_with_event(root, endpoint, Some(capture_event.0))?;
    let mut monitor = if progress.is_some() && input.flow == "playback" {
        PlaybackMonitor::open(&input.id).ok()
    } else {
        None
    };
    let mut loopback_health = LoopbackHealth::default();
    let target = if seconds == 0 {
        u64::MAX
    } else {
        seconds * format.rate as u64
    };
    let mut counters = Counters::default();
    let progress_state = Progress(progress);
    let mut trace = Trace::new(diagnostic, diagnostic_enabled);
    let mut buffer = Vec::<f32>::with_capacity(2048);
    let loopback = input.flow == "playback";
    let mut timeline = Timeline::new(format.rate, target, loopback, clock::origin(loopback)?);
    unsafe {
        client.Start()?;
    }
    let _running = Running(&client);
    let started = Instant::now();
    trace.start(&input, &format, &client, loopback)?;
    let mut last_packet = Instant::now();
    let mut loopback_ready = false;
    if seconds == 0 {
        println!("音频持续采集已开始；Ctrl+C 可正常停止。");
    } else {
        println!("音频持续采集已开始，最多 {seconds} 秒；Ctrl+C 可正常停止。");
    }
    while counters.frames < target && !stop.load(std::sync::atomic::Ordering::Relaxed) {
        if seconds != 0 && started.elapsed() > Duration::from_secs(seconds + 2) {
            return Err("持续采集未在期限内收到完整音频".into());
        }
        // On a ready event drain every available packet before waiting again.
        // Events are the primary wakeup. On timeout, check whether the driver
        // has queued a packet without signalling; only a nonzero size permits
        // GetBuffer. This also avoids inserting idle silence over queued audio.
        if loopback && !loopback_ready {
            let idle_target = timeline.idle_target(started.elapsed());
            // Catch up idle silence without paying another wait per 10 ms block.
            // Still check the event first so resumed audio wins over silence.
            let timeout = if last_packet.elapsed() >= Duration::from_millis(40)
                && counters.frames < idle_target.min(target)
            {
                0
            } else {
                20
            };
            loopback_ready = capture_event.wait(timeout)?;
        }
        let next_packet = unsafe { capture.GetNextPacketSize()? };
        if loopback && !loopback_ready && next_packet > 0 {
            loopback_ready = true;
            progress_state.fallback();
            trace.fallback(started.elapsed().as_secs_f64() * 1000.0, next_packet);
        }
        if next_packet == 0 {
            loopback_ready = false;
            // Render engines can stop supplying packets when no app is playing.
            // Keep the sender clock running with intentional silence, leaving
            // 40 ms for engine delivery. Packet QPC timestamps trim overlap
            // when playback resumes so silence cannot duplicate real frames.
            if loopback {
                if last_packet.elapsed() < Duration::from_millis(40) {
                    continue;
                }
                let count =
                    timeline.idle_frames(counters.frames, started.elapsed(), last_packet.elapsed());
                if count > 0 {
                    buffer.clear();
                    buffer.resize(count as usize * 2, 0.0);
                    progress_state.silence(count);
                    check_loopback_health(&mut monitor, &mut loopback_health, progress, false)?;
                    let delivered = sink(&buffer, format.rate);
                    trace.silence(
                        started.elapsed().as_secs_f64() * 1000.0,
                        counters.frames,
                        count,
                        last_packet.elapsed().as_secs_f64() * 1000.0,
                        &delivered,
                    );
                    delivered?;
                    counters.delivered_silence(count);
                    timeline.mark_silence();
                    continue;
                }
                continue;
            }
            if last_packet.elapsed() > Duration::from_secs(1) {
                return Err("采集设备超过 1 秒没有音频包".into());
            }
            thread::sleep(Duration::from_millis(2));
            continue;
        }
        let mut pointer = std::ptr::null_mut();
        let packet_read_started = Instant::now();
        let mut available = 0;
        let mut flags = 0;
        let mut qpc = 0u64;
        let mut device_position = 0u64;
        unsafe {
            capture.GetBuffer(
                &mut pointer,
                &mut available,
                &mut flags,
                Some(&mut device_position),
                Some(&mut qpc),
            )?;
        }
        // AUDCLNT_S_BUFFER_EMPTY is a successful HRESULT with zero frames.
        // Do not validate its unset timestamp or release a nonexistent packet.
        if available == 0 {
            loopback_ready = false;
            continue;
        }
        let info = PacketInfo {
            index: counters.packets,
            device_position,
            qpc,
            available,
            flags,
        };
        trace.flags(info, started.elapsed().as_secs_f64() * 1000.0);
        let packet = Packet {
            client: &capture,
            frames: available,
        };
        let previous_qpc = timeline.previous_qpc();
        let plan = match timeline.plan(
            counters.frames,
            device_position,
            available,
            qpc,
            flags & AUDCLNT_BUFFERFLAGS_TIMESTAMP_ERROR.0 as u32 != 0,
            started.elapsed(),
            Instant::now(),
        ) {
            Ok(plan) => plan,
            Err(error) => {
                trace.fault(info, previous_qpc, error.as_ref());
                return Err(error);
            }
        };
        let timeline::PacketPlan {
            skip,
            used,
            mut prefix_silence,
            repaired_gap,
        } = plan;
        if repaired_gap > 0 {
            progress_state.gap(repaired_gap);
            trace.gap(info, repaired_gap, format.rate);
        }
        let mut raw_summary = serde_json::Value::Null;
        let mut raw_nonzero = false;
        buffer.clear();
        if flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 != 0 {
            counters.silent_packets += 1;
            buffer.resize(used * 2, 0.0);
        } else {
            if pointer.is_null() {
                return Err("采集缓冲区为空".into());
            }
            let bytes = unsafe {
                std::slice::from_raw_parts(
                    pointer,
                    available as usize * format.block_align as usize,
                )
            };
            // 判断原始全部声道是否全零，不受 mapping、重采样或后端发送影响。
            // 检查必须发生在 ReleaseBuffer 前；非零字节采用保守策略，避免误重建。
            // Inspect every raw channel independently of mapping/resampling/backend delivery.
            // Inspect before ReleaseBuffer; treat any nonzero byte conservatively to avoid false reopenings.
            if monitor.is_some() {
                raw_nonzero = bytes.iter().any(|byte| *byte != 0);
            }
            if used > 0 {
                if trace.active() {
                    raw_summary = raw_packet_summary(bytes, &format);
                }
                decode_stereo(
                    bytes,
                    &format,
                    skip..skip + used,
                    mapping.map(|m| *m.lock().unwrap()),
                    &mut buffer,
                )?;
            }
        }
        packet.release()?;
        check_loopback_health(&mut monitor, &mut loopback_health, progress, raw_nonzero)?;
        let received = Instant::now();
        let trace_enabled = trace.active();
        let packet_metrics = PacketMetrics::measure(&buffer, trace_enabled);
        let mut diagnostic_entry = trace_enabled.then(|| {
            trace.packet(
                PacketTrace {
                    packet: info,
                    received,
                    elapsed_ms: started.elapsed().as_secs_f64() * 1000.0,
                    used,
                    skipped: skip,
                    prefix_silence,
                    frames_before: counters.frames,
                    raw: raw_summary,
                    mapping: mapping
                        .map(|m| *m.lock().unwrap())
                        .unwrap_or([0, if format.channels > 1 { 1 } else { 0 }]),
                },
                format.rate,
                &packet_metrics,
            )
        });
        Trace::endpoint(
            &mut diagnostic_entry,
            progress_state.packet(info, skip, &packet_metrics),
        );
        while prefix_silence > 0 && !stop.load(std::sync::atomic::Ordering::Relaxed) {
            let count = prefix_silence.min((format.rate / 100).max(1) as u64);
            let silence = vec![0.0; count as usize * 2];
            progress_state.silence(count);
            sink(&silence, format.rate)?;
            counters.delivered_silence(count);
            prefix_silence -= count;
        }
        counters.observe(info, &packet_metrics);
        let sink_started = Instant::now();
        let delivered = sink(&buffer, format.rate);
        trace.complete(
            diagnostic_entry,
            sink_started,
            packet_read_started,
            &delivered,
        );
        delivered?;
        counters.delivered_packet(used);
        last_packet = Instant::now();
    }
    Ok(diagnostics::report(
        &counters,
        &input,
        &format,
        loopback,
        stop.load(std::sync::atomic::Ordering::Relaxed),
    ))
}

pub fn record(root: &Path, seconds: u64, endpoint: Option<&str>) -> Result<PathBuf> {
    let (_com, client, capture, format, blob, _input) = open_source(root, endpoint)?;
    let directory = root.join("captures");
    fs::create_dir_all(&directory)?;
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();
    let path = directory.join(format!("capture-{stamp}.wav"));
    let mut wave = File::create(&path)?;
    wave.write_all(b"RIFF\0\0\0\0WAVEfmt ")?;
    wave.write_all(&(blob.len() as u32).to_le_bytes())?;
    wave.write_all(&blob)?;
    if blob.len() % 2 != 0 {
        wave.write_all(&[0])?;
    }
    // The fact chunk supplies the sample count for non-PCM WAV formats.
    wave.write_all(b"fact\x04\0\0\0")?;
    let fact_offset = wave.stream_position()?;
    wave.write_all(&[0; 4])?;
    wave.write_all(b"data")?;
    let size_offset = wave.stream_position()?;
    wave.write_all(&[0; 4])?;
    let target = seconds * format.rate as u64;
    let mut frames = 0u64;
    let mut packets = 0u64;
    let mut signal_frames = 0u64;
    let mut silent_packets = 0u64;
    let mut discontinuities = 0u64;
    let mut initial_discontinuity = false;
    let mut timestamp_errors = 0u64;
    let mut invalid_samples = 0u64;
    let mut peaks = vec![0f64; format.channels as usize];
    let mut squares = vec![0f64; format.channels as usize];
    unsafe {
        client.Start()?;
    }
    let running = Running(&client);
    let started = Instant::now();
    println!("开始采集 {seconds} 秒；请让所选音频来源持续提供声音。");
    while frames < target && started.elapsed() < Duration::from_secs(seconds + 2) {
        if unsafe { capture.GetNextPacketSize()? } == 0 {
            thread::sleep(Duration::from_millis(5));
            continue;
        }
        let mut pointer = std::ptr::null_mut();
        let mut available = 0;
        let mut flags = 0;
        unsafe {
            capture.GetBuffer(&mut pointer, &mut available, &mut flags, None, None)?;
        }
        let packet = Packet {
            client: &capture,
            frames: available,
        };
        let used = (target - frames).min(available as u64) as usize;
        let size = used * format.block_align as usize;
        let silent = flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 != 0;
        let bytes = if silent {
            silent_packets += 1;
            vec![
                if format.encoding == "pcm" && format.bits == 8 {
                    128
                } else {
                    0
                };
                size
            ]
        } else {
            if pointer.is_null() && size > 0 {
                return Err("非静音缓冲区为空".into());
            }
            if size == 0 {
                Vec::new()
            } else {
                unsafe { std::slice::from_raw_parts(pointer, size) }.to_vec()
            }
        };
        packet.release()?;
        if flags & AUDCLNT_BUFFERFLAGS_DATA_DISCONTINUITY.0 as u32 != 0 {
            if packets == 0 {
                initial_discontinuity = true;
            } else {
                discontinuities += 1;
            }
        }
        if flags & AUDCLNT_BUFFERFLAGS_TIMESTAMP_ERROR.0 as u32 != 0 {
            timestamp_errors += 1;
        }
        for frame in bytes.chunks_exact(format.block_align as usize) {
            let mut signal = false;
            for (channel, raw) in frame.chunks_exact((format.bits / 8) as usize).enumerate() {
                let value = sample(raw, &format);
                if !value.is_finite() {
                    invalid_samples += 1;
                    continue;
                }
                peaks[channel] = peaks[channel].max(value.abs());
                squares[channel] += value * value;
                signal |= value.abs() > 0.000001;
            }
            if signal {
                signal_frames += 1;
            }
        }
        wave.write_all(&bytes)?;
        frames += used as u64;
        packets += 1;
    }
    drop(running);
    let data_size = (frames * format.block_align as u64) as u32;
    if data_size % 2 != 0 {
        wave.write_all(&[0])?;
    }
    let end = wave.stream_position()?;
    wave.seek(SeekFrom::Start(fact_offset))?;
    wave.write_all(&(frames as u32).to_le_bytes())?;
    wave.seek(SeekFrom::Start(size_offset))?;
    wave.write_all(&data_size.to_le_bytes())?;
    wave.seek(SeekFrom::Start(4))?;
    wave.write_all(&((end - 8) as u32).to_le_bytes())?;
    wave.flush()?;
    let report = serde_json::json!({ "endpoint": _input, "mode": "shared", "format": format,
        "requested_seconds": seconds, "frames": frames, "packets": packets, "signal_frames": signal_frames,
        "silent_packets": silent_packets, "initial_discontinuity": initial_discontinuity,
        "discontinuities": discontinuities, "timestamp_errors": timestamp_errors,
        "invalid_samples": invalid_samples, "channel_peaks": peaks, "wave": path,
        "stream_complete": frames == target });
    fs::write(
        path.with_extension("json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    println!("采集结果：{frames} 帧 / {packets} 包 / {signal_frames} 个非静音帧");
    for (index, peak) in peaks.iter().enumerate() {
        let rms = if frames > 0 {
            (squares[index] / frames as f64).sqrt()
        } else {
            0.0
        };
        println!("声道 {}：峰值 {}，RMS {}", index + 1, db(*peak), db(rms));
    }
    println!(
        "中途不连续标记：{discontinuities}；时间戳错误：{timestamp_errors}；非法样本：{invalid_samples}"
    );
    println!("录音：{}", path.display());
    if frames != target {
        return Err("未在期限内采到完整音频帧，请查看旁边的 JSON 报告".into());
    }
    if signal_frames == 0 {
        println!("共享模式已成功接收音频帧，但全是静音；请确认所选音频来源的路由和电平。");
    } else {
        println!("音频来源共享模式采集通过，检测到音频信号。请试听录音确认内容。 ");
    }
    Ok(path)
}

#[cfg(test)]
#[path = "../../test/core/unit/capture/gap_recovery.rs"]
mod gap_recovery_tests;
#[cfg(test)]
#[path = "../../test/core/unit/capture/loopback.rs"]
mod loopback_checks;
