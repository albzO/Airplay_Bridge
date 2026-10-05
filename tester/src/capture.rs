//! Shared-mode recording endpoint capture and playback endpoint loopback.
use serde::{Deserialize, Serialize};
use std::{
    error::Error,
    fs::{self, File},
    io::{Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use windows::{
    Win32::{
        Foundation::{PROPERTYKEY, HANDLE, CloseHandle, WAIT_OBJECT_0, WAIT_TIMEOUT},
        Media::Audio::*,
        System::Com::{
            CLSCTX_ALL, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree,
            CoUninitialize, STGM_READ,
            StructuredStorage::{PROPVARIANT, PropVariantClear, PropVariantToStringAlloc},
        },
        System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency},
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
    fn drop(&mut self) { unsafe { let _ = CloseHandle(self.0); } }
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
fn is_b3(input: &Input) -> bool {
    [&input.name, &input.description].iter().any(|text| {
        text.split(|c: char| !c.is_ascii_alphanumeric())
            .any(|word| word.eq_ignore_ascii_case("B3"))
    })
}
pub fn list(root: &Path) -> Result<()> {
    let _com = Com::open()?;
    let enumerator: IMMDeviceEnumerator =
        unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)? };
    let devices = inputs(&enumerator)?;
    let records: Vec<_> = devices.into_iter().map(|(input, _)| input).collect();
    for input in &records {
        println!(
            "{}{}\n  ID: {}",
            if is_b3(input) { "[B3] " } else { "" },
            input.name,
            input.id
        );
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
fn sample(bytes: &[u8], format: &Format) -> f64 {
    if format.encoding == "float32" {
        return f32::from_le_bytes(bytes.try_into().unwrap()) as f64;
    }
    match format.bits {
        8 => (bytes[0] as f64 - 128.0) / 128.0,
        16 => i16::from_le_bytes(bytes.try_into().unwrap()) as f64 / 32768.0,
        24 => {
            ((bytes[0] as i32 | (bytes[1] as i32) << 8 | (bytes[2] as i32) << 16) << 8 >> 8) as f64
                / 8388608.0
        }
        32 => i32::from_le_bytes(bytes.try_into().unwrap()) as f64 / 2147483648.0,
        _ => unreachable!(),
    }
}
// Inspect the packet while WASAPI still owns it. Never retain the buffer pointer
// or raw audio in a log; this separates driver data from mapping/decoding.
fn raw_packet_summary(bytes: &[u8], format: &Format) -> serde_json::Value {
    let mut peaks = vec![0.0f64; format.channels as usize];
    let mut nonfinite = 0u64;
    let width = (format.bits / 8) as usize;
    for frame in bytes.chunks_exact(format.block_align as usize) {
        for (channel, peak) in peaks.iter_mut().enumerate() {
            let value = sample(&frame[channel * width..(channel + 1) * width], format);
            if value.is_finite() { *peak = peak.max(value.abs()); } else { nonfinite += 1; }
        }
    }
    serde_json::json!({"channel_peaks":peaks,"nonfinite_samples":nonfinite,
        "nonzero_bytes":bytes.iter().filter(|byte|**byte != 0).count(),"bytes":bytes.len()})
}
fn db(value: f64) -> String {
    if value > 0.0 {
        format!("{:.1} dBFS", 20.0 * value.log10())
    } else {
        "-inf dBFS".into()
    }
}

fn open_b3(
    root: &Path,
) -> Result<(
    Com,
    IAudioClient,
    IAudioCaptureClient,
    Format,
    Vec<u8>,
    Input,
)> {
    open_source(root, None)
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
    root: &Path,
    endpoint: Option<&str>,
    event: Option<HANDLE>,
) -> Result<(Com, IAudioClient, IAudioCaptureClient, Format, Vec<u8>, Input)> {
    let _com = Com::open()?;
    let enumerator: IMMDeviceEnumerator =
        unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)? };
    let candidates: Vec<_> = inputs(&enumerator)?
        .into_iter()
        .filter(|(input, _)| endpoint.map_or_else(|| is_b3(input), |id| input.id == id))
        .collect();
    if candidates.len() != 1 {
        return Err(format!("找到 {} 个匹配的音频端点，请核对来源选择", candidates.len()).into());
    }
    let (input, device) = &candidates[0];
    let config_path = root.join("b3-endpoint.json");
    if endpoint.is_none() && config_path.exists() {
        let previous: Input = serde_json::from_slice(&fs::read(&config_path)?)?;
        if previous.id != input.id {
            return Err(
                "B3 设备 ID 已改变。请核对 audio-devices；确认后移走旧 b3-endpoint.json 再采集"
                    .into(),
            );
        }
    }
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
            AUDCLNT_STREAMFLAGS_LOOPBACK | if event.is_some() { AUDCLNT_STREAMFLAGS_EVENTCALLBACK } else { 0 }
        } else {
            0
        };
        client.Initialize(AUDCLNT_SHAREMODE_SHARED, flags, 1_000_000, 0, mix.0, None)?;
        if input.flow == "playback" {
            if let Some(event) = event { client.SetEventHandle(event)?; }
        }
    }
    let capture: IAudioCaptureClient = unsafe { client.GetService()? };
    if endpoint.is_none() {
        fs::write(&config_path, serde_json::to_vec_pretty(input)?)?;
    }
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
    packets: u64,
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
// Missing device frames are recoverable only when the packet clock agrees.
fn recoverable_gap(
    previous: (u64, u32, u64),
    position: u64,
    qpc: u64,
    rate: u32,
    timestamp_bad: bool,
) -> Result<u64> {
    let expected = previous.0.saturating_add(previous.1 as u64);
    if position < expected {
        return Err("采集设备位置倒退或重复，无法可信对齐音频".into());
    }
    let gap = position - expected;
    if gap == 0 {
        return Ok(0);
    }
    if timestamp_bad || qpc < previous.2 {
        return Err("采集缺口的时间戳无效，无法恢复".into());
    }
    if gap > rate as u64 / 4 {
        return Err("采集缺口超过 250 ms，已停止".into());
    }
    let device_ms = (position - previous.0) as f64 * 1000.0 / rate as f64;
    let clock_ms = (qpc - previous.2) as f64 / 10000.0;
    if (device_ms - clock_ms).abs() > 5.0 {
        return Err("采集设备位置与时间戳不一致，无法恢复缺口".into());
    }
    Ok(gap)
}
fn validate_packet_timestamp(previous: Option<u64>, current: u64, timestamp_bad: bool) -> Result<()> {
    if timestamp_bad { return Err("WASAPI 采集包时间戳无效，已停止以避免错误对齐".into()); }
    if previous.is_some_and(|previous|current<=previous) {
        return Err("WASAPI 采集包时间戳倒退或重复，已停止；请查看采集诊断".into());
    }
    Ok(())
}
#[cfg(test)]
mod gap_recovery_tests {
    use super::*;
    #[test]
    fn capture_ready_event_is_auto_reset_and_idle_wait_is_bounded() {
        let event = CaptureEvent::new().unwrap();
        assert!(!event.wait(0).unwrap());
        unsafe { windows::Win32::System::Threading::SetEvent(event.0).unwrap(); }
        assert!(event.wait(20).unwrap());
        assert!(!event.wait(0).unwrap());
    }
    #[test]
    fn raw_summary_separates_channels_and_nonfinite_float_samples() {
        let format = Format { rate: 48000, channels: 2, bits: 32, valid_bits: 32,
            block_align: 8, channel_mask: 3, encoding: "float32".into() };
        let bytes: Vec<u8> = [0.0f32, 0.75, -0.25, f32::NAN]
            .into_iter().flat_map(f32::to_le_bytes).collect();
        let summary = raw_packet_summary(&bytes, &format);
        assert_eq!(summary["channel_peaks"], serde_json::json!([0.25, 0.75]));
        assert_eq!(summary["nonfinite_samples"], 1);
        assert_eq!(summary["bytes"], 16);
        // Negative float zero has nonzero raw bytes but no audio signal.
        let negative_zero: Vec<u8> = [-0.0f32, 0.0].into_iter().flat_map(f32::to_le_bytes).collect();
        let summary = raw_packet_summary(&negative_zero, &format);
        assert_eq!(summary["channel_peaks"], serde_json::json!([0.0, 0.0]));
        assert_eq!(summary["nonzero_bytes"], 1);
    }
    #[test]
    fn short_gap_requires_consistent_clock_and_rejects_bad_timeline() {
        let previous = (0, 480, 1_000_000);
        assert_eq!(
            recoverable_gap(previous, 5760, 2_200_000, 48000, false).unwrap(),
            5280
        );
        assert_eq!(
            recoverable_gap(previous, 480, 1_100_000, 48000, false).unwrap(),
            0
        );
        assert!(recoverable_gap(previous, 5760, 1_100_000, 48000, false).is_err());
        assert!(recoverable_gap(previous, 5760, 2_200_000, 48000, true).is_err());
        assert!(recoverable_gap(previous, 0, 1_100_000, 48000, false).is_err());
        assert!(recoverable_gap(previous, 14400, 4_000_000, 48000, false).is_err());
    }
    #[test]
    fn timestamp_regression_is_rejected_even_without_error_flag() {
        assert!(validate_packet_timestamp(Some(2_000_000),1_844_185,false).is_err());
        assert!(validate_packet_timestamp(Some(2_000_000),2_000_000,false).is_err());
        assert!(validate_packet_timestamp(None,2_000_000,true).is_err());
        assert!(validate_packet_timestamp(Some(2_000_000),2_100_000,false).is_ok());
    }
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
    mut diagnostic: Option<&mut dyn FnMut(serde_json::Value)>,
    diagnostic_enabled: Option<&std::sync::atomic::AtomicBool>,
    mut sink: impl FnMut(&[f32], u32) -> Result<()>,
) -> Result<serde_json::Value> {
    // Created before the client so it stays valid until after client release.
    let capture_event = CaptureEvent::new()?;
    let (_com, client, capture, format, _blob, input) =
        open_source_with_event(root, endpoint, Some(capture_event.0))?;
    let meter = if progress.is_some() && input.flow == "playback" {
        (|| -> Result<windows::Win32::Media::Audio::Endpoints::IAudioMeterInformation> {
            let enumerator: IMMDeviceEnumerator =
                unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)? };
            let wide: Vec<u16> = input.id.encode_utf16().chain(Some(0)).collect();
            let device = unsafe { enumerator.GetDevice(windows::core::PCWSTR(wide.as_ptr()))? };
            Ok(unsafe { device.Activate(CLSCTX_ALL, None)? })
        })()
        .ok()
    } else {
        None
    };
    let mut meter_updated: Option<Instant> = None;
    let target = if seconds == 0 {
        u64::MAX
    } else {
        seconds * format.rate as u64
    };
    let mut frames = 0u64;
    let mut packets = 0u64;
    let mut discontinuities = 0u64;
    let mut timestamp_errors = 0u64;
    let mut signal_frames = 0u64;
    let mut channel_peaks = [0f32; 2];
    let mut silent_packets = 0u64;
    let mut buffer = Vec::<f32>::with_capacity(2048);
    let loopback = input.flow == "playback";
    let mut frequency = 0i64;
    let mut origin = 0i64;
    if loopback {
        unsafe {
            QueryPerformanceFrequency(&mut frequency)?;
            QueryPerformanceCounter(&mut origin)?;
        }
    }
    let origin_100ns = if loopback {
        origin as u128 * 10_000_000 / frequency as u128
    } else {
        0
    };
    let mut silent_frames = 0u64;
    let mut timeline_silence = false;
    unsafe {
        client.Start()?;
    }
    let _running = Running(&client);
    let started = Instant::now();
    if let Some(log) = diagnostic.as_mut() {
        let mut diagnostic_frequency = 0i64;
        unsafe {
            QueryPerformanceFrequency(&mut diagnostic_frequency)?;
        }
        log(
            serde_json::json!({"kind":"capture_start","input":input,"format":format,"qpc_frequency":diagnostic_frequency,
                "capture_wait":if loopback {"wasapi_event"}else{"polling"},
                "endpoint_buffer_frames":unsafe {client.GetBufferSize().ok()}}),
        );
    }
    let mut previous_packet: Option<(Instant, u64, u64)> = None;
    let mut previous_device: Option<(u64, u32, u64)> = None;
    let mut repairs = std::collections::VecDeque::<(Instant, u64)>::new();
    let diagnostics_active =
        || diagnostic_enabled.is_none_or(|flag| flag.load(std::sync::atomic::Ordering::Relaxed));
    let mut last_packet = Instant::now();
    let mut loopback_ready = false;
    if seconds == 0 {
        println!("音频持续采集已开始；Ctrl+C 可正常停止。");
    } else {
        println!("音频持续采集已开始，最多 {seconds} 秒；Ctrl+C 可正常停止。");
    }
    while frames < target && !stop.load(std::sync::atomic::Ordering::Relaxed) {
        if seconds != 0 && started.elapsed() > Duration::from_secs(seconds + 2) {
            return Err("持续采集未在期限内收到完整音频".into());
        }
        // On a ready event drain every available packet before waiting again.
        // A timeout only drives idle silence; it never authorizes a buffer read.
        if loopback && !loopback_ready {
            let idle_target = ((started.elapsed().as_secs_f64() - 0.04).max(0.0)
                * format.rate as f64) as u64;
            // Catch up idle silence without paying another wait per 10 ms block.
            // Still check the event first so resumed audio wins over silence.
            let timeout = if last_packet.elapsed() >= Duration::from_millis(40)
                && frames < idle_target.min(target) { 0 } else { 20 };
            loopback_ready = capture_event.wait(timeout)?;
        }
        let next_packet = if loopback && !loopback_ready { 0 } else {
            unsafe { capture.GetNextPacketSize()? }
        };
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
                let expected =
                    ((started.elapsed().as_secs_f64() - 0.04).max(0.0) * format.rate as f64) as u64;
                let gap = expected.min(target).saturating_sub(frames);
                if gap > 0 {
                    let count = gap.min((format.rate / 100).max(1) as u64);
                    buffer.clear();
                    buffer.resize(count as usize * 2, 0.0);
                    if let Some(progress) = progress {
                        progress.lock().unwrap().synthesized_silent_frames += count;
                    }
                    let delivered = sink(&buffer, format.rate);
                    if let Some(log) = diagnostic.as_mut() {
                        log(
                            serde_json::json!({"kind":"synthetic_silence","capture_elapsed_ms":started.elapsed().as_secs_f64()*1000.0,"frames_before":frames,"frames":count,"idle_ms":last_packet.elapsed().as_secs_f64()*1000.0,"error":delivered.as_ref().err().map(ToString::to_string)}),
                        );
                    }
                    delivered?;
                    frames += count;
                    silent_frames += count;
                    timeline_silence = true;
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
        if available == 0 { loopback_ready = false; continue; }
        if flags
            & (AUDCLNT_BUFFERFLAGS_TIMESTAMP_ERROR.0 as u32
                | AUDCLNT_BUFFERFLAGS_DATA_DISCONTINUITY.0 as u32)
            != 0
        {
            if let Some(log) = diagnostic.as_mut() {
                log(
                    serde_json::json!({"kind":"packet_flags","packet_index":packets,"capture_elapsed_ms":started.elapsed().as_secs_f64()*1000.0,"device_position":device_position,"packet_qpc_100ns":qpc,"available_frames":available,"flags":flags}),
                );
            }
        }
        let packet = Packet {
            client: &capture,
            frames: available,
        };
        let mut skip = 0usize;
        let mut prefix_silence = 0u64;
        if let Err(error) = validate_packet_timestamp(previous_device.map(|(_,_,qpc)|qpc),qpc,flags & AUDCLNT_BUFFERFLAGS_TIMESTAMP_ERROR.0 as u32 != 0) {
            if let Some(log) = diagnostic.as_mut() {
                log(serde_json::json!({"kind":"timestamp_fault","device_position":device_position,"packet_qpc_100ns":qpc,"previous_packet_qpc_100ns":previous_device.map(|(_,_,qpc)|qpc),"flags":flags,"error":error.to_string()}));
            }
            return Err(error);
        }
        // Playback idle silence already covers elapsed time; QPC alignment below
        // trims that overlap. Real packet gaps use the same recovery as recording.
        if !loopback || !timeline_silence {
            if let Some(previous) = previous_device {
                let gap = recoverable_gap(
                    previous,
                    device_position,
                    qpc,
                    format.rate,
                    flags & AUDCLNT_BUFFERFLAGS_TIMESTAMP_ERROR.0 as u32 != 0,
                )?;
                if gap > 0 {
                    while repairs
                        .front()
                        .is_some_and(|(t, _)| t.elapsed() > Duration::from_secs(60))
                    {
                        repairs.pop_front();
                    }
                    if repairs.len() >= 5
                        || repairs.iter().map(|(_, f)| *f).sum::<u64>() + gap
                            > format.rate as u64 / 2
                    {
                        return Err(
                            "采集缺口频繁发生，60 秒内超过 5 次或累计 500 ms，已停止".into()
                        );
                    }
                    repairs.push_back((Instant::now(), gap));
                    if let Some(progress) = progress {
                        let mut p = progress.lock().unwrap();
                        p.repaired_gaps += 1;
                        p.repaired_gap_frames += gap;
                    }
                    prefix_silence = gap.min(target - frames);
                    if let Some(log) = diagnostic.as_mut() {
                        log(
                            serde_json::json!({"kind":"gap_repaired","gap_frames":gap,"gap_ms":gap as f64*1000.0/format.rate as f64,"device_position":device_position,"packet_qpc_100ns":qpc}),
                        );
                    }
                }
            }
        }
        previous_device = Some((device_position, available, qpc));
        if loopback && timeline_silence {
            if flags & AUDCLNT_BUFFERFLAGS_TIMESTAMP_ERROR.0 as u32 != 0 {
                return Err("播放设备 loopback 时间戳无效，无法对齐采集".into());
            }
            let packet_start = ((qpc as u128).saturating_sub(origin_100ns) * format.rate as u128
                / 10_000_000) as u64;
            if packet_start > ((started.elapsed().as_secs_f64() + 0.2) * format.rate as f64) as u64
            {
                return Err("播放设备 loopback 时间戳超出采集时钟".into());
            }
            if packet_start > frames {
                prefix_silence = (packet_start - frames).min(target - frames);
            }
            skip = (frames + prefix_silence)
                .saturating_sub(packet_start)
                .min(available as u64) as usize;
        }
        let used = (target - frames - prefix_silence).min(available as u64 - skip as u64) as usize;
        if used > 0 {
            timeline_silence = false;
        }
        let mut raw_summary = serde_json::Value::Null;
        buffer.clear();
        if flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 != 0 {
            silent_packets += 1;
            buffer.resize(used * 2, 0.0);
        } else {
            if pointer.is_null() && used > 0 {
                return Err("B3 缓冲区为空".into());
            }
            if used > 0 {
                if diagnostic.is_some() && diagnostics_active() {
                    raw_summary = raw_packet_summary(unsafe {
                        std::slice::from_raw_parts(pointer, available as usize * format.block_align as usize)
                    }, &format);
                }
                let selected = mapping
                    .map(|m| *m.lock().unwrap())
                    .unwrap_or([0, if format.channels > 1 { 1 } else { 0 }]);
                if selected.iter().any(|ch| *ch >= format.channels as usize) {
                    return Err("输入声道超出设备范围".into());
                }
                let bytes = unsafe {
                    std::slice::from_raw_parts(
                        pointer.add(skip * format.block_align as usize),
                        used * format.block_align as usize,
                    )
                };
                let width = (format.bits / 8) as usize;
                for frame in bytes.chunks_exact(format.block_align as usize) {
                    for channel in selected {
                        buffer.push(
                            sample(&frame[channel * width..(channel + 1) * width], &format) as f32,
                        );
                    }
                }
            }
        }
        packet.release()?;
        let received = Instant::now();
        let mut diagnostic_entry = diagnostic.as_ref().filter(|_|diagnostics_active()).map(|_| {
            let mut hash = 0xcbf29ce484222325u64;
            let mut peaks = [0f32;2];
            for sample in &buffer {
                hash ^= sample.to_bits() as u64;
                hash = hash.wrapping_mul(0x100000001b3);
            }
            for frame in buffer.chunks_exact(2) {
                for ch in 0..2 { if frame[ch].is_finite() { peaks[ch] = peaks[ch].max(frame[ch].abs()); } }
            }
            let mut read_qpc = 0i64;
            unsafe { let _ = QueryPerformanceCounter(&mut read_qpc); }
            let entry = serde_json::json!({"kind":"packet","packet_index":packets,
                "capture_elapsed_ms":started.elapsed().as_secs_f64()*1000.0,
                "read_qpc_ticks":read_qpc,"device_position":device_position,"packet_qpc_100ns":qpc,
                "read_interval_ms":previous_packet.map(|p|received.duration_since(p.0).as_secs_f64()*1000.0),
                "device_delta_frames":previous_packet.map(|p|device_position as i128-p.1 as i128),
                "timestamp_delta_ms":previous_packet.map(|p|(qpc as i128-p.2 as i128) as f64/10000.0),
                "available_frames":available,"used_frames":used,"skipped_frames":skip,
                "prefix_silent_frames":prefix_silence,"flags":flags,"input_rate":format.rate,
                "frames_before":frames,"peaks":peaks,"sample_fingerprint":format!("{hash:016x}"),
                "raw_packet":raw_summary,"mapping":mapping.map(|m|*m.lock().unwrap()).unwrap_or([0,if format.channels>1 {1}else{0}])});
            previous_packet = Some((received, device_position, qpc));
            entry
        });
        if let Some(progress) = progress {
            let mut p = progress.lock().unwrap();
            p.packets += 1;
            p.skipped_overlap_frames += skip as u64;
            p.device_position = device_position;
            p.packet_qpc_100ns = qpc;
            p.packet_frames = available;
            p.packet_flags = flags;
            p.packet_peaks = [0.0; 2];
            for frame in buffer.chunks_exact(2) {
                if frame.iter().any(|s| s.is_finite() && s.abs() > 1e-6) {
                    p.signal_frames += 1;
                }
                for ch in 0..2 {
                    if frame[ch].is_finite() {
                        p.packet_peaks[ch] = p.packet_peaks[ch].max(frame[ch].abs());
                    }
                }
            }
            if meter_updated.is_none_or(|t| t.elapsed() >= Duration::from_millis(100)) {
                p.windows_endpoint_peak = meter
                    .as_ref()
                    .and_then(|m| unsafe { m.GetPeakValue().ok() });
                meter_updated = Some(Instant::now());
            }
            p.windows_endpoint_peak_age_ms = meter_updated.map(|t| t.elapsed().as_secs_f64() * 1000.0);
            if let Some(entry) = diagnostic_entry.as_mut() {
                entry["windows_endpoint_peak"] = serde_json::json!(p.windows_endpoint_peak);
                entry["windows_endpoint_peak_age_ms"] = serde_json::json!(p.windows_endpoint_peak_age_ms);
            }
            if flags & AUDCLNT_BUFFERFLAGS_DATA_DISCONTINUITY.0 as u32 != 0 {
                p.discontinuities += 1;
            }
            if flags & AUDCLNT_BUFFERFLAGS_TIMESTAMP_ERROR.0 as u32 != 0 {
                p.timestamp_errors += 1;
            }
        }
        while prefix_silence > 0 && !stop.load(std::sync::atomic::Ordering::Relaxed) {
            let count = prefix_silence.min((format.rate / 100).max(1) as u64);
            let silence = vec![0.0; count as usize * 2];
            if let Some(progress) = progress {
                progress.lock().unwrap().synthesized_silent_frames += count;
            }
            sink(&silence, format.rate)?;
            frames += count;
            silent_frames += count;
            prefix_silence -= count;
        }
        if flags & AUDCLNT_BUFFERFLAGS_DATA_DISCONTINUITY.0 as u32 != 0 && packets > 0 {
            discontinuities += 1;
        }
        if flags & AUDCLNT_BUFFERFLAGS_TIMESTAMP_ERROR.0 as u32 != 0 {
            timestamp_errors += 1;
        }
        signal_frames += buffer
            .chunks_exact(2)
            .filter(|f| f.iter().any(|s| s.is_finite() && s.abs() > 1e-6))
            .count() as u64;
        for frame in buffer.chunks_exact(2) {
            for ch in 0..2 {
                if frame[ch].is_finite() {
                    channel_peaks[ch] = channel_peaks[ch].max(frame[ch].abs());
                }
            }
        }
        let sink_started = Instant::now();
        let delivered = sink(&buffer, format.rate);
        if let (Some(log), Some(mut entry)) = (diagnostic.as_mut(), diagnostic_entry) {
            entry["sink_ms"] = serde_json::json!(sink_started.elapsed().as_secs_f64() * 1000.0);
            entry["processing_ms"] =
                serde_json::json!(packet_read_started.elapsed().as_secs_f64() * 1000.0);
            entry["error"] = serde_json::json!(delivered.as_ref().err().map(ToString::to_string));
            log(entry);
        }
        delivered?;
        frames += used as u64;
        packets += 1;
        last_packet = Instant::now();
    }
    Ok(
        serde_json::json!({"mode":"shared", "input":input, "format":format, "frames":frames,
        "packets":packets,"signal_frames":signal_frames,"channel_peaks":channel_peaks,"silent_packets":silent_packets,"discontinuities":discontinuities,
        "timestamp_errors":timestamp_errors,"loopback":loopback,"intentional_silent_frames":silent_frames,"stopped_by_user":stop.load(std::sync::atomic::Ordering::Relaxed)}),
    )
}

