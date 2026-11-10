#![windows_subsystem = "windows"]
//! GPUI 播放入口及 Windows 事件循环。
//! GPUI player entry point and Windows event loop.
mod diagnostics;
mod inbox;
mod player;
mod session;
mod shell;
mod volume;

use airplay_desktop::Engine;
use anyhow::Result;
use gpui::{
    App, AppContext, Application, Bounds, TitlebarOptions, WindowBounds, WindowOptions, px, size,
};
use gpui_component::Root;
use player::Player;
use std::{cell::Cell, rc::Rc};
use windows::{
    Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MessageBoxW},
    core::{HSTRING, w},
};

fn main() {
    match run() {
        Ok(code) => std::process::exit(code),
        Err(error) => {
            let message = HSTRING::from(error.to_string());
            unsafe { MessageBoxW(None, &message, w!("AirPlay Bridge"), MB_ICONERROR) };
            std::process::exit(1);
        }
    }
}

fn run() -> Result<i32> {
    let Some(_instance) = shell::primary_instance()? else {
        return Ok(0);
    };
    let engine = Engine::load()?;
    let exit_code = Rc::new(Cell::new(0));
    let setup_error = Rc::new(Cell::new(None));
    let app_exit = exit_code.clone();
    let app_error = setup_error.clone();
    Application::new().run(move |cx| {
        gpui_component::init(cx);
        if let Err(error) = open_player(cx, engine, app_exit) {
            app_error.set(Some(error));
            cx.quit();
        }
    });
    match setup_error.take() {
        Some(error) => Err(error),
        None => Ok(exit_code.get()),
    }
}

fn open_player(
    cx: &mut App,
    engine: std::sync::Arc<Engine>,
    exit_code: Rc<Cell<i32>>,
) -> Result<()> {
    let bounds = Bounds::centered(None, size(px(900.), px(670.)), cx);
    let player_engine = engine.clone();
    let window = cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            window_min_size: Some(size(px(680.), px(520.))),
            // 保留 Windows 原生标题栏的拖动、缩放及系统菜单。
            // Keep Windows caption dragging, resizing and the system menu.
            titlebar: Some(TitlebarOptions {
                title: Some("AirPlay Bridge · GPUI".into()),
                ..Default::default()
            }),
            ..Default::default()
        },
        move |window, cx| {
            let player = cx.new(|cx| Player::new(player_engine, window, cx));
            cx.new(|cx| Root::new(player, window, cx))
        },
    )?;
    shell::attach(cx, window, engine, exit_code)?;
    cx.activate(true);
    Ok(())
}
