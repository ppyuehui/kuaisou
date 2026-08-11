use std::sync::Mutex;

use serde::Serialize;
use tauri::{AppHandle, Manager};
use tauri::window::{ProgressBarState, ProgressBarStatus};

/// 建索引流程当前处在哪个阶段。`Idle` 也是"没有在建索引"的常态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum IndexingPhase {
    #[default]
    Idle,
    /// 全量重建的文本阶段：总量未知（走到哪算哪），只有"已处理数 + 当前文件"。
    Text,
    /// OCR 回填阶段：总量已知（文本阶段结束那一刻的图片队列长度，期间如果
    /// 常驻监听又发现新图片会顺势抬高），可以算出真实的完成比例。
    Ocr,
}

/// 建索引进度的一份快照，直接序列化给前端。窗口每次呼出都可以主动拉一次
/// 这份快照（见 `commands::indexing_status`），不用只靠事件流——事件在窗口
/// 隐藏期间照样会发，但前端没监听、也没地方存，重新唤出时必须能补一次。
#[derive(Debug, Clone, Serialize, Default)]
pub struct IndexingSnapshot {
    pub phase: IndexingPhase,
    pub text_processed: usize,
    pub text_current_file: String,
    pub ocr_processed: usize,
    pub ocr_total: usize,
}

/// 进程内常驻的建索引进度状态。写端是 `commands::rebuild_index`（文本阶段）
/// 和 OCR worker 池的进度回调（`watcher.rs`，OCR 阶段）；读端是
/// `commands::indexing_status`（前端窗口每次呼出时拉一次）和事件推送
/// （两条写路径各自顺手 `app.emit` 一次，供窗口开着时的实时刷新）。
///
/// fork 改动：持有一份 `AppHandle`（setup 时 `attach`），每次状态变化顺手把
/// 快照同步到"索引进度窗口"（`sync_window`）——建索引时弹出那个可最小化的
/// 独立窗口并更新任务栏图标进度，完成/失败回到 Idle 时收起。
pub struct IndexingStatus {
    inner: Mutex<IndexingSnapshot>,
    /// 用于操作"索引窗口"的 AppHandle，setup 阶段 attach 进来；attach 之前
    /// 是 None，`sync_window` 直接跳过（不影响任何既有行为）。
    app: Mutex<Option<AppHandle>>,
}

