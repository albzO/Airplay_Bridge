//! Authentication policy only. Passwords never enter this cache.
use homepod_test::discovery::Device;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::BTreeSet, fs, path::Path};

#[derive(Default, Serialize, Deserialize)]
pub struct Memory {
    required: BTreeSet<String>,
}
pub fn key(device: &Device) -> String {
    let identity = device
        .properties
        .get("deviceid")
        .filter(|v| !v.is_empty())
        .map(|v| v.to_lowercase())
        .unwrap_or_else(|| device.service.to_lowercase());
    // A factory reset can change the public key and should invalidate policy.
    format!(
        "{}|{}",
        identity,
        device
            .properties
            .get("pk")
            .map(String::as_str)
            .unwrap_or("")
    )
}
pub fn password_host(event: &Value) -> Option<&str> {
    if event["kind"] == "password_required" {
        return event["host"].as_str();
    }
    if event["kind"] != "native" {
        return None;
    }
    let line = event["line"].as_str()?;
    if !line.contains("[PROBE] AUTH_CHALLENGE ") && !line.contains("[PROBE] AUTH_COMPLETE ") {
        return None;
    }
    let field = |name: &str| line.split_whitespace().find_map(|v| v.strip_prefix(name));
    if field("code=") == Some("PASSWORD_REQUIRED") || field("method=") == Some("password") {
        field("host=")
    } else {
        None
    }
}
impl Memory {
    pub fn load(root: &Path, devices: &[Device]) -> Result<Self, String> {
        let path = root.join("auth-policy.json");
        if path.exists() {
            return serde_json::from_slice(&fs::read(path).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string());
        }
        let mut memory = Self::default();
        // Upgrade existing installs using explicit authentication markers in
        // recent local fault logs, never infer policy from Backoff or HTTP errors.
        let mut logs: Vec<_> = fs::read_dir(root.join("logs"))
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
            .filter(|e| {
                e.file_name().to_string_lossy().starts_with("live-")
                    && e.path().extension().is_some_and(|x| x == "log")
            })
            .collect();
        logs.sort_by_key(|e| std::cmp::Reverse(e.file_name()));
        for log in logs.into_iter().take(64) {
            if log.metadata().map(|m| m.len() > 262_144).unwrap_or(true) {
                continue;
            }
            if let Ok(text) = fs::read_to_string(log.path()) {
                for line in text.lines() {
                    let event = serde_json::json!({"kind":"native","line":line});
                    if let Some(host) = password_host(&event) {
                        for device in devices
                            .iter()
                            .filter(|d| d.addresses.iter().any(|a| a.to_string() == host))
                        {
                            memory.required.insert(key(device));
                        }
                    }
                }
            }
        }
        memory.save(root)?;
        Ok(memory)
    }
    fn save(&self, root: &Path) -> Result<(), String> {
        let pending = root.join("auth-policy.pending.json");
        fs::write(
            &pending,
            serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        fs::rename(pending, root.join("auth-policy.json")).map_err(|e| e.to_string())
    }
    pub fn needs_password(&self, device: &Device) -> bool {
        self.required.contains(&key(device))
    }
    pub fn remember(&mut self, root: &Path, identity: &str) -> Result<(), String> {
        if self.required.insert(identity.to_owned()) {
            self.save(root)?;
        }
        Ok(())
    }
    pub fn forget(&mut self, root: &Path, devices: &[Device]) -> Result<(), String> {
        for device in devices {
            self.required.remove(&key(device));
        }
        self.save(root)
    }
}

#[cfg(test)]
#[path = "../../../test/frontend/desktop/auth_memory.rs"]
mod tests;