pub fn b3(root: &Path, seconds: u64) -> Result<PathBuf> {
    let (_com, client, capture, format, blob, _input) = open_b3(root)?;
    let directory = root.join("captures");
    fs::create_dir_all(&directory)?;
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();
    let path = directory.join(format!("b3-{stamp}.wav"));
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
    println!("开始采集 {seconds} 秒；请让需要测试的声音路由到 B3。");
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
        println!("共享模式已成功接收音频帧，但全是静音；还需要确认 B3 路由和电平。");
    } else {
        println!("B3 共享模式采集通过，检测到音频信号。请试听录音确认内容。 ");
    }
    Ok(path)
}

#[cfg(test)]
mod loopback_checks {
    use super::*;
    #[test]
    #[ignore = "opens a real playback endpoint for three seconds; no HomePod connection"]
    fn real_playback_capture_keeps_running() {
        let devices = enumerate().unwrap();
        let selected = std::env::var("AIRPLAY_LOOPBACK_SOURCE").unwrap_or("MOTU".into());
        let source = devices
            .iter()
            .find(|d| d.flow == "playback" && d.name.contains(&selected))
            .or_else(|| {
                devices
                    .iter()
                    .find(|d| d.flow == "playback" && d.channels == Some(2))
            })
            .expect("no stereo playback source");
        let stop = std::sync::atomic::AtomicBool::new(false);
        let mut received = 0u64;
        let _com = Com::open().unwrap();
        let enumerator: IMMDeviceEnumerator =
            unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).unwrap() };
        let wide: Vec<u16> = source.id.encode_utf16().chain(Some(0)).collect();
        let device = unsafe {
            enumerator
                .GetDevice(windows::core::PCWSTR(wide.as_ptr()))
                .unwrap()
        };
        let meter: windows::Win32::Media::Audio::Endpoints::IAudioMeterInformation =
            unsafe { device.Activate(CLSCTX_ALL, None).unwrap() };
        let mut captured_peak = 0f32;
        let mut endpoint_peak = 0f32;
        let report = live_selected(
            Path::new("."),
            3,
            &stop,
            Some(&source.id),
            None,
            |samples, rate| {
                assert_eq!(rate, source.rate.unwrap());
                assert!(samples.iter().all(|v| v.is_finite()));
                received += samples.len() as u64 / 2;
                for sample in samples {
                    captured_peak = captured_peak.max(sample.abs());
                }
                endpoint_peak = endpoint_peak.max(unsafe { meter.GetPeakValue().unwrap_or(0.0) });
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(received, source.rate.unwrap() as u64 * 3);
        assert_eq!(report["loopback"], true);
        println!(
            "PASS playback loopback: {received} frames; {} intentional silent frames; {} device packets",
            report["intentional_silent_frames"], report["packets"]
        );
        println!(
            "{}: capture peak={captured_peak:.6}, Windows endpoint peak={endpoint_peak:.6}, non-silent frames={}",
            source.name, report["signal_frames"]
        );
    }
}
