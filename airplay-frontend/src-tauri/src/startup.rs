//! Current-user startup entry; absent until explicitly enabled in Settings.
use windows::{
    Win32::{Foundation::ERROR_FILE_NOT_FOUND, System::Registry::*},
    core::{PCWSTR, w},
};

const RUN: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
const NAME: PCWSTR = w!("AirPlay Hub");

pub fn enabled() -> Result<bool, String> {
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            RUN,
            NAME,
            RRF_RT_REG_SZ,
            None,
            None,
            None,
        )
    };
    if status == ERROR_FILE_NOT_FOUND {
        return Ok(false);
    }
    status.ok().map(|_| true).map_err(|e| e.to_string())
}

pub fn set(enabled: bool) -> Result<(), String> {
    let mut key = HKEY::default();
    unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            RUN,
            None,
            PCWSTR::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_SET_VALUE,
            None,
            &mut key,
            None,
        )
        .ok()
        .map_err(|e| e.to_string())?;
    }
    let result = (|| {
        if enabled {
            let exe = std::env::current_exe().map_err(|e| e.to_string())?;
            use std::os::windows::ffi::OsStrExt;
            let mut command = vec![b'"' as u16];
            command.extend(exe.as_os_str().encode_wide());
            command.extend([b'"' as u16, 0]);
            let bytes: Vec<u8> = command.iter().flat_map(|c| c.to_le_bytes()).collect();
            unsafe { RegSetValueExW(key, NAME, None, REG_SZ, Some(&bytes)) }
                .ok()
                .map_err(|e| e.to_string())
        } else {
            let status = unsafe { RegDeleteValueW(key, NAME) };
            if status == ERROR_FILE_NOT_FOUND {
                Ok(())
            } else {
                status.ok().map_err(|e| e.to_string())
            }
        }
    })();
    unsafe {
        let _ = RegCloseKey(key);
    }
    result
}
