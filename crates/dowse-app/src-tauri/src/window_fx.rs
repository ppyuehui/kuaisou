use std::sync::Mutex;

use serde::Serialize;
use tauri::{Emitter, PhysicalPosition, WebviewWindow};

/// 材质级别。fork 改动：透明度/玻璃效果已完全移除，窗口固定不透明纯色背景
/// （前端 `data-effect='solid'` 走 app.css 的纯色水蓝白渐变）。这个枚举只保留
/// `Solid` 一档，供前端启动时查询确认兜底背景。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum EffectLevel {
    Acrylic,
    Mica,
    Solid,
}

/// 当前生效的材质级别，进程内常驻一份供前端启动时查询
/// （启动阶段 emit 的事件前端不一定来得及监听到，State 查询更可靠）。
pub struct EffectLevelState(pub Mutex<EffectLevel>);

impl EffectLevelState {
    pub fn new(level: EffectLevel) -> Self {
        Self(Mutex::new(level))
    }

    pub fn get(&self) -> EffectLevel {
        *self.0.lock().expect("effect level mutex poisoned")
    }
}

/// Win11 原生窗口圆角裁切：`DwmSetWindowAttribute` 设置
/// `DWMWA_WINDOW_CORNER_PREFERENCE`（33）= `DWMWCP_ROUND`（2），让 DWM 把整个
/// 窗口按系统圆角裁掉。不这么做的话，面板本体的 CSS 圆角只裁了内容，窗口
/// 本体依然是直角矩形，四角会露出"直角窗口 - 圆角面板"的三角形缝隙。
#[cfg(target_os = "windows")]
fn apply_rounded_corners(window: &WebviewWindow) {
    const DWMWA_WINDOW_CORNER_PREFERENCE: u32 = 33;
    const DWMWCP_ROUND: i32 = 2;

    #[link(name = "dwmapi")]
    unsafe extern "system" {
        fn DwmSetWindowAttribute(
            hwnd: *mut core::ffi::c_void,
            dw_attribute: u32,
            pv_attribute: *const core::ffi::c_void,
            cb_attribute: u32,
        ) -> i32;
    }

    let Ok(hwnd) = window.hwnd() else {
        eprintln!("圆角裁切：拿不到 HWND，跳过");
        return;
    };

    let preference: i32 = DWMWCP_ROUND;
    let hr = unsafe {
        DwmSetWindowAttribute(
            hwnd.0,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            &preference as *const i32 as *const core::ffi::c_void,
            std::mem::size_of::<i32>() as u32,
        )
    };
    if hr != 0 {
        eprintln!(
            "圆角裁切：DwmSetWindowAttribute 返回 HRESULT 0x{hr:x}，可能是系统版本太老（Win10 v20H1 以前没有这个属性）"
        );
    }
}

/// 应用窗口材质。fork 改动：透明/玻璃效果已完全移除，窗口固定不透明纯色——
/// 前端据此用 `data-effect='solid'` 的纯色水蓝白渐变兜底。只做圆角裁切，不再
/// 申请任何 Acrylic/Mica。
pub fn apply_with_fallback(window: &WebviewWindow) -> EffectLevel {
    #[cfg(target_os = "windows")]
    apply_rounded_corners(window);
    EffectLevel::Solid
}

/// 窗口居中偏上——参照 Spotlight/Raycast 的位置习惯，不是正中央。
/// 屏幕高度的约 22% 处起摆，比 50% 正中更符合"呼出即用"的视觉预期。
pub fn position_upper_center(window: &WebviewWindow) -> tauri::Result<()> {
    let Some(monitor) = window.current_monitor()? else {
        return Ok(());
    };
    let screen_size = *monitor.size();
    let screen_pos = *monitor.position();
    let win_size = window.outer_size()?;

    let x = screen_pos.x + (screen_size.width as i32 - win_size.width as i32) / 2;
    let y = screen_pos.y + (screen_size.height as f64 * 0.22) as i32;

    window.set_position(PhysicalPosition::new(x, y))
}

/// 呼出：显示、抢焦点，再广播一个事件给前端——前端监听它来做"输入框自动聚焦、
/// 上次查询词全选"（设计文档的交互规则）。
///
/// 这里**不再**每次呼出都重新定位：窗口隐藏是 Windows 原生 `SW_HIDE`，隐藏期间
/// 窗口矩形（位置/大小）原样保留，重新显示自然回到隐藏前的位置和大小——用户
/// 拖动/缩放过的浮窗，呼出时不该被拽回居中偏上。首次定位（启动时居中偏上）
/// 由 lib.rs setup 里的 `position_upper_center` 负责，只有那次会摆位置。
pub fn show_window(window: &WebviewWindow) {
    let _ = window.show();
    let _ = window.set_focus();
    let _ = window.emit("dowse://shown", ());
}

pub fn hide_window(window: &WebviewWindow) {
    let _ = window.hide();
}

pub fn toggle_window(window: &WebviewWindow) {
    if window.is_visible().unwrap_or(false) {
        hide_window(window);
    } else {
        show_window(window);
    }
}
