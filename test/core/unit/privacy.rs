use super::*;
#[test]
fn unknown_credentials_and_nested_values_are_redacted_without_a_device_list() {
    let redactor = Redactor::default();
    let text = redactor.text("password=测试密码 sent=123\nAuthorization: Bearer fictional-token\nCookie: session=fictional-cookie\n\"private_key\": \"fictional-key\", elapsed=1.25");
    for secret in [
        "测试密码",
        "fictional-token",
        "fictional-cookie",
        "fictional-key",
    ] {
        assert!(!text.contains(secret), "leaked test credential");
    }
    assert!(text.contains("sent=123") && text.contains("elapsed=1.25"));
    let disguised = redactor.text("password=[DEVICE_1]secret sent=1");
    assert!(!disguised.contains("secret"));
    let safe = redactor.value(&serde_json::json!({
        "Password": "example", "credentials": {"nested": "example"},
        "access_token": ["example"], "endpoint": "example endpoint", "frames": 480
    }));
    for key in ["Password", "credentials", "access_token", "endpoint"] {
        assert_eq!(safe[key], REDACTED);
    }
    assert_eq!(safe["frames"], 480);
}

#[test]
fn unknown_addresses_mac_and_uuid_are_hidden_but_timings_remain() {
    let safe = Redactor::default().text("peer 192.0.2.15 [2001:db8::1] 02:00:00:00:00:01 00000000-0000-4000-8000-000000000099 lead_ms=300 ratio=1.0002");
    for identifier in [
        "192.0.2.15",
        "2001:db8::1",
        "02:00:00:00:00:01",
        "00000000-0000-4000-8000-000000000099",
    ] {
        assert!(!safe.contains(identifier));
    }
    assert!(safe.contains("lead_ms=300") && safe.contains("ratio=1.0002"));
}

#[test]
fn logs_keep_statistics_without_persistent_discovery_identifiers() {
    let device: Device = serde_json::from_value(serde_json::json!({
        "name":"Example Speaker","service":"Example Speaker._airplay._tcp.local.",
        "host":"speaker.example.","addresses":["192.0.2.10"],"port":7000,
        "properties":{"psi":"00000000-0000-4000-8000-000000000001","pk":"example-public-key","deviceid":"02:00:00:00:00:01"}
    })).unwrap();
    let redactor = Redactor::new(&[device]);
    let text = redactor.text("host=192.0.2.10 name=Example Speaker sent=123 pk=example-public-key");
    assert!(
        !text.contains("192.0.2.10")
            && !text.contains("Example Speaker")
            && !text.contains("example-public-key")
    );
    assert!(!text.contains("00000000-0000-4000-8000-000000000001") && text.contains("sent=123"));
    let value = redactor.value(&serde_json::json!({"name":"Unknown Endpoint","id":"00000000-0000-4000-8000-000000000002","frames":480}));
    assert_eq!(value["name"], "[REDACTED]");
    assert_eq!(value["id"], REDACTED);
    assert_eq!(value["frames"], 480);
}
