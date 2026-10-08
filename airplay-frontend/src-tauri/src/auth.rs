//! GUI 与原生后端之间的密码管道。密码仅在本次请求内存和管道中传递。
//! 后端先发送主机名（u32 小端字节长度 + UTF-8），再读取同格式的密码回复。
//! 当前用户和 SYSTEM 可访问；拒绝远程客户端，并要求首次创建以防管道被抢占。
//! Server 销毁时取消阻塞 I/O 并等待线程退出；Reply 销毁时清理剩余密码字节。
//!
//! GUI/native-backend password pipe; passwords exist only in request memory and transport.
//! Requests send hostnames as a little-endian u32 byte length plus UTF-8; replies use the same framing.
//! Allow the current user/SYSTEM, reject remote clients and require first-instance creation.
//! Server cleanup cancels blocking I/O and joins its thread; Reply cleanup wipes retained password bytes.
use homepod_test::live::GuiEmitter;
use std::{
    fs::File,
    io::{Read, Write},
    os::windows::io::{AsRawHandle, FromRawHandle},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::Duration,
};
use windows::{
    Win32::{
        Foundation::{CloseHandle, ERROR_PIPE_CONNECTED, HANDLE, HLOCAL, LocalFree},
        Security::{
            Authorization::{
                ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
            },
            GetTokenInformation, PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES, TOKEN_QUERY,
            TOKEN_USER, TokenUser,
        },
        Storage::FileSystem::{
            FILE_FLAG_FIRST_PIPE_INSTANCE, FlushFileBuffers, PIPE_ACCESS_DUPLEX,
        },
        System::{
            IO::CancelSynchronousIo,
            Pipes::{
                ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, PIPE_READMODE_BYTE,
                PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_WAIT,
            },
            Threading::{GetCurrentProcess, OpenProcessToken},
        },
    },
    core::{PCWSTR, PWSTR},
};

pub struct Reply {
    pub host: String,
    pub password: String,
}
impl Drop for Reply {
    fn drop(&mut self) {
        // 发送失败、会话取消或队列销毁时，也清理仍归本对象所有的密码。
        // Wipe owned password bytes even on send failure, session cancellation or queue destruction.
        unsafe {
            self.password.as_bytes_mut().fill(0);
        }
    }
}
pub struct Server {
    pub replies: mpsc::Sender<Reply>,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}
