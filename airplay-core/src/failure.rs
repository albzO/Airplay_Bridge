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
#[path = "../../test/core/unit/failure.rs"]
mod tests;
