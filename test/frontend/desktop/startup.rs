//! 桌面测试的页面就绪、窗口及单实例检查；默认启动时不执行测试操作。
//! Desktop UI/window/single-instance checks; ordinary launches perform no test actions.
use crate::{quit, reveal};
use airplay_desktop::Engine;
use homepod_test::capture;
use serde_json::{Value, json};
use std::{fs, sync::Arc, thread, time::Duration};
use tauri::{Manager, State};

struct QuitOnSmokeClose;

pub(crate) fn quit_on_smoke_close(app: &tauri::AppHandle) -> bool {
    app.try_state::<QuitOnSmokeClose>().is_some()
}

pub(crate) fn ui_ready(
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
        let capture = engine.capture_status();
        let report = json!({
            "frontendReady": true,
            "backendAvailable": engine.backend_available(),
            "deviceCount": engine.devices().len(),
            "dataPath": engine.data_root(),
            "inputs": inputs,
            "trayInstalled": app.tray_by_id("main-tray").is_some(),
            "hidden": hidden,
            "restored": restored,
            "decorations": window.is_decorated().map_err(|e| e.to_string())?,
            "captureEnabledAtUiReady": capture.enabled,
            "sourcePresentAtUiReady": capture.present,
        });
        fs::write(&path, serde_json::to_vec_pretty(&report).unwrap()).map_err(|e| e.to_string())?;
        // Exercise foreground window close, including the close-action handler.
        app.manage(QuitOnSmokeClose);
        let engine = engine.inner().clone();
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(600));
            // 验证页面就绪时尚未采集、之后才开启；记录状态后仍走正常窗口退出流程。
            // Verify capture is off at UI readiness and enabled afterward; close through the normal window path.
            if let Ok(bytes) = fs::read(&path) {
                if let Ok(mut report) = serde_json::from_slice::<Value>(&bytes) {
                    let capture = engine.capture_status();
                    report["captureEnabledAfterUiReady"] = json!(capture.enabled);
                    report["sourcePresentAfterUiReady"] = json!(capture.present);
                    report["sourceRunningAfterUiReady"] = json!(capture.running);
                    let _ = fs::write(&path, serde_json::to_vec_pretty(&report).unwrap());
                }
            }
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.close();
            }
        });
    }
    Ok(())
}
pub(crate) fn on_second_launch(app: &tauri::AppHandle) {
    if std::env::args().any(|a| a == "--instance-smoke-test") {
        if let (Ok(exe), Some(window)) = (std::env::current_exe(), app.get_webview_window("main")) {
            let _ = fs::write(exe.parent().unwrap().join("instance-smoke.json"), serde_json::to_vec(&json!({"pid":std::process::id(),"restored":window.is_visible().unwrap_or(false),"reusedExistingInstance":true})).unwrap());
        }
        quit(app);
    }
}
