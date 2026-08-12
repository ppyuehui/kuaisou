import { invoke } from '@tauri-apps/api/core';
import type {
	AppSettings,
	EffectLevel,
	ExtGroup,
	IndexingSnapshot,
	IndexRules,
	IndexStats,
	IndexStatus,
	LangOption,
	PreviewResult,
	SearchHit,
	SortOption,
	ThemeOption
} from './types';

export function indexStatus(): Promise<IndexStatus> {
	return invoke('index_status');
}

/// 建索引进度的当前快照——窗口每次呼出都应该拉一次，跟事件流（
/// `dowse://rebuild-progress`/`dowse://ocr-progress`）接续起来，见 +page.svelte
/// 的 `dowse://shown` 处理。
export function indexingStatus(): Promise<IndexingSnapshot> {
	return invoke('indexing_status');
}

export function search(
	query: string,
	limit = 30,
	extGroup: ExtGroup = 'all',
	sort: SortOption = 'relevance'
): Promise<SearchHit[]> {
	return invoke('search', { query, limit, extGroup, sort });
}

export function preview(path: string, query: string): Promise<PreviewResult | null> {
	return invoke('preview', { path, query });
}

export function openFile(path: string): Promise<void> {
	return invoke('open_file', { path });
}

export function revealInFolder(path: string): Promise<void> {
	return invoke('reveal_in_folder', { path });
}

export function rebuildIndex(dir: string): Promise<IndexStats> {
	return invoke('rebuild_index', { dir });
}

/// 添加一个索引根（多根索引）：不动现有内容，只对新目录做一次收录。
/// 空态"添加文件夹"链接走这个，跟 rebuildIndex 是姊妹命令，返回同一套统计。
export function addRoot(dir: string): Promise<IndexStats> {
	return invoke('add_root', { dir });
}

/// 从索引里移除一个根（设置面板"索引目录"列表的移除按钮用）：前缀圈选删
/// 文档并更新索引，不影响其它根。完成后监听 `dowse://root-removed` 事件刷新
/// 列表。
export function removeRoot(dir: string): Promise<{ removed: number }> {
	return invoke('remove_root', { dir });
}

export function getEffectLevel(): Promise<EffectLevel> {
	return invoke('get_effect_level');
}

/// 当前生效的全局呼出快捷键，`tauri-plugin-global-shortcut` 的原始格式
/// （如 "Alt+Backquote"）——快捷键速查浮层拿去做人类可读的转换再显示。
export function getHotkey(): Promise<string> {
	return invoke('get_hotkey');
}

/// 按扩展名（不带点，小写与否都行）取系统关联图标的 PNG data URI，
/// 取不到返回 null——由调用方（FileIcon 组件）回落到手绘图标。
export function fileIcon(ext: string): Promise<string | null> {
	return invoke('file_icon', { ext });
}

/// 图钉固定开关：会话级，不落盘。固定期间失焦不再自动隐藏浮窗
/// （见 Rust 侧 autohide.rs 的 AutoHideSuppressor）。
export function setPinned(pinned: boolean): Promise<void> {
	return invoke('set_pinned', { pinned });
}

/// 结果行右键：在给定路径上弹出 Win32 原生上下文菜单（打开/打开所在
/// 文件夹/复制路径/复制文件名），菜单本身由 Rust 侧构造和处理，这里只是
/// 触发弹出，不需要等待用户选了哪一项。
export function showResultContextMenu(path: string): Promise<void> {
	return invoke('show_result_context_menu', { path });
}

/// 呼出延迟性能埋点：窗口 `dowse://shown` 之后确认首帧真正绘制完成（双重
/// requestAnimationFrame）才调这个，让 Rust 侧拿热键回调进入的单调时钟算
/// 差值打日志。非热键触发的显示（比如托盘点击）Rust 侧没有起始时刻，
/// 命令内部会静默跳过，前端不需要关心这个区分。
export function reportShownPerf(): Promise<void> {
	return invoke('report_shown_perf');
}

/// 击键到渲染性能埋点：搜索防抖触发、拿到结果、DOM 渲染完成后调一次。
/// `e2eMs` 含防抖等待（触发输入事件到渲染完成），`netMs` 不含（发起后端
/// 调用到渲染完成），`debounceMs` 是当前防抖窗口，一并打进日志避免端到端
/// 数字被误读。
export function reportSearchPerf(e2eMs: number, netMs: number, debounceMs: number): Promise<void> {
	return invoke('report_search_perf', { e2eMs, netMs, debounceMs });
}

