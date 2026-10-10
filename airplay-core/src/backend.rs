use crate::discovery::Device;
use std::{
    fs,
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

/// 运行程序与 CLI 的用户数据分开存放。
/// Runtime files are separate from per-user CLI data.
pub fn executable() -> PathBuf {
    let directory = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf))
        // 定位失败时返回无效路径，禁止退回用户数据目录加载程序。
        // Return an invalid path on lookup failure; never load an executable from user data.
        .unwrap_or_default();
    resolve_executable(&directory)
}

fn resolve_executable(directory: &Path) -> PathBuf {
    if directory.as_os_str().is_empty() {
        return PathBuf::new();
    }
    let mut candidates = vec![
        directory.join("runtime/airplay-backend.exe"),
        directory.join("airplay-backend.exe"),
    ];
    // 只有 tools/ 中的 CLI 可以向上查找同一安装根目录的 runtime/。
    // Only a CLI under tools/ may search the parent installation's runtime/ directory.
    if directory
        .file_name()
        .is_some_and(|name| name.eq_ignore_ascii_case("tools"))
    {
        if let Some(root) = directory.parent() {
            candidates.insert(0, root.join("runtime/airplay-backend.exe"));
        }
    }
    #[cfg(debug_assertions)]
    candidates.push(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../dist/runtime/airplay-backend.exe"),
    );
    candidates
        .iter()
        .find(|path| path.is_file())
        .cloned()
        .unwrap_or_else(|| candidates[0].clone())
}

