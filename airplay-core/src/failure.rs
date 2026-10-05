//! Stable error catalog shared by the CLI, sessions and UI.
use serde::Deserialize;
use std::{error::Error, fmt, sync::OnceLock};
#[derive(Deserialize)]
struct Definition {
    code: String,
    exit: i32,
    message: String,
    patterns: Vec<String>,
}
fn catalog() -> &'static [Definition] {
    static CATALOG: OnceLock<Vec<Definition>> = OnceLock::new();
    CATALOG.get_or_init(|| {
        serde_json::from_str(include_str!("../../docs/error-codes.json"))
            .expect("valid bundled error catalog")
    })
}
pub fn code_for(detail: &str, fallback: &str) -> String {
    if let Some(row) = catalog().iter().find(|r| {
        detail.starts_with(&format!("[{}]", r.code))
            || detail.contains(&format!("code={}", r.code))
            || detail.starts_with(&format!("{}（", r.code))
    }) {
        return row.code.clone();
    }
    catalog()
        .iter()
        .find(|r| r.patterns.iter().any(|p| detail.contains(p)))
        .map(|r| r.code.clone())
        .unwrap_or_else(|| fallback.to_owned())
}
/// Keep operating-system namespaces separate from application exit codes.
pub fn origin(detail: &str) -> String {
    if let Some((_, tail)) = detail.split_once("os error ") {
        let digits: String = tail.chars().take_while(char::is_ascii_digit).collect();
        if let Ok(code) = digits.parse::<u32>() {
            let namespace = if (10000..12000).contains(&code) {
                "Winsock"
            } else {
                "Win32"
            };
            return format!("Windows / {namespace} {code}");
        }
    }
    for (start, _) in detail.match_indices("0x").chain(detail.match_indices("0X")) {
        let hex: String = detail[start + 2..]
            .chars()
            .take_while(char::is_ascii_hexdigit)
            .collect();
        if hex.len() == 8 {
            if let Ok(code) = u32::from_str_radix(&hex, 16) {
                if (code >> 16) & 0x1fff == 7 {
                    return format!("Windows / Win32 {}（HRESULT 0x{code:08X}）", code & 0xffff);
                }
                return format!("Windows / HRESULT 0x{code:08X}");
            }
        }
    }
    "AirPlay Hub / 应用或协议处理".into()
}
pub fn describe(detail: &str, fallback: &str) -> String {
    let code = code_for(detail, fallback);
    if detail.starts_with(&format!("[{code}]")) {
        return if detail.contains("来源：") {
            detail.to_owned()
        } else {
            format!("{detail} 来源：{}", origin(detail))
        };
    }
    let message = catalog()
        .iter()
        .find(|r| r.code == code)
        .map(|r| r.message.as_str())
        .unwrap_or("未分类的内部故障。");
    format!(
        "[{code}] {message} 来源：{}；详情：{detail}",
        origin(detail)
    )
}
pub fn exit_for(detail: &str) -> i32 {
    let code = code_for(detail, "INTERNAL_ERROR");
    catalog()
        .iter()
        .find(|r| r.code == code)
        .map_or(1, |r| r.exit)
}

#[derive(Clone, Debug)]
pub struct SessionError {
    pub code: String,
    pub exit: i32,
    pub host: String,
    pub phase: String,
}
impl SessionError {
    pub fn parse(line: &str) -> Option<Self> {
        let fields = line.split_once("[PROBE] ERROR ")?.1;
        let field = |name: &str| fields.split_whitespace().find_map(|v| v.strip_prefix(name));
        let code = match field("code=")? {
            "AUTH_REQUIRED" => "PASSWORD_REQUIRED",
            code => code,
        };
        let exit = catalog().iter().find(|r| r.code == code)?.exit;
        if field("exit=")?.parse::<i32>().ok()? != exit {
            return None;
        }
        Some(Self {
            code: code.to_owned(),
            exit,
            host: field("host=")?.to_owned(),
            phase: field("phase=")?.to_owned(),
        })
    }
    pub fn explanation(&self) -> &'static str {
        catalog()
            .iter()
            .find(|r| r.code == self.code)
            .map(|r| r.message.as_str())
            .unwrap_or("设备认证失败。")
    }
}
impl fmt::Display for SessionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "[{}] {}（来源：AirPlay 后端 / 设备响应，退出码 {}，设备 {}，阶段 {}）",
            self.code,
            self.explanation(),
            self.exit,
            self.host,
            self.phase
        )
    }
}
impl Error for SessionError {}

#[cfg(test)]
mod tests {
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
            let line =
                format!("[PROBE] ERROR code={code} exit={exit} host=192.0.2.10 phase=pairing");
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
            SessionError::parse(
                "[PROBE] ERROR code=PASSWORD_REJECTED exit=10 host=x phase=pairing"
            )
            .unwrap()
            .explanation()
            .contains("AirPlay 密码")
        );
    }
}
