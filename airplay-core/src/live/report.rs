//! 会话结果与报告落盘分别处理，磁盘错误不覆盖串流首因。
//! Session outcome and best-effort report persistence are independent.
use super::{GuiEmitter, Result};
use serde_json::Value;
use std::{
    path::Path,
    time::{Duration, Instant},
};

pub(super) fn session_result(
    backend_error: Option<String>,
    capture: Result<()>,
    stderr_error: Option<String>,
    pipe: Result<()>,
    transport_ok: bool,
    log_path: &Path,
) -> Result<()> {
    // 协议首因优先于采集、断管和退出状态，报告写入错误最后考虑。
    // Preserve protocol, capture, pipe and exit priority before considering report writes.
    if let Some(cause) = backend_error {
        return Err(format!("后端串流中断：{cause}；日志：{}", log_path.display()).into());
    }
    capture?;
    if let Some(error) = stderr_error {
        return Err(error.into());
    }
    pipe?;
    if !transport_ok {
        return Err("持续流发送失败，请查看日志".into());
    }
    Ok(())
}

#[cfg(test)]
fn finish(
    path: &Path,
    report: Value,
    primary: Result<()>,
    emit: Option<&GuiEmitter>,
) -> Result<()> {
    finish_until(
        path,
        report,
        primary,
        emit,
        Instant::now() + Duration::from_secs(2),
    )
}
pub(super) fn finish_until(
    path: &Path,
    mut report: Value,
    primary: Result<()>,
    emit: Option<&GuiEmitter>,
    deadline: Instant,
) -> Result<()> {
    report["report_write_error"] = Value::Null;
    let output = report.clone();
    let path = path.to_owned();
    let display_path = path.clone();
    let saved = crate::shutdown::run_until(
        deadline.min(Instant::now() + Duration::from_millis(250)),
        move || -> std::result::Result<(), String> {
            let bytes = serde_json::to_vec_pretty(&output).map_err(|e| e.to_string())?;
            crate::log_store::write(&path, bytes, crate::log_store::REPORT_BYTES)
                .map_err(|e| e.to_string())
        },
    )
    .and_then(|result| result)
    .map_err(|e| crate::failure::describe(&format!("写入串流报告：{e}"), "REPORT_WRITE_FAILED"));
    if let Err(error) = &saved {
        report["report_write_error"] = Value::String(error.clone());
    }
    // 即使磁盘不可写，GUI 仍收到完整的内存报告和独立的写入错误。
    // Deliver the in-memory report even when disk persistence fails.
    let delivered = if let Some(emit) = emit.cloned() {
        crate::shutdown::run_until(deadline, move || {
            emit(serde_json::json!({"kind":"report","report":report}))
        })
    } else {
        let error = saved.as_ref().err().cloned();
        crate::shutdown::run_until(deadline, move || {
            if let Some(error) = error {
                eprintln!("{error}");
            } else {
                println!("流报告：{}", display_path.display());
                println!("持续流发送完成；请核对 HomePod 内容、连续性和停止是否正常。");
            }
        })
    };
    primary?;
    saved?;
    delivered.map_err(Into::into)
}

#[cfg(test)]
#[path = "../../../test/core/unit/live/report.rs"]
mod tests;
