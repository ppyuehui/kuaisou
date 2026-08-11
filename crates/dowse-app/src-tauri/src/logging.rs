//! 文件日志设施：仿 `C:\Users\hui\工作\Agent\自编软件\Logging\FileLogger.cs`
//! 的轻量设计——5 个级别（Debug/Info/Warn/Error/Fatal）、按天分文件
//! （`YYYY-MM-DD.log`）、只保留最近 7 天、可运行时调整最低级别（低于它的
//! 日志不写盘）。日志目录仍固定在 `%LOCALAPPDATA%\dowse\logs`。
//!
//! 同时保留原设施的崩溃取证能力：把进程 stdout/stderr 重定向到"今天的日志
//! 文件"，再挂 panic hook 记崩溃线程/位置/信息——dowse 库内部各处排障用的
//! `eprintln!` 因此继续原样落进当天日志，不用逐个调用点改造。

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Mutex;
use std::time::SystemTime;

/// 日志级别。排序即优先级：Debug < Info < Warn < Error < Fatal，
/// `rank` 数字越小级别越低；"最低级别"过滤是"级别 >= 最低级别才写盘"。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    Debug,
    Info,
    Warn,
    Error,
    Fatal,
}

impl LogLevel {
    /// 配置字符串 → 级别。只认小写英文名（serde 落盘的就是这些）。
    pub fn parse(s: &str) -> Option<LogLevel> {
        match s.to_ascii_lowercase().as_str() {
            "debug" => Some(LogLevel::Debug),
            "info" => Some(LogLevel::Info),
            "warn" => Some(LogLevel::Warn),
            "error" => Some(LogLevel::Error),
            "fatal" => Some(LogLevel::Fatal),
            _ => None,
        }
    }

    fn name(&self) -> &'static str {
        match self {
            LogLevel::Debug => "DEBUG",
            LogLevel::Info => "INFO",
            LogLevel::Warn => "WARN",
            LogLevel::Error => "ERROR",
            LogLevel::Fatal => "FATAL",
        }
    }

    fn rank(&self) -> u8 {
        *self as u8
    }
}

/// 当前最低日志级别（低于它的日志不写盘），运行时可通过 `set_min_level`
/// 调整（设置面板"日志级别"落盘后即时生效）。默认 Debug——在 `init()` 之后
/// 由 `run()` 读配置覆盖成用户选择的值。
static MIN_LEVEL: AtomicU8 = AtomicU8::new(LogLevel::Debug as u8);

/// 串行化所有日志文件写操作（同一个文件句柄的多线程追加）。
static WRITE_LOCK: Mutex<()> = Mutex::new(());

/// 日志目录（`init()` 时定死），None 表示还没初始化。
static LOG_DIR: Mutex<Option<PathBuf>> = Mutex::new(None);

/// "今天"的日志文件句柄（`YYYY-MM-DD.log`），跨天时重新打开。
static CURRENT_FILE: Mutex<Option<File>> = Mutex::new(None);

/// CURRENT_FILE 对应的日期串，用于跨天判断。
static CURRENT_DAY: Mutex<String> = Mutex::new(String::new());

/// 保留天数：C# 版默认 7，这里保持一致。
const KEEP_DAYS: u64 = 7;

/// 运行时调整最低日志级别（低于它的日志不写盘）。设置面板改级别落盘后调用。
pub fn set_min_level(level: LogLevel) {
    MIN_LEVEL.store(level.rank(), Ordering::Relaxed);
}

/// 日志目录：`%LOCALAPPDATA%\dowse\logs`。`pub` 供设置面板"打开日志文件夹"
/// （`commands::open_log_dir`）复用同一个路径来源，避免两处各写一份。
pub fn log_dir() -> Option<PathBuf> {
    directories::ProjectDirs::from("", "", "dowse").map(|dirs| dirs.data_local_dir().join("logs"))
}

/// 初始化：建目录、清理过期日志、打开今天的文件、重定向 stdout/stderr、挂
/// panic hook。任何一步失败都只是放弃日志能力，不影响应用正常启动——诊断设施
/// 本身不该成为新的故障点。必须在 `run()` 最开始调用。
pub fn init() {
    let Some(dir) = log_dir() else {
        return;
    };
    if fs::create_dir_all(&dir).is_err() {
        return;
    }
    clean_old_logs(&dir);

    *LOG_DIR.lock().expect("log dir mutex poisoned") = Some(dir);
    ensure_day_file();

    log_line("startup", &format!("dowse {} 启动", env!("CARGO_PKG_VERSION")));
    install_panic_hook();
}

