//! 「有事要你看」的提醒：任务栏按钮闪烁加角标、托盘图标加一个点、托盘提示写原因。
//!
//! 现在只有一个来源：SVN 自动跟进被挡下（2026-11-11 使用报告 9.5）。原因只写在窗口里的横幅上时，
//! 人不在看这个窗口就一直不知道，那次隔了两个小时。
//!
//! 只用 Tauri 自带的窗口 / 托盘接口，不发系统通知：toast 要新增依赖，而且便携版没有登记过的
//! AUMID，弹不弹得出来没有把握（决策日志「方案取舍」）。只做窗口层面的事，不碰任何 git 状态。

use std::sync::Mutex;
use tauri::image::Image;
use tauri::{AppHandle, Manager, Runtime, UserAttentionType};

/// 提醒用的橙色，不透明。和横幅上「请确认」那一档是同一个意思。
const DOT: [u8; 4] = [0xF5, 0x9E, 0x0B, 0xFF];

/// 任务栏角标的边长。Windows 按 16×16 显示。
const OVERLAY_SIZE: u32 = 16;

/// Windows 托盘提示的上限：`NOTIFYICONDATAW::szTip` 是 128 个 UTF-16 单位，含结尾的 0。
const TOOLTIP_MAX_UTF16: usize = 127;

/// 在图标右下角盖一个实心圆点，交回新的 RGBA。
///
/// `base` 的长度和宽高对不上时原样交回：宁可没有点，也不能越界或交出一张花掉的图标。
pub fn badge_rgba(base: &[u8], width: u32, height: u32) -> Vec<u8> {
    let mut out = base.to_vec();
    if !fits(base, width, height) {
        return out;
    }
    // 直径取短边的 46%：16×16 的托盘图标上是 7 像素，看得见，也不把图标盖没
    let r = (width.min(height) as f32 * 0.46 / 2.0).max(1.0);
    paint_dot(
        &mut out,
        width,
        height,
        width as f32 - r,
        height as f32 - r,
        r,
    );
    out
}

/// 任务栏角标：透明底上一个占满的圆点。
pub fn overlay_rgba() -> Vec<u8> {
    let s = OVERLAY_SIZE;
    let mut out = vec![0u8; (s * s * 4) as usize];
    let r = s as f32 / 2.0;
    paint_dot(&mut out, s, s, r, r, r);
    out
}

fn fits(rgba: &[u8], width: u32, height: u32) -> bool {
    width > 0 && height > 0 && rgba.len() as u64 == u64::from(width) * u64::from(height) * 4
}

/// 按像素中心到圆心的距离填色。不做抗锯齿：16 像素上糊一圈半透明反而发灰。
fn paint_dot(rgba: &mut [u8], width: u32, height: u32, cx: f32, cy: f32, r: f32) {
    for y in 0..height {
        for x in 0..width {
            let dx = x as f32 + 0.5 - cx;
            let dy = y as f32 + 0.5 - cy;
            if dx * dx + dy * dy <= r * r {
                let i = ((y * width + x) * 4) as usize;
                rgba[i..i + 4].copy_from_slice(&DOT);
            }
        }
    }
}

/// 托盘提示：第一行是平时的标题，有原因时第二行写原因。
///
/// 自己截到上限以内：超长的提示交给系统，有的版本整条不显示。按 UTF-16 单位数截，
/// 不拆开代理对，截过的以「…」结尾。原因里的换行换成空格，托盘提示里多行只会被吃掉。
pub fn tooltip_text(title: &str, reason: Option<&str>) -> String {
    let Some(reason) = reason.map(str::trim).filter(|r| !r.is_empty()) else {
        return title.to_string();
    };
    let one_line = reason.split_whitespace().collect::<Vec<_>>().join(" ");
    truncate_utf16(&format!("{title}\n{one_line}"), TOOLTIP_MAX_UTF16)
}

fn truncate_utf16(text: &str, max: usize) -> String {
    if text.encode_utf16().count() <= max {
        return text.to_string();
    }
    let mut out = String::new();
    let mut used = 1; // 给结尾的「…」留一个单位
    for ch in text.chars() {
        let n = ch.len_utf16();
        if used + n > max {
            break;
        }
        used += n;
        out.push(ch);
    }
    out.push('…');
    out
}

