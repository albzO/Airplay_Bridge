#![windows_subsystem = "windows"]
//! 桌面入口：接收 Tauri 命令，管理一份持续采集来源和至多一个播放会话。
//! 页面不直接启动原生后端；此处校验设置、设备和声道，再委托 live::run_gui。
//! 命令返回与工作线程事件是两条异步通道，所有 stream-event 都附带会话 id。
//! 修改入口时同时核对前端 types.ts/protocol.ts，退出逻辑见 window.rs。
//!
//! Desktop entry: accept Tauri commands, manage persistent capture and at most one stream session.
//! Validate settings/devices/channels before delegating to live::run_gui; the UI never starts it directly.
//! Command responses and worker events are separate asynchronous channels; stream events carry an id.
//! Keep types.ts/protocol.ts aligned with changes here; see window.rs for shutdown ordering.
mod auth;
mod auth_memory;
mod awake;
mod configuration;
mod routing;
mod session;
mod settings;
mod startup;
#[path = "../../../test/frontend/desktop/startup.rs"]
mod startup_checks;
mod stream_start;
mod window;
use homepod_test::{capture, discovery::Device, source::Source};
use serde_json::{Value, json};
use session::Session;
use settings::Settings;
use std::{
    fs,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};
use tauri::{
    Emitter, Manager, State,
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
};
use window::{quit, reveal, window_action};
/// 应用级共享状态。Mutex 保护可替换对象；AtomicBool 用于跨线程快速通知。
/// session 的 Some 同时充当连接/播放占用标志，直到工作线程收尾才清除。
/// source 可以在 session 为 None 时继续运行，为页面提供音量预览。
/// App-wide state: Mutex protects replaceable objects; AtomicBool sends fast cross-thread signals.
/// Some(session) reserves preparation/playback until worker cleanup; source can outlive the session
/// and continue producing the UI's level preview while session is None.
struct Engine {
    root: PathBuf,
    portable: bool,
    backend: PathBuf,
    settings: Mutex<Settings>,
    auth_memory: Mutex<auth_memory::Memory>,
    session: Mutex<Option<Session>>,
    source: Mutex<Option<Arc<Source>>>,
    configuration_busy: AtomicBool,
    quitting: AtomicBool,
    capture_enabled: AtomicBool,
    awake: Mutex<awake::Awake>,
    awake_startup_error: Option<String>,
}
fn stop_source(e: &Engine) -> Result<(), String> {
    stop_source_until(e, Instant::now() + homepod_test::shutdown::STOP_TIMEOUT)
}
fn stop_source_until(e: &Engine, deadline: Instant) -> Result<(), String> {
    let mut active = e.source.lock().unwrap();
    if let Some(source) = active.as_ref() {
        source.stop_until(deadline)?;
    }
    *active = None;
    Ok(())
}
/// 同端点且线程健康时只更新声道映射，避免每次连接都重开 WASAPI。
/// 切换端点时先等待旧线程退出再创建新来源，保证不会同时占用两份采集资源。
/// Reuse a healthy source at the same endpoint and update mapping instead of reopening WASAPI.
/// Join the previous thread before switching endpoints to avoid holding two capture resources.
fn ensure_source(
    app: tauri::AppHandle,
    e: &Engine,
    settings: &Settings,
    check: impl Fn() -> Result<(), String>,
) -> Result<(Arc<Source>, bool), String> {
    check()?;
    if e.quitting.load(Ordering::Relaxed) {
        return Err("应用正在退出".into());
    }
    let mut active = e.source.lock().unwrap();
    check()?;
    if let Some(source) = active.as_ref() {
        if source.endpoint == settings.endpoint && source.is_running() && !source.is_stopping() {
            *source.mapping.lock().unwrap() = settings.mapping;
            return Ok((source.clone(), false));
        }
    }
    if let Some(source) = active.as_ref() {
        source.stop()?;
    }
    *active = None;
    check()?;
    if settings.endpoint.is_empty() {
        return Err("未选择音频来源".into());
    }
    let emit = Arc::new(move |event: Value| {
        let _ = app.emit("source-level", event);
    });
    let source = Source::start(
        e.root.clone(),
        settings.endpoint.clone(),
        settings.mapping,
        emit,
    );
    // 新来源先由调用者持有；准备成功/再次核对取消后才发布到 Engine。
    // The caller owns a new source until validation/cancellation checks allow publication.
    Ok((source, true))
}
#[tauri::command]
fn set_autostart(enabled: bool) -> Result<(), String> {
    startup::set(enabled)
}
fn devices(e: &Engine) -> Vec<Device> {
    fs::read(e.root.join("devices.json"))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}