/// 跨天切换"今天的文件"：日期没变就什么都不做。新开当天文件时顺带把
/// stdout/stderr 重定向到新句柄（旧句柄已泄漏、OS 表里那项还能用，但既然
/// 已经打开新的一天，就让它也指向新文件，保证 eprintln! 内容按天归位）。
fn ensure_day_file() {
    let dir = LOG_DIR.lock().expect("log dir mutex poisoned").clone();
    let Some(dir) = dir else {
        return;
    };
    let today = today_str();
    let mut day_guard = CURRENT_DAY.lock().expect("log day mutex poisoned");
    if *day_guard == today {
        return;
    }
    let path = dir.join(format!("{today}.log"));
    if let Ok(file) = OpenOptions::new().create(true).append(true).open(&path) {
        #[cfg(target_os = "windows")]
        if let Ok(stdio) = file.try_clone() {
            // 泄漏 stdio 句柄：SetStdHandle 只把表项指到这个句柄值，不接管
            // 所有权；如果允许 drop，CloseHandle 会立刻让整条 stderr 失效。
            redirect_std_handles(&stdio);
            std::mem::forget(stdio);
        }
        *CURRENT_FILE.lock().expect("log file mutex poisoned") = Some(file);
        *day_guard = today;
    }
}

/// 记一行日志。`component` 是简短的来源标签（"startup"/"watch"/"rebuild"/
/// "ocr"/"perf"/"panic"），`msg` 是人类可读的一句话。先过级别过滤，低于
/// 当前最低级别的直接丢弃，不落盘也不做任何额外工作。
///
/// 克制使用：只记生命周期/错误/降级事件，不记每次搜索/每次文件事件——那些
/// 量级太大，按天文件也会被几分钟撑爆。
pub fn log_line_at(level: LogLevel, component: &str, msg: &str) {
    if level.rank() < MIN_LEVEL.load(Ordering::Relaxed) {
        return;
    }
    let line = format!("[{0}] [{1}] {2}: {3}", format_now(), component, level.name(), msg);
    let _guard = WRITE_LOCK.lock().expect("log write lock poisoned");
    ensure_day_file();
    if let Some(file) = CURRENT_FILE.lock().expect("log file mutex poisoned").as_mut() {
        let _ = writeln!(file, "{line}");
    }
}

/// 缺省级别（Info）的日志入口——历史调用点（startup/perf/watch/ocr 等）原样
/// 迁移，不用逐个改签名。真正需要不同级别的调用点用 `log_line_at`。
pub fn log_line(component: &str, msg: &str) {
    log_line_at(LogLevel::Info, component, msg);
}

/// 删除 `KEEP_DAYS` 天前的 `.log` 文件（按最后修改时间判断）。只在 `init()`
/// 时跑一次，够了——过期文件不会因为不跑就爆炸。
fn clean_old_logs(dir: &std::path::Path) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let now = SystemTime::now();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("log") {
            continue;
        }
        let keep_until = now
            .checked_sub(std::time::Duration::from_secs(KEEP_DAYS * 86_400))
            .unwrap_or(now);
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        if let Ok(modified) = meta.modified() {
            if modified < keep_until {
                let _ = fs::remove_file(&path);
            }
        }
    }
}

/// 把当前进程的 STD_OUTPUT_HANDLE / STD_ERROR_HANDLE 都指向给定文件的底层
/// Win32 句柄——重定向之后，进程内任何地方（包括 dowse 的 `eprintln!`）
/// 原样落进日志文件。调用方负责 `mem::forget` 保持句柄有效。
#[cfg(target_os = "windows")]
fn redirect_std_handles(file: &File) {
    use std::os::windows::io::AsRawHandle;
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::System::Console::{STD_ERROR_HANDLE, STD_OUTPUT_HANDLE, SetStdHandle};

    let handle = HANDLE(file.as_raw_handle());
    unsafe {
        let _ = SetStdHandle(STD_OUTPUT_HANDLE, handle);
        let _ = SetStdHandle(STD_ERROR_HANDLE, handle);
    }
}

