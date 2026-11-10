//! 当前用户的开机自启项，启用后才写入注册表。
//! Current-user startup entry; absent until explicitly enabled in Settings.
use std::path::Path;
use windows::{
    Win32::{Foundation::ERROR_FILE_NOT_FOUND, System::Registry::*},
    core::{PCWSTR, w},
};

const RUN: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");

fn entry_name(exe: &Path) -> PCWSTR {
    // 预览版与现有安装可并存，各自维护开机自启项。
    // The preview and installed app can coexist with separate startup entries.
    if exe
        .file_name()
        .is_some_and(|name| name.eq_ignore_ascii_case("airplay-bridge-gpui.exe"))
    {
        w!("AirPlay Bridge GPUI Preview")
    } else {
        w!("AirPlay Hub")
    }
}

pub fn enabled() -> Result<bool, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            RUN,
            entry_name(&exe),
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
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let name = entry_name(&exe);
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
            use std::os::windows::ffi::OsStrExt;
            let mut command = vec![b'"' as u16];
            command.extend(exe.as_os_str().encode_wide());
            command.extend([b'"' as u16, 0]);
            let bytes: Vec<u8> = command.iter().flat_map(|c| c.to_le_bytes()).collect();
            unsafe { RegSetValueExW(key, name, None, REG_SZ, Some(&bytes)) }
                .ok()
                .map_err(|e| e.to_string())
        } else {
            let status = unsafe { RegDeleteValueW(key, name) };
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
