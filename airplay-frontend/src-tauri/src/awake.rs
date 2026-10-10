//! 应用级系统/显示器电源请求，避免自动熄屏触发采集端点时钟异常。
//! App-scoped system/display requests avoid automatic display-off transitions during capture.
use windows::{
    Win32::{
        Foundation::{CloseHandle, HANDLE},
        System::{
            Power::{
                POWER_REQUEST_TYPE, PowerClearRequest, PowerCreateRequest,
                PowerRequestDisplayRequired, PowerRequestSystemRequired, PowerSetRequest,
            },
            Threading::{POWER_REQUEST_CONTEXT_SIMPLE_STRING, REASON_CONTEXT, REASON_CONTEXT_0},
        },
    },
    core::PWSTR,
};

// 两类请求都成功后才能发布 active；第二步失败要撤销第一步。
// Publish active only after both requests succeed; roll back the first if the second fails.
fn activate(
    mut apply: impl FnMut(POWER_REQUEST_TYPE, bool) -> Result<(), String>,
) -> Result<(), String> {
    apply(PowerRequestSystemRequired, true).map_err(|e| format!("启用系统唤醒失败：{e}"))?;
    if let Err(error) = apply(PowerRequestDisplayRequired, true) {
        let rollback = apply(PowerRequestSystemRequired, false).err();
        return Err(format!(
            "阻止屏幕自动熄灭失败：{error}{}",
            rollback
                .map(|e| format!("；撤销系统请求失败：{e}"))
                .unwrap_or_default()
        ));
    }
    Ok(())
}
#[derive(Default)]
pub struct Awake {
    handle: Option<usize>,
}
impl Awake {
    pub fn active(&self) -> bool {
        self.handle.is_some()
    }
    pub fn set(&mut self, enabled: bool) -> Result<(), String> {
        if enabled == self.active() {
            return Ok(());
        }
        if enabled {
            let mut reason: Vec<u16> = "AirPlay Hub：用户要求保持系统与屏幕唤醒，避免音频采集中断"
                .encode_utf16()
                .chain(Some(0))
                .collect();
            let context = REASON_CONTEXT {
                Version: 0,
                Flags: POWER_REQUEST_CONTEXT_SIMPLE_STRING,
                Reason: REASON_CONTEXT_0 {
                    SimpleReasonString: PWSTR(reason.as_mut_ptr()),
                },
            };
            let handle = unsafe { PowerCreateRequest(&context) }
                .map_err(|e| format!("创建系统唤醒请求失败：{e}"))?;
            if let Err(error) = activate(|kind, enabled| {
                unsafe {
                    if enabled {
                        PowerSetRequest(handle, kind)
                    } else {
                        PowerClearRequest(handle, kind)
                    }
                }
                .map_err(|e| e.to_string())
            }) {
                let _ = unsafe { CloseHandle(handle) };
                return Err(error);
            }
            self.handle = Some(handle.0 as usize);
        } else if let Some(handle) = self.handle {
            let handle = HANDLE(handle as *mut _);
            // 销毁同一请求对象，一次释放两类请求，避免部分清除后 active 与实际状态不一致。
            // Destroy the shared request object to release both types without a partially cleared state.
            unsafe { CloseHandle(handle) }
                .map_err(|e| format!("释放系统与屏幕唤醒请求失败：{e}"))?;
            self.handle = None;
        }
        Ok(())
    }
}
impl Drop for Awake {
    fn drop(&mut self) {
        if let Some(handle) = self.handle.take() {
            let handle = HANDLE(handle as *mut _);
            let _ = unsafe { PowerClearRequest(handle, PowerRequestDisplayRequired) };
            let _ = unsafe { PowerClearRequest(handle, PowerRequestSystemRequired) };
            let _ = unsafe { CloseHandle(handle) };
        }
    }
}
#[cfg(test)]
#[path = "../../../test/frontend/desktop/awake.rs"]
mod tests;
