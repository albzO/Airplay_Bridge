use super::*;
#[test]
fn portable_marker_selects_gui_and_cli_but_not_other_directories() {
    let base = base();
    assert_eq!(portable_root(&base.join("airplay-bridge.exe")), None);
    fs::write(base.join("portable.flag"), b"").unwrap();
    assert_eq!(
        portable_root(&base.join("airplay-bridge.exe")),
        Some(base.clone())
    );
    assert_eq!(
        portable_root(&base.join("tools/homepod-test.exe")),
        Some(base.clone())
    );
    assert_eq!(portable_root(&base.join("other/test.exe")), None);
    assert!(!base.join("data").exists());
    fs::remove_file(base.join("portable.flag")).unwrap();
    fs::remove_dir(base).unwrap();
}
fn base() -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "airplay-data-dir-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir(&path).unwrap();
    path.canonicalize().unwrap()
}
#[test]
fn migrates_legacy_settings_and_cli_together_and_is_repeatable() {
    let base = base();
    let old = base.join("com.airplaywin.bridge");
    fs::create_dir_all(old.join("cli")).unwrap();
    fs::write(old.join("settings.json"), b"existing settings").unwrap();
    fs::write(old.join("cli/devices.json"), b"existing devices").unwrap();
    let new = prepare_at(&base).unwrap();
    assert_eq!(new, base.join("AirPlay Hub"));
    assert!(old.exists());
    assert_eq!(
        fs::read(new.join("settings.json")).unwrap(),
        b"existing settings"
    );
    assert_eq!(
        fs::read(new.join("cli/devices.json")).unwrap(),
        b"existing devices"
    );
    assert_eq!(prepare_at(&base).unwrap(), new);
    fs::remove_file(new.join("settings.json")).unwrap();
    fs::remove_file(new.join("cli/devices.json")).unwrap();
    fs::remove_file(new.join(".legacy-data-migrated")).unwrap();
    fs::remove_file(old.join("settings.json")).unwrap();
    fs::remove_file(old.join("cli/devices.json")).unwrap();
    fs::remove_dir(old.join("cli")).unwrap();
    fs::remove_dir(old).unwrap();
    fs::remove_dir(new.join("cli")).unwrap();
    fs::remove_dir(new).unwrap();
    fs::remove_dir(base).unwrap();
}
#[test]
fn existing_new_directory_is_preserved() {
    let base = base();
    let old = base.join("com.airplaywin.bridge");
    let new = base.join("AirPlay Hub");
    fs::create_dir(&old).unwrap();
    fs::create_dir(&new).unwrap();
    fs::write(old.join("settings.json"), b"old").unwrap();
    fs::write(old.join("old.log"), b"history").unwrap();
    fs::write(new.join("settings.json"), b"new").unwrap();
    assert_eq!(prepare_at(&base).unwrap(), new);
    assert_eq!(fs::read(new.join("settings.json")).unwrap(), b"new");
    assert!(old.exists());
    assert_eq!(fs::read(new.join("old.log")).unwrap(), b"history");
    // Once migrated, deleted data must not reappear on subsequent launches.
    fs::remove_file(new.join("old.log")).unwrap();
    prepare_at(&base).unwrap();
    assert!(!new.join("old.log").exists());
    fs::remove_file(old.join("settings.json")).unwrap();
    fs::remove_file(old.join("old.log")).unwrap();
    fs::remove_file(new.join("settings.json")).unwrap();
    fs::remove_file(new.join(".legacy-data-migrated")).unwrap();
    fs::remove_dir(old).unwrap();
    fs::remove_dir(new).unwrap();
    fs::remove_dir(base).unwrap();
}