#[tauri::command]
async fn initialize(engine: State<'_, Arc<Engine>>) -> Result<Value, String> {
    let e = engine.inner().clone();
    let inputs =
        tauri::async_runtime::spawn_blocking(|| capture::enumerate().map_err(|e| e.to_string()))
            .await
            .map_err(|e| e.to_string())??;
    let startup = startup::enabled();
    Ok(
        json!({"devices":devices(&e),"inputs":inputs,"settings":e.settings.lock().unwrap().clone(),"dataPath":e.root,"dataMode":if e.portable { "portable" } else { "installed" },"backendAvailable":e.backend.exists(),"awakeActive":e.awake.lock().unwrap().active(),"awakeError":e.awake_startup_error,"autostart":startup.as_ref().copied().unwrap_or(false),"autostartError":startup.err(),"captureEnabled":e.capture_enabled.load(Ordering::SeqCst)}),
    )
}
fn finished_error(
    result: Result<(), String>,
    cancelled: bool,
    cleanup_complete: bool,
) -> Option<String> {
    let original = result.err();
    if !cleanup_complete {
        return Some(format!(
            "{}[STOP_TIMEOUT] 旧会话资源尚未确认退出，请重启应用后连接",
            original.map_or(String::new(), |error| format!("{error}；"))
        ));
    }
    original.filter(|error| {
        !cancelled
            || !(error.ends_with("详情：已取消建立流") || error.ends_with("详情：启动采集已取消"))
    })
}

