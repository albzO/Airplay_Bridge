//! 将 Tauri IPC 转发到共享的 Windows 桌面服务。
//! Tauri IPC adapter for the shared Windows desktop service.
use airplay_desktop::{DesktopEvent, Engine, EventSink, Initialization, Settings};
use homepod_test::discovery::Device;
use serde_json::json;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, State};

fn event_sink(app: AppHandle) -> EventSink {
    Arc::new(move |event| match event {
        DesktopEvent::SourceLevel(payload) => {
            let _ = app.emit("source-level", payload);
        }
        DesktopEvent::Stream {
            session_id,
            mut payload,
        } => {
            payload["session_id"] = json!(session_id);
            let _ = app.emit("stream-event", payload);
        }
    })
}

async fn run_blocking<T: Send + 'static>(
    job: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(job)
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn initialize(engine: State<'_, Arc<Engine>>) -> Result<Initialization, String> {
    let engine = engine.inner().clone();
    run_blocking(move || engine.initialize()).await
}

#[tauri::command]
pub async fn discover_devices(engine: State<'_, Arc<Engine>>) -> Result<Vec<Device>, String> {
    run_blocking(engine.inner().prepare_discover_devices()?).await
}

#[tauri::command]
pub async fn monitor_source(app: AppHandle, engine: State<'_, Arc<Engine>>) -> Result<(), String> {
    run_blocking(engine.inner().prepare_monitor_source(event_sink(app))?).await
}

#[tauri::command]
pub async fn set_capture_enabled(
    app: AppHandle,
    engine: State<'_, Arc<Engine>>,
    enabled: bool,
) -> Result<(), String> {
    run_blocking(
        engine
            .inner()
            .prepare_set_capture_enabled(event_sink(app), enabled)?,
    )
    .await
}

#[tauri::command]
pub async fn save_settings(
    engine: State<'_, Arc<Engine>>,
    settings: Settings,
) -> Result<(), String> {
    run_blocking(engine.inner().prepare_save_settings(settings)?).await
}

#[tauri::command]
pub async fn set_mapping(
    engine: State<'_, Arc<Engine>>,
    mapping: [usize; 2],
) -> Result<(), String> {
    run_blocking(engine.inner().prepare_set_mapping(mapping)?).await
}

#[tauri::command]
pub async fn set_speaker_order(
    engine: State<'_, Arc<Engine>>,
    swapped: bool,
) -> Result<(), String> {
    run_blocking(engine.inner().prepare_set_speaker_order(swapped)?).await
}

#[tauri::command]
pub async fn forget_auth_policy(
    engine: State<'_, Arc<Engine>>,
    names: Vec<String>,
) -> Result<(), String> {
    run_blocking(engine.inner().prepare_forget_auth_policy(names)?).await
}

#[tauri::command]
pub fn set_autostart(enabled: bool) -> Result<(), String> {
    airplay_desktop::set_autostart(enabled)
}

#[tauri::command]
pub fn start_stream(
    app: AppHandle,
    engine: State<Arc<Engine>>,
    names: Vec<String>,
    settings: Settings,
    password_first: bool,
) -> Result<u64, String> {
    engine
        .inner()
        .start_stream(event_sink(app), names, settings, password_first)
}

#[tauri::command]
pub fn stop_stream(engine: State<Arc<Engine>>, session_id: u64) {
    engine.stop_stream(session_id);
}

#[tauri::command]
pub fn submit_password(
    engine: State<Arc<Engine>>,
    session_id: u64,
    host: String,
    password: String,
) -> Result<(), String> {
    engine.submit_password(session_id, host, password)
}

#[tauri::command]
pub fn set_volume(engine: State<Arc<Engine>>, percent: f64) -> Result<(), String> {
    engine.set_volume(percent)
}

#[tauri::command]
pub fn open_logs(engine: State<Arc<Engine>>) -> Result<(), String> {
    engine.open_logs()
}
