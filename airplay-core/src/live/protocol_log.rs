//! 协议展示/落盘独立于原始标记解析；阻塞输出不占用协议读线程。
//! Protocol display/persistence runs separately from raw marker parsing; keep blocking output off the reader.
use super::diagnostics::LogWorker;
use std::{
    collections::VecDeque,
    io::{self, Write},
    sync::atomic::Ordering,
};

pub(super) const QUEUE_CAPACITY: usize = 128;

// 队列只传脱敏文本，原文及控制事件不经过可丢弃的日志队列。
// Queue only redacted text; raw markers and control events bypass this lossy logging queue.
pub(super) struct Entry {
    pub(super) safe_line: String,
    pub(super) is_fault: bool,
    pub(super) password_needed: bool,
}

pub(super) fn start(
    mut file: impl Write + Send + 'static,
    mut console: impl Write + Send + 'static,
    detailed: bool,
) -> std::io::Result<LogWorker<Entry>> {
    LogWorker::spawn_worker(QUEUE_CAPACITY, move |rx, pending, abandon| {
        let mut recent = VecDeque::<String>::new();
        let mut context_written = false;
        let mut log_bytes = 0usize;
        for entry in rx {
            if abandon.load(Ordering::Acquire) {
                return Ok(());
            }
            let Entry {
                safe_line,
                is_fault,
                password_needed,
            } = entry;
            write_line(&mut console, "控制台", &safe_line)?;
            if detailed {
                write_line(&mut file, "文件", &safe_line)?;
            } else if is_fault && log_bytes < 262144 {
                if !context_written {
                    write_line(
                        &mut file,
                        "文件",
                        "[CONTEXT] 最近的会话标记，仅在故障时保存",
                    )?;
                    for item in &recent {
                        write_line(&mut file, "文件", item)?;
                        log_bytes += item.len() + 1;
                    }
                    context_written = true;
                }
                let bounded: String = safe_line.chars().take(2048).collect();
                write_line(&mut file, "文件", &bounded)?;
                log_bytes += bounded.len() + 1;
            }
            if !detailed {
                recent.push_back(safe_line.chars().take(1024).collect());
                if recent.len() > 24 {
                    recent.pop_front();
                }
            }
            if password_needed {
                write_line(
                    &mut console,
                    "控制台",
                    "设备要求 AirPlay 密码，请在本窗口隐藏输入并按回车。",
                )?;
            }
            pending.fetch_sub(1, Ordering::Relaxed);
        }
        file.flush().map_err(|e| output_error("文件", "刷新", e))?;
        console
            .flush()
            .map_err(|e| output_error("控制台", "刷新", e))
    })
}

fn write_line(writer: &mut impl Write, destination: &str, line: &str) -> io::Result<()> {
    writeln!(writer, "{line}").map_err(|e| output_error(destination, "写入", e))
}
fn output_error(destination: &str, operation: &str, error: io::Error) -> io::Error {
    // 保留底层错误文字（包括 OS 代码），补充输出目标和操作上下文。
    // Preserve underlying error text, including OS codes, with sink and operation context.
    io::Error::new(
        error.kind(),
        format!("协议日志{destination}{operation}：{error}"),
    )
}