#[tauri::command]
fn submit_password(
    engine: State<Arc<Engine>>,
    session_id: u64,
    host: String,
    password: String,
) -> Result<(), String> {
    let reply = auth::Reply { host, password };
    let session = engine.session.lock().unwrap();
    let s = session
        .as_ref()
        .filter(|s| s.id == session_id)
        .ok_or("会话已结束")?;
    if !s.hosts.contains(&reply.host)
        || reply.password.is_empty()
        || reply.password.len() > 1023
        || reply.password.contains(['\0', '\r', '\n'])
    {
        return Err("密码或设备无效".into());
    }
    s.reply
        .as_ref()
        .ok_or("密码通道尚未建立")?
        .send(reply)
        .map_err(|_| "密码请求已结束".into())
}
#[tauri::command]
fn stop_stream(engine: State<Arc<Engine>>, session_id: u64) {
    session::stop(&engine.session.lock().unwrap(), session_id);
}
#[tauri::command]
fn set_volume(engine: State<Arc<Engine>>, percent: f64) -> Result<(), String> {
    if !percent.is_finite() || !(0.0..=100.0).contains(&percent) {
        return Err("音量超出范围".into());
    }
    let session = engine.session.lock().unwrap();
    let s = session.as_ref().ok_or("尚未连接")?;
    let senders = s.control.volume.lock().unwrap();
    if senders.is_empty() {
        return Err("音量通道尚未建立".into());
    }
    for tx in senders.iter() {
        tx.try_send(format!("SET {percent:.1}\n"))
            .map_err(|_| "音量通道繁忙或已关闭")?;
    }
    Ok(())
}
#[tauri::command]
fn open_logs(engine: State<Arc<Engine>>) -> Result<(), String> {
    std::process::Command::new("explorer.exe")
        .arg(engine.root.join("logs"))
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string())
}
#[tauri::command]
async fn forget_auth_policy(
    engine: State<'_, Arc<Engine>>,
    names: Vec<String>,
) -> Result<(), String> {
    let e = engine.inner().clone();
    let operation = configuration::Operation::begin(e.clone(), false)?;
    tauri::async_runtime::spawn_blocking(move || {
        operation.check()?;
        let selected: Vec<_> = devices(&e)
            .into_iter()
            .filter(|d| names.contains(&d.name))
            .collect();
        let mut next = e.auth_memory.lock().unwrap().clone();
        operation.check()?;
        next.forget(&e.root, &selected)?;
        // Disk replacement committed; publish the same state even if quitting began meanwhile.
        *e.auth_memory.lock().unwrap() = next;
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command]
fn ui_ready(
    app: tauri::AppHandle,
    engine: State<Arc<Engine>>,
    inputs: Vec<capture::Input>,
) -> Result<(), String> {
    startup_checks::ui_ready(app, engine, inputs)
}

fn main() {
    let mut context = tauri::generate_context!();
    let exe = std::env::current_exe().expect("无法定位程序目录");
    if homepod_test::data_dir::portable_root(&exe).is_some() {
        let root =
            homepod_test::data_dir::prepare().expect("便携版 data 目录不可写，请移到可写目录");
        for window in &mut context.config_mut().app.windows {
            window.data_directory = Some(root.join("webview"));
        }
    }
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            reveal(app);
            startup_checks::on_second_launch(app);
        }))
        .setup(|app| {
            let root = homepod_test::data_dir::prepare()?;
            fs::create_dir_all(root.join("logs"))?;
            let exe = std::env::current_exe()?;
            let adjacent = exe.parent().unwrap().join("runtime/airplay-backend.exe");
            let backend = if adjacent.exists() {
                adjacent
            } else {
                #[cfg(debug_assertions)]
                let fallback = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("../../dist/runtime/airplay-backend.exe");
                #[cfg(not(debug_assertions))]
                let fallback = adjacent;
                fallback
            };
            let settings = Settings::load(&root);
            let seed = backend.parent().unwrap().join("devices.json");
            if !root.join("devices.json").exists() && seed.exists() {
                fs::copy(seed, root.join("devices.json"))?;
            }
            let known_devices = fs::read(root.join("devices.json"))
                .ok()
                .and_then(|b| serde_json::from_slice::<Vec<Device>>(&b).ok())
                .unwrap_or_default();
            let auth_memory =
                auth_memory::Memory::load(&root, &known_devices).map_err(std::io::Error::other)?;
            let mut awake = awake::Awake::default();
            let awake_startup_error = awake.set(settings.keep_awake).err();
            app.manage(Arc::new(Engine {
                root,
                portable: homepod_test::data_dir::portable_root(&exe).is_some(),
                backend,
                settings: Mutex::new(settings),
                auth_memory: Mutex::new(auth_memory),
                session: Mutex::new(None),
                source: Mutex::new(None),
                configuration_busy: AtomicBool::new(false),
                quitting: AtomicBool::new(false),
                // 仅创建应用状态；页面显示完成后通过采集开关命令创建首个来源。
                // Create app state only; the rendered page enables its first source through the capture command.
                capture_enabled: AtomicBool::new(false),
                awake: Mutex::new(awake),
                awake_startup_error,
            }));
            let show = MenuItem::with_id(app, "show", "打开 AirPlay Bridge", true, None::<&str>)?;
            let exit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show, &exit])?;
            TrayIconBuilder::with_id("main-tray")
                .icon(app.default_window_icon().ok_or("缺少托盘图标")?.clone())
                .tooltip("AirPlay Bridge")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => reveal(app),
                    "quit" => quit(app),
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if matches!(
                        event,
                        TrayIconEvent::Click {
                            button: MouseButton::Left,
                            button_state: MouseButtonState::Up,
                            ..
                        }
                    ) {
                        reveal(tray.app_handle());
                    }
                })
                .build(app)?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            initialize,
            configuration::discover_devices,
            configuration::save_settings,
            configuration::monitor_source,
            configuration::set_capture_enabled,
            set_autostart,
            stream_start::start_stream,
            submit_password,
            stop_stream,
            set_volume,
            configuration::set_mapping,
            configuration::set_speaker_order,
            open_logs,
            forget_auth_policy,
            ui_ready,
            window_action
        ])
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let engine = window.state::<Arc<Engine>>();
                if !engine.quitting.load(Ordering::SeqCst) {
                    api.prevent_close();
                    if engine.settings.lock().unwrap().close_action == "quit" {
                        quit(window.app_handle());
                    } else {
                        let _ = window.hide();
                    }
                }
            }
        })
        .run(context)
        .expect("无法启动 AirPlay Bridge");
}

#[cfg(test)]
#[path = "../../../test/frontend/desktop/shutdown.rs"]
mod shutdown_checks;
