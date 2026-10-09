use super::*;

#[test]
fn executable_search_never_uses_user_data_or_working_directory() {
    let root = std::env::temp_dir().join(format!("airplay-loader-{}", std::process::id()));
    let installed = root.join("installed");
    let data = root.join("data");
    fs::create_dir_all(&installed).unwrap();
    fs::create_dir_all(&data).unwrap();
    fs::write(data.join("airplay-backend.exe"), b"untrusted").unwrap();
    assert_ne!(
        resolve_executable(&installed),
        data.join("airplay-backend.exe")
    );
    assert!(resolve_executable(Path::new("")).as_os_str().is_empty());
    fs::create_dir_all(installed.join("runtime")).unwrap();
    fs::write(
        installed.join("runtime/airplay-backend.exe"),
        b"trusted location",
    )
    .unwrap();
    assert_eq!(
        resolve_executable(&installed),
        installed.join("runtime/airplay-backend.exe")
    );
    let tools = installed.join("tools");
    fs::create_dir(&tools).unwrap();
    assert_eq!(
        resolve_executable(&tools),
        installed.join("runtime/airplay-backend.exe")
    );
    fs::remove_dir(tools).unwrap();
    fs::remove_file(data.join("airplay-backend.exe")).unwrap();
    fs::remove_file(installed.join("runtime/airplay-backend.exe")).unwrap();
    fs::remove_dir(installed.join("runtime")).unwrap();
    fs::remove_dir(installed).unwrap();
    fs::remove_dir(data).unwrap();
    fs::remove_dir(root).unwrap();
}
