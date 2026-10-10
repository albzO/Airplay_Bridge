//! 会话结果与报告落盘分别处理，磁盘错误不覆盖串流首因。
//! Session outcome and best-effort report persistence are independent.
use super::{GuiEmitter, Result};
use serde_json::Value;
use std::{fs, path::Path};

pub(super) fn session_result(
    backend_error: Option<String>,
    capture: Result<()>,
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
    pipe?;
    if !transport_ok {
        return Err("持续流发送失败，请查看日志".into());
    }
    Ok(())
}

pub(super) fn finish(
    path: &Path,
    mut report: Value,
    primary: Result<()>,
    emit: Option<&GuiEmitter>,
) -> Result<()> {
    report["report_write_error"] = Value::Null;
    let saved = (|| -> std::result::Result<(), String> {
        let bytes = serde_json::to_vec_pretty(&report).map_err(|e| e.to_string())?;
        fs::write(path, bytes).map_err(|e| e.to_string())
    })()
    .map_err(|e| crate::failure::describe(&format!("写入串流报告：{e}"), "REPORT_WRITE_FAILED"));
    if let Err(error) = &saved {
        report["report_write_error"] = Value::String(error.clone());
        eprintln!("{error}");
    } else {
        println!("流报告：{}", path.display());
    }
    // 即使磁盘不可写，GUI 仍收到完整的内存报告和独立的写入错误。
    // Deliver the in-memory report even when disk persistence fails.
    if let Some(emit) = emit {
        emit(serde_json::json!({"kind":"report","report":report}));
    }
    primary?;
    saved.map_err(Into::into)
}

#[cfg(test)]
#[path = "../../../test/core/unit/live/report.rs"]
mod tests;
