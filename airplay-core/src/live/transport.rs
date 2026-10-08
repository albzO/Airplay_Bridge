//! 原生进程生命周期及有界 PCM 发送；采集线程只尝试入队，不等待管道写入。
//! Native process lifetime and bounded PCM delivery; capture only tries enqueueing, never pipe writes.
use super::{GuiContext, Result};
use std::{
    io::{self, Write},
    os::windows::process::CommandExt,
    path::PathBuf,
    process::{Child, ChildStderr, ChildStdin, Command, ExitStatus, Stdio},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    thread,
};
use windows::Win32::System::Performance::QueryPerformanceCounter;

pub(super) const QUEUE_BLOCKS: usize = 64;
// 一帧含左右两个 i16：4 字节；统一单位换算，防止把采样数当帧数或把字节当毫秒。
// One frame contains two i16 channels: four bytes. Centralize conversions between samples, frames and ms.
pub(super) const OUTPUT_RATE: f64 = 44100.0;
pub(super) const FRAMES_PER_MS: f64 = OUTPUT_RATE / 1000.0;
pub(super) const BYTES_PER_FRAME: u64 = 4;
pub(super) const BYTES_PER_MS: f64 = FRAMES_PER_MS * BYTES_PER_FRAME as f64;

/// 已校验会话的后端参数；密码仍通过命名管道传输，不加入命令参数。
/// Backend options for a validated session; passwords stay in the named pipe, not command arguments.
pub(super) struct BackendOptions<'a> {
    pub(super) executable: PathBuf,
    pub(super) gui: Option<&'a GuiContext>,
    pub(super) host: &'a str,
    pub(super) port: u16,
    pub(super) name: &'a str,
    pub(super) txt: &'a str,
    pub(super) identity: &'a str,
    pub(super) active_remote: &'a str,
    pub(super) volume_control_port: u16,
    pub(super) latency_ms: u32,
    pub(super) buffer_ms: u32,
    pub(super) detailed_logs: bool,
    pub(super) peer_args: &'a [String],
}

