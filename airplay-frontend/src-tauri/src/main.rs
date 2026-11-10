#![windows_subsystem = "windows"]
//! Tauri 窗口、托盘与 IPC 入口。桌面服务在 airplay-desktop。
//! Tauri window, tray and IPC entry point. Desktop services live in airplay-desktop.
mod commands;
#[path = "../../../test/frontend/desktop/startup.rs"]
mod startup_checks;
mod window;

use airplay_desktop::Engine;
use homepod_test::capture;
use std::sync::Arc;
use tauri::{
    Manager, State,
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
};
use window::{quit, reveal, window_action};

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
            app.manage(Engine::load()?);
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
            commands::initialize,
            commands::discover_devices,
            commands::save_settings,
            commands::monitor_source,
            commands::set_capture_enabled,
            commands::set_autostart,
            commands::start_stream,
            commands::submit_password,
            commands::stop_stream,
            commands::set_volume,
            commands::set_mapping,
            commands::set_speaker_order,
            commands::open_logs,
            commands::forget_auth_policy,
            ui_ready,
            window_action
        ])
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let engine = window.state::<Arc<Engine>>();
                if !engine.is_quitting() {
                    api.prevent_close();
                    if engine.quit_on_close()
                        || startup_checks::quit_on_smoke_close(window.app_handle())
                    {
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
