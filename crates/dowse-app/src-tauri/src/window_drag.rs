//! 原生窗口拖动/缩放（fork 版核心修复）。
//!
//! 背景：这台机器上 Tauri 的 `startDragging`（WM_NCLBUTTONDOWN + HTCAPTION
//! 模态循环）对透明无边框窗口实测失效；前端的"指针事件 + setPosition"方案
//! 依赖 WebView2 把按下后的 pointermove 送达页面，而这台机器上真实鼠标按下
//! 拖动时 move 事件并不可靠（点击能到、拖动到不了）。两条路都走不通，改走
//! **系统层**：装一个 `WH_MOUSE_LL` 全局低层鼠标钩子，在钩子回调里直接看
//! 鼠标消息、`SetWindowPos` 移动/缩放主窗口——完全不经过 WebView，跟页面
//! 有没有收到事件无关，100% 可靠。
//!
//! 行为：
//! - 在主窗口内按住左键移动超过阈值（4px）→ 拖动窗口跟随鼠标。
//! - 按住左键落在窗口边缘 8px 内 → 进入缩放模式，按所在边/角调整尺寸
//!   （西/北边同时平移窗口，四角可双向拉伸），下限 640×420 逻辑像素。
//! - 点一下不移动 → 什么都不做，鼠标事件照常透传给应用（按钮/输入框
//!   正常工作）。
//!
//! 钩子是**观察者**：只读鼠标消息、不改不吞，`CallNextHookEx` 原样放行，
//! 对其它程序和窗口零影响；只有光标落在这一个主窗口内才参与。

use std::sync::atomic::{AtomicBool, AtomicI32, AtomicIsize, Ordering};

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, GetCursorPos, GetMessageW, GetWindowRect, IsChild,
    MSLLHOOKSTRUCT, MSG, SetWindowsHookExW, SetWindowPos, TranslateMessage, UnhookWindowsHookEx,
    WH_MOUSE_LL, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WindowFromPoint, HHOOK,
    SET_WINDOW_POS_FLAGS, SWP_NOACTIVATE, SWP_NOSIZE, SWP_NOZORDER,
};

/// 主窗口句柄（setup 阶段写入）。
static TARGET_HWND: AtomicIsize = AtomicIsize::new(0);
/// 左键是否按在主窗口上。
static ARMED: AtomicBool = AtomicBool::new(false);
/// 按下瞬间的光标位置（屏幕坐标）。
static START_X: AtomicI32 = AtomicI32::new(0);
static START_Y: AtomicI32 = AtomicI32::new(0);
/// 按下瞬间的窗口位置/尺寸（屏幕坐标，物理像素）。
static WIN_X: AtomicI32 = AtomicI32::new(0);
static WIN_Y: AtomicI32 = AtomicI32::new(0);
static WIN_W: AtomicI32 = AtomicI32::new(0);
static WIN_H: AtomicI32 = AtomicI32::new(0);
/// 缩放方向："drag" 或 n/ne/e/se/s/sw/w/nw。
static MODE: AtomicIsize = AtomicIsize::new(0); // 0=drag
/// 按下的物理像素下限（= 逻辑 640×420 × scale，取整）。
static MIN_W: AtomicI32 = AtomicI32::new(640);
static MIN_H: AtomicI32 = AtomicI32::new(420);

const MODE_DRAG: isize = 0;
const MODE_RESIZE: isize = 1;

/// 触发拖动的位移阈值（物理像素）。Windows 的点击抖动标准是 4px，但浮窗
/// 按钮都贴边，用户点关闭/设置时手抖个 4~8px 很常见——阈值太小会把"点击"
/// 误判成"拖窗"（窗口被拖走、点击被取消，按钮就没反应）。取 10px：
/// 正常点击几乎不可能超，真要拖窗拉 10px 也就一瞬间。
const DRAG_THRESHOLD_PX: i32 = 10;
const EDGE_HIT_PX: i32 = 8;

