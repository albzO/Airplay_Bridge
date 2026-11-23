//! 将 Cargo 版本和程序图标写入 Windows 文件资源。
//! Embed the Cargo version and app icon in Windows file resources.
use std::{env, fs, path::PathBuf};

fn main() {
    let icon = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../src-tauri/icons/icon.ico");
    println!("cargo:rerun-if-changed={}", icon.display());
    println!("cargo:rerun-if-changed=build.rs");
    let icon = icon.to_string_lossy().replace('\\', "\\\\");
    let major = env!("CARGO_PKG_VERSION_MAJOR");
    let minor = env!("CARGO_PKG_VERSION_MINOR");
    let patch = env!("CARGO_PKG_VERSION_PATCH");
    // 补丁号为 0 时显示两段版本（2.0），数字资源版本仍为四段。
    // Show a two-part version (2.0) when the patch is 0; the numeric resource version stays four-part.
    let version = match patch {
        "0" => format!("{major}.{minor}"),
        _ => env!("CARGO_PKG_VERSION").to_owned(),
    };
    let resource = PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("airplay-bridge.rc");
    fs::write(
        &resource,
        format!(
            r#"
#pragma code_page(65001)
1 ICON "{icon}"
1 VERSIONINFO
FILEVERSION {major},{minor},{patch},0
PRODUCTVERSION {major},{minor},{patch},0
BEGIN
    BLOCK "StringFileInfo"
    BEGIN
        BLOCK "040904b0"
        BEGIN
            VALUE "FileDescription", "AirPlay Bridge\0"
            VALUE "FileVersion", "{version}\0"
            VALUE "ProductName", "AirPlay Bridge\0"
            VALUE "ProductVersion", "{version}\0"
        END
    END
    BLOCK "VarFileInfo"
    BEGIN
        VALUE "Translation", 0x0409, 1200
    END
END
"#
        ),
    )
    .unwrap();
    embed_resource::compile(&resource, embed_resource::NONE)
        .manifest_required()
        .unwrap();
}