/// 进程守卫：正常结束保留退出状态，错误返回或提前退出时终止并回收子进程。
/// Process guard: preserve normal exits; kill and reap the child on errors or early returns.
pub(super) struct Backend(Child);
impl Backend {
    pub(super) fn spawn(options: BackendOptions<'_>) -> Result<Self> {
        Ok(Self(
            Command::new(options.executable)
                .creation_flags(if options.gui.is_some() { 0x08000000 } else { 0 })
                .args([
                    "--host",
                    options.host,
                    "--port",
                    &options.port.to_string(),
                    "--name",
                    options.name,
                    "--txt",
                    options.txt,
                    "--timing",
                    "ptp",
                    "--hold-seconds",
                    "0",
                    "--password-auto",
                    "--pcm-stdin",
                    "--identity",
                    options.identity,
                    "--active-remote",
                    options.active_remote,
                    "--volume-control-port",
                    &options.volume_control_port.to_string(),
                    "--latency-ms",
                    &options.latency_ms.to_string(),
                    "--buffer-ms",
                    &options.buffer_ms.to_string(),
                ])
                .args(options.peer_args)
                .args(if options.detailed_logs {
                    vec!["--debug"]
                } else {
                    Vec::new()
                })
                .args(
                    options
                        .gui
                        .map(|g| {
                            let mut args =
                                vec!["--password-pipe".to_owned(), g.password_pipe.clone()];
                            if g.password_first {
                                args.push("--password-pipe-first".into());
                            }
                            if g.peer_password_first {
                                args.push("--peer-password-first".into());
                            }
                            args
                        })
                        .unwrap_or_default(),
                )
                .stdin(Stdio::piped())
                .stdout(Stdio::inherit())
                .stderr(Stdio::piped())
                .spawn()?,
        ))
    }
    pub(super) fn stderr(&mut self) -> Result<ChildStderr> {
        self.0
            .stderr
            .take()
            .ok_or_else(|| "缺少后端日志管道".into())
    }
    pub(super) fn stdin(&mut self) -> Result<ChildStdin> {
        self.0.stdin.take().ok_or_else(|| "缺少 PCM 管道".into())
    }
    pub(super) fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        self.0.try_wait()
    }
    pub(super) fn kill(&mut self) -> io::Result<()> {
        self.0.kill()
    }
    pub(super) fn wait(&mut self) -> io::Result<ExitStatus> {
        self.0.wait()
    }
}
impl Drop for Backend {
    fn drop(&mut self) {
        if !matches!(self.0.try_wait(), Ok(Some(_))) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}

/// 队列与写线程的累计计数，用于遥测和诊断；不是另一套播放时钟。
/// Queue/writer counters for telemetry and diagnostics, not a second playback clock.
#[derive(Clone, Default)]
pub(super) struct PcmCounters {
    pub(super) pending_bytes: Arc<AtomicU64>,
    pub(super) written_bytes: Arc<AtomicU64>,
    pub(super) last_write_qpc: Arc<AtomicU64>,
}
pub(super) struct PcmSender {
    tx: mpsc::SyncSender<Vec<u8>>,
    counters: PcmCounters,
}
impl PcmSender {
    /// PCM 转为 S16LE，入队失败撤销待发送计数；队列满立即报错，保护采集时序。
    /// Encode S16LE and roll back pending bytes on enqueue failure; overflow fails without blocking capture.
    pub(super) fn send(&self, pcm: &[i16]) -> Result<()> {
        let bytes: Vec<u8> = pcm.iter().flat_map(|s| s.to_le_bytes()).collect();
        let length = bytes.len() as u64;
        self.counters
            .pending_bytes
            .fetch_add(length, Ordering::Relaxed);
        self.tx.try_send(bytes).map_err(|err| {
            self.counters
                .pending_bytes
                .fetch_sub(length, Ordering::Relaxed);
            match err {
                mpsc::TrySendError::Full(_) => "PCM 队列已满，发送跟不上采集；已停止",
                mpsc::TrySendError::Disconnected(_) => "后端 PCM 管道已关闭",
            }
        })?;
        Ok(())
    }
}
/// 一块转换输出约 10 ms，64 块限制约 640 ms 积压；关闭发送端以 EOF 请求收尾。
/// Each conversion block is roughly 10 ms; 64 blocks bound backlog near 640 ms. Sender drop requests EOF.
pub(super) struct PcmWriter {
    pub(super) counters: PcmCounters,
    worker: thread::JoinHandle<io::Result<u64>>,
}
impl PcmWriter {
    pub(super) fn start(mut pipe: impl Write + Send + 'static) -> (PcmSender, Self) {
        let (tx, rx) = mpsc::sync_channel::<Vec<u8>>(QUEUE_BLOCKS);
        let counters = PcmCounters::default();
        let output = counters.clone();
        let worker = thread::spawn(move || -> io::Result<u64> {
            let mut bytes = 0;
            for block in rx {
                pipe.write_all(&block)?;
                bytes += block.len() as u64;
                output
                    .pending_bytes
                    .fetch_sub(block.len() as u64, Ordering::Relaxed);
                output.written_bytes.store(bytes, Ordering::Relaxed);
                let mut counter = 0i64;
                if unsafe { QueryPerformanceCounter(&mut counter) }.is_ok() {
                    output
                        .last_write_qpc
                        .store(counter as u64, Ordering::Relaxed);
                }
            }
            // EOF 请求尾部静音和 TEARDOWN，由原生后端完成协议收尾。
            // EOF requests tail silence and TEARDOWN in the native backend.
            drop(pipe);
            Ok(bytes)
        });
        (
            PcmSender {
                tx,
                counters: counters.clone(),
            },
            Self { counters, worker },
        )
    }
    pub(super) fn join(self) -> Result<io::Result<u64>> {
        self.worker.join().map_err(|_| "PCM 写入线程异常".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    struct MemoryPipe(Arc<Mutex<Vec<u8>>>);
    impl Write for MemoryPipe {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn queue_overflow_and_disconnect_roll_back_pending_bytes() {
        let (tx, rx) = mpsc::sync_channel(1);
        let counters = PcmCounters::default();
        let sender = PcmSender {
            tx,
            counters: counters.clone(),
        };
        sender.send(&[1, -2]).unwrap();
        assert_eq!(counters.pending_bytes.load(Ordering::Relaxed), 4);
        assert!(
            sender
                .send(&[3, 4])
                .unwrap_err()
                .to_string()
                .contains("队列已满")
        );
        assert_eq!(counters.pending_bytes.load(Ordering::Relaxed), 4);
        assert_eq!(rx.recv().unwrap(), [1, 0, 254, 255]);
        drop(rx);
        assert!(
            sender
                .send(&[5, 6])
                .unwrap_err()
                .to_string()
                .contains("管道已关闭")
        );
        assert_eq!(counters.pending_bytes.load(Ordering::Relaxed), 4);
    }

    #[test]
    fn writer_drains_accepted_stereo_blocks_before_eof() {
        let bytes = Arc::new(Mutex::new(Vec::new()));
        let (sender, writer) = PcmWriter::start(MemoryPipe(bytes.clone()));
        let counters = writer.counters.clone();
        sender.send(&[i16::MIN, i16::MAX]).unwrap();
        sender.send(&[1, -2]).unwrap();
        drop(sender);
        assert_eq!(writer.join().unwrap().unwrap(), 8);
        assert_eq!(*bytes.lock().unwrap(), [0, 128, 255, 127, 1, 0, 254, 255]);
        assert_eq!(counters.pending_bytes.load(Ordering::Relaxed), 0);
        assert_eq!(counters.written_bytes.load(Ordering::Relaxed), 8);
        assert!(counters.last_write_qpc.load(Ordering::Relaxed) > 0);
    }

    #[test]
    fn pipe_failure_is_reported_and_later_sends_fail_without_blocking() {
        struct BrokenPipe;
        impl Write for BrokenPipe {
            fn write(&mut self, _: &[u8]) -> io::Result<usize> {
                Err(io::Error::new(io::ErrorKind::BrokenPipe, "fixture failure"))
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let (sender, writer) = PcmWriter::start(BrokenPipe);
        let counters = writer.counters.clone();
        sender.send(&[1, 2]).unwrap();
        assert_eq!(
            writer.join().unwrap().unwrap_err().kind(),
            io::ErrorKind::BrokenPipe
        );
        assert!(sender.send(&[3, 4]).is_err());
        assert_eq!(counters.written_bytes.load(Ordering::Relaxed), 0);
        assert_eq!(counters.pending_bytes.load(Ordering::Relaxed), 4);
    }
}
