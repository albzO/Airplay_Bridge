//! Win32 单实例信号、托盘和窗口退出。
//! Win32 single-instance signal, tray and window shutdown.
use airplay_desktop::Engine;
use anyhow::{Result, anyhow};
use async_channel::Sender;
use gpui::{App, Window, WindowHandle};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::{cell::Cell, rc::Rc, sync::Arc, thread, time::Instant};
use tray_icon::{
    Icon, MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent,
    menu::{Menu, MenuEvent, MenuItem},
};
use windows::{
    Win32::{
        Foundation::{
            ERROR_ALREADY_EXISTS, ERROR_SUCCESS, GetLastError, HANDLE, HWND, SetLastError,
            WAIT_OBJECT_0,
        },
        System::Threading::{
            CreateEventW, INFINITE, OpenEventW, SYNCHRONIZATION_SYNCHRONIZE, SetEvent,
            WaitForSingleObject,
        },
        UI::WindowsAndMessaging::{
            IsIconic, SW_HIDE, SW_RESTORE, SW_SHOW, SetForegroundWindow, ShowWindow,
        },
    },
    core::{Owned, PCWSTR, w},
};

fn wake_event(preview: bool) -> PCWSTR {
    if preview {
        w!("Local\\AirPlayBridge.GPUI.Preview.Reveal.v1")
    } else {
        w!("Local\\AirPlayBridge.GPUI.Reveal.v1")
    }
}

pub fn title(preview: bool) -> &'static str {
    if preview {
        "AirPlay Bridge · GPUI"
    } else {
        "AirPlay Bridge"
    }
}

enum Action {
    Reveal,
    Hide,
    Quit,
    Exit(i32),
}

pub fn primary_instance(preview: bool) -> Result<Option<Owned<HANDLE>>> {
    let (event, already_running) = unsafe {
        SetLastError(ERROR_SUCCESS);
        let event = CreateEventW(None, false, false, wake_event(preview))?;
        (event, GetLastError() == ERROR_ALREADY_EXISTS)
    };
    let event = unsafe { Owned::new(event) };
    if already_running {
        unsafe { SetEvent(*event)? };
        return Ok(None);
    }
    Ok(Some(event))
}

pub fn attach<T: 'static>(
    cx: &mut App,
    window: WindowHandle<T>,
    engine: Arc<Engine>,
    exit_code: Rc<Cell<i32>>,
    preview: bool,
) -> Result<()> {
    let (send, receive) = async_channel::unbounded();
    let hwnd = window.update(cx, |_, window, cx| {
        let engine = engine.clone();
        let send = send.clone();
        window.on_window_should_close(cx, move |_, _| {
            let action = if engine.quit_on_close() {
                Action::Quit
            } else {
                Action::Hide
            };
            let _ = send.try_send(action);
            false
        });
        native_window(window)
    })??;

    let icon =
        image::load_from_memory(include_bytes!("../../src-tauri/icons/icon.png"))?.into_rgba8();
    let (width, height) = icon.dimensions();
    let menu = Menu::new();
    menu.append_items(&[
        &MenuItem::with_id("open", "打开 AirPlay Bridge", true, None),
        &MenuItem::with_id("quit", "退出", true, None),
    ])?;
    let tray = TrayIconBuilder::new()
        .with_icon(Icon::from_rgba(icon.into_raw(), width, height)?)
        .with_tooltip(title(preview))
        .with_menu(Box::new(menu))
        .with_menu_on_left_click(false)
        .build()?;
    let menu_send = send.clone();
    MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
        let action = match event.id.as_ref() {
            "open" => Action::Reveal,
            "quit" => Action::Quit,
            _ => return,
        };
        let _ = menu_send.try_send(action);
    }));
    let tray_send = send.clone();
    TrayIconEvent::set_event_handler(Some(move |event| {
        if matches!(
            event,
            TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            }
        ) {
            let _ = tray_send.try_send(Action::Reveal);
        }
    }));

    let wake_send = send.clone();
    thread::Builder::new()
        .name("gpui-instance-wake".into())
        .spawn(move || {
            let Ok(event) =
                (unsafe { OpenEventW(SYNCHRONIZATION_SYNCHRONIZE, false, wake_event(preview)) })
            else {
                return;
            };
            let event = unsafe { Owned::new(event) };
            while unsafe { WaitForSingleObject(*event, INFINITE) } == WAIT_OBJECT_0 {
                if wake_send.try_send(Action::Reveal).is_err() {
                    break;
                }
            }
        })?;

    cx.spawn(async move |cx| {
        while let Ok(action) = receive.recv().await {
            match action {
                Action::Reveal if !engine.is_quitting() => unsafe {
                    let _ = ShowWindow(
                        hwnd,
                        if IsIconic(hwnd).as_bool() {
                            SW_RESTORE
                        } else {
                            SW_SHOW
                        },
                    );
                    let _ = SetForegroundWindow(hwnd);
                },
                Action::Hide => unsafe {
                    let _ = ShowWindow(hwnd, SW_HIDE);
                },
                Action::Quit => begin_quit(&engine, send.clone()),
                Action::Exit(code) => {
                    exit_code.set(code);
                    drop(tray);
                    let _ = cx.update(|cx| cx.quit());
                    break;
                }
                Action::Reveal => {}
            }
        }
    })
    .detach();
    Ok(())
}

fn native_window(window: &Window) -> Result<HWND> {
    let handle = HasWindowHandle::window_handle(window)
        .map_err(|error| anyhow!("无法取得 Windows 窗口句柄：{error:?}"))?;
    match handle.as_raw() {
        RawWindowHandle::Win32(handle) => Ok(HWND(handle.hwnd.get() as _)),
        _ => Err(anyhow!("无法取得 Windows 窗口句柄")),
    }
}

fn begin_quit(engine: &Arc<Engine>, send: Sender<Action>) {
    let Some((deadline, done)) = engine.begin_shutdown() else {
        return;
    };
    thread::spawn(move || {
        let clean = done
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .unwrap_or(false);
        let code = if clean { 0 } else { 1 };
        let _ = send.try_send(Action::Exit(code));
        // 复用桌面服务的截止时间，避免窗口线程阻塞时留下后端进程。
        // Reuse the service deadline so a blocked window thread cannot leave the backend running.
        thread::sleep(deadline.saturating_duration_since(Instant::now()));
        std::process::exit(code);
    });
}
