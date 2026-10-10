use super::*;
use std::{fs, path::PathBuf};

struct Fixture(PathBuf);
impl Fixture {
    fn new(name: &str) -> Self {
        let root =
            std::env::temp_dir().join(format!("airplay-routing-{name}-{}", std::process::id()));
        fs::create_dir(&root).unwrap();
        Settings::default().save(&root).unwrap();
        // 在暂存文件位置创建目录，触发真实写入失败。
        // A directory at the temporary-file path produces a real write failure.
        fs::create_dir(root.join("settings.pending.json")).unwrap();
        Self(root)
    }
    fn allow_save(&self) {
        fs::remove_dir(self.0.join("settings.pending.json")).unwrap();
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn mapping_save_failure_preserves_disk_memory_and_both_live_controls_then_retries() {
    let fixture = Fixture::new("mapping");
    let mut settings = Settings::default();
    let session = Mutex::new(settings.mapping);
    let source = Mutex::new(settings.mapping);
    assert!(
        set_mapping(
            &fixture.0,
            &mut settings,
            Some(&session),
            Some(&source),
            [1, 0]
        )
        .is_err()
    );
    assert_eq!(settings.mapping, [0, 1]);
    assert_eq!(Settings::load(&fixture.0).mapping, [0, 1]);
    assert_eq!(*session.lock().unwrap(), [0, 1]);
    assert_eq!(*source.lock().unwrap(), [0, 1]);
    fixture.allow_save();
    set_mapping(
        &fixture.0,
        &mut settings,
        Some(&session),
        Some(&source),
        [1, 0],
    )
    .unwrap();
    assert_eq!(settings.mapping, [1, 0]);
    assert_eq!(Settings::load(&fixture.0).mapping, [1, 0]);
    assert_eq!(*session.lock().unwrap(), [1, 0]);
    assert_eq!(*source.lock().unwrap(), [1, 0]);
}

#[test]
fn speaker_save_failure_preserves_disk_memory_and_live_flag_then_retries() {
    let fixture = Fixture::new("speaker");
    let mut settings = Settings::default();
    let active = AtomicBool::new(false);
    assert!(set_speaker_order(&fixture.0, &mut settings, Some(&active), true).is_err());
    assert!(!settings.speakers_swapped);
    assert!(!Settings::load(&fixture.0).speakers_swapped);
    assert!(!active.load(Ordering::Relaxed));
    fixture.allow_save();
    set_speaker_order(&fixture.0, &mut settings, Some(&active), true).unwrap();
    assert!(settings.speakers_swapped);
    assert!(Settings::load(&fixture.0).speakers_swapped);
    assert!(active.load(Ordering::Relaxed));
}
