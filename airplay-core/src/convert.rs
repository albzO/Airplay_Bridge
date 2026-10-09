//! 有状态 sinc 重采样及 TPDF 抖动量化：float32 立体声 → 44.1 kHz i16。
//! 每次 push 的 WASAPI 包长可变，pending 凑够 480 帧后交给滤波器，
//! 不能在包边界重建 Converter，否则滤波器历史丢失会造成接缝和错误时长。
//! 输入/输出计数按帧计算，一帧含左右两个采样；sink 接收交错 i16 切片。
//! live 再按小端字节序写入管道；finish 补齐滤波器尾部并检查最终输出长度。
//!
//! Stateful sinc resampling with TPDF dither: float32 stereo to 44.1 kHz i16.
//! WASAPI packets can vary in length; pending collects 480 frames before filtering.
//! Keep the Converter across packets or lost filter history creates seams and duration errors.
//! Counters use frames, each containing two samples; sink receives interleaved i16 samples.
//! live serializes little-endian pipe bytes; finish flushes the filter and checks output length.
use audioadapter_buffers::direct::InterleavedSlice;
use rubato::{Adjustable, Async, FixedAsync, Resampler, SincInterpolationParameters};
use serde::Serialize;
use std::{
    collections::VecDeque,
    error::Error,
    fs::{self, File},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

type Result<T> = std::result::Result<T, Box<dyn Error>>;
const RATE: u32 = 44100;
const CHUNK: usize = 480;

#[derive(Default, Serialize)]
pub struct Stats {
    input_frames: u64,
    output_frames: u64,
    invalid_samples: u64,
    clipped_samples: u64,
}
struct Quantizer {
    state: u64,
}
impl Quantizer {
    fn uniform(&mut self) -> f64 {
        self.state = self.state.wrapping_add(0x9e3779b97f4a7c15);
        let mut x = self.state;
        x = (x ^ (x >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        x = (x ^ (x >> 27)).wrapping_mul(0x94d049bb133111eb);
        x ^= x >> 31;
        (x >> 32) as f64 / 4294967296.0
    }
    fn sample(&mut self, input: f32, stats: &mut Stats) -> i16 {
        if !input.is_finite() {
            stats.invalid_samples += 1;
            return 0;
        }
        if input.abs() > 1.0 {
            stats.clipped_samples += 1;
        }
        let tpdf = self.uniform() - self.uniform(); // +/- 1 LSB, zero mean
        (input as f64 * 32768.0 + tpdf)
            .round()
            .clamp(-32768.0, 32767.0) as i16
    }
}
pub struct Converter {
    sampler: Async<f32>,
    pending: VecDeque<f32>,
    input: Vec<f32>,
    output: Vec<f32>,
    pcm: Vec<i16>,
    skip: usize,
    input_rate: u32,
    relative_ratio: f64,
    previous_ratio: f64,
    expected_frames: f64,
    adaptive: bool,
    quantizer: Quantizer,
    pub stats: Stats,
}
impl Converter {
    pub fn new(input_rate: u32, seed: u64) -> Result<Self> {
        if !(8000..=192000).contains(&input_rate) {
            return Err("不支持的输入采样率".into());
        }
        let sampler = Async::new_sinc(
            RATE as f64 / input_rate as f64,
            1.001,
            &SincInterpolationParameters::default(),
            CHUNK,
            2,
            FixedAsync::Input,
        )?;
        let output_max = sampler.output_frames_max();
        let skip = sampler.output_delay();
        Ok(Self {
            sampler,
            pending: VecDeque::with_capacity(CHUNK * 4),
            input: vec![0.0; CHUNK * 2],
            output: vec![0.0; output_max * 2],
            pcm: Vec::with_capacity(output_max * 2),
            skip,
            input_rate,
            relative_ratio: 1.0,
            previous_ratio: 1.0,
            expected_frames: 0.0,
            adaptive: false,
            quantizer: Quantizer { state: seed },
            stats: Stats::default(),
        })
    }
    pub fn output_frames(&self) -> u64 {
        self.stats.output_frames
    }
    /// 相对基准转换比例微调（±800 ppm）；平滑切换，避免突然变速形成可闻突变。
    /// Smoothly adjust the baseline conversion ratio within ±800 ppm to avoid audible rate jumps.
    pub fn set_correction_ppm(&mut self, ppm: f64) -> Result<()> {
        if !ppm.is_finite() || ppm.abs() > 800.0 {
            return Err("漂移校正超过 +/-800 ppm".into());
        }
        self.relative_ratio = 1.0 + ppm / 1_000_000.0;
        self.sampler
            .set_resample_ratio_relative(self.relative_ratio, true)?;
        self.adaptive = true;
        Ok(())
    }
    fn process(&mut self, limit: u64, sink: &mut impl FnMut(&[i16]) -> Result<()>) -> Result<()> {
        let input = InterleavedSlice::new(&self.input, 2, CHUNK)?;
        let output_frames = self.output.len() / 2;
        let mut output = InterleavedSlice::new_mut(&mut self.output, 2, output_frames)?;
        let (used, produced) = self
            .sampler
            .process_into_buffer(&input, &mut output, None)?;
        if used != CHUNK {
            return Err("重采样器未消耗完整输入块".into());
        }
        let skip = self.skip.min(produced);
        self.skip -= skip;
        let count = (produced - skip).min(limit.saturating_sub(self.stats.output_frames) as usize);
        self.pcm.clear();
        for value in &self.output[skip * 2..(skip + count) * 2] {
            self.pcm
                .push(self.quantizer.sample(*value, &mut self.stats));
        }
        if !self.pcm.is_empty() {
            sink(&self.pcm)?;
        }
        self.stats.output_frames += count as u64;
        self.previous_ratio = self.relative_ratio;
        Ok(())
    }
    pub fn push(
        &mut self,
        samples: &[f32],
        sink: &mut impl FnMut(&[i16]) -> Result<()>,
    ) -> Result<()> {
        if samples.len() % 2 != 0 {
            return Err("双声道输入包含不完整的帧".into());
        }
        self.stats.input_frames += (samples.len() / 2) as u64;
        // 不要求包长是 CHUNK 的整数倍；累积剩余采样，跨包保留滤波器状态。
        // Packet lengths need not be multiples of CHUNK; carry remaining samples and filter state.
        for value in samples {
            let value = if value.is_finite() {
                *value
            } else {
                self.stats.invalid_samples += 1;
                0.0
            };
            self.pending.push_back(value);
            if self.pending.len() == CHUNK * 2 {
                for item in &mut self.input {
                    *item = self.pending.pop_front().unwrap();
                }
                self.expected_frames += CHUNK as f64 * RATE as f64 / self.input_rate as f64
                    * (self.previous_ratio + self.relative_ratio)
                    / 2.0;
                self.process(u64::MAX, sink)?;
            }
        }
        Ok(())
    }
    pub fn finish(&mut self, sink: &mut impl FnMut(&[i16]) -> Result<()>) -> Result<()> {
        // 实时漂移校正会微调时长：按累计转换比例算尾帧，不能再裁回文件转换的
        // 名义帧数，否则会丢掉已经校正的音频。未启用校正时按原始时长精确取整。
        // Live correction changes duration: integrate the actual ratio rather than trimming back
        // to nominal file-conversion length. Without correction, round the original duration exactly.
        let target = if self.adaptive {
            (self.expected_frames
                + (self.pending.len() / 2) as f64 * RATE as f64 / self.input_rate as f64
                    * (self.previous_ratio + self.relative_ratio)
                    / 2.0)
                .round() as u64
        } else {
            (self.stats.input_frames * RATE as u64 + self.input_rate as u64 / 2)
                / self.input_rate as u64
        };
        self.input.fill(0.0);
        for item in &mut self.input {
            if let Some(value) = self.pending.pop_front() {
                *item = value;
            } else {
                break;
            }
        }
        while self.stats.output_frames < target {
            self.process(target, sink)?;
            self.input.fill(0.0);
        }
        if self.stats.output_frames != target {
            return Err("重采样输出长度不符合时长约定".into());
        }
        Ok(())
    }
}

pub fn wave(path: &Path) -> Result<PathBuf> {
    let mut source = File::open(path)?;
    let mut header = [0; 12];
    source.read_exact(&mut header)?;
    if &header[..4] != b"RIFF" || &header[8..] != b"WAVE" {
        return Err("输入不是 WAV 文件".into());
    }
    let length = source.metadata()?.len();
    let mut format = None;
    let mut data = None;
    while source.stream_position()? + 8 <= length {
        let mut chunk = [0; 8];
        source.read_exact(&mut chunk)?;
        let size = u32::from_le_bytes(chunk[4..].try_into().unwrap()) as u64;
        let position = source.stream_position()?;
        if position + size > length {
            return Err("WAV 数据块越过文件尾".into());
        }
        if &chunk[..4] == b"fmt " {
            if size > 65536 {
                return Err("WAV 格式头过大".into());
            }
            let mut blob = vec![0; size as usize];
            source.read_exact(&mut blob)?;
            format = Some(crate::capture::parse_format(&blob)?);
        } else if &chunk[..4] == b"data" {
            data = Some((position, size));
        }
        source.seek(SeekFrom::Start(position + size + (size % 2)))?;
    }
    let format = format.ok_or("WAV 缺少 fmt 块")?;
    let (position, size) = data.ok_or("WAV 缺少 data 块")?;
    if format.channels != 2 || format.encoding != "float32" || size % 8 != 0 || size == 0 {
        return Err("转换入口目前需要双声道 float32 WAV".into());
    }
    if size / 8 > format.rate as u64 * 60 {
        return Err("第一轮转换测试限制在 60 秒以内".into());
    }
    let raw_path = path.with_extension("pcm");
    let wav_path = path.with_extension("pcm.wav");
    let mut raw = File::create(&raw_path)?;
    let mut wav = File::create(&wav_path)?;
    let output_frames = (size / 8 * RATE as u64 + format.rate as u64 / 2) / format.rate as u64;
    let bytes = (output_frames * 4) as u32;
    wav.write_all(b"RIFF")?;
    wav.write_all(&(bytes + 36).to_le_bytes())?;
    wav.write_all(b"WAVEfmt \x10\0\0\0\x01\0\x02\0")?;
    wav.write_all(&RATE.to_le_bytes())?;
    wav.write_all(&(RATE * 4).to_le_bytes())?;
    wav.write_all(b"\x04\0\x10\0data")?;
    wav.write_all(&bytes.to_le_bytes())?;
    let mut write_buffer = Vec::with_capacity(4096);
    let mut sink = |samples: &[i16]| -> Result<()> {
        write_buffer.clear();
        for sample in samples {
            write_buffer.extend_from_slice(&sample.to_le_bytes());
        }
        raw.write_all(&write_buffer)?;
        wav.write_all(&write_buffer)?;
        Ok(())
    };
    let mut converter = Converter::new(format.rate, 0x42335f6469746865)?;
    source.seek(SeekFrom::Start(position))?;
    let mut remaining = size as usize;
    let mut buffer = [0u8; CHUNK * 8];
    let mut samples = Vec::with_capacity(CHUNK * 2);
    while remaining > 0 {
        let count = remaining.min(buffer.len());
        source.read_exact(&mut buffer[..count])?;
        samples.clear();
        samples.extend(
            buffer[..count]
                .chunks_exact(4)
                .map(|bytes| f32::from_le_bytes(bytes.try_into().unwrap())),
        );
        converter.push(&samples, &mut sink)?;
        remaining -= count;
    }
    converter.finish(&mut sink)?;
    raw.flush()?;
    wav.flush()?;
    let report = serde_json::json!({ "source": path, "input_rate": format.rate, "output_rate": RATE,
        "channels": 2, "output_bits": 16, "encoding": "signed-pcm-little-endian",
        "resampler": "rubato 5.0.1 Async sinc", "dither": "TPDF +/-1 LSB", "gain": 1.0,
        "stats": converter.stats, "pcm": raw_path, "wave": wav_path });
    fs::write(
        path.with_extension("pcm.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    println!(
        "转换完成：{} Hz float32 → 44100 Hz int16，双声道，增益 1.0",
        format.rate
    );
    println!(
        "输入 {} 帧，输出 {} 帧；越界样本 {}，非法样本 {}",
        converter.stats.input_frames,
        converter.stats.output_frames,
        converter.stats.clipped_samples,
        converter.stats.invalid_samples
    );
    println!(
        "PCM：{}\n试听 WAV：{}",
        raw_path.display(),
        wav_path.display()
    );
    Ok(raw_path)
}

#[cfg(test)]
#[path = "../../test/core/unit/convert.rs"]
mod tests;