/// Esc 收起浮窗。不用 `@tauri-apps/api/window` 的 `getCurrentWindow().hide()`——
/// 那走的是 Tauri core 插件的 `window|hide` 权限点，默认 capability 没放开，
/// 真机上会被 ACL 拒绝。这里走自定义命令，复用全局呼出快捷键同一条隐藏路径，
/// 自定义命令不受 ACL 权限点约束。
export function hideWindow(): Promise<void> {
	return invoke('hide_window');
}

/// 前端拖动分隔条等自带拖拽语义的控件时调用（pointerdown 置真 / pointerup
/// 置假）：让原生拖窗钩子让路，否则"拖分隔条调左右宽度"会被误判成"拖窗口"。
export function setDragSuppressed(suppressed: boolean): Promise<void> {
	return invoke('set_drag_suppressed', { suppressed });
}

/// 最大化/还原切换。同样走自定义命令（`window|toggle-maximize` ACL 权限点
/// 默认没放开），前端只管调用。
export function toggleMaximize(): Promise<void> {
	return invoke('toggle_maximize');
}

/// 当前窗口是否最大化，前端据此切换 最大化/还原 图标。
export function isMaximized(): Promise<boolean> {
	return invoke('is_maximized');
}

/// 设置面板"改键"的冲突预检：探测某个组合键是否已被其它程序占用。
/// resolve true = 可用，false = 已被占用。不做持久化、不动当前快捷键。
export function checkHotkey(hotkey: string): Promise<boolean> {
	return invoke('check_hotkey_available', { hotkey });
}

/// 索引规则面板 Ctrl+, 打开时拉一次当前规则填表单初值。
export function getRules(): Promise<IndexRules> {
	return invoke('get_rules');
}

/// 索引规则面板"保存"：`maxFileMb` 必须是非负整数（Rust 侧是 u64，负数/小数
/// 会在反序列化阶段直接报错，调用方要先归一；0 会被 Rust 侧兜底成 1）；
/// 列表项的 trim/去空/大小写/去重由 Rust 侧统一处理，前端不用重复一遍
/// 规范化逻辑。返回规范化之后的最终值，用来回填表单，让展示的就是落盘的样子。
export function setRules(
	excludeDirs: string[],
	extraTextExts: string[],
	maxFileMb: number
): Promise<IndexRules> {
	return invoke('set_rules', {
		excludeDirs,
		extraTextExts,
		maxFileMb
	});
}

/// 设置面板打开时拉一次通用区（改键/透明/自启/语言）的全部初值。
export function getConfig(): Promise<AppSettings> {
	return invoke('get_config');
}

/// 设置面板"改键"：`hotkey` 是 `tauri-plugin-global-shortcut` 认的格式
/// （如 "Ctrl+Alt+KeyK"，修饰键在前、一个主键在后）。注册失败（多半被别的
/// 程序占用）时 Promise reject，错误文案里说清楚——Rust 侧已回滚到旧键。
export function setHotkey(hotkey: string): Promise<void> {
	return invoke('set_hotkey', { hotkey });
}

/// 设置面板"开机自启"——复用托盘同一条命令路径。系统拒绝写自启项时 reject，
/// 调用方据此回滚 UI 勾选态。
export function setAutostart(enabled: boolean): Promise<void> {
	return invoke('set_autostart', { enabled });
}

/// 设置面板"界面语言"：只落盘，重启后生效（不做运行时热切换，见 i18n.ts）。
export function setLang(lang: LangOption): Promise<void> {
	return invoke('set_lang', { lang });
}

/// 设置面板"失焦自动隐藏"开关：持久化到 config（默认关）。失焦时 Rust 侧
/// 每次现读这个值决定要不要隐藏，落盘即生效，不需要额外同步。
export function setAutoHideOnBlur(enabled: boolean): Promise<void> {
	return invoke('set_auto_hide_on_blur', { enabled });
}

/// 设置面板"日志级别"（debug/info/warn/error/fatal）：即时应用到日志过滤，
/// 并持久化到 config（重启后仍生效）。
export function setLogLevel(level: string): Promise<void> {
	return invoke('set_log_level', { level });
}

/// 设置面板"深色模式"（auto/light/dark）：落盘到 config。热切换由前端完成
/// ——调用方按新值把 `data-theme` 写到 <html> 上，CSS 的 light-dark() 立即
/// 重算（见 app.css），不用重启。
export function setTheme(theme: ThemeOption): Promise<void> {
	return invoke('set_theme', { theme });
}

/// 在资源管理器里打开日志文件夹（%LOCALAPPDATA%\dowse\logs），resolve 出
/// 打开的路径。
export function openLogDir(): Promise<string> {
	return invoke('open_log_dir');
}

/// 在资源管理器里打开索引文件夹（%LOCALAPPDATA%\dowse\index），resolve 出
/// 打开的路径。
export function openIndexDir(): Promise<string> {
	return invoke('open_index_dir');
}
