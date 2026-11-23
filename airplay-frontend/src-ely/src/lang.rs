//! 界面语言：中文或英文。默认跟随系统，可在设置中更改；安装程序把所选语言写入同一个文件。
//! UI language: Chinese or English. Follows the system by default, can be changed in settings;
//! the installer writes its chosen language to the same file.
use ely_core::i18n::I18n;
use gpui::App;
use std::{
    fs,
    path::Path,
    sync::atomic::{AtomicU8, Ordering},
};
use windows::Win32::Globalization::GetUserDefaultUILanguage;

/// 数据目录下保存语言选择的文件名，内容为 `system`、`zh` 或 `en`。
/// File under the data directory holding the choice: `system`, `zh` or `en`.
const FILE: &str = "language";
/// Windows 主语言标识：中文。
/// Windows primary language identifier for Chinese.
const LANG_CHINESE: u16 = 0x04;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    Zh,
    En,
}

/// 用户的选择；`System` 在每次启动时按 Windows 显示语言解析。
/// What the user picked; `System` resolves from the Windows display language at each start.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Choice {
    System,
    Zh,
    En,
}

impl Choice {
    pub fn key(self) -> &'static str {
        match self {
            Choice::System => "system",
            Choice::Zh => "zh",
            Choice::En => "en",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        [Choice::System, Choice::Zh, Choice::En]
            .into_iter()
            .find(|choice| choice.key() == key)
    }

    fn resolve(self) -> Lang {
        match self {
            Choice::Zh => Lang::Zh,
            Choice::En => Lang::En,
            Choice::System => {
                let primary = unsafe { GetUserDefaultUILanguage() } & 0x3ff;
                if primary == LANG_CHINESE {
                    Lang::Zh
                } else {
                    Lang::En
                }
            }
        }
    }
}

static CURRENT: AtomicU8 = AtomicU8::new(0);

pub fn current() -> Lang {
    if CURRENT.load(Ordering::Relaxed) == 0 {
        Lang::Zh
    } else {
        Lang::En
    }
}

/// 当前语言下的文字。两种语言写在同一处，改文案时不会漏掉另一种。
/// The text in the current language. Both languages sit side by side, so neither is forgotten
/// when the wording changes.
pub fn t(zh: &'static str, en: &'static str) -> &'static str {
    match current() {
        Lang::Zh => zh,
        Lang::En => en,
    }
}

/// 带参数的文字：按当前语言选择格式串，参数只求值一次。
/// Text with arguments: picks the format string for the current language; arguments are
/// evaluated once.
#[macro_export]
macro_rules! tf {
    ($zh:literal, $en:literal $(, $arg:expr)* $(,)?) => {
        match $crate::lang::current() {
            $crate::lang::Lang::Zh => format!($zh $(, $arg)*),
            $crate::lang::Lang::En => format!($en $(, $arg)*),
        }
    };
}

/// 读取保存的选择；文件缺失或内容无法识别时跟随系统。
/// Reads the saved choice; a missing or unrecognized file follows the system.
pub fn load(root: &Path) -> Choice {
    fs::read_to_string(root.join(FILE))
        .ok()
        .and_then(|text| Choice::from_key(text.trim()))
        .unwrap_or(Choice::System)
}

pub fn save(root: &Path, choice: Choice) -> Result<(), String> {
    fs::write(root.join(FILE), choice.key()).map_err(|error| {
        tf!(
            "无法保存语言设置：{error}",
            "Could not save the language: {error}"
        )
    })
}

/// 切换界面语言。窗口创建前调用即可决定首屏语言。
/// Switches the UI language. Call before the window opens to set the first frame's language.
pub fn set(choice: Choice) {
    let value = match choice.resolve() {
        Lang::Zh => 0,
        Lang::En => 1,
    };
    CURRENT.store(value, Ordering::Relaxed);
}

/// 让 Ely 组件自带的文字（如密码框的显示 / 隐藏提示）跟随同一语言，并重绘窗口。
/// Makes Ely's own component text (such as the password field's show / hide tip) follow the
/// same language, and redraws the windows.
pub fn sync_components(cx: &mut App) {
    let tag = match current() {
        Lang::Zh => "zh-CN",
        Lang::En => "en-US",
    };
    if !cx.has_global::<I18n>() {
        // 应用自己的文字不走 Ely 的目录；空目录只为让 Ely 接受这两种语言。
        // The app's own text does not use Ely's catalogs; the empty ones only register the two
        // locales with Ely.
        cx.set_global(I18n::new(tag).catalog("en-US", &[]).catalog("zh-CN", &[]));
    }
    I18n::set_locale(tag, cx);
}