/// 挂 panic hook：崩溃时把线程名、位置、payload 落一行 ERROR 日志（走的是
/// 重定向过的 stderr），再链到系统默认 hook——开发时 `cargo tauri dev` 带
/// 控制台，原有的默认崩溃输出照样能看到，不丢失。
///
/// fork 改动：**去重**。dowse 库抽取文本时对畸形 PDF/Office 文件用
/// `catch_unwind` 兜底（见 `dowse::extract`），这些 panic 是预期内的、文件
/// 会被安静跳过；但 panic hook 对每个被接住的 panic 也会执行，逐条打日志
/// 会把日志刷爆（中文 PDF 目录下每次对账都能刷几百上千行）。改成：同一
/// （位置+信息）的 panic 只完整记第一条并链默认 hook；重复的只计数，每满
/// 100 次记一行"同一 panic 已重复 N 次"的汇总——真正的偶发崩溃信息一条
/// 不少，被兜底的批量 panic 不再淹没日志。
fn install_panic_hook() {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};

    static PANIC_COUNTS: OnceLock<Mutex<HashMap<String, u32>>> = OnceLock::new();

    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let thread = std::thread::current();
        let thread_name = thread.name().unwrap_or("<unnamed>");
        let location = info
            .location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
            .unwrap_or_else(|| "<unknown>".to_string());
        let payload = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "<non-string panic payload>".to_string());

        let key = format!("{location} | {payload}");
        let counts = PANIC_COUNTS.get_or_init(|| Mutex::new(HashMap::new()));
        let mut guard = counts.lock().unwrap_or_else(|e| e.into_inner());
        let n = *guard.entry(key.clone()).or_insert(0);
        *guard.entry(key.clone()).or_insert(0) += 1;
        drop(guard);

        if n == 0 {
            // 第一条：完整记一行并链默认 hook（保留 stderr 的原始崩溃输出）。
            log_line_at(
                LogLevel::Error,
                "panic",
                &format!("线程 [{thread_name}] 在 {location} 崩溃: {payload}"),
            );
            default_hook(info);
        } else if n % 100 == 0 {
            // 同一 panic 反复出现：每满 100 次汇总一行，不再逐条刷屏。
            log_line_at(
                LogLevel::Warn,
                "panic",
                &format!("同一 panic 已重复 {n} 次（{key}）"),
            );
        }
        // 其它重复：静默计数，不写日志。
    }));
}

/// 本地时间（Windows：`GetLocalTime`；其它平台回落 UTC）。返回
/// (年,月,日,时,分,秒)。
#[cfg(target_os = "windows")]
fn local_time() -> (u16, u16, u16, u16, u16, u16) {
    use windows::Win32::Foundation::SYSTEMTIME;
    use windows::Win32::System::SystemInformation::GetLocalTime;
    let st: SYSTEMTIME = unsafe { GetLocalTime() };
    (st.wYear, st.wMonth, st.wDay, st.wHour, st.wMinute, st.wSecond)
}

#[cfg(not(target_os = "windows"))]
fn local_time() -> (u16, u16, u16, u16, u16, u16) {
    let now = SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = now.as_secs();
    let (days, rem) = (secs / 86_400, secs % 86_400);
    let (hour, rem) = (rem / 3600, rem % 3600);
    let (min, sec) = (rem / 60, rem % 60);
    let (year, month, day) = civil_from_days(days as i64);
    (year as u16, month as u16, day as u16, hour as u16, min as u16, sec as u16)
}

/// 时间戳格式化（本地时间）——日志只是给人看的排障材料，精确到秒完全够用。
/// fork 改动：原来是 UTC，改成系统本地时间（跟 Windows 资源管理器里看到的
/// 文件时间一致，排查更直观）。
fn format_now() -> String {
    let (year, month, day, hour, min, sec) = local_time();
    format!("{year:04}-{month:02}-{day:02} {hour:02}:{min:02}:{sec:02}")
}

/// 今天的日期串 `YYYY-MM-DD`（本地日期），用作日志文件名。
fn today_str() -> String {
    let (year, month, day, _, _, _) = local_time();
    format!("{year:04}-{month:02}-{day:02}")
}

/// `days` = 自 1970-01-01 起的天数，返回 (year, month, day)。算法来自
/// Howard Hinnant 的 `civil_from_days`（公开的、被广泛引用的无分支实现）。
/// Windows 主路径用 `GetLocalTime`，这个函数只服务非 Windows 回落和单测。
#[allow(dead_code)]
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_from_days_epoch_is_1970_01_01() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
    }

    #[test]
    fn civil_from_days_known_date() {
        // 2024-01-01 是 epoch 之后第 19723 天。
        assert_eq!(civil_from_days(19_723), (2024, 1, 1));
    }

    #[test]
    fn level_parse_and_rank() {
        assert_eq!(LogLevel::parse("info"), Some(LogLevel::Info));
        assert_eq!(LogLevel::parse("ERROR"), Some(LogLevel::Error));
        assert_eq!(LogLevel::parse("verbose"), None);
        assert!(LogLevel::Error.rank() > LogLevel::Info.rank());
    }
}