/// 命令传进来的原因：空白当作没有。
pub fn normalize_reason(reason: Option<String>) -> Option<String> {
    reason
        .map(|r| r.trim().to_string())
        .filter(|r| !r.is_empty())
}

/// 现在挂着的提醒，以及「该闪还没闪」的那一次。
///
/// 窗口外的东西（任务栏按钮、托盘）不会自己记住这些：藏到托盘再显示时任务栏按钮是重建的、角标没了
/// （独立 review 第 4 条）；被挡下时窗口恰好在前台，不闪，之后人切走也不会再闪（第 6 条）。
/// 所以后端自己记一份，在窗口显示、失焦时补上。
#[derive(Debug, Default, PartialEq)]
pub struct Attention {
    reason: Option<String>,
    pending_flash: bool,
}

impl Attention {
    /// 换成新的提醒，交回这一刻要不要闪。
    ///
    /// 窗口在前台时不闪（人正看着横幅），记成待闪，等失焦再闪。不带 `flash` 的更新（别的仓库撤掉了，
    /// 剩下的那条不是新的事）不动待闪标记。撤掉时连待闪一起清。
    pub fn set(&mut self, reason: Option<String>, flash: bool, focused: bool) -> bool {
        self.reason = reason;
        if self.reason.is_none() {
            self.pending_flash = false;
            return false;
        }
        if !flash {
            return false;
        }
        self.pending_flash = focused;
        !focused
    }

    /// 窗口失焦：有待闪的就这时闪，只闪一次。
    pub fn on_blur(&mut self) -> bool {
        let flash = self.pending_flash && self.reason.is_some();
        self.pending_flash = false;
        flash
    }

    pub fn reason(&self) -> Option<&str> {
        self.reason.as_deref()
    }
}

/// 全进程一份：任务栏和托盘只有一个。
///
/// **不能持着这把锁调窗口 / 托盘的接口**：那些调用会派到主线程并阻塞等结果，而窗口事件
/// （`on_blur`）就跑在主线程上，持锁等主线程的工作线程和等锁的主线程会一起卡死
/// （`desktop::DesktopState` 的注释记过同一个坑）。所以只在锁里算决定，出了锁再动窗口。
static STATE: Mutex<Attention> = Mutex::new(Attention {
    reason: None,
    pending_flash: false,
});

fn state() -> std::sync::MutexGuard<'static, Attention> {
    // 中毒只说明别处 panic 过；这份状态没有跨字段的不变量，拿回来接着用
    STATE.lock().unwrap_or_else(|e| e.into_inner())
}

/// 挂上或撤掉提醒（命令 `set_attention` 的实现）。`reason` 为 `None` 时全部复原。
///
/// 每一步失败都只记日志：提醒是锦上添花，不能让它把调用它的流程带成失败；托盘没建出来、
/// 平台不支持角标都属于这种情况。
pub fn set<R: Runtime>(app: &AppHandle<R>, reason: Option<String>, flash: bool) {
    let window = app.get_webview_window(crate::desktop::MAIN_WINDOW);
    // 读不到焦点状态按不在前台算，宁可多闪一次
    let focused = window
        .as_ref()
        .is_some_and(|w| w.is_focused().unwrap_or(false));
    let flash_now = state().set(reason.clone(), flash, focused);
    paint(app, reason.as_deref());
    if let Some(w) = window {
        let attention = match (&reason, flash_now) {
            (Some(_), true) => Some(UserAttentionType::Informational),
            (Some(_), false) => return,
            (None, _) => None,
        };
        if let Err(e) = w.request_user_attention(attention) {
            log::warn!("任务栏闪烁失败: {e}");
        }
    }
}

/// 主窗口失焦（`desktop::on_window_event`）：被挡下时窗口在前台、当时没闪的，这时补闪。
pub fn on_blur<R: Runtime>(app: &AppHandle<R>) {
    if !state().on_blur() {
        return;
    }
    if let Some(w) = app.get_webview_window(crate::desktop::MAIN_WINDOW) {
        if let Err(e) = w.request_user_attention(Some(UserAttentionType::Informational)) {
            log::warn!("任务栏闪烁失败: {e}");
        }
    }
}

/// 主窗口刚显示（从托盘、快捷键呼出）：任务栏按钮是重建的，重新挂角标。
pub fn on_show<R: Runtime>(app: &AppHandle<R>) {
    let reason = state().reason().map(str::to_string);
    if reason.is_some() {
        paint(app, reason.as_deref());
    }
}

