//! An application-scoped system sleep request; does not keep the display on.
use windows::{
    Win32::{
        Foundation::{CloseHandle, HANDLE},
        System::{
            Power::{
                PowerClearRequest, PowerCreateRequest, PowerRequestSystemRequired, PowerSetRequest,
            },
            Threading::{POWER_REQUEST_CONTEXT_SIMPLE_STRING, REASON_CONTEXT, REASON_CONTEXT_0},
        },
    },
    core::PWSTR,
};
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
            let mut reason: Vec<u16> = "AirPlay Bridge: user requested system awake"
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
            if let Err(error) = unsafe { PowerSetRequest(handle, PowerRequestSystemRequired) } {
                let _ = unsafe { CloseHandle(handle) };
                return Err(format!("启用系统唤醒失败：{error}"));
            }
            self.handle = Some(handle.0 as usize);
        } else if let Some(handle) = self.handle {
            let handle = HANDLE(handle as *mut _);
            unsafe { PowerClearRequest(handle, PowerRequestSystemRequired) }
                .map_err(|e| format!("释放系统唤醒请求失败：{e}"))?;
            self.handle = None;
            let _ = unsafe { CloseHandle(handle) };
        }
        Ok(())
    }
}
impl Drop for Awake {
    fn drop(&mut self) {
        if let Some(handle) = self.handle.take() {
            let handle = HANDLE(handle as *mut _);
            let _ = unsafe { PowerClearRequest(handle, PowerRequestSystemRequired) };
            let _ = unsafe { CloseHandle(handle) };
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn system_request_can_be_enabled_and_released_without_reference_leaks() {
        let mut request = Awake::default();
        request.set(true).unwrap();
        request.set(true).unwrap();
        assert!(request.active());
        request.set(false).unwrap();
        request.set(false).unwrap();
        assert!(!request.active());
    }
}
