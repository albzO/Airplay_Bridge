//! 纯采集包解码：借用已验证格式的原始字节，复用调用者的立体声缓冲。
//! Pure packet decoding: borrow bytes in a validated format and reuse caller-owned stereo storage.
//! WASAPI pointers, silent flags and ReleaseBuffer remain the capture loop's responsibility.

use super::{Format, Result};
use std::ops::Range;

pub(super) fn sample(bytes: &[u8], format: &Format) -> f64 {
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

/// Select frames and channels before decoding. Errors leave the output untouched.
/// Mono defaults to duplication; multichannel defaults to the first two channels.
pub(super) fn decode_stereo(
    bytes: &[u8],
    format: &Format,
    frames: Range<usize>,
    mapping: Option<[usize; 2]>,
    output: &mut Vec<f32>,
) -> Result<()> {
    let align = format.block_align as usize;
    let window = frames
        .start
        .checked_mul(align)
        .zip(frames.end.checked_mul(align))
        .filter(|(start, end)| start <= end)
        .and_then(|(start, end)| bytes.get(start..end));
    let Some(window) = window.filter(|_| bytes.len() % align == 0) else {
        return Err("采集包帧范围或字节长度无效".into());
    };
    let selected = mapping.unwrap_or([0, usize::from(format.channels > 1)]);
    if selected.iter().any(|ch| *ch >= format.channels as usize) {
        return Err("输入声道超出设备范围".into());
    }
    let width = (format.bits / 8) as usize;
    output.clear();
    for frame in window.chunks_exact(align) {
        for channel in selected {
            output.push(sample(&frame[channel * width..(channel + 1) * width], format) as f32);
        }
    }
    Ok(())
}

// Inspect the full packet before ReleaseBuffer, independently of the selected frame/channel window.
// Never retain raw audio or a WASAPI buffer pointer in a log.
pub(super) fn raw_packet_summary(bytes: &[u8], format: &Format) -> serde_json::Value {
    let mut peaks = vec![0.0f64; format.channels as usize];
    let mut nonfinite = 0u64;
    let width = (format.bits / 8) as usize;
    for frame in bytes.chunks_exact(format.block_align as usize) {
        for (channel, peak) in peaks.iter_mut().enumerate() {
            let value = sample(&frame[channel * width..(channel + 1) * width], format);
            if value.is_finite() {
                *peak = peak.max(value.abs());
            } else {
                nonfinite += 1;
            }
        }
    }
    serde_json::json!({"channel_peaks":peaks,"nonfinite_samples":nonfinite,
        "nonzero_bytes":bytes.iter().filter(|byte|**byte != 0).count(),"bytes":bytes.len()})
}

#[cfg(test)]
#[path = "../../../test/core/unit/capture/decode.rs"]
mod tests;