impl Server {
    /// hosts 是本次会话已选择的设备地址白名单；未经选择的请求不能弹出密码框。
    /// stop 与播放线程共用，停止播放也会取消等待密码的流程。
    /// hosts is the selected-device allowlist; unrelated requests cannot trigger password prompts.
    /// stop is shared with playback so stopping the stream also cancels password waits.
    pub fn start(
        name: String,
        stop: Arc<AtomicBool>,
        hosts: Vec<String>,
        emit: GuiEmitter,
    ) -> Result<Self, String> {
        let wide: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
        let mut descriptor = PSECURITY_DESCRIPTOR::default();
        let handle = unsafe {
            let mut token = HANDLE::default();
            OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token)
                .map_err(|e| e.to_string())?;
            let mut size = 0;
            let _ = GetTokenInformation(token, TokenUser, None, 0, &mut size);
            let mut storage = vec![0usize; (size as usize + 7) / 8];
            let result = GetTokenInformation(
                token,
                TokenUser,
                Some(storage.as_mut_ptr().cast()),
                size,
                &mut size,
            );
            let _ = CloseHandle(token);
            result.map_err(|e| e.to_string())?;
            let user = &*(storage.as_ptr().cast::<TOKEN_USER>());
            let mut sid = PWSTR::null();
            ConvertSidToStringSidW(user.User.Sid, &mut sid).map_err(|e| e.to_string())?;
            let sid_text = sid.to_string().map_err(|e| e.to_string());
            let _ = LocalFree(Some(HLOCAL(sid.0.cast())));
            let security: Vec<u16> = format!("D:P(A;;GA;;;{})(A;;GA;;;SY)", sid_text?)
                .encode_utf16()
                .chain(Some(0))
                .collect();
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                PCWSTR(security.as_ptr()),
                1,
                &mut descriptor,
                None,
            )
            .map_err(|e| e.to_string())?;
            let attributes = SECURITY_ATTRIBUTES {
                nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
                lpSecurityDescriptor: descriptor.0,
                bInheritHandle: false.into(),
            };
            let handle = CreateNamedPipeW(
                PCWSTR(wide.as_ptr()),
                PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                1,
                2048,
                2048,
                5000,
                Some(&attributes),
            );
            let _ = LocalFree(Some(HLOCAL(descriptor.0)));
            if handle.is_invalid() {
                return Err(windows::core::Error::from_thread().to_string());
            }
            handle
        };
        let mut pipe = unsafe { File::from_raw_handle(handle.0) };
        let (tx, rx) = mpsc::channel::<Reply>();
        let worker_stop = stop.clone();
        let thread = thread::spawn(move || {
            while !worker_stop.load(Ordering::Relaxed) {
                let handle = HANDLE(pipe.as_raw_handle());
                let connected = unsafe { ConnectNamedPipe(handle, None) };
                if let Err(error) = connected {
                    if error.code().0 as u32 != (0x80070000 | ERROR_PIPE_CONNECTED.0) {
                        break;
                    }
                }
                if worker_stop.load(Ordering::Relaxed) {
                    break;
                }
                let request = (|| -> Result<(), String> {
                    let mut length = [0; 4];
                    pipe.read_exact(&mut length)
                        .map_err(|e| format!("密码通信 / 读取请求长度：{e}"))?;
                    let length = u32::from_le_bytes(length) as usize;
                    if !(1..=64).contains(&length) {
                        return Err("无效密码请求".into());
                    }
                    let mut host = vec![0; length];
                    pipe.read_exact(&mut host)
                        .map_err(|e| format!("密码通信 / 读取请求设备：{e}"))?;
                    let host = String::from_utf8(host).map_err(|e| e.to_string())?;
                    if !hosts.contains(&host) {
                        return Err("密码请求来自非会话设备".into());
                    }
                    emit(serde_json::json!({"kind":"password_required","host":host}));
                    let mut password = String::new();
                    loop {
                        if worker_stop.load(Ordering::Relaxed) {
                            break;
                        }
                        match rx.recv_timeout(Duration::from_millis(100)) {
                            Ok(mut reply) => {
                                if reply.host == host {
                                    password = std::mem::take(&mut reply.password);
                                    break;
                                }
                            }
                            Err(mpsc::RecvTimeoutError::Timeout) => {}
                            Err(_) => break,
                        }
                    }
                    let valid = password.len() <= 1023 && !password.contains(['\0', '\r', '\n']);
                    let bytes = if valid { password.as_bytes() } else { &[] };
                    let written = pipe
                        .write_all(&(bytes.len() as u32).to_le_bytes())
                        .and_then(|_| pipe.write_all(bytes));
                    unsafe {
                        password.as_bytes_mut().fill(0);
                    }
                    written.map_err(|e| format!("密码通信 / 发送响应：{e}"))?;
                    unsafe { FlushFileBuffers(handle) }
                        .map_err(|e| format!("密码通信 / 等待响应读取完成：{e}"))
                })();
                if let Err(error) = request {
                    emit(
                        serde_json::json!({"kind":"auth_pipe_error","error":homepod_test::failure::describe(&error, "AUTH_PIPE_FAILED")}),
                    );
                }
                let _ = unsafe { DisconnectNamedPipe(handle) };
            }
        });
        Ok(Self {
            replies: tx,
            stop,
            thread: Some(thread),
        })
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        // Wake blocking ConnectNamedPipe / ReadFile / FlushFileBuffers without
        // opening a second pipe client that can itself block during shutdown.
        if let Some(worker) = self.thread.take() {
            let deadline = std::time::Instant::now();
            while !worker.is_finished() && deadline.elapsed() < Duration::from_secs(2) {
                let _ = unsafe { CancelSynchronousIo(HANDLE(worker.as_raw_handle())) };
                thread::sleep(Duration::from_millis(10));
            }
            if worker.is_finished() {
                let _ = worker.join();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn local_password_roundtrip_and_cancel() {
        let stop = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::channel();
        let emit: GuiEmitter = Arc::new(move |v| {
            tx.send(v).unwrap();
        });
        let name = format!("\\\\.\\pipe\\airplay-bridge-test-{}", std::process::id());
        let server = Server::start(name.clone(), stop, vec!["127.0.0.1".into()], emit).unwrap();
        fn client(name: String) -> thread::JoinHandle<Vec<u8>> {
            thread::spawn(move || {
                let mut f = std::fs::OpenOptions::new()
                    .read(true)
                    .write(true)
                    .open(name)
                    .unwrap();
                f.write_all(&9u32.to_le_bytes()).unwrap();
                f.write_all(b"127.0.0.1").unwrap();
                let mut size = [0; 4];
                f.read_exact(&mut size).unwrap();
                let mut value = vec![0; u32::from_le_bytes(size) as usize];
                f.read_exact(&mut value).unwrap();
                value
            })
        }
        let c = client(name.clone());
        let event = rx.recv_timeout(Duration::from_secs(3)).unwrap();
        assert_eq!(event["host"], "127.0.0.1");
        assert!(event.get("password").is_none());
        server
            .replies
            .send(Reply {
                host: "127.0.0.1".into(),
                password: "local-test-only".into(),
            })
            .unwrap();
        assert_eq!(c.join().unwrap(), b"local-test-only");
        thread::sleep(Duration::from_millis(30));
        let c = client(name);
        assert_eq!(
            rx.recv_timeout(Duration::from_secs(3)).unwrap()["kind"],
            "password_required"
        );
        drop(server);
        assert!(c.join().unwrap().is_empty());
    }
}
