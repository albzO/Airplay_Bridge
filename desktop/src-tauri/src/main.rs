#![windows_subsystem = "windows"]
mod auth;
mod awake;
mod auth_memory;
use homepod_test::{
    capture,
    discovery::{self, Device},
    live::{self, GuiContext, GuiControl, GuiEmitter},
    source::Source,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    fs,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::Sender,
    },
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tauri::{
    Emitter, Manager, State,
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
};
#[derive(Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct Settings {
    endpoint: String,
    latency: u32,
    buffer: u32,
    mapping: [usize; 2],
    detailed_logs: bool,
    capture_diagnostics: bool,
    close_action: String,
    speakers_swapped: bool,
    keep_awake: bool,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            endpoint: String::new(),
            latency: 300,
            buffer: 128,
            mapping: [0, 1],
            detailed_logs: false,
            capture_diagnostics: false,
            close_action: "tray".into(),
            speakers_swapped: false,
            keep_awake: false,
        }
    }
}
struct Session {
    id: u64,
    control: Arc<GuiControl>,
    reply: Sender<auth::Reply>,
    hosts: Vec<String>,
}
struct Engine {
    root: PathBuf,
    backend: PathBuf,
    settings: Mutex<Settings>,
    auth_memory: Mutex<auth_memory::Memory>,
    session: Mutex<Option<Session>>,
    source: Mutex<Option<Arc<Source>>>,
    discovering: AtomicBool,
    quitting: AtomicBool,
    awake: Mutex<awake::Awake>,
    awake_startup_error: Option<String>,
}
fn stop_source(e: &Engine) {
    if let Some(source) = e.source.lock().unwrap().take() {
        source.stop();
    }
}
fn ensure_source(app: tauri::AppHandle, e: &Engine) -> Result<Arc<Source>, String> {
    if e.quitting.load(Ordering::Relaxed) {
        return Err("应用正在退出".into());
    }
    let settings = e.settings.lock().unwrap().clone();
    let mut active = e.source.lock().unwrap();
    if let Some(source) = active.as_ref() {
        if source.endpoint == settings.endpoint && source.is_running() {
            *source.mapping.lock().unwrap() = settings.mapping;
            return Ok(source.clone());
        }
    }
    if let Some(source) = active.take() {
        source.stop();
    }
    if settings.endpoint.is_empty() {
        return Err("未选择音频来源".into());
    }
    let emit = Arc::new(move |event: Value| {
        let _ = app.emit("source-level", event);
    });
    let source = Source::start(e.root.clone(), settings.endpoint, settings.mapping, emit);
    *active = Some(source.clone());
    Ok(source)
}
#[tauri::command]
fn monitor_source(app: tauri::AppHandle, engine: State<Arc<Engine>>) -> Result<(), String> {
    if engine.session.lock().unwrap().is_some() {
        return Err("串流时不能更换来源".into());
    }
    ensure_source(app, &engine).map(|_| ())
}
fn devices(e: &Engine) -> Vec<Device> {
    fs::read(e.root.join("devices.json"))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}
