//! 窗口操作与退出顺序，避免关闭窗口时遗漏音频或后台会话。
//! Window actions and shutdown ordering ensure audio and background sessions are cleaned up.
use crate::{Engine, stop_source_until};
use std::{
    sync::{Arc, atomic::Ordering, mpsc},
    thread,
    time::{Duration, Instant},
};
use tauri::{Manager, State};

pub(crate) fn reveal(app: &tauri::AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}
/// 托盘和窗口共用的退出入口。swap 保证重复点击不会启动多个退出线程。
/// 先通知串流停止，再释放防休眠、停止采集，等会话及密码管道收尾后退出进程。
/// 等待放在后台线程中，避免主窗口线程卡住，阻碍收尾事件和界面响应。
/// Shared tray/window exit entry; swap prevents duplicate shutdown workers.
/// Signal stream stop, release awake state, stop capture, then wait for session/pipe cleanup before exit.
/// Wait off the window thread so cleanup events and UI processing can continue.
pub(crate) fn quit(app: &tauri::AppHandle) {
    let engine = app.state::<Arc<Engine>>().inner().clone();
    if engine.quitting.swap(true, Ordering::SeqCst) {
        return;
    }
    let deadline = Instant::now() + homepod_test::shutdown::STOP_TIMEOUT;
    let (done, completed) = mpsc::sync_channel(1);
    let exit_app = app.clone();
    // Independent of Engine locks and OS callbacks. Job Objects close with this process,
    // so a stuck cleanup cannot leave our native streaming child behind.
    thread::spawn(move || {
        let clean = completed
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .unwrap_or(false);
        let code = if clean { 0 } else { 1 };
        exit_app.exit(code);
        // Even a successful cleanup does not prove the main event loop processed exit.
        // Keep the fallback alive until the original deadline, with no fresh timeout.
        thread::sleep(deadline.saturating_duration_since(Instant::now()));
        std::process::exit(code);
    });
    thread::spawn(move || {
        if let Some(s) = engine.session.lock().unwrap().as_ref() {
            s.control.request_stop_until(deadline);
        }
        let _ = engine.awake.lock().unwrap().set(false);
        let source_stopped = stop_source_until(&engine, deadline).is_ok();
        while engine.session.lock().unwrap().is_some() && Instant::now() < deadline {
            thread::sleep(
                deadline
                    .saturating_duration_since(Instant::now())
                    .min(Duration::from_millis(20)),
            );
        }
        let clean = source_stopped && engine.session.lock().unwrap().is_none();
        let _ = done.send(clean);
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
