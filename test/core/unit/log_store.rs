use super::*;
struct Fixture(PathBuf);
impl Fixture {
    fn new(name: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("airplay-log-store-{name}-{}", std::process::id()));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
#[test]
fn file_cap_does_not_partially_write_records_or_resume_after_overflow() {
    let f = Fixture::new("cap");
    let path = f.0.join("live-1.log");
    let mut log = LogFile::create_limited(&path, 8).unwrap();
    log.write_all(b"123456\n").unwrap();
    assert!(log.write_all(b"oversized\n").is_err());
    log.record(b"x").unwrap();
    log.flush().unwrap();
    assert!(log.capped());
    assert_eq!(fs::read(&path).unwrap(), b"123456\n");
    assert!(LogFile::create(&path).is_err());
    drop(log);
    assert!(write(&path, b"oversized", 8).is_err());
    assert_eq!(fs::read(&path).unwrap(), b"123456\n");
}
#[test]
fn retention_reserves_active_growth_and_preserves_unknown_files_and_directories() {
    let f = Fixture::new("retain");
    let path = f.0.join("live-2.log");
    let _live = LogFile::create_limited(&path, 8).unwrap();
    fs::write(f.0.join("live-1.json"), b"12345").unwrap();
    fs::write(f.0.join("notes.log"), b"user contents").unwrap();
    fs::create_dir(f.0.join("live-3.log")).unwrap();
    let root = f.0.canonicalize().unwrap();
    reserve(
        &root,
        &root.join("live-4.log"),
        8,
        &active().lock().unwrap(),
        16,
        2,
    )
    .unwrap();
    assert!(!f.0.join("live-1.json").exists());
    assert!(path.exists());
    assert!(f.0.join("notes.log").exists());
    assert!(f.0.join("live-3.log").is_dir());
    assert!(
        reserve(
            &root,
            &root.join("live-5.log"),
            9,
            &active().lock().unwrap(),
            16,
            2
        )
        .is_err()
    );
}
#[test]
fn retention_count_and_recognized_names_cover_rotations() {
    let f = Fixture::new("count");
    let root = f.0.canonicalize().unwrap();
    for name in [
        "live-1.capture.previous-1.jsonl",
        "backend-2.log",
        "ui-fault-3.log",
        "source-startup.previous.jsonl",
    ] {
        assert!(managed_name(name));
        fs::write(root.join(name), b"1").unwrap();
    }
    for name in [
        "live-manual.log",
        "notes.json",
        "live-1.wav",
        "source-startup.custom.jsonl",
    ] {
        assert!(!managed_name(name));
    }
    reserve(&root, &root.join("live-4.json"), 1, &HashMap::new(), 100, 2).unwrap();
    assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
    // Windows 创建符号链接可能需要开发者模式，拒绝跟随的行为另以普通目录覆盖。
    // Windows symlink creation can require Developer Mode; ordinary directories are covered separately.
}

#[cfg(windows)]
#[test]
fn retention_preserves_open_files_even_without_a_local_registry_entry() {
    use std::os::windows::fs::OpenOptionsExt;
    let f = Fixture::new("external");
    let root = f.0.canonicalize().unwrap();
    let path = root.join("live-1.log");
    let handle = OpenOptions::new()
        .create_new(true)
        .write(true)
        .share_mode(1)
        .open(&path)
        .unwrap();
    assert!(reserve(&root, &root.join("live-2.log"), 8, &HashMap::new(), 8, 1).is_err());
    assert!(path.exists());
    drop(handle);
    reserve(&root, &root.join("live-2.log"), 8, &HashMap::new(), 8, 1).unwrap();
    assert!(!path.exists());
}

#[test]
fn bounded_log_text_preserves_utf8_and_short_messages() {
    assert_eq!(bounded_text("short".into()), "short");
    let value = bounded_text("音".repeat(9000));
    assert_eq!(value.chars().count(), 8192);
    assert!(value.ends_with('…'));
}
