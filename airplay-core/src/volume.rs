//! DACP device callbacks, separated from the binary PCM pipe.
use mdns_sd::{ServiceDaemon, ServiceInfo};
use std::{
    error::Error,
    io::{Read, Write},
    net::{IpAddr, Ipv4Addr, TcpListener, TcpStream, UdpSocket},
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::Duration,
};
type Result<T> = std::result::Result<T, Box<dyn Error>>;
struct Log {
    file: crate::log_store::LogFile,
    faults_only: bool,
    bytes: usize,
}
fn record(log: &Mutex<Log>, text: &str) {
    println!("{text}");
    let mut log = log.lock().unwrap();
    if log.faults_only && (!text.contains("失败") && !text.contains("拒绝") || log.bytes >= 65536)
    {
        return;
    }
    let safe = crate::privacy::Redactor::default().text(text);
    let _ = log.file.record(format!("{safe}\n").as_bytes());
    log.bytes += text.len() + 1;
}

fn command(path: &str) -> Option<String> {
    if path == "/ctrl-int/1/volumeup" {
        return Some("STEP 5\n".into());
    }
    if path == "/ctrl-int/1/volumedown" {
        return Some("STEP -5\n".into());
    }
    let query = path.strip_prefix("/ctrl-int/1/setproperty?")?;
    let mut result = None;
    for pair in query.split('&') {
        let (key, value) = pair.split_once('=')?;
        let value = value
            .replace("%2D", "-")
            .replace("%2d", "-")
            .replace("%2E", ".")
            .replace("%2e", ".");
        if key == "dmcp.volume" {
            let percent = value.parse::<f64>().ok()?;
            if !percent.is_finite() || !(0.0..=100.0).contains(&percent) {
                return None;
            }
            result = Some(format!("SET {percent}\n"));
        } else if key == "dmcp.device-volume" {
            let db = value.parse::<f64>().ok()?;
            if !db.is_finite() || !(-144.0..=0.0).contains(&db) {
                return None;
            }
            result = Some(format!("REPORT {db}\n"));
        }
    }
    result
}
fn request(stream: &mut TcpStream, active: &str) -> std::io::Result<Option<String>> {
    stream.set_read_timeout(Some(Duration::from_millis(750)))?;
    stream.set_write_timeout(Some(Duration::from_millis(750)))?;
    let mut bytes = Vec::new();
    let mut block = [0; 512];
    while !bytes.windows(4).any(|w| w == b"\r\n\r\n") {
        let count = stream.read(&mut block)?;
        if count == 0 || bytes.len() + count > 8192 {
            return Ok(None);
        }
        bytes.extend_from_slice(&block[..count]);
    }
    let Ok(text) = std::str::from_utf8(&bytes) else {
        return Ok(None);
    };
    let mut lines = text.split("\r\n");
    let first = lines.next().unwrap_or("");
    let mut parts = first.split_whitespace();
    let method = parts.next();
    let path = parts.next();
    let protocol = parts.next();
    if method != Some("GET") || !matches!(protocol, Some("HTTP/1.0" | "HTTP/1.1")) {
        return Ok(None);
    }
    let found = lines
        .filter_map(|line| line.split_once(':'))
        .find(|(k, _)| k.eq_ignore_ascii_case("Active-Remote"));
    if !found.is_some_and(|(_, v)| v.trim() == active) {
        stream.write_all(
            b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        )?;
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "Active-Remote mismatch",
        ));
    }
    Ok(path.and_then(command))
}
pub struct Dacp {
    pub control_port: u16,
    commands: mpsc::SyncSender<String>,
    #[cfg(test)]
    callback_port: u16,
    stop: Arc<AtomicBool>,
    threads: Vec<thread::JoinHandle<()>>,
    daemon: ServiceDaemon,
    fullname: String,
}
impl Dacp {
    pub fn start(target: Ipv4Addr, identity: &str, active: &str, log_path: &Path) -> Result<Self> {
        Self::start_with_logging(target, identity, active, log_path, false)
    }
    pub fn start_with_logging(
        target: Ipv4Addr,
        identity: &str,
        active: &str,
        log_path: &Path,
        faults_only: bool,
    ) -> Result<Self> {
        let log = Arc::new(Mutex::new(Log {
            file: crate::log_store::LogFile::create_limited(
                log_path,
                if faults_only { 65536 } else { 1024 * 1024 },
            )?,
            faults_only,
            bytes: 0,
        }));
        let route = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))?;
        route.connect((target, 7000))?;
        let local_ip = route.local_addr()?.ip();
        let callback = TcpListener::bind((local_ip, 0))?;
        callback.set_nonblocking(true)?;
        let callback_port = callback.local_addr()?.port();
        let control = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
        control.set_nonblocking(true)?;
        let control_port = control.local_addr()?.port();
        let daemon = ServiceDaemon::new()?;
        let info = ServiceInfo::new(
            "_dacp._tcp.local.",
            &format!("iTunes_Ctrl_{identity}"),
            &format!("airplay-win-{identity}.local."),
            local_ip,
            callback_port,
            [
                ("txtvers", "1"),
                ("Ver", "131075"),
                ("DbId", identity),
                ("OSsi", "0"),
            ]
            .as_slice(),
        )?;
        let fullname = info.get_fullname().to_owned();
        daemon.register(info)?;
        let (tx, rx) = mpsc::sync_channel::<String>(32);
        let commands = tx.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let callback_stop = stop.clone();
        let callback_log = log.clone();
        let active = active.to_owned();
        let callbacks = thread::spawn(move || {
            while !callback_stop.load(Ordering::Relaxed) {
                match callback.accept() {
                    Ok((mut stream, peer)) => {
                        if peer.ip() != IpAddr::V4(target) {
                            let _=stream.write_all(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                            record(
                                &callback_log,
                                &format!("[VOLUME] 已拒绝非目标设备的回传：{}", peer.ip()),
                            );
                            continue;
                        }
                        match request(&mut stream, &active) {
                            Ok(Some(command)) => {
                                let accepted = tx.try_send(command.clone()).is_ok();
                                record(
                                    &callback_log,
                                    &format!(
                                        "[VOLUME] HomePod 回传 {} accepted={accepted}",
                                        command.trim()
                                    ),
                                );
                                let status = if accepted {
                                    "204 No Content"
                                } else {
                                    "503 Service Unavailable"
                                };
                                let _ = write!(
                                    stream,
                                    "HTTP/1.1 {status}\r\nDAAP-Server: iTunes/7.6.2 (Windows; N;)\r\nContent-Type: application/x-dmap-tagged\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                                );
                            }
                            Ok(None) => {
                                let _=stream.write_all(b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                            }
                            Err(err) => {
                                record(&callback_log, &format!("[VOLUME] 回传读取失败：{err}"))
                            }
                        }
                    }
                    Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(20))
                    }
                    Err(err) => {
                        record(&callback_log, &format!("[VOLUME] 回传监听失败：{err}"));
                        break;
                    }
                }
            }
        });
        let control_stop = stop.clone();
        let control_log = log.clone();
        let forwarding = thread::spawn(move || {
            let mut connection = None;
            while !control_stop.load(Ordering::Relaxed) {
                if connection.is_none() {
                    match control.accept() {
                        Ok((stream, _)) => {
                            let _ = stream.set_write_timeout(Some(Duration::from_millis(750)));
                            connection = Some(stream);
                        }
                        Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(20));
                            continue;
                        }
                        Err(_) => break,
                    }
                }
                match rx.recv_timeout(Duration::from_millis(50)) {
                    Ok(command) => {
                        if connection
                            .as_mut()
                            .unwrap()
                            .write_all(command.as_bytes())
                            .is_err()
                        {
                            record(&control_log, "[VOLUME] 后端音量控制通道已关闭");
                            break;
                        }
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                }
            }
        });
        record(
            &log,
            &format!("[VOLUME] DACP 音量回传监听 {local_ip}:{callback_port}，identity={identity}"),
        );
        Ok(Self {
            control_port,
            commands,
            #[cfg(test)]
            callback_port,
            stop,
            threads: vec![callbacks, forwarding],
            daemon,
            fullname,
        })
    }
    pub fn command_sender(&self) -> mpsc::SyncSender<String> {
        self.commands.clone()
    }
}
impl Drop for Dacp {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = self.daemon.unregister(&self.fullname);
        let _ = self.daemon.shutdown();
        for thread in self.threads.drain(..) {
            let _ = thread.join();
        }
    }
}
#[cfg(test)]
#[path = "../../test/core/unit/volume.rs"]
mod tests;