/// 主窗口创建完成后调用：把主窗口句柄交给钩子并启动钩子线程。
pub fn start(hwnd: HWND) {
    TARGET_HWND.store(hwnd.0 as isize, Ordering::SeqCst);
    let _ = std::thread::Builder::new()
        .name("window-drag-hook".into())
        .spawn(hook_thread);
}

/// 钩子线程：装钩子 + 消息泵。`WH_MOUSE_LL` 的回调在安装线程的队列里被
/// 调用，必须在这里 pump 消息钩子才会触发。
fn hook_thread() {
    let hmod = unsafe { GetModuleHandleW(None) }.unwrap_or_default();
    let hinst = windows::Win32::Foundation::HINSTANCE(hmod.0);
    let hook: HHOOK = match unsafe { SetWindowsHookExW(WH_MOUSE_LL, Some(hook_proc), Some(hinst), 0) } {
        Ok(h) => h,
        Err(err) => {
            crate::logging::log_line("drag", &format!("安装鼠标钩子失败，窗口将不能拖动: {err}"));
            return;
        }
    };
    crate::logging::log_line("drag", "原生拖动钩子已安装");
    let mut msg = MSG::default();
    unsafe {
        while GetMessageW(&mut msg, None, 0, 0).0 > 0 {
            let _ = TranslateMessage(&msg);
            let _ = DispatchMessageW(&msg);
        }
        let _ = UnhookWindowsHookEx(hook);
    }
}

/// 方向编码：按位标记边。bit0=E, bit1=S, bit2=W, bit3=N。
const E: i32 = 1;
const S: i32 = 2;
const W: i32 = 4;
const N: i32 = 8;

/// 从光标相对窗口的位置判定命中的边，返回按位标记（0 = 内部，走拖动）。
fn hit_edge(ex: i32, ey: i32, r: &RECT) -> i32 {
    let left = r.left;
    let top = r.top;
    let right = r.right;
    let bottom = r.bottom;
    let mut bits = 0;
    if (ex - left).abs() <= EDGE_HIT_PX {
        bits |= W;
    }
    if (right - ex).abs() <= EDGE_HIT_PX {
        bits |= E;
    }
    if (ey - top).abs() <= EDGE_HIT_PX {
        bits |= N;
    }
    if (bottom - ey).abs() <= EDGE_HIT_PX {
        bits |= S;
    }
    bits
}

unsafe extern "system" fn hook_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        if code >= 0 {
        let msg = wparam.0 as u32;
        let info = &*(lparam.0 as *const MSLLHOOKSTRUCT);
        let target = HWND(TARGET_HWND.load(Ordering::SeqCst) as *mut core::ffi::c_void);
        if !target.is_invalid() {
            match msg {
                WM_LBUTTONDOWN => {
                    let hit = WindowFromPoint(info.pt);
                    if hit == target || IsChild(target, hit).as_bool() {
                        ARMED.store(true, Ordering::SeqCst);
                        START_X.store(info.pt.x, Ordering::SeqCst);
                        START_Y.store(info.pt.y, Ordering::SeqCst);
                        let mut r = RECT::default();
                        if GetWindowRect(target, &mut r).is_ok() {
                            WIN_X.store(r.left, Ordering::SeqCst);
                            WIN_Y.store(r.top, Ordering::SeqCst);
                            WIN_W.store(r.right - r.left, Ordering::SeqCst);
                            WIN_H.store(r.bottom - r.top, Ordering::SeqCst);
                        }
                        let bits = hit_edge(info.pt.x, info.pt.y, &r);
                        if bits != 0 {
                            MODE.store(MODE_RESIZE, Ordering::SeqCst);
                            RESIZE_BITS.store(bits as isize, Ordering::SeqCst);
                        } else {
                            MODE.store(MODE_DRAG, Ordering::SeqCst);
                        }
                    }
                }
                WM_MOUSEMOVE => {
                    if ARMED.load(Ordering::SeqCst) {
                        let sx = START_X.load(Ordering::SeqCst);
                        let sy = START_Y.load(Ordering::SeqCst);
                        let dx = info.pt.x - sx;
                        let dy = info.pt.y - sy;
                        if dx.abs() > DRAG_THRESHOLD_PX || dy.abs() > DRAG_THRESHOLD_PX {
                            if MODE.load(Ordering::SeqCst) == MODE_DRAG {
                                let wx = WIN_X.load(Ordering::SeqCst) + dx;
                                let wy = WIN_Y.load(Ordering::SeqCst) + dy;
                                let _ = SetWindowPos(
                                    target,
                                    None,
                                    wx,
                                    wy,
                                    0,
                                    0,
                                    SET_WINDOW_POS_FLAGS(SWP_NOSIZE.0 | SWP_NOZORDER.0 | SWP_NOACTIVATE.0),
                                );
                            } else {
                                apply_resize(target, dx, dy);
                            }
                        }
                    }
                }
                WM_LBUTTONUP => {
                    ARMED.store(false, Ordering::SeqCst);
                    MODE.store(MODE_DRAG, Ordering::SeqCst);
                }
                _ => {}
            }
        }
    }
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

