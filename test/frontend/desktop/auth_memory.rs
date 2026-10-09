use super::*;
#[test]
fn policy_survives_restart_and_address_change_but_not_key_change() {
    let root = std::env::temp_dir().join(format!("airplay-policy-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let mut device: Device = serde_json::from_value(serde_json::json!({"name":"test","service":"test._airplay._tcp.local.","host":"test.local.","addresses":["127.0.0.1"],"port":7000,"properties":{"deviceid":"AA:BB","pk":"key1"}})).unwrap();
    let mut memory = Memory::default();
    memory.remember(&root, &key(&device)).unwrap();
    memory.remember(&root, "other-device|key").unwrap();
    let memory = Memory::load(&root, &[]).unwrap();
    device.name = "renamed".into();
    device.addresses = vec!["127.0.0.2".parse().unwrap()];
    assert!(memory.needs_password(&device));
    let mut cleared = Memory::load(&root, &[]).unwrap();
    cleared.forget(&root, &[device.clone()]).unwrap();
    assert!(!Memory::load(&root, &[]).unwrap().needs_password(&device));
    device.properties.insert("pk".into(), "key2".into());
    assert!(!memory.needs_password(&device));
    assert!(
        !fs::read_to_string(root.join("auth-policy.json"))
            .unwrap()
            .contains("secret")
    );
    fs::remove_file(root.join("auth-policy.json")).unwrap();
    fs::remove_dir(root).unwrap();
}
#[test]
fn only_explicit_password_evidence_is_learned() {
    for line in [
        "[PROBE] AUTH_CHALLENGE host=x code=PASSWORD_REQUIRED",
        "[PROBE] AUTH_COMPLETE host=x method=password",
    ] {
        assert_eq!(
            password_host(&serde_json::json!({"kind":"native","line":line})),
            Some("x")
        );
    }
    for line in [
        "[PROBE] ERROR code=PAIRING_BACKOFF host=x",
        "[PROBE] AUTH_COMPLETE host=x method=transient",
        "[PROBE] ERROR code=ACCESS_DENIED host=x",
    ] {
        assert_eq!(
            password_host(&serde_json::json!({"kind":"native","line":line})),
            None
        );
    }
}
