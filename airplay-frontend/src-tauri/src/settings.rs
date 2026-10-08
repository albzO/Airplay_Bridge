//! 设置的数据结构、输入校验与持久化。
use serde::{Deserialize, Serialize};
use std::{fs, path::Path};

#[derive(Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub(crate) struct Settings {
    pub endpoint: String,
    pub latency: u32,
    pub buffer: u32,
    pub mapping: [usize; 2],
    pub detailed_logs: bool,
    pub capture_diagnostics: bool,
    pub close_action: String,
    pub speakers_swapped: bool,
    pub keep_awake: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            endpoint: String::new(),
            latency: 300,
            buffer: 128,
            mapping: [0, 1],
            detailed_logs: false,
            capture_diagnostics: false,
            close_action: "tray".into(),
            speakers_swapped: false,
            keep_awake: true,
        }
    }
}

impl Settings {
    pub fn validate(&self) -> Result<(), String> {
        if !(250..=2000).contains(&self.latency) || !(64..=512).contains(&self.buffer) {
            return Err("播放提前量为 250–2000 ms，Buffer 为 64–512 ms".into());
        }
        if !["tray", "quit"].contains(&self.close_action.as_str()) {
            return Err("未知窗口关闭动作".into());
        }
        // 声道数由实际端点决定，连接和更换映射时继续校验。
        Ok(())
    }

    pub fn load(root: &Path) -> Self {
        fs::read(root.join("settings.json"))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Self>(&bytes).ok())
            .filter(|settings| settings.validate().is_ok())
            .unwrap_or_default()
    }

    pub fn save(&self, root: &Path) -> Result<(), String> {
        self.validate()?;
        let pending = root.join("settings.pending.json");
        let bytes = serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?;
        // 完整写入后再替换，避免中断时截断现有设置文件。
        fs::write(&pending, bytes).map_err(|e| e.to_string())?;
        fs::rename(pending, root.join("settings.json")).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
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
}
