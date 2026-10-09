use super::*;

#[test]
fn invalid_saved_values_fall_back_and_invalid_writes_preserve_previous_settings() {
    let root = std::env::temp_dir().join(format!("airplay-settings-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let settings = Settings::default();
    settings.save(&root).unwrap();
    let mut invalid = settings.clone();
    invalid.close_action = "unknown".into();
    assert!(invalid.save(&root).is_err());
    assert_eq!(Settings::load(&root).close_action, "tray");
    fs::write(root.join("settings.json"), br#"{"latency":0}"#).unwrap();
    assert_eq!(Settings::load(&root).latency, 300);
    let updated = Settings {
        latency: 500,
        ..settings
    };
    updated.save(&root).unwrap();
    updated.save(&root).unwrap();
    assert_eq!(Settings::load(&root).latency, 500);
    assert!(!root.join("settings.pending.json").exists());
    fs::remove_file(root.join("settings.json")).unwrap();
    fs::remove_dir(root).unwrap();
}
