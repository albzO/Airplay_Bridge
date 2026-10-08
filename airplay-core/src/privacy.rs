//! Log-only redaction. Control messages and device discovery retain real values.
use crate::discovery::Device;
use serde_json::Value;
use std::{net::IpAddr, path::Path};

const REDACTED: &str = "[REDACTED]";

fn sensitive_field(key: &str) -> bool {
    matches!(
        key.to_ascii_lowercase().replace(['-', '_'], "").as_str(),
        "name"
            | "description"
            | "service"
            | "host"
            | "hostname"
            | "gpn"
            | "pk"
            | "btaddr"
            | "deviceid"
            | "id"
            | "endpoint"
            | "psi"
            | "pi"
            | "tsid"
            | "password"
            | "passwd"
            | "secret"
            | "token"
            | "accesstoken"
            | "refreshtoken"
            | "authorization"
            | "proxyauthorization"
            | "cookie"
            | "setcookie"
            | "credentials"
            | "credential"
            | "privatekey"
            | "publickey"
            | "sessionkey"
            | "sharedsecret"
            | "apikey"
            | "activeremote"
    )
}

// 不依赖设备清单：新字段或陌生设备也不能把凭据写入日志。
fn redact_fields(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut safe = String::new();
    let mut copied = 0;
    let mut index = 0;
    while index < bytes.len() {
        if !bytes[index].is_ascii_alphabetic() {
            index += 1;
            continue;
        }
        let start = index;
        while index < bytes.len()
            && (bytes[index].is_ascii_alphanumeric() || matches!(bytes[index], b'-' | b'_'))
        {
            index += 1;
        }
        let key = &text[start..index];
        if !sensitive_field(key) {
            continue;
        }
        let mut value = index;
        if value < bytes.len() && matches!(bytes[value], b'\'' | b'"') {
            value += 1;
        }
        while value < bytes.len() && matches!(bytes[value], b' ' | b'\t') {
            value += 1;
        }
        if value == bytes.len() || !matches!(bytes[value], b'=' | b':') {
            continue;
        }
        value += 1;
        while value < bytes.len() && matches!(bytes[value], b' ' | b'\t') {
            value += 1;
        }
        if matches!(key, "host" | "name" | "service") {
            if let Some(alias) = text[value..].strip_prefix("[DEVICE_") {
                if let Some((number, suffix)) = alias.split_once(']') {
                    if !number.is_empty()
                        && number.bytes().all(|c| c.is_ascii_digit())
                        && (suffix.is_empty() || suffix.starts_with(char::is_whitespace))
                    {
                        continue;
                    }
                }
            }
        }
        let quote = bytes
            .get(value)
            .copied()
            .filter(|c| matches!(c, b'\'' | b'"'));
        let mut end = value + usize::from(quote.is_some());
        if let Some(quote) = quote {
            while end < bytes.len() {
                if bytes[end] == b'\\' && end + 1 < bytes.len() {
                    end += 2;
                } else if bytes[end] == quote {
                    end += 1;
                    break;
                } else {
                    end += 1;
                }
            }
        } else {
            while end < bytes.len() && !matches!(bytes[end], b'\r' | b'\n' | b',' | b';' | b'}') {
                // 保留同一行后面的统计字段，例如 password=x sent=123。
                if bytes[end].is_ascii_whitespace() {
                    let remaining = &text[end..];
                    let next = remaining.trim_start();
                    let key_end = next
                        .find(|c: char| !c.is_ascii_alphanumeric() && c != '_' && c != '-')
                        .unwrap_or(next.len());
                    if key_end > 0 && next[key_end..].trim_start().starts_with('=') {
                        break;
                    }
                }
                end += 1;
            }
        }
        safe.push_str(&text[copied..value]);
        safe.push_str(REDACTED);
        copied = end;
        index = end;
    }
    safe.push_str(&text[copied..]);
    safe
}

fn redact_identifiers(text: &str) -> String {
    let mut safe = String::new();
    let mut token = String::new();
    let flush = |out: &mut String, token: &mut String| {
        let mac = token.len() == 17
            && token.split([':', '-']).count() == 6
            && token
                .split([':', '-'])
                .all(|part| part.len() == 2 && part.bytes().all(|c| c.is_ascii_hexdigit()));
        if token.parse::<IpAddr>().is_ok() {
            out.push_str("[IP]");
        } else if is_uuid(token) || mac {
            out.push_str(REDACTED);
        } else {
            out.push_str(token);
        }
        token.clear();
    };
    for c in text.chars() {
        if c.is_ascii_hexdigit() || matches!(c, '.' | ':' | '-') {
            token.push(c);
        } else {
            flush(&mut safe, &mut token);
            safe.push(c);
        }
    }
    flush(&mut safe, &mut token);
    safe
}

