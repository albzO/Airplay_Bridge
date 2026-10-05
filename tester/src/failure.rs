//! Stable authentication codes shared with the native backend.
use std::{error::Error, fmt};

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
        let code = field("code=")?;
        let exit = match code {
            "PASSWORD_REJECTED" => 10,
            "PASSWORD_REQUIRED" | "AUTH_REQUIRED" => 11,
            "PAIRING_BACKOFF" => 12,
            "AUTH_REJECTED" => 13,
            "PAIRING_REQUIRED" => 14,
            "ACCESS_DENIED" => 15,
            _ => return None,
        };
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
        match self.exit {
            10 => "HomePod 拒绝了输入的 AirPlay 密码，请核对家庭 App 中的 AirPlay 密码后重试。",
            11 => "HomePod 拒绝了自动认证的临时 PIN，需要输入 AirPlay 密码。",
            12 => "HomePod 暂时限制了配对尝试，请稍后再试，避免连续重试。",
            14 => "设备要求完整配对或有效的配对凭据；仅输入 AirPlay 访问密码不能完成此授权。",
            15 => {
                "设备拒绝了访问，请核对家庭 App 的扬声器访问权限及配对要求；此错误不代表密码输错。"
            }
            _ => "HomePod 拒绝了认证，请核对设备访问权限及配对方式。",
        }
    }
}
impl fmt::Display for SessionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}（退出码 {}，设备 {}）：{}",
            self.code,
            self.exit,
            self.host,
            self.explanation()
        )
    }
}
impl Error for SessionError {}

#[cfg(test)]
mod tests {
    use super::*;
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
        ] {
            let line =
                format!("[PROBE] ERROR code={code} exit={exit} host=192.168.31.188 phase=pairing");
            let error = SessionError::parse(&line).unwrap();
            assert_eq!(error.exit, exit);
            assert!(error.to_string().contains(code));
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
