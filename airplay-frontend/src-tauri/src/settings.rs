//! 设置的数据结构、输入校验与持久化。
//! serde 使用 camelCase 与前端对齐；缺少新字段时使用 Default，便于读取旧配置。
//! 文件内容损坏或值越界时回退整份默认配置；写入前校验，避免覆盖有效设置。
//!
//! Settings contracts, validation and persistence. serde camelCase matches frontend fields;
//! Default fills missing fields in older files. Corrupt/out-of-range files fall back to defaults;
//! validate before writing so invalid updates cannot overwrite valid settings.
use serde::{Deserialize, Serialize};
use std::{fs, path::Path};

#[derive(Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub(crate) struct Settings {
    /// Windows 音频端点标识；空字符串表示尚未选择来源。
    /// Windows audio endpoint identity; empty means no source has been selected.
    pub endpoint: String,
    /// 接收端播放提前量，单位 ms，合法范围 250–2000。
    /// Receiver playback lead time, in ms, within 250–2000.
    pub latency: u32,
    /// 后端预缓冲时长，单位 ms，合法范围 64–512；不是控制器直接设定值。
    /// Backend prebuffer duration, 64–512 ms; it is not the controller's direct setpoint.
    pub buffer: u32,
    /// 输出左右声道对应的来源下标，从 0 开始；单声道可映射为 [0, 0]。
    /// Zero-based source indices for output left/right; mono can map to [0, 0].
    pub mapping: [usize; 2],
    /// 详细协议日志和采集诊断分别控制，普通播放不需要打开全部高频日志。
    /// Protocol logging and capture diagnostics are separate; normal playback needs neither in full.
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
        // Channel count depends on the real endpoint; check it again on connection/mapping changes.
        Ok(())
    }

    /// 路径由数据目录模块决定；设置文件只包含偏好，不保存密码或设备凭据。
    /// The data-directory module chooses the path; settings hold preferences, not passwords/credentials.
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
        // Write the complete pending file before replacing the saved file to avoid truncating it.
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
