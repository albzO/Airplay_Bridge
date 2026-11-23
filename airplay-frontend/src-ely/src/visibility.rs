//! 窗口是否能被看到：未收到托盘、未最小化、显示器未熄灭。未串流时据此暂停和恢复采集。
//! Whether the window can be seen: not in the tray, not minimized, display not off. Capture is
//! paused and resumed from this while nothing is streaming.
use async_channel::{Receiver, Sender};
use std::{
    ffi::c_void,
    sync::{
        OnceLock,
        atomic::{AtomicBool, Ordering},
    },
};
use windows::Win32::{
    Foundation::{HANDLE, HWND},
    System::{
        Power::{
            DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS, POWERBROADCAST_SETTING,
            PowerSettingRegisterNotification,
        },
        SystemServices::GUID_CONSOLE_DISPLAY_STATE,
    },
    UI::WindowsAndMessaging::{DEVICE_NOTIFY_CALLBACK, IsIconic, IsWindowVisible},
};

static DISPLAY_ON: AtomicBool = AtomicBool::new(true);
static CHANGED: OnceLock<Sender<()>> = OnceLock::new();

/// 可能影响可见性的事件发生后调用；接收方自行重新判断，多次通知合并成一次。
/// Call after anything that may affect visibility; the receiver re-evaluates, and repeated
/// notifications collapse into one.
pub fn changed() {
    if let Some(sender) = CHANGED.get() {
        let _ = sender.try_send(());
    }
}

/// 开始监听，只调用一次。无法订阅显示器状态时视为一直亮屏。
/// Starts watching; call once. If the display state cannot be subscribed to, it counts as on.
pub fn watch() -> Receiver<()> {
    let (send, receive) = async_channel::bounded(1);
    let _ = CHANGED.set(send);
    // 登记持续到进程退出，参数随之常驻；回调只访问静态数据。
    // Registered for the life of the process, so the parameters live as long; the callback
    // touches only statics.
    let parameters = Box::leak(Box::new(DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS {
        Callback: Some(display_changed),
        Context: std::ptr::null_mut(),
    }));
    let mut registration = std::ptr::null_mut();
    let _ = unsafe {
        PowerSettingRegisterNotification(
            &GUID_CONSOLE_DISPLAY_STATE,
            DEVICE_NOTIFY_CALLBACK,
            HANDLE(std::ptr::from_mut(parameters).cast::<c_void>()),
            &mut registration,
        )
    };
    receive
}

unsafe extern "system" fn display_changed(_: *const c_void, _: u32, setting: *const c_void) -> u32 {
    if let Some(setting) = unsafe { (setting as *const POWERBROADCAST_SETTING).as_ref() }
        && setting.PowerSetting == GUID_CONSOLE_DISPLAY_STATE
    {
        // 0 熄灭，1 点亮，2 变暗。
        // 0 off, 1 on, 2 dimmed.
        DISPLAY_ON.store(setting.Data[0] != 0, Ordering::Relaxed);
        changed();
    }
    0
}

pub fn seen(hwnd: HWND) -> bool {
    DISPLAY_ON.load(Ordering::Relaxed)
        && unsafe { IsWindowVisible(hwnd).as_bool() && !IsIconic(hwnd).as_bool() }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Pause,
    Resume,
}

/// 未串流时该对采集做什么。`paused` 表示采集是被本机制停掉的，用户自己关闭的采集不会被恢复。
/// What to do with capture while idle. `paused` means this mechanism stopped it; capture the user
/// turned off is never resumed.
pub fn next_step(seen: bool, locked: bool, capturing: bool, paused: bool) -> Option<Step> {
    match (locked, seen) {
        (false, false) if capturing => Some(Step::Pause),
        (false, true) if paused => Some(Step::Resume),
        _ => None,
    }
}

#[cfg(test)]
#[path = "../../../test/frontend/desktop/ely_visibility.rs"]
mod tests;