/// 主窗口开始加载新页面（F5、开发时整页重载）：前端的汇总从头开始、以为什么都没挂，
/// 后端这边不清的话角标就再也撤不掉了（独立 review 第 2 条）。新页面上的标签会重新上报。
pub fn on_page_reload<R: Runtime>(app: &AppHandle<R>) {
    if state().reason().is_some() {
        set(app, None, false);
    }
}

/// 按 `reason` 画托盘图标、托盘提示和任务栏角标。
fn paint<R: Runtime>(app: &AppHandle<R>, reason: Option<&str>) {
    let title = crate::build_info::window_title();
    if let Some(tray) = app.tray_by_id(crate::desktop::TRAY_ID) {
        if let Err(e) = tray.set_tooltip(Some(tooltip_text(&title, reason))) {
            log::warn!("改托盘提示失败: {e}");
        }
        if let Some(icon) = app.default_window_icon() {
            let image = match reason {
                Some(_) => Image::new_owned(
                    badge_rgba(icon.rgba(), icon.width(), icon.height()),
                    icon.width(),
                    icon.height(),
                ),
                None => icon.clone(),
            };
            if let Err(e) = tray.set_icon(Some(image)) {
                log::warn!("改托盘图标失败: {e}");
            }
        }
    }
    let Some(window) = app.get_webview_window(crate::desktop::MAIN_WINDOW) else {
        return;
    };
    let overlay = reason.map(|_| Image::new_owned(overlay_rgba(), OVERLAY_SIZE, OVERLAY_SIZE));
    if let Err(e) = window.set_overlay_icon(overlay) {
        log::warn!("改任务栏角标失败: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pixel(rgba: &[u8], width: u32, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * width + x) * 4) as usize;
        [rgba[i], rgba[i + 1], rgba[i + 2], rgba[i + 3]]
    }

    #[test]
    fn badge_paints_the_bottom_right_corner_and_leaves_the_rest_alone() {
        let base = vec![7u8; 32 * 32 * 4];
        let out = badge_rgba(&base, 32, 32);
        assert_eq!(out.len(), base.len());
        // 圆心附近是点
        assert_eq!(pixel(&out, 32, 24, 24), DOT);
        // 左上、右上、左下都没动
        for (x, y) in [(0, 0), (31, 0), (0, 31), (8, 8)] {
            assert_eq!(pixel(&out, 32, x, y), [7, 7, 7, 7], "({x}, {y})");
        }
        // 圆的外接方形的角在圆外：点是圆的，不是方块
        assert_eq!(pixel(&out, 32, 17, 17), [7, 7, 7, 7]);
    }

    #[test]
    fn badge_is_visible_on_a_16px_tray_icon() {
        let base = vec![0u8; 16 * 16 * 4];
        let out = badge_rgba(&base, 16, 16);
        let painted = out.chunks(4).filter(|p| *p == DOT).count();
        // 16×16 = 256 像素里圆点占三十几个：看得见，也没盖掉大半个图标
        assert!((20..=60).contains(&painted), "画了 {painted} 个像素");
    }

    #[test]
    fn badge_returns_the_input_when_the_size_does_not_match() {
        let base = vec![1u8; 10];
        assert_eq!(badge_rgba(&base, 16, 16), base);
        assert_eq!(badge_rgba(&[], 0, 0), Vec::<u8>::new());
        assert_eq!(badge_rgba(&base, 0, 16), base);
    }

    #[test]
    fn badge_handles_non_square_icons() {
        let base = vec![0u8; 20 * 10 * 4];
        let out = badge_rgba(&base, 20, 10);
        assert_eq!(out.len(), base.len());
        // 右下角有点，左边没有
        assert_eq!(pixel(&out, 20, 17, 7), DOT);
        assert_eq!(pixel(&out, 20, 2, 7), [0, 0, 0, 0]);
    }

    #[test]
    fn overlay_is_a_dot_on_a_transparent_canvas() {
        let out = overlay_rgba();
        assert_eq!(out.len(), (OVERLAY_SIZE * OVERLAY_SIZE * 4) as usize);
        assert_eq!(pixel(&out, OVERLAY_SIZE, 8, 8), DOT);
        assert_eq!(pixel(&out, OVERLAY_SIZE, 0, 0), [0, 0, 0, 0]);
        assert_eq!(pixel(&out, OVERLAY_SIZE, 15, 15), [0, 0, 0, 0]);
    }

    #[test]
    fn tooltip_without_a_reason_is_the_plain_title() {
        assert_eq!(tooltip_text("jkk 0.8.3", None), "jkk 0.8.3");
        assert_eq!(tooltip_text("jkk 0.8.3", Some("")), "jkk 0.8.3");
        assert_eq!(tooltip_text("jkk 0.8.3", Some("  \n ")), "jkk 0.8.3");
    }

    #[test]
    fn tooltip_puts_the_reason_on_a_second_line_and_flattens_newlines() {
        assert_eq!(
            tooltip_text("jkk", Some("Client：没有自动跟进\n  重放有冲突")),
            "jkk\nClient：没有自动跟进 重放有冲突"
        );
    }

    #[test]
    fn tooltip_is_cut_to_the_windows_limit_in_utf16_units() {
        let long = "路径".repeat(200);
        let text = tooltip_text("jkk", Some(&long));
        assert_eq!(text.encode_utf16().count(), TOOLTIP_MAX_UTF16);
        assert!(text.ends_with('…'));
        assert!(text.starts_with("jkk\n路径"));
    }

    #[test]
    fn tooltip_never_splits_a_surrogate_pair() {
        // 每个 emoji 是 2 个 UTF-16 单位：截断点落在一对中间时整个丢掉
        let long = "😀".repeat(100);
        let text = tooltip_text("ab", Some(&long));
        assert!(text.encode_utf16().count() <= TOOLTIP_MAX_UTF16);
        assert!(text.ends_with('…'));
        // 能无损转回来就说明没有落单的代理项
        assert_eq!(
            String::from_utf16(&text.encode_utf16().collect::<Vec<_>>()).unwrap(),
            text
        );
    }

    #[test]
    fn a_reason_exactly_at_the_limit_is_not_cut() {
        let reason = "a".repeat(TOOLTIP_MAX_UTF16 - "jkk\n".len());
        let text = tooltip_text("jkk", Some(&reason));
        assert_eq!(text.encode_utf16().count(), TOOLTIP_MAX_UTF16);
        assert!(!text.ends_with('…'));
    }

    fn on(reason: &str) -> Option<String> {
        Some(reason.to_string())
    }

    #[test]
    fn a_new_reason_flashes_at_once_when_the_window_is_in_the_background() {
        let mut a = Attention::default();
        assert!(a.set(on("冲突"), true, false));
        assert_eq!(a.reason(), Some("冲突"));
        // 已经闪过，失焦不再闪
        assert!(!a.on_blur());
    }

    #[test]
    fn a_new_reason_while_focused_waits_and_flashes_on_blur_once() {
        let mut a = Attention::default();
        assert!(!a.set(on("冲突"), true, true));
        assert!(a.on_blur());
        assert!(!a.on_blur(), "只补闪一次");
    }

    #[test]
    fn clearing_drops_the_pending_flash() {
        let mut a = Attention::default();
        a.set(on("冲突"), true, true);
        assert!(!a.set(None, false, true));
        assert_eq!(a.reason(), None);
        assert!(!a.on_blur());
    }

    #[test]
    fn an_update_without_flash_keeps_a_pending_flash() {
        // 另一个仓库撤掉了，剩下那条原样留着：之前欠的那次闪还要补
        let mut a = Attention::default();
        a.set(on("2 个仓库"), true, true);
        assert!(!a.set(on("Client 没有自动跟进：冲突"), false, true));
        assert!(a.on_blur());
    }

    #[test]
    fn an_update_without_flash_never_flashes() {
        let mut a = Attention::default();
        assert!(!a.set(on("x"), false, false));
        assert!(!a.on_blur());
    }

    #[test]
    fn a_later_background_flash_clears_the_pending_one() {
        let mut a = Attention::default();
        a.set(on("a"), true, true);
        assert!(a.set(on("b"), true, false));
        assert!(!a.on_blur(), "后台那次已经闪过，失焦不再补");
    }

    #[test]
    fn blank_reasons_count_as_none() {
        assert_eq!(normalize_reason(None), None);
        assert_eq!(normalize_reason(Some(String::new())), None);
        assert_eq!(normalize_reason(Some(" \t\n".into())), None);
        assert_eq!(normalize_reason(Some(" 冲突 ".into())), Some("冲突".into()));
    }
}
