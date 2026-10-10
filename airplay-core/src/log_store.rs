//! 应用日志的单文件上限和目录保留；只清理已识别的普通日志文件。
//! Bound application log files and retention; prune only recognized regular log files.
use std::{
    collections::HashMap,
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
    time::SystemTime,
};
pub const FILE_BYTES: u64 = 8 * 1024 * 1024;
pub const REPORT_BYTES: u64 = 1024 * 1024;
// 仅截断展示/日志副本，协议控制使用原始行。
// Truncate presentation/log copies only; protocol control uses the original line.
pub(crate) fn bounded_text(mut text: String) -> String {
    if text.chars().count() > 8192 {
        let end = text.char_indices().nth(8191).unwrap().0;
        text.truncate(end);
        text.push('…');
    }
    text
}

const DIRECTORY_BYTES: u64 = 256 * 1024 * 1024;
const DIRECTORY_FILES: usize = 256;
fn active() -> &'static Mutex<HashMap<PathBuf, u64>> {
    static ACTIVE: OnceLock<Mutex<HashMap<PathBuf, u64>>> = OnceLock::new();
    ACTIVE.get_or_init(Mutex::default)
}
fn managed_name(name: &str) -> bool {
    if matches!(
        name,
        "source-startup.jsonl" | "source-startup.previous.jsonl"
    ) {
        return true;
    }
    for prefix in ["live-", "backend-", "ui-fault-"] {
        let Some(rest) = name.strip_prefix(prefix) else {
            continue;
        };
        let Some((stamp, suffix)) = rest.split_once('.') else {
            continue;
        };
        if stamp.is_empty() || !stamp.bytes().all(|b| b.is_ascii_digit()) {
            return false;
        }
        return matches!(
            suffix,
            "log"
                | "json"
                | "volume.log"
                | "peer.volume.log"
                | "drift.jsonl"
                | "pipeline.jsonl"
                | "capture.jsonl"
                | "capture.previous-1.jsonl"
                | "capture.previous-2.jsonl"
                | "capture.previous-3.jsonl"
                | "pipeline.previous-1.jsonl"
                | "pipeline.previous-2.jsonl"
                | "pipeline.previous-3.jsonl"
        );
    }
    false
}
fn reserve(
    root: &Path,
    target: &Path,
    limit: u64,
    active: &HashMap<PathBuf, u64>,
    budget: u64,
    count_limit: usize,
) -> io::Result<()> {
    let mut files = Vec::new();
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        // 不递归、不跟随符号链接；目标来自已规范化目录的直接子项。
        // Never recurse or follow links; targets are direct children of the canonical directory.
        if !entry.file_type()?.is_file() || !managed_name(&entry.file_name().to_string_lossy()) {
            continue;
        }
        let path = entry.path();
        if path == target {
            continue;
        }
        let metadata = entry.metadata()?;
        files.push((
            path.clone(),
            active.get(&path).copied().unwrap_or(metadata.len()),
            metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH),
        ));
    }
    let mut bytes = files
        .iter()
        .fold(0u64, |sum, file| sum.saturating_add(file.1));
    let mut count = files.len();
    files.sort_by(|a, b| a.2.cmp(&b.2).then(a.0.cmp(&b.0)));
    for (path, size, _) in files {
        if bytes.saturating_add(limit) <= budget && count < count_limit {
            break;
        }
        if active.contains_key(&path) {
            continue;
        }
        // 其他进程的 Windows 句柄可能禁止删除；跳过后若仍超限则拒绝新日志。
        // Other processes may hold a Windows handle that denies deletion.
        // Skip such files, then reject the new log if the budget is still full.
        match fs::remove_file(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::PermissionDenied => continue,
            Err(error) => return Err(error),
        }
        bytes = bytes.saturating_sub(size);
        count -= 1;
    }
    if bytes.saturating_add(limit) > budget || count >= count_limit {
        return Err(io::Error::other(
            "[LOG_LIMIT] 日志目录已达到保留上限，无法安全回收旧文件",
        ));
    }
    Ok(())
}

pub struct LogFile {
    file: File,
    path: PathBuf,
    bytes: u64,
    limit: u64,
    capped: bool,
}
impl LogFile {
    pub fn create(path: &Path) -> io::Result<Self> {
        Self::open(path, FILE_BYTES, false)
    }
    pub fn create_limited(path: &Path, limit: u64) -> io::Result<Self> {
        Self::open(path, limit, false)
    }
    pub(crate) fn append(path: &Path, limit: u64) -> io::Result<Self> {
        Self::open(path, limit, true)
    }
    fn open(path: &Path, limit: u64, append: bool) -> io::Result<Self> {
        let root = path
            .parent()
            .ok_or_else(|| io::Error::other("日志目录缺失"))?
            .canonicalize()?;
        let path = root.join(
            path.file_name()
                .ok_or_else(|| io::Error::other("日志文件名缺失"))?,
        );
        if fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_symlink()) {
            return Err(io::Error::other("日志目标不能是符号链接"));
        }
        let mut registry = active().lock().unwrap();
        if registry.contains_key(&path) {
            return Err(io::Error::other("日志文件正在使用"));
        }
        reserve(
            &root,
            &path,
            limit,
            &registry,
            DIRECTORY_BYTES,
            DIRECTORY_FILES,
        )?;
        let mut options = OpenOptions::new();
        // 文件打开时禁止删除/替换，保护其他应用进程正在写入的日志。
        // Deny deletion/replacement while open, including from another app process.
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.share_mode(1); // FILE_SHARE_READ
        }
        let file = options
            .create(true)
            .write(true)
            .append(append)
            .truncate(!append)
            .open(&path)?;
        let bytes = file.metadata()?.len();
        if bytes > limit {
            return Err(io::Error::other("[LOG_LIMIT] 追加日志已超过单文件上限"));
        }
        registry.insert(path.clone(), limit);
        Ok(Self {
            file,
            path,
            bytes,
            limit,
            capped: false,
        })
    }
    pub fn capped(&self) -> bool {
        self.capped
    }
    /// CLI/音量日志达到上限只停止记录，不中断控制；真实磁盘错误仍返回。
    /// CLI/volume caps stop logging, not control; real disk errors still propagate.
    pub fn record(&mut self, bytes: &[u8]) -> io::Result<()> {
        if self.capped {
            return Ok(());
        }
        match self.write_all(bytes) {
            Err(error) if self.capped => {
                eprintln!("{error}");
                Ok(())
            }
            result => result,
        }
    }
}
impl Write for LogFile {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.capped || self.bytes.saturating_add(bytes.len() as u64) > self.limit {
            self.capped = true;
            return Err(io::Error::other(format!(
                "[LOG_LIMIT] 日志达到 {} 字节上限，后续记录已省略",
                self.limit
            )));
        }
        let count = self.file.write(bytes)?;
        self.bytes += count as u64;
        Ok(count)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}
impl Drop for LogFile {
    fn drop(&mut self) {
        active().lock().unwrap().remove(&self.path);
    }
}
pub fn write(path: &Path, bytes: impl AsRef<[u8]>, limit: u64) -> io::Result<()> {
    let bytes = bytes.as_ref();
    if bytes.len() as u64 > limit {
        return Err(io::Error::other("[LOG_LIMIT] 报告/故障记录超过单文件上限"));
    }
    LogFile::create_limited(path, limit)?.write_all(bytes)
}
#[cfg(test)]
#[path = "../../test/core/unit/log_store.rs"]
mod tests;
