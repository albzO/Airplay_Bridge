//! 窗口操作与退出顺序，避免关闭窗口时遗漏音频或后台会话。
use crate::{Engine, stop_source};
use std::{
    sync::{Arc, atomic::Ordering},
    thread,
    time::Duration,
};
use tauri::{Manager, State};

pub(crate) fn reveal(app: &tauri::AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}
pub(crate) fn quit(app: &tauri::AppHandle) {
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
pub(crate) fn window_action(
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
