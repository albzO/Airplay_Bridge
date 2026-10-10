use super::*;
#[test]
fn system_errors_keep_namespace_context_and_application_category() {
    let detail = "密码通信 / 发送响应：管道正在关闭 (os error 232)";
    let formatted = describe(detail, "AUTH_PIPE_FAILED");
    assert!(formatted.starts_with("[AUTH_PIPE_FAILED]"));
    assert!(formatted.contains("来源：Windows / Win32 232"));
    assert!(formatted.contains("密码通信 / 发送响应"));
    assert_eq!(describe(&formatted, "INTERNAL_ERROR"), formatted);
    assert_eq!(
        origin("failure (0x800700E8)"),
        "Windows / Win32 232（HRESULT 0x800700E8）"
    );
    assert_eq!(
        origin("failure (0x88890004)"),
        "Windows / HRESULT 0x88890004"
    );
    assert_eq!(
        origin("failure (os error 10054)"),
        "Windows / Winsock 10054"
    );
    assert!(!origin("HTTP 403; exit=12").contains("Windows"));
    assert_eq!(code_for("[LOG_LIMIT] full", "INTERNAL_ERROR"), "LOG_LIMIT");
    assert_eq!(
        code_for("report write: [LOG_LIMIT] full", "REPORT_WRITE_FAILED"),
        "REPORT_WRITE_FAILED"
    );
}
#[test]
fn authentication_codes_and_messages_are_unambiguous() {
    for (code, exit) in [
        ("PASSWORD_REJECTED", 10),
        ("AUTH_REQUIRED", 11),
        ("PAIRING_BACKOFF", 12),
        ("AUTH_REJECTED", 13),
        ("PASSWORD_REQUIRED", 11),
        ("PAIRING_REQUIRED", 14),
        ("ACCESS_DENIED", 15),
        ("PAIRING_MAX_TRIES", 16),
        ("PAIRING_MAX_PEERS", 17),
        ("PAIRING_UNAVAILABLE", 18),
        ("PAIRING_BUSY", 19),
    ] {
        let line = format!("[PROBE] ERROR code={code} exit={exit} host=192.0.2.10 phase=pairing");
        let error = SessionError::parse(&line).unwrap();
        assert_eq!(error.exit, exit);
        assert!(error.to_string().contains(if code == "AUTH_REQUIRED" {
            "PASSWORD_REQUIRED"
        } else {
            code
        }));
    }
    assert!(
        SessionError::parse("[PROBE] ERROR code=PASSWORD_REJECTED exit=1 host=x phase=pairing")
            .is_none()
    );
    assert!(SessionError::parse("[PROBE] FAILED phase=pairing http=403").is_none());
    assert!(
        SessionError::parse("[PROBE] ERROR code=PASSWORD_REJECTED exit=10 host=x phase=pairing")
            .unwrap()
            .explanation()
            .contains("AirPlay 密码")
    );
}
