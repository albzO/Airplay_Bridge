#![windows_subsystem = "windows"]
//! Ely GPUI 播放入口及 Windows 事件循环。
//! Ely GPUI player entry point and Windows event loop.
//!
//! 会话、事件收件箱、诊断、音量和后台作业与 src-gpui 共用同一份源码；它们不依赖 gpui。
//! Session, inbox, diagnostics, volume and worker jobs share one source with src-gpui; none of
//! them depends on gpui.
#[path = "../../src-gpui/src/desktop_job.rs"]
mod desktop_job;
#[path = "../../src-gpui/src/diagnostics.rs"]
mod diagnostics;
mod elastic;
#[path = "../../src-gpui/src/inbox.rs"]
mod inbox;
mod lang;
mod panel;
mod player;
// Phase::label 只有 src-gpui 使用；这里按当前语言另取文字。
// Only src-gpui uses Phase::label; this UI picks the text for the current language itself.
#[allow(dead_code)]
#[path = "../../src-gpui/src/session.rs"]
mod session;
mod settings_view;
mod shell;
mod source_picker;
mod view;
mod visibility;
#[path = "../../src-gpui/src/volume.rs"]
mod volume;

use airplay_desktop::Engine;
use anyhow::Result;
use ely_core::Assets;
use gpui::{App, AppContext, Bounds, TitlebarOptions, WindowBounds, WindowOptions, px, size};
use player::Player;
use std::{cell::Cell, rc::Rc, sync::Arc};
use windows::{
    Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MessageBoxW},
    core::{HSTRING, w},
};

fn main() {
    match run() {
        Ok(code) => std::process::exit(code),
        Err(error) => {
            let message = HSTRING::from(error.to_string());
            unsafe { MessageBoxW(None, &message, w!("AirPlay Hub"), MB_ICONERROR) };
            std::process::exit(1);
        }
    }
}

fn run() -> Result<i32> {
    let Some(_instance) = shell::primary_instance()? else {
        return Ok(0);
    };
    let engine = Engine::load()?;
    // 窗口和托盘创建前确定语言，首屏即为所选语言。
    // Settle the language before the window and tray exist, so the first frame uses it.
    lang::set(lang::load(engine.data_root()));
    let exit_code = Rc::new(Cell::new(0));
    let setup_error = Rc::new(Cell::new(None));
    let app_exit = exit_code.clone();
    let app_error = setup_error.clone();
    gpui_platform::application()
        .with_assets(Assets)
        .run(move |cx: &mut App| {
            let opened = ely_core::init(cx).and_then(|()| {
                lang::sync_components(cx);
                open_player(cx, engine, app_exit)
            });
            if let Err(error) = opened {
                app_error.set(Some(error));
                cx.quit();
            }
        });
    match setup_error.take() {
        Some(error) => Err(error),
        None => Ok(exit_code.get()),
    }
}

fn open_player(cx: &mut App, engine: Arc<Engine>, exit_code: Rc<Cell<i32>>) -> Result<()> {
    let bounds = Bounds::centered(None, size(px(900.), px(720.)), cx);
    let player_engine = engine.clone();
    let window = cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            // 最小尺寸：三张指标卡、扬声器两列和底部栏在此宽度下仍不换行、不截断。
            // Minimum size: the three stat tiles, both speaker columns and the dock still fit.
            window_min_size: Some(size(px(720.), px(560.))),
            titlebar: Some(TitlebarOptions {
                title: Some(shell::TITLE.into()),
                // 隐藏系统标题栏；窗口按钮由界面自己绘制。
                // Hide the system title bar; the view draws its own window buttons.
                appears_transparent: true,
                ..Default::default()
            }),
            ..Default::default()
        },
        move |window, cx| cx.new(|cx| Player::new(player_engine, window, cx)),
    )?;
    shell::attach(cx, window, engine, exit_code)?;
    cx.activate(true);
    Ok(())
}
