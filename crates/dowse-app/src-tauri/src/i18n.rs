//! 托盘菜单、原生右键菜单等 Rust 侧界面文案的中英双语，跟随系统 UI 语言：
//! `GetUserDefaultUILanguage` 的主语言是中文（LANG_CHINESE）走中文，其余一律
//! 英文。判定只做一次并缓存（`OnceLock`），没有运行时切换、没有设置项。跟前端
//! `lib/i18n.ts` 是对称的两套（各自平台各自检测），不共享数据。开发者面向的
//! `eprintln!` 诊断日志不在此列，仍按仓库惯例保留中文。

use std::sync::OnceLock;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    Zh,
    En,
}

#[cfg(target_os = "windows")]
fn detect_lang() -> Lang {
    use windows::Win32::Globalization::GetUserDefaultUILanguage;
    // LANGID 低 10 位是主语言 ID；中文（简繁及各地区变体）主语言都是
    // LANG_CHINESE = 0x04，用它一刀切判定"是不是中文系统"，不细分地区。
    let langid = unsafe { GetUserDefaultUILanguage() };
    if (langid & 0x3ff) == 0x04 {
        Lang::Zh
    } else {
        Lang::En
    }
}

#[cfg(not(target_os = "windows"))]
fn detect_lang() -> Lang {
    Lang::En
}

/// 当前进程生效的界面语言，首次调用时解析一次后缓存。
pub fn lang() -> Lang {
    static LANG: OnceLock<Lang> = OnceLock::new();
    *LANG.get_or_init(resolve_lang)
}

/// 语言解析：设置面板可以把界面语言钉死（写进 `config.lang`），config 是
/// 权威存储。这里在进程首次取语言时读一次配置——显式选了 "zh"/"en" 就用它，
/// "auto"/没配过才回落到系统 UI 语言检测。只解析一次（`OnceLock` 缓存），
/// 运行中改了配置也不热切，跟设置面板"改语言重启后生效"的交互一致。这里读
/// 文件（`config::load`）而不是 `ConfigState`，是为了让不持有 `AppHandle` 的
/// `strings()` 调用点也能用；首次之外不再重复读，多一次启动期文件读可忽略。
fn resolve_lang() -> Lang {
    match crate::config::load().lang.as_str() {
        "zh" => Lang::Zh,
        "en" => Lang::En,
        _ => detect_lang(),
    }
}

/// 所有 Rust 侧界面可见文案的一张表。字段全是 `&'static str`，带插值的地方
/// （tooltip、每根文档数）拆成前后缀，由调用方 `format!` 拼。
pub struct Strings {
    pub tooltip_idle_prefix: &'static str,
    pub tooltip_idle_suffix: &'static str,
    pub menu_show: &'static str,
    pub menu_autostart: &'static str,
    pub menu_quit: &'static str,
    pub folders_submenu: &'static str,
    /// 每根子菜单标题里文档数的单位，拼成 "{路径} · {数量} {单位}"。
    pub root_docs_unit: &'static str,
    pub rebuild_item: &'static str,
    pub remove_item: &'static str,
    pub add_folder_item: &'static str,
    /// 文本索引 tooltip 拼成 "{prefix}{数量}{suffix}"。
    pub tooltip_indexing_prefix: &'static str,
    pub tooltip_indexing_suffix: &'static str,
    /// 图片识别 tooltip 拼成 "{prefix}{已处理} / {总数}"。
    pub tooltip_ocr_prefix: &'static str,
    pub dialog_pick_folder: &'static str,
    pub stale_root_error: &'static str,
    pub ctx_open: &'static str,
    pub ctx_reveal: &'static str,
    pub ctx_copy_path: &'static str,
    pub ctx_copy_name: &'static str,
}

const ZH: Strings = Strings {
    tooltip_idle_prefix: "dowse — ",
    tooltip_idle_suffix: " 呼出",
    menu_show: "呼出",
    menu_autostart: "开机自启",
    menu_quit: "退出",
    folders_submenu: "索引文件夹",
    root_docs_unit: "篇",
    rebuild_item: "重建",
    remove_item: "移除",
    add_folder_item: "添加文件夹…",
    tooltip_indexing_prefix: "dowse — 索引中 ",
    tooltip_indexing_suffix: " 篇",
    tooltip_ocr_prefix: "dowse — 图片识别 ",
    dialog_pick_folder: "选择要索引的文件夹",
    stale_root_error: "找不到这个索引根，菜单可能已过期，请重新打开托盘菜单",
    ctx_open: "打开文件",
    ctx_reveal: "打开所在文件夹",
    ctx_copy_path: "复制完整路径",
    ctx_copy_name: "复制文件名",
};

const EN: Strings = Strings {
    tooltip_idle_prefix: "dowse — ",
    tooltip_idle_suffix: " to open",
    menu_show: "Show dowse",
    menu_autostart: "Launch at startup",
    menu_quit: "Quit",
    folders_submenu: "Indexed folders",
    root_docs_unit: "docs",
    rebuild_item: "Rebuild",
    remove_item: "Remove",
    add_folder_item: "Add folder…",
    tooltip_indexing_prefix: "dowse — indexing ",
    tooltip_indexing_suffix: " docs",
    tooltip_ocr_prefix: "dowse — image OCR ",
    dialog_pick_folder: "Choose a folder to index",
    stale_root_error: "Could not find this index root. The menu may be stale; reopen the tray menu.",
    ctx_open: "Open file",
    ctx_reveal: "Reveal in File Explorer",
    ctx_copy_path: "Copy full path",
    ctx_copy_name: "Copy file name",
};

/// 当前语言对应的整张文案表。
pub fn strings() -> &'static Strings {
    match lang() {
        Lang::Zh => &ZH,
        Lang::En => &EN,
    }
}

/// 把 `tauri-plugin-global-shortcut` 认的原始快捷键字符串（如 "Alt+Backquote"）
/// 转成人类习惯的显示形式（"Alt+`"），跟前端 `lib/hotkey.ts` 的 `formatHotkey`
/// 保持同一套映射。托盘 tooltip 里展示"当前呼出键"用（见 tray.rs 的
/// `idle_tooltip`），改键后立即重拼，不用重启。
pub fn format_hotkey(raw: &str) -> String {
    if raw.is_empty() {
        return raw.to_string();
    }
    raw.split('+')
        .map(|part| label_of(part.trim()))
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("+")
}

fn label_of(segment: &str) -> &str {
    match segment {
        "Backquote" => "`",
        "Minus" => "-",
        "Equal" => "=",
        "BracketLeft" => "[",
        "BracketRight" => "]",
        "Backslash" => "\\",
        "Semicolon" => ";",
        "Quote" => "'",
        "Comma" => ",",
        "Period" => ".",
        "Slash" => "/",
        "Space" => "Space",
        "Super" | "Meta" => "Win",
        "Enter" => "↵",
        "Tab" => "Tab",
        "Escape" => "Esc",
        "Backspace" => "Backspace",
        "Delete" => "Del",
        "ArrowUp" => "↑",
        "ArrowDown" => "↓",
        "ArrowLeft" => "←",
        "ArrowRight" => "→",
        _ => {
            // keyboard_types::Code 的序列化命名规律：KeyA -> A，Digit1 -> 1。
            if let Some(rest) = segment.strip_prefix("Key")
                && rest.len() == 1
                && rest.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
            {
                return rest;
            }
            if let Some(rest) = segment.strip_prefix("Digit")
                && rest.len() == 1
                && rest.chars().next().is_some_and(|c| c.is_ascii_digit())
            {
                return rest;
            }
            segment
        }
    }
}
