//! Windows 时钟读取集中在这里；时间线与诊断分别使用同一时钟的不同单位。
//! Centralize Windows clock access; timeline and diagnostics use different QPC units.
use super::Result;
use windows::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};

pub(super) fn frequency() -> Result<i64> {
    let mut value = 0;
    unsafe { QueryPerformanceFrequency(&mut value)? };
    Ok(value)
}
fn ticks_to_100ns(ticks: i64, frequency: i64) -> Result<u128> {
    if ticks < 0 || frequency <= 0 {
        return Err("Windows QPC 时钟值无效".into());
    }
    Ok(ticks as u128 * 10_000_000 / frequency as u128)
}
pub(super) fn origin(loopback: bool) -> Result<u128> {
    if !loopback {
        return Ok(0);
    }
    let frequency = frequency()?;
    let mut ticks = 0;
    unsafe { QueryPerformanceCounter(&mut ticks)? };
    ticks_to_100ns(ticks, frequency)
}
// 诊断读取失败沿用 0；不可影响有效音频的交付。
// Diagnostic read failures retain zero and must not interrupt valid audio delivery.
pub(super) fn read_ticks() -> i64 {
    let mut ticks = 0;
    unsafe {
        let _ = QueryPerformanceCounter(&mut ticks);
    }
    ticks
}

#[cfg(test)]
#[path = "../../../test/core/unit/capture/clock.rs"]
mod tests;