#[derive(Clone, Default)]
pub struct Redactor {
    replacements: Vec<(String, String)>,
}
fn is_uuid(s: &str) -> bool {
    s.len() == 36
        && s.bytes().enumerate().all(|(i, c)| {
            if [8, 13, 18, 23].contains(&i) {
                c == b'-'
            } else {
                c.is_ascii_hexdigit()
            }
        })
}
impl Redactor {
    pub fn from_root(root: &Path) -> Self {
        let devices = std::fs::read(root.join("devices.json"))
            .ok()
            .and_then(|b| serde_json::from_slice::<Vec<Device>>(&b).ok())
            .unwrap_or_default();
        Self::new(&devices)
    }
    pub fn new(devices: &[Device]) -> Self {
        let mut result = Self::default();
        for (index, device) in devices.iter().enumerate() {
            // 编号只在本次清单中关联设备，不保留可跨日志追踪的 UUID。
            let alias = format!("[DEVICE_{}]", index + 1);
            for text in [&device.service, &device.host, &device.name] {
                if !text.is_empty() {
                    result.replacements.push((text.clone(), alias.clone()));
                }
            }
            for address in &device.addresses {
                result
                    .replacements
                    .push((address.to_string(), alias.clone()));
            }
            for key in ["gpn", "pk", "btaddr", "deviceid", "psi", "pi", "tsid"] {
                if let Some(value) = device.properties.get(key).filter(|v| !v.is_empty()) {
                    result
                        .replacements
                        .push((value.clone(), "[REDACTED]".into()));
                }
            }
        }
        result
            .replacements
            .sort_by_key(|(from, _)| std::cmp::Reverse(from.len()));
        result
    }
    pub fn text(&self, text: &str) -> String {
        let mut result = text.to_owned();
        // A raw failed-response body may encode names/keys as hex. Keep its
        // length and status, but not bytes that a plain-text filter cannot see.
        if let Some(start) = result.find(" | body[") {
            if let Some(end) = result[start..].find("]=") {
                result.truncate(start + end + 2);
                result.push_str("[REDACTED]");
            }
        }
        // Paths are not useful identifiers and can expose account/project names.
        for key in ["APPDATA", "LOCALAPPDATA", "USERPROFILE"] {
            if let Some(path) = std::env::var_os(key) {
                let path = path.to_string_lossy();
                result = result
                    .replace(path.as_ref(), "[USER_DIR]")
                    .replace(&path.replace('\\', "/"), "[USER_DIR]");
            }
        }
        if let Ok(hostname) = std::env::var("COMPUTERNAME") {
            if !hostname.is_empty() {
                result = result.replace(&hostname, "[COMPUTER]");
            }
        }
        for (from, to) in &self.replacements {
            let mut replaced = String::new();
            let mut previous = 0;
            for (index, _) in result.match_indices(from) {
                let end = index + from.len();
                let ascii_name = from.chars().all(|c| c.is_ascii_alphanumeric());
                let embedded = ascii_name
                    && (result[..index]
                        .chars()
                        .next_back()
                        .is_some_and(|c| c.is_ascii_alphanumeric())
                        || result[end..]
                            .chars()
                            .next()
                            .is_some_and(|c| c.is_ascii_alphanumeric()));
                if !embedded {
                    replaced.push_str(&result[previous..index]);
                    replaced.push_str(to);
                    previous = end;
                }
            }
            replaced.push_str(&result[previous..]);
            result = replaced;
        }
        redact_identifiers(&redact_fields(&result))
    }
    pub fn value(&self, value: &Value) -> Value {
        match value {
            Value::Object(fields) => Value::Object(
                fields
                    .iter()
                    .map(|(key, value)| {
                        let value = if sensitive_field(key) && !value.is_null() {
                            Value::String(REDACTED.into())
                        } else {
                            self.value(value)
                        };
                        (self.text(key), value)
                    })
                    .collect(),
            ),
            Value::Array(items) => Value::Array(items.iter().map(|v| self.value(v)).collect()),
            Value::String(text) => Value::String(self.text(text)),
            _ => value.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
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
        let text =
            redactor.text("host=192.0.2.10 name=Example Speaker sent=123 pk=example-public-key");
        assert!(
            !text.contains("192.0.2.10")
                && !text.contains("Example Speaker")
                && !text.contains("example-public-key")
        );
        assert!(
            !text.contains("00000000-0000-4000-8000-000000000001") && text.contains("sent=123")
        );
        let value = redactor.value(&serde_json::json!({"name":"Unknown Endpoint","id":"00000000-0000-4000-8000-000000000002","frames":480}));
        assert_eq!(value["name"], "[REDACTED]");
        assert_eq!(value["id"], REDACTED);
        assert_eq!(value["frames"], 480);
    }
}
