//! Log-only redaction. Control messages and device discovery retain real values.
use crate::discovery::Device;
use serde_json::Value;
use std::{net::Ipv4Addr, path::Path};

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
        for device in devices {
            let uuid = ["psi", "pi", "tsid"]
                .iter()
                .filter_map(|k| device.properties.get(*k))
                .find(|id| is_uuid(id))
                .cloned();
            let alias = uuid
                .map(|id| format!("device:{id}"))
                .unwrap_or_else(|| "[DEVICE]".into());
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
            for key in ["gpn", "pk", "btaddr", "deviceid"] {
                if let Some(value) = device
                    .properties
                    .get(key)
                    .filter(|v| !v.is_empty() && !is_uuid(v))
                {
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
        let mut safe = String::new();
        let mut numeric = String::new();
        let flush = |out: &mut String, token: &mut String| {
            if token.parse::<Ipv4Addr>().is_ok() {
                out.push_str("[IP]");
            } else {
                out.push_str(token);
            }
            token.clear();
        };
        for c in result.chars() {
            if c.is_ascii_digit() || c == '.' {
                numeric.push(c);
            } else {
                flush(&mut safe, &mut numeric);
                safe.push(c);
            }
        }
        flush(&mut safe, &mut numeric);
        safe
    }
    pub fn value(&self, value: &Value) -> Value {
        match value {
            Value::Object(fields) => Value::Object(
                fields
                    .iter()
                    .map(|(key, value)| {
                        let sensitive = [
                            "name",
                            "description",
                            "service",
                            "host",
                            "gpn",
                            "pk",
                            "btaddr",
                            "deviceid",
                        ]
                        .contains(&key.as_str());
                        let value = if sensitive
                            && value.is_string()
                            && !value.as_str().is_some_and(is_uuid)
                        {
                            Value::String("[REDACTED]".into())
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
    fn logs_keep_uuid_and_statistics_without_discovery_identifiers() {
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
        assert!(text.contains("00000000-0000-4000-8000-000000000001") && text.contains("sent=123"));
        let value = redactor.value(&serde_json::json!({"name":"Unknown Endpoint","id":"00000000-0000-4000-8000-000000000002","frames":480}));
        assert_eq!(value["name"], "[REDACTED]");
        assert_eq!(value["id"], "00000000-0000-4000-8000-000000000002");
        assert_eq!(value["frames"], 480);
    }
}