static RESIZE_BITS: AtomicIsize = AtomicIsize::new(0);

/// 按方向位标记计算缩放后的新尺寸/位置并落地（SetWindowPos 一次搞定尺寸和
/// 位置，西/北边缩放需要同步平移右/下锚点）。
fn apply_resize(target: HWND, dx: i32, dy: i32) {
    let bits = RESIZE_BITS.load(Ordering::SeqCst) as i32;
    let base_w = WIN_W.load(Ordering::SeqCst);
    let base_h = WIN_H.load(Ordering::SeqCst);
    let min_w = MIN_W.load(Ordering::SeqCst);
    let min_h = MIN_H.load(Ordering::SeqCst);

    let mut new_w = base_w;
    let mut new_h = base_h;
    let mut new_x = WIN_X.load(Ordering::SeqCst);
    let mut new_y = WIN_Y.load(Ordering::SeqCst);

    if bits & E != 0 {
        new_w = (base_w + dx).max(min_w);
    }
    if bits & W != 0 {
        let w = (base_w - dx).max(min_w);
        new_x = WIN_X.load(Ordering::SeqCst) + (base_w - w);
        new_w = w;
    }
    if bits & S != 0 {
        new_h = (base_h + dy).max(min_h);
    }
    if bits & N != 0 {
        let h = (base_h - dy).max(min_h);
        new_y = WIN_Y.load(Ordering::SeqCst) + (base_h - h);
        new_h = h;
    }

    let _ = unsafe {
        SetWindowPos(
            target,
            None,
            new_x,
            new_y,
            new_w,
            new_h,
            SET_WINDOW_POS_FLAGS(SWP_NOZORDER.0 | SWP_NOACTIVATE.0),
        )
    };
}

/// 缩放下限：逻辑 640×420 × scaleFactor。`scale` 由 `GetDpiForWindow` 换算
/// （dpi / 96）。主窗口创建后调用一次。
pub fn set_min_size(hwnd: HWND, logical_w: i32, logical_h: i32) {
    let scale = window_scale(hwnd);
    MIN_W.store((logical_w as f64 * scale).round() as i32, Ordering::SeqCst);
    MIN_H.store((logical_h as f64 * scale).round() as i32, Ordering::SeqCst);
}

fn window_scale(hwnd: HWND) -> f64 {
    #[cfg(target_os = "windows")]
    {
        use windows::Win32::UI::HiDpi::GetDpiForWindow;
        let dpi = unsafe { GetDpiForWindow(hwnd) };
        if dpi > 0 {
            return dpi as f64 / 96.0;
        }
    }
    1.0
}

/// 供 `GetCursorPos` 之类的未来需求引用（当前实现用 MSLLHOOKSTRUCT 自带坐标，
/// 不需要它，保留调用以防误删）。
#[allow(dead_code)]
fn _cursor_pos() -> POINT {
    let mut p = POINT::default();
    unsafe {
        let _ = GetCursorPos(&mut p);
    }
    p
}