fn save(e: &Engine, s: &Settings) -> Result<(), String> {
    fs::write(
        e.root.join("settings.json"),
        serde_json::to_vec_pretty(s).unwrap(),
    )
    .map_err(|e| e.to_string())
}
#[tauri::command]
async fn initialize(engine: State<'_, Arc<Engine>>) -> Result<Value, String> {
    let e = engine.inner().clone();
    let inputs =
        tauri::async_runtime::spawn_blocking(|| capture::enumerate().map_err(|e| e.to_string()))
            .await
            .map_err(|e| e.to_string())??;
    Ok(
        json!({"devices":devices(&e),"inputs":inputs,"settings":e.settings.lock().unwrap().clone(),"dataPath":e.root,"backendAvailable":e.backend.exists(),"awakeActive":e.awake.lock().unwrap().active(),"awakeError":e.awake_startup_error}),
    )
}
#[tauri::command]
async fn discover_devices(engine: State<'_, Arc<Engine>>) -> Result<Vec<Device>, String> {
    let e = engine.inner().clone();
    if e.session.lock().unwrap().is_some() {
        return Err("请停止串流后刷新设备".into());
    }
    if e.discovering.swap(true, Ordering::SeqCst) {
        return Err("正在发现设备".into());
    }
    let worker = e.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        discovery::discover(&worker.root, 6).map_err(|e| e.to_string())
    })
    .await;
    e.discovering.store(false, Ordering::SeqCst);
    result.map_err(|e| e.to_string())??;
    Ok(devices(&e))
}
#[tauri::command]
fn save_settings(engine: State<Arc<Engine>>, settings: Settings) -> Result<(), String> {
    if !(250..=2000).contains(&settings.latency) || !(64..=512).contains(&settings.buffer) {
        return Err("播放提前量为 250–2000 ms，Buffer 为 64–512 ms".into());
    }
    if !["tray", "quit"].contains(&settings.close_action.as_str()) {
        return Err("未知窗口关闭动作".into());
    }
    if engine.quitting.load(Ordering::SeqCst) { return Err("应用正在退出".into()); }
    let mut current_settings = engine.settings.lock().unwrap();
    let mut request = engine.awake.lock().unwrap();
    let was_active = request.active();
    request.set(settings.keep_awake)?;
    if let Err(error) = save(&engine, &settings) {
        let _ = request.set(was_active);
        return Err(error);
    }
    *current_settings = settings;
    Ok(())
}
#[tauri::command]
fn start_stream(
    app: tauri::AppHandle,
    engine: State<Arc<Engine>>,
    names: Vec<String>,
    settings: Settings,
    password_first: bool,
) -> Result<u64, String> {
    let e = engine.inner().clone();
    let mut active = e.session.lock().unwrap();
    if active.is_some() || e.discovering.load(Ordering::SeqCst) {
        return Err("已有会话或正在发现设备".into());
    }
    if !e.backend.exists() {
        return Err("缺少 cliairplay-probe.exe，请从完整 dist 文件夹启动".into());
    }
    if names.is_empty() || names.len() > 2 {
        return Err("请选择一台设备或一个立体声对".into());
    }
    if !(250..=2000).contains(&settings.latency) || !(64..=512).contains(&settings.buffer) {
        return Err("缓冲或提前量超出范围".into());
    }
    let found = devices(&e);
    let mut chosen = Vec::new();
    for name in &names {
        let d: Vec<_> = found.iter().filter(|d| &d.name == name).collect();
        if d.len() != 1 {
            return Err("设备名称不唯一，请刷新设备".into());
        }
        chosen.push(d[0]);
    }
    if names.len() == 2
        && (names[0] == names[1]
            || chosen[0]
                .properties
                .get("tsid")
                .filter(|v| !v.is_empty())
                .is_none()
            || chosen[0].properties.get("tsid") != chosen[1].properties.get("tsid"))
    {
        return Err("这两台设备不是同一个立体声对".into());
    }
    let hosts: Vec<String> = chosen
        .iter()
        .map(|d| {
            d.addresses
                .first()
                .map(|a| a.to_string())
                .ok_or("设备没有 IPv4 地址".to_string())
        })
        .collect::<Result<_, _>>()?;
    let policies = e.auth_memory.lock().unwrap();
    let password_first = password_first || policies.needs_password(chosen[0]);
    let peer_password_first = chosen.get(1).is_some_and(|d| policies.needs_password(d));
    drop(policies);
    let identities: Vec<_> = hosts
        .iter()
        .cloned()
        .zip(chosen.iter().map(|d| auth_memory::key(d)))
        .collect();
    let endpoint = settings.endpoint.clone();
    let inputs = thread::spawn(|| capture::enumerate().map_err(|e| e.to_string()))
        .join()
        .map_err(|_| "枚举采集设备失败")??;
    let input = inputs
        .iter()
        .find(|i| i.id == endpoint)
        .ok_or("请选择有效的音频来源")?;
    let channels = input.channels.ok_or("不支持此采集格式")? as usize;
    if settings.mapping.iter().any(|i| *i >= channels) {
        return Err("声道映射超出了输入设备的声道数".into());
    }
    save(&e, &settings)?;
    *e.settings.lock().unwrap() = settings.clone();
    let id = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    let control = Arc::new(GuiControl::new(settings.mapping));
    let event_app = app.clone();
    let event_engine = e.clone();
    let emit: GuiEmitter = Arc::new(move |mut event| {
        if let Some(host) = auth_memory::password_host(&event) {
            if let Some((_, identity)) = identities.iter().find(|(address, _)| address == host) {
                if let Err(error) = event_engine
                    .auth_memory
                    .lock()
                    .unwrap()
                    .remember(&event_engine.root, identity)
                {
                    let _ = event_app.emit("stream-event", json!({"session_id":id,"kind":"auth_memory_error","error":format!("认证方式记录保存失败：{error}")}));
                }
            }
        }
        event["session_id"] = json!(id);
        let _ = event_app.emit("stream-event", event);
    });
    let pipe = format!("\\\\.\\pipe\\airplay-bridge-{}-{id}", std::process::id());
    let server = auth::Server::start(
        pipe.clone(),
        control.stop.clone(),
        hosts.clone(),
        emit.clone(),
    )?;
    let source = ensure_source(app.clone(), &e)?;
    control.speakers_swapped.store(
        settings.speakers_swapped && names.len() == 2,
        Ordering::Relaxed,
    );
    *active = Some(Session {
        id,
        control: control.clone(),
        reply: server.replies.clone(),
        hosts,
    });
    drop(active);
    thread::spawn(move || {
        let context = GuiContext {
            backend: e.backend.clone(),
            endpoint,
            source,
            password_pipe: pipe,
            password_first,
            peer_password_first,
            detailed_logs: settings.detailed_logs,
            capture_diagnostics: settings.capture_diagnostics,
            control: control.clone(),
            emit: emit.clone(),
        };
        let result = live::run_gui(
            &e.root,
            &names[0],
            names.get(1).map(String::as_str),
            settings.latency,
            settings.buffer,
            context,
        )
        .map_err(|e| e.to_string());
        let cancelled = control.stop.load(Ordering::Relaxed);
        if !cancelled {
            if let Err(error) = &result {
                let _ = fs::write(
                    e.root.join("logs").join(format!("ui-fault-{id}.log")),
                    error,
                );
            }
        }
        drop(server);
        let mut session = e.session.lock().unwrap();
        if session.as_ref().map(|s| s.id) == Some(id) {
            *session = None;
        }
        drop(session);
        emit(
            json!({"kind":"finished","error":if cancelled{None}else{result.err()},"cancelled":cancelled}),
        );
    });
    Ok(id)
}
#[tauri::command]
fn submit_password(
    engine: State<Arc<Engine>>,
    session_id: u64,
    host: String,
    password: String,
) -> Result<(), String> {
    let session = engine.session.lock().unwrap();
    let s = session
        .as_ref()
        .filter(|s| s.id == session_id)
        .ok_or("会话已结束")?;
    if !s.hosts.contains(&host)
        || password.is_empty()
        || password.len() > 1023
        || password.contains(['\0', '\r', '\n'])
    {
        return Err("密码或设备无效".into());
    }
    s.reply
        .send(auth::Reply { host, password })
        .map_err(|_| "密码请求已结束".into())
}
#[tauri::command]
fn stop_stream(engine: State<Arc<Engine>>) {
    if let Some(s) = engine.session.lock().unwrap().as_ref() {
        s.control.stop.store(true, Ordering::Relaxed);
    }
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
fn set_mapping(engine: State<Arc<Engine>>, mapping: [usize; 2]) -> Result<(), String> {
    let endpoint = engine.settings.lock().unwrap().endpoint.clone();
    let inputs = thread::spawn(|| capture::enumerate().map_err(|e| e.to_string()))
        .join()
        .map_err(|_| "枚举采集设备失败")??;
    let channels = inputs
        .iter()
        .find(|i| i.id == endpoint)
        .and_then(|i| i.channels)
        .ok_or("无有效采集设备")? as usize;
    if mapping.iter().any(|c| *c >= channels) {
        return Err("声道超出范围".into());
    }
    let session = engine.session.lock().unwrap();
    if let Some(s) = session.as_ref() {
        *s.control.mapping.lock().unwrap() = mapping;
    }
    if let Some(source) = engine.source.lock().unwrap().as_ref() {
        *source.mapping.lock().unwrap() = mapping;
    }
    let mut settings = engine.settings.lock().unwrap();
    settings.mapping = mapping;
    save(&engine, &settings)
}
#[tauri::command]
fn set_speaker_order(engine: State<Arc<Engine>>, swapped: bool) -> Result<(), String> {
    if let Some(session) = engine.session.lock().unwrap().as_ref() {
        if session.hosts.len() != 2 {
            return Err("仅立体声对可交换扬声器".into());
        }
        session
            .control
            .speakers_swapped
            .store(swapped, Ordering::Relaxed);
    }
    let mut settings = engine.settings.lock().unwrap();
    settings.speakers_swapped = swapped;
    save(&engine, &settings)
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
fn forget_auth_policy(engine: State<Arc<Engine>>, names: Vec<String>) -> Result<(), String> {
    if engine.session.lock().unwrap().is_some() {
        return Err("请停止串流后重新检测认证方式".into());
    }
    let selected: Vec<_> = devices(&engine)
        .into_iter()
        .filter(|d| names.contains(&d.name))
        .collect();
    engine
        .auth_memory
        .lock()
        .unwrap()
        .forget(&engine.root, &selected)
}
#[tauri::command]
fn ui_ready(
    app: tauri::AppHandle,
    engine: State<Arc<Engine>>,
    inputs: Vec<capture::Input>,
) -> Result<(), String> {
    if std::env::args().any(|a| a == "--instance-smoke-test") {
        let window = app.get_webview_window("main").ok_or("缺少主窗口")?;
        window.hide().map_err(|e| e.to_string())?;
        let path = std::env::current_exe()
            .map_err(|e| e.to_string())?
            .parent()
            .unwrap()
            .join("instance-smoke.json");
        fs::write(path, serde_json::to_vec(&json!({"pid":std::process::id(),"hidden":!window.is_visible().map_err(|e| e.to_string())?,"waitingForSecondLaunch":true})).unwrap()).map_err(|e| e.to_string())?;
    }
    if std::env::args().any(|a| a == "--smoke-test") {
        let path = std::env::current_exe()
            .map_err(|e| e.to_string())?
            .parent()
            .unwrap()
            .join("ui-smoke.json");
        let window = app.get_webview_window("main").ok_or("缺少主窗口")?;
        window.hide().map_err(|e| e.to_string())?;
        let hidden = !window.is_visible().map_err(|e| e.to_string())?;
        reveal(&app);
        let restored = window.is_visible().map_err(|e| e.to_string())?;
        fs::write(path,serde_json::to_vec_pretty(&json!({"frontendReady":true,"backendAvailable":engine.backend.exists(),"deviceCount":devices(&engine).len(),"dataPath":engine.root,"inputs":inputs,"trayInstalled":app.tray_by_id("main-tray").is_some(),"hidden":hidden,"restored":restored,"decorations":window.is_decorated().map_err(|e|e.to_string())?})).unwrap()).map_err(|e|e.to_string())?;
        // Exercise foreground window close, including the close-action handler.
        engine.settings.lock().unwrap().close_action = "quit".into();
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(200));
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.close();
            }
        });
    }
    Ok(())
}
fn reveal(app: &tauri::AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}
fn quit(app: &tauri::AppHandle) {
    let engine = app.state::<Arc<Engine>>().inner().clone();
    if engine.quitting.swap(true, Ordering::SeqCst) {
        return;
    }
    if let Some(s) = engine.session.lock().unwrap().as_ref() {
        s.control.stop.store(true, Ordering::Relaxed);
    }
    let app = app.clone();
    thread::spawn(move || {
        let _ = engine.awake.lock().unwrap().set(false);
        stop_source(&engine);
        while engine.session.lock().unwrap().is_some() {
            thread::sleep(Duration::from_millis(100));
        }
        app.exit(0);
    });
}
#[tauri::command]
fn window_action(
    app: tauri::AppHandle,
    engine: State<Arc<Engine>>,
    window: tauri::WebviewWindow,
    action: String,
) -> Result<(), String> {
    match action.as_str() {
        "minimize" => window.minimize(),
        "maximize" => {
            if window.is_maximized().map_err(|e| e.to_string())? {
                window.unmaximize()
            } else {
                window.maximize()
            }
        }
        "close" => {
            if engine.settings.lock().unwrap().close_action == "quit" {
                quit(&app);
                Ok(())
            } else {
                window.hide()
            }
        }
        "drag" => window.start_dragging(),
        _ => return Err("未知窗口操作".into()),
    }
    .map_err(|e| e.to_string())
}
fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            reveal(app);
            if std::env::args().any(|a| a == "--instance-smoke-test") {
                if let (Ok(exe), Some(window)) = (std::env::current_exe(), app.get_webview_window("main")) {
                    let _ = fs::write(exe.parent().unwrap().join("instance-smoke.json"), serde_json::to_vec(&json!({"pid":std::process::id(),"restored":window.is_visible().unwrap_or(false),"reusedExistingInstance":true})).unwrap());
                }
                quit(app);
            }
        }))
        .setup(|app| {
            let root = app.path().app_data_dir()?;
            fs::create_dir_all(root.join("logs"))?;
            let exe = std::env::current_exe()?;
            let adjacent = exe.parent().unwrap().join("cliairplay-probe.exe");
            let backend = if adjacent.exists() {
                adjacent
            } else {
                PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../dist/cliairplay-probe.exe")
            };
            let settings: Settings = fs::read(root.join("settings.json"))
                .ok()
                .and_then(|b| serde_json::from_slice(&b).ok())
                .unwrap_or_default();
            let seed = backend.parent().unwrap().join("devices.json");
            if !root.join("devices.json").exists() && seed.exists() {
                fs::copy(seed, root.join("devices.json"))?;
            }
            let known_devices = fs::read(root.join("devices.json")).ok().and_then(|b| serde_json::from_slice::<Vec<Device>>(&b).ok()).unwrap_or_default();
            let auth_memory = auth_memory::Memory::load(&root, &known_devices).map_err(std::io::Error::other)?;
            let mut awake = awake::Awake::default();
            let awake_startup_error = awake.set(settings.keep_awake).err();
            app.manage(Arc::new(Engine {
                root,
                backend,
                settings: Mutex::new(settings),
                auth_memory: Mutex::new(auth_memory),
                session: Mutex::new(None),
                source: Mutex::new(None),
                discovering: AtomicBool::new(false),
                quitting: AtomicBool::new(false),
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
            discover_devices,
            save_settings,
            monitor_source,
            start_stream,
            submit_password,
            stop_stream,
            set_volume,
            set_mapping,
            set_speaker_order,
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
                    if engine.settings.lock().unwrap().close_action == "quit" { quit(window.app_handle()); }
                    else { let _ = window.hide(); }
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("无法启动 AirPlay Bridge");
}