pub fn test(
    root: &Path,
    name: &str,
    timing: &str,
    password_prompt: bool,
    play_tone: bool,
    pcm_file: Option<&Path>,
) -> Result<(), Box<dyn std::error::Error>> {
    let play_audio = play_tone || pcm_file.is_some();
    if let Some(path) = pcm_file {
        let report: serde_json::Value = serde_json::from_slice(
            &fs::read(path.with_extension("pcm.json"))
                .map_err(|_| "请使用本工具 convert-pcm 生成的 PCM 文件及其 pcm.json 报告")?,
        )?;
        let frames = report["stats"]["output_frames"]
            .as_u64()
            .ok_or("PCM 报告缺少输出帧数")?;
        if report["output_rate"] != 44100
            || report["output_bits"] != 16
            || report["channels"] != 2
            || frames == 0
            || frames > 60 * 44100
            || fs::metadata(path)?.len() != frames * 4
        {
            return Err("PCM 格式报告或文件长度不符合 44100Hz / S16LE / 双声道约定".into());
        }
    }
    let devices: Vec<Device> =
        serde_json::from_str(&fs::read_to_string(root.join("devices.json"))?)?;
    let matches: Vec<_> = devices
        .iter()
        .filter(|device| device.name == name)
        .collect();
    if matches.len() != 1 {
        return Err(format!(
            "名称匹配到 {} 台，请先 discover 并使用显示的完整名称",
            matches.len()
        )
        .into());
    }
    let privacy = crate::privacy::Redactor::new(&devices);
    let device = matches[0];
    let host = device
        .addresses
        .first()
        .ok_or("设备没有 IPv4 地址")?
        .to_string();
    let txt = device
        .properties
        .iter()
        .map(|(key, value)| format!("{key}={value}"))
        .collect::<Vec<_>>()
        .join(" ");
    fs::create_dir_all(root.join("logs"))?;
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
    let log_path = root.join("logs").join(format!("backend-{stamp}.log"));
    let mut log = crate::log_store::LogFile::create(&log_path)?;
    println!(
        "连接 {}（{}:{}），timing={}。",
        device.name, host, device.port, timing
    );
    println!(
        "设备公布的密码要求：{}",
        device
            .properties
            .get("pw")
            .map(String::as_str)
            .unwrap_or("未公布")
    );
    if password_prompt {
        println!("先自动认证；设备要求密码时才提示隐藏输入。");
    }
    if play_tone {
        println!(
            "将发送低音量 440 Hz 测试音，持续 4 秒；保持 HomePod 当前音量。请确认是否实际出声。"
        );
    }
    if let Some(path) = pcm_file {
        println!("回放已转换的录音：{}；这次是录音回放测试。", path.display());
    }
    let mut command = Command::new(executable());
    command.args([
        "--host",
        &host,
        "--port",
        &device.port.to_string(),
        "--name",
        &device.name,
        "--txt",
        &txt,
        "--timing",
        timing,
        "--debug",
    ]);
    if password_prompt {
        command.arg("--password-auto");
    }
    if play_tone {
        command.args(["--tone", "--hold-seconds", "0"]);
    }
    if let Some(path) = pcm_file {
        command
            .arg("--pcm-file")
            .arg(path)
            .args(["--hold-seconds", "0"]);
    }
    let mut child = command
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::piped())
        .spawn()?;
    let stderr = child.stderr.take().ok_or("无法读取后端日志")?;
    let (sender, receiver) = mpsc::sync_channel(128);
    let reader = thread::spawn(move || {
        for line in BufReader::new(stderr).lines() {
            match line {
                Ok(line) => {
                    if sender.send(crate::log_store::bounded_text(line)).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });
    let mut markers = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(180);
    let status = loop {
        for line in receiver.try_iter().take(128) {
            println!("{}", privacy.text(&line));
            log.record(format!("{}\n", privacy.text(&line)).as_bytes())?;
            record_marker(&mut markers, &line);
        }
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if Instant::now() > deadline {
            child.kill()?;
            child.wait()?;
            return Err("后端测试超过 180 秒，已终止；尚未输入密码也会计入等待时间".into());
        }
        thread::sleep(Duration::from_millis(50));
    };
    // 先排空有界通道，避免 reader 等待发送而 join 等待 reader。
    // Drain the bounded channel before joining a reader that may be waiting to send.
    for line in receiver.iter() {
        println!("{}", privacy.text(&line));
        if line.contains("[PROBE] PASSWORD_NEEDED ") {
            println!("设备要求 AirPlay 密码，请在本窗口隐藏输入并按回车。");
        }
        log.record(format!("{}\n", privacy.text(&line)).as_bytes())?;
        record_marker(&mut markers, &line);
    }
    let _ = reader.join();
    println!("\n测试结果：");
    let auth_error = markers
        .iter()
        .find_map(|line| crate::failure::SessionError::parse(line));
    if let Some(error) = auth_error.as_ref().filter(|e| e.phase == "pairing") {
        for (marker, label) in [
            ("TCP_CONNECTED", "TCP 连接"),
            ("GET_INFO_OK", "读取设备信息"),
        ] {
            println!(
                "{label}：{}",
                if markers.iter().any(|line| line.contains(marker)) {
                    "通过"
                } else {
                    "未确认"
                }
            );
        }
        println!("密码/配对认证：未通过（{}）", error.code);
        println!("后续加密会话及音频步骤：未执行（认证未通过）");
        println!("日志：{}", log_path.display());
        return Err(error.clone().into());
    }
    for (marker, label) in [
        ("TCP_CONNECTED", "TCP 连接"),
        ("GET_INFO_OK", "读取设备信息"),
        ("PASSWORD_ACCEPTED", "实际密码作为 SRP secret 被接受"),
        ("ENCRYPTED_RESPONSE_OK", "加密通信"),
        ("SESSION_ACCEPTED", "session SETUP"),
        ("CONTROL_HOLD_OK", "控制会话保持"),
    ] {
        if play_audio && marker == "CONTROL_HOLD_OK" {
            continue;
        }
        if marker == "PASSWORD_ACCEPTED"
            && markers
                .iter()
                .any(|line| line.contains("AUTH_METHOD value=fixed-pin"))
        {
            println!("用户密码：无需输入（自动认证通过）");
            continue;
        }
        if marker == "PASSWORD_ACCEPTED"
            && markers
                .iter()
                .any(|line| line.contains("AUTH_METHOD value=credentials"))
        {
            println!("用户密码：未使用（使用配对凭据）");
            continue;
        }
        println!(
            "{}：{}",
            label,
            if markers.iter().any(|line| line.contains(marker)) {
                "通过"
            } else {
                "未通过或未执行"
            }
        );
    }
    if let Some(method) = markers.iter().find(|line| line.contains("AUTH_METHOD")) {
        println!("{method}");
    }
    if play_audio {
        for (marker, label) in [
            ("RECORD_OK", "RECORD"),
            ("STREAM_SETUP_OK", "音频流 SETUP"),
            ("AUDIO_TRANSPORT_OK", "固定音频发送完成"),
        ] {
            println!(
                "{}：{}",
                label,
                if markers.iter().any(|line| line.contains(marker)) {
                    "通过"
                } else {
                    "未通过或未执行"
                }
            );
        }
    }
    if let Some(timing) = markers.iter().find(|line| line.contains("TIMING value=")) {
        println!("实际时钟协议：{timing}");
    }
    println!("日志：{}", log_path.display());
    if !status.success() {
        if let Some(error) = auth_error {
            return Err(error.into());
        }
        let code = status
            .code()
            .map(|v| v.to_string())
            .unwrap_or_else(|| "未知".to_owned());
        let phase = markers
            .iter()
            .find(|line| line.contains("[PROBE] FAILED phase="))
            .and_then(|line| {
                line.split_whitespace()
                    .find_map(|v| v.strip_prefix("phase="))
            })
            .unwrap_or("见日志");
        return Err(format!(
            "后端测试失败（退出码 {code}，阶段 {phase}）；日志：{}",
            log_path.display()
        )
        .into());
    }
    if play_audio {
        println!("音频传输测试完成；实际播放是否成功，请以 HomePod 出声为准。");
    } else {
        println!("控制会话测试通过；尚未测试音频播放。");
    }
    Ok(())
}

// 只保留结果摘要需要的首条标记，重复统计不能无限累积。
// Retain only first markers needed by the result summary; repeated statistics cannot grow history.
fn record_marker(markers: &mut Vec<String>, line: &str) {
    if !line.contains("[PROBE]") {
        return;
    }
    let key = [
        "TCP_CONNECTED",
        "GET_INFO_OK",
        "PASSWORD_ACCEPTED",
        "ENCRYPTED_RESPONSE_OK",
        "SESSION_ACCEPTED",
        "CONTROL_HOLD_OK",
        "AUTH_METHOD",
        "RECORD_OK",
        "STREAM_SETUP_OK",
        "AUDIO_TRANSPORT_OK",
        "TIMING value=",
        "[PROBE] FAILED phase=",
    ]
    .into_iter()
    .find(|key| line.contains(key));
    if let Some(key) = key {
        if !markers.iter().any(|saved| saved.contains(key)) {
            markers.push(crate::log_store::bounded_text(line.to_owned()));
        }
    }
    if crate::failure::SessionError::parse(line).is_some()
        && !markers
            .iter()
            .any(|saved| crate::failure::SessionError::parse(saved).is_some())
    {
        markers.push(crate::log_store::bounded_text(line.to_owned()));
    }
}

#[cfg(test)]
#[path = "../../test/core/unit/backend.rs"]
mod executable_tests;