impl IndexingStatus {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(IndexingSnapshot::default()),
            app: Mutex::new(None),
        }
    }

    /// setup 阶段把 AppHandle 存进来，之后每次状态变化都能驱动索引窗口。
    pub fn attach(&self, app: AppHandle) {
        *self.app.lock().expect("indexing status app mutex poisoned") = Some(app);
    }

    pub fn snapshot(&self) -> IndexingSnapshot {
        self.inner
            .lock()
            .expect("indexing status mutex poisoned")
            .clone()
    }

    /// 全量重建开始：清空上一轮的状态，进入文本阶段。
    pub fn begin_text(&self) {
        {
            let mut guard = self.inner.lock().expect("indexing status mutex poisoned");
            *guard = IndexingSnapshot {
                phase: IndexingPhase::Text,
                ..Default::default()
            };
        }
        self.sync_window(true);
    }

    /// 文本阶段的一次进度汇报：累计处理数 + 当前文件，节奏跟
    /// `dowse://rebuild-progress` 事件完全一致（同一处回调顺手更新两边）。
    pub fn set_text_progress(&self, processed: usize, current_file: String) {
        {
            let mut guard = self.inner.lock().expect("indexing status mutex poisoned");
            guard.phase = IndexingPhase::Text;
            guard.text_processed = processed;
            guard.text_current_file = current_file;
        }
        // 窗口在 begin_text 时已经弹出来过；这里只刷新任务栏进度，不重新弹
        // （万一被用户关了/藏了，不该因为进度更新又冒出来）。
        self.sync_window(false);
    }

    /// 文本阶段结束、准备进入 OCR 阶段：`total` 是这一刻 OCR 队列里还有多少张
    /// 图片没处理。0 张的话没有 OCR 阶段可言，直接回到 idle。
    pub fn begin_ocr(&self, total: usize) {
        {
            let mut guard = self.inner.lock().expect("indexing status mutex poisoned");
            if total == 0 {
                *guard = IndexingSnapshot::default();
            } else {
                guard.phase = IndexingPhase::Ocr;
                guard.ocr_total = total;
                guard.ocr_processed = 0;
            }
        }
        // begin_ocr 只在显式建索引流程里出现，force_show 弹出窗口。
        self.sync_window(total > 0);
    }

    /// OCR worker 每 flush 一批就调一次：`pending` 是那一刻队列里还剩多少张。
    /// 剩 0 张直接回到 idle（前端据此让"图片识别"那行淡出）。
    ///
    /// 常驻监听期间可能在 OCR 阶段途中又发现新图片（`pending` 超过当初
    /// `begin_ocr` 记的 `ocr_total`）——这种情况下把 `ocr_total` 顺势抬高，
    /// 保证 `ocr_processed = ocr_total - pending` 不会算出负数。
    ///
    /// 注意这里不 force_show：后台 OCR（启动对账 / 常驻监听发现新图片）也会
    /// 走到这里，不该在用户没有主动建索引时把进度窗口弹出来刷存在感；只有
    /// 窗口已经显示（一次显式建索引正在跑）时才更新任务栏进度。
    pub fn set_ocr_pending(&self, pending: usize) {
        {
            let mut guard = self.inner.lock().expect("indexing status mutex poisoned");
            if pending == 0 {
                *guard = IndexingSnapshot::default();
            } else {
                guard.phase = IndexingPhase::Ocr;
                if pending > guard.ocr_total {
                    guard.ocr_total = pending;
                }
                guard.ocr_processed = guard.ocr_total.saturating_sub(pending);
            }
        }
        self.sync_window(false);
    }

    /// 重建失败等场景下强制回到 idle，不留半截进度。
    pub fn reset_idle(&self) {
        {
            let mut guard = self.inner.lock().expect("indexing status mutex poisoned");
            *guard = IndexingSnapshot::default();
        }
        self.sync_window(false);
    }

    /// 把当前快照同步到"索引进度窗口"（label="indexing"）：
    /// - Idle：隐藏窗口、清掉任务栏进度。
    /// - Text：窗口任务栏进度设成不定态（总量未知）。
    /// - Ocr：任务栏进度设为完成比例（已知总量，0~100）。
    /// `force_show` 为 true 时（显式建索引的 begin_text/begin_ocr）顺带把窗口
    /// 显示出来；后台 OCR 的进度更新只改任务栏，不弹窗。
    /// 窗口内容本身由该窗口的前端页面订阅既有事件流渲染，这里只管显示/隐藏
    /// 和任务栏那格进度。
    ///
    /// attach 之前（启动早期）app 是 None，直接返回；窗口不存在也静默跳过，
    /// 诊断设施绝不因为进度展示再引入新的故障点。
    fn sync_window(&self, force_show: bool) {
        let Some(app) = self.app.lock().expect("indexing status app mutex poisoned").clone() else {
            return;
        };
        let Some(win) = app.get_webview_window("indexing") else {
            return;
        };
        let snap = self.snapshot();
        match snap.phase {
            IndexingPhase::Idle => {
                let _ = win.set_progress_bar(ProgressBarState {
                    status: Some(ProgressBarStatus::None),
                    progress: None,
                });
                if win.is_visible().unwrap_or(false) {
                    let _ = win.hide();
                }
            }
            IndexingPhase::Text => {
                if force_show && !win.is_visible().unwrap_or(false) {
                    let _ = win.show();
                }
                let _ = win.set_progress_bar(ProgressBarState {
                    status: Some(ProgressBarStatus::Indeterminate),
                    progress: None,
                });
            }
            IndexingPhase::Ocr => {
                if force_show && !win.is_visible().unwrap_or(false) {
                    let _ = win.show();
                }
                let frac = if snap.ocr_total > 0 {
                    snap.ocr_processed as f64 / snap.ocr_total as f64
                } else {
                    0.0
                };
                let _ = win.set_progress_bar(ProgressBarState {
                    status: Some(ProgressBarStatus::Normal),
                    progress: Some((frac.clamp(0.0, 1.0) * 100.0).round() as u64),
                });
            }
        }
    }
}
