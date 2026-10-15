// GUI 程序：release 下不挂控制台窗口；debug 保留以便看日志输出
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use eframe::egui;
#[cfg(windows)]
use selinga_capture::ScreenCapture;
#[cfg(windows)]
use selinga_ocr::Ocr;
use selinga_pipeline::PipelineOutput;
use selinga_translate::Translator;
use std::sync::Arc;
#[cfg(windows)]
use std::sync::Mutex;
use std::time::{Duration, Instant};

mod settings;
#[cfg(windows)]
mod tray;
use settings::Settings;
#[cfg(windows)]
use tray::{Tray, TrayCommand};

const DEBOUNCE: Duration = Duration::from_millis(700);
const POLL_INTERVAL: Duration = Duration::from_millis(1000);
const EXCLUDE_INTERVAL: Duration = Duration::from_secs(1);
const HANDLE_MARGIN: f32 = 10.0;
fn mask_color() -> egui::Color32 {
    egui::Color32::from_rgba_unmultiplied(16, 16, 22, 230)
}
const TEXT_COLOR: egui::Color32 = egui::Color32::from_rgb(0xF2, 0xF2, 0xF5);
const SOURCE_COLOR: egui::Color32 = egui::Color32::from_rgb(0x9A, 0x9A, 0xA8);
const LINE_PAD: f32 = 3.0;
const BILINGUAL_EXTRA: f32 = 0.62;
const TARGET_LANGS: [&str; 4] = ["zh-CN", "en-US", "ja-JP", "ko-KR"];
/// 框选面积小于该值视为误触，回退默认大小
const MIN_SELECT: f32 = 80.0;
/// 框选取消/误触时的默认方框尺寸
const DEFAULT_SIZE: [f32; 2] = [380.0, 240.0];

fn build_translator(
    target: &str,
    vlm: Option<&selinga_translate::vlm::VlmConfig>,
) -> Arc<dyn Translator> {
    let mut backends: Vec<Box<dyn Translator>> = vec![Box::new(
        selinga_translate::tencent::TransmartTranslator::new(target),
    )];
    if let Some(cfg) = vlm {
        backends.push(Box::new(selinga_translate::vlm::VlmTranslator::new(
            cfg.clone(),
        )));
    }
    backends.push(Box::new(selinga_translate::google::GoogleTranslator::new(
        target,
    )));
    backends.push(Box::new(
        selinga_translate::mymemory::MyMemoryTranslator::new("en-US", target),
    ));
    Arc::new(selinga_translate::CachedTranslator::new(
        selinga_translate::FallbackTranslator::new(backends),
        2048,
    ))
}

type ToolbarAction = Box<dyn Fn(&mut SelingaApp, &egui::Ui)>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ToolbarIcon {
    Mode,
    Copy,
    Lang,
    Refresh,
    Pause,
    Resume,
}

#[cfg(windows)]
#[derive(Debug, Clone)]
struct FrameCache {
    hash: u64,
    output: PipelineOutput,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DisplayMode {
    Translation,
    Bilingual,
}

impl DisplayMode {
    fn toggle(self) -> Self {
        match self {
            DisplayMode::Translation => DisplayMode::Bilingual,
            DisplayMode::Bilingual => DisplayMode::Translation,
        }
    }
}

fn main() -> eframe::Result<()> {
    // 无边框窗口直接按主屏尺寸创建，铺满屏幕供框选，不做运行时全屏切换
    let (screen_w, screen_h) = selinga_platform::primary_screen_points();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Selinga")
            .with_decorations(false)
            .with_transparent(true)
            .with_resizable(true)
            .with_active(true)
            .with_visible(false)
            .with_inner_size([screen_w, screen_h])
            .with_position([0.0, 0.0])
            .with_window_level(egui::WindowLevel::AlwaysOnTop),
        ..Default::default()
    };
    eframe::run_native(
        "Selinga",
        options,
        Box::new(|cc| {
            selinga_platform::exclude_self_from_capture();
            install_cjk_fonts(&cc.egui_ctx);
            Ok(Box::new(SelingaApp::new()))
        }),
    )
}

fn install_cjk_fonts(ctx: &egui::Context) {
    let candidates = [
        ("cjk-msyh", "C:\\Windows\\Fonts\\msyh.ttc"),
        ("cjk-msyh-ttf", "C:\\Windows\\Fonts\\msyh.ttf"),
        ("cjk-simhei", "C:\\Windows\\Fonts\\simhei.ttf"),
        ("cjk-simsun", "C:\\Windows\\Fonts\\simsun.ttc"),
    ];
    let mut fonts = egui::FontDefinitions::default();
    for (name, path) in candidates {
        match std::fs::read(path) {
            Ok(data) => {
                fonts.font_data.insert(
                    name.to_owned(),
                    std::sync::Arc::new(egui::FontData::from_owned(data)),
                );
                for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
                    fonts
                        .families
                        .entry(family)
                        .or_default()
                        .push(name.to_owned());
                }
                break;
            }
            Err(_) => continue,
        }
    }
    ctx.set_fonts(fonts);
}

/// 应用阶段：Select=全屏冻结窗框选；Box=方框视口稳态（root 永久隐藏）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Select,
    Box,
}

struct SelingaApp {
    paused: bool,
    busy: bool,
    dragging: bool,
    resizing: Option<egui::ResizeDirection>,
    last_rect: Option<egui::Rect>,
    stable_since: Option<Instant>,
    last_trigger: Option<Instant>,
    last_exclude: Instant,
    display_mode: DisplayMode,
    copied_at: Option<Instant>,
    result: Option<Result<PipelineOutput, String>>,
    translate_error: Option<String>,
    target_lang: usize,
    translator: Arc<std::sync::RwLock<Arc<dyn Translator>>>,
    vlm: settings::VlmSettings,
    hidden: bool,
    /// 双视口阶段：root 全屏冻结窗 → 框选完成后以选区几何 spawn BOX 视口，root 永久隐藏
    phase: Phase,
    /// 框选得到的方框几何（逻辑点，屏幕坐标）
    sel_rect: Option<egui::Rect>,
    marquee: Option<(egui::Pos2, egui::Pos2)>,
    /// root 视口隐藏待显示（启动时）
    show_pending: bool,
    pending_frames: u32,
    /// BOX 视口隐藏待显示（创建后画 2 帧再显示，随后才隐藏 root —— Snipaste 交接顺序）
    box_show_pending: bool,
    box_pending_frames: u32,
    /// 框选背景：启动时冻结的全屏截图（GL 表面不支持透明时的可靠方案）
    #[cfg(windows)]
    select_bg: Option<egui::TextureHandle>,
    #[cfg(windows)]
    tray: Option<Tray>,
    #[cfg(windows)]
    cache: Arc<Mutex<Option<FrameCache>>>,
    #[cfg(windows)]
    worker: Option<Worker>,
}

#[cfg(windows)]
struct Worker {
    tx: std::sync::mpsc::Sender<WorkerMsg>,
    rx: std::sync::mpsc::Receiver<WorkerMsg>,
}

#[cfg(windows)]
enum WorkerMsg {
    Done {
        output: PipelineOutput,
        translate_error: Option<String>,
    },
    Failed(String),
}

impl SelingaApp {
    fn new() -> Self {
        #[cfg(windows)]
        let (tx, rx) = std::sync::mpsc::channel();
        let saved = Settings::load();
        let target_lang = TARGET_LANGS
            .iter()
            .position(|l| *l == saved.target_lang)
            .unwrap_or(0);
        let display_mode = match saved.display_mode.as_str() {
            "bilingual" => DisplayMode::Bilingual,
            _ => DisplayMode::Translation,
        };
        Self {
            paused: saved.paused,
            busy: false,
            dragging: false,
            resizing: None,
            last_rect: None,
            stable_since: None,
            last_trigger: None,
            last_exclude: Instant::now(),
            display_mode,
            copied_at: None,
            result: None,
            translate_error: None,
            target_lang,
            hidden: false,
            phase: Phase::Select,
            sel_rect: None,
            marquee: None,
            show_pending: true,
            pending_frames: 0,
            box_show_pending: false,
            box_pending_frames: 0,
            #[cfg(windows)]
            select_bg: None,
            vlm: saved.vlm.clone(),
            translator: Arc::new(std::sync::RwLock::new(build_translator(
                TARGET_LANGS[target_lang],
                saved.vlm.config().as_ref(),
            ))),
            #[cfg(windows)]
            tray: Tray::new().ok(),
            #[cfg(windows)]
            cache: Arc::new(Mutex::new(None)),
            #[cfg(windows)]
            worker: Some(Worker { tx, rx }),
        }
    }

    fn persist_settings(&self) {
        Settings {
            target_lang: TARGET_LANGS[self.target_lang].to_owned(),
            display_mode: match self.display_mode {
                DisplayMode::Translation => "translation".to_owned(),
                DisplayMode::Bilingual => "bilingual".to_owned(),
            },
            paused: self.paused,
            vlm: self.vlm.clone(),
        }
        .save();
    }
}

impl eframe::App for SelingaApp {
    // eframe 默认清屏色自带 70% 暗灰（12,12,12,180），叠上框选遮罩后屏幕几乎不可见；
    // Select 阶段改为全透明（遮罩自带薄纱），Box 阶段方框窗口保持默认暗底
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        if self.phase == Phase::Select {
            egui::Color32::TRANSPARENT.to_normalized_gamma_f32()
        } else {
            egui::Color32::from_rgba_unmultiplied(12, 12, 12, 180).to_normalized_gamma_f32()
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();

        match self.phase {
            Phase::Select => {
                #[cfg(windows)]
                if self.select_bg.is_none() {
                    self.load_select_background(&ctx);
                }
                self.draw_select_overlay(ui, &ctx);
                self.handle_pending_show(&ctx);
                ctx.request_repaint_after(Duration::from_millis(50));
            }
            Phase::Box => {
                // root 已隐藏，唯一职责是每帧驱动 BOX 视口
                self.draw_box_viewport(&ctx);
                ctx.request_repaint_after(Duration::from_millis(100));
            }
        }
    }
}

impl SelingaApp {
    /// BOX 视口：以选区几何创建（创建即最终尺寸，不走 resize），先隐藏画 2 帧，
    /// 显示之后才隐藏 root —— 与 Snipaste 相同的交接顺序，全程屏幕有内容
    fn draw_box_viewport(&mut self, root_ctx: &egui::Context) {
        let Some(sel) = self.sel_rect else {
            return;
        };
        let builder = egui::ViewportBuilder::default()
            .with_title("Selinga Box")
            .with_decorations(false)
            .with_transparent(true)
            .with_resizable(true)
            .with_active(true)
            .with_visible(false)
            .with_position(sel.min)
            .with_inner_size(sel.size())
            .with_window_level(egui::WindowLevel::AlwaysOnTop);
        // Context 内部是同一个 Arc；在 BOX 回调内操作 root 必须显式指定视口 id
        let root_ctx = root_ctx.clone();
        root_ctx.show_viewport_immediate(
            egui::ViewportId(egui::Id::new("selinga_box")),
            builder,
            |ui, _class| {
                let box_ctx = ui.ctx().clone();

                // 隐藏期间画 2 帧内容后再显示；显示完成后才收走 root（无缝交接）
                if self.box_show_pending {
                    self.box_pending_frames += 1;
                    if self.box_pending_frames >= 2 {
                        self.box_show_pending = false;
                        box_ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                        // root 移出屏幕并缩到 1x1，释放显存。
                        // 注意：不能 Visible(false) —— eframe 对隐藏视口不调用 App::ui
                        // （epi_integration.rs: if is_visible），immediate 子视口会被冻结。
                        // 冻结截图与真实屏幕内容一致，root 移出瞬间视觉无缝。
                        root_ctx.send_viewport_cmd_to(
                            egui::ViewportId::ROOT,
                            egui::ViewportCommand::OuterPosition(egui::pos2(-16000.0, -16000.0)),
                        );
                        root_ctx.send_viewport_cmd_to(
                            egui::ViewportId::ROOT,
                            egui::ViewportCommand::InnerSize(egui::vec2(1.0, 1.0)),
                        );
                    }
                }

                if self.last_exclude.elapsed() > EXCLUDE_INTERVAL {
                    self.last_exclude = Instant::now();
                    selinga_platform::exclude_self_from_capture();
                }

                let outer = box_ctx.input(|i| i.viewport().outer_rect);
                let ppp = box_ctx.native_pixels_per_point().unwrap_or(1.0);

                if outer != self.last_rect {
                    self.last_rect = outer;
                    self.stable_since = Some(Instant::now());
                }
                let stable_for = self
                    .stable_since
                    .map(|t| t.elapsed())
                    .unwrap_or(Duration::ZERO);

                self.drain_worker(&box_ctx);
                self.drain_tray(&box_ctx);
                self.draw_box(ui, &box_ctx);

                let pointer_idle = !box_ctx.input(|i| i.pointer.any_down());
                let debounce_due = self.stable_since.is_some() && stable_for >= DEBOUNCE;
                let poll_due = self.stable_since.is_none()
                    && self
                        .last_trigger
                        .is_some_and(|t| t.elapsed() >= POLL_INTERVAL);
                if (debounce_due || poll_due)
                    && pointer_idle
                    && !self.paused
                    && !self.hidden
                    && !self.busy
                    && !self.dragging
                    && self.resizing.is_none()
                    && let Some(rect) = outer
                {
                    self.trigger(&box_ctx, rect, ppp, false);
                }

                self.draw_overlay_results(ui, ppp);
            },
        );
    }

    /// 延迟显示：等几何调整生效且至少画过一帧内容后再显示窗口（命令在 present 后应用，
    /// 首个可见帧即完整内容），消除表面创建/重建时的黑屏闪帧
    fn handle_pending_show(&mut self, ctx: &egui::Context) {
        if !self.show_pending {
            return;
        }
        self.pending_frames += 1;
        if self.pending_frames >= 2 {
            self.show_pending = false;
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
        }
    }

    /// 全屏抓一张作为框选背景。GL 表面不支持透明时（纯黑遮罩的根因），
    /// 用"冻结屏幕 + 半透明暗纱"达到同样的视觉蒙版效果，且不依赖驱动。
    #[cfg(windows)]
    fn load_select_background(&mut self, ctx: &egui::Context) {
        let (w, h) = selinga_platform::primary_screen_px();
        let capture = match selinga_capture::win32::GdiCapture::new() {
            Ok(c) => c,
            Err(_) => return,
        };
        let Ok(frame) = capture.capture_region(selinga_capture::Region { x: 0, y: 0, w, h }) else {
            return;
        };
        let image = egui::ColorImage::from_rgba_unmultiplied(
            [frame.width as usize, frame.height as usize],
            &frame.to_rgba(),
        );
        self.select_bg = Some(ctx.load_texture("select_bg", image, egui::TextureOptions::LINEAR));
    }

    fn draw_select_overlay(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let rect = ui.max_rect();
        let painter = ui.painter().clone();

        #[cfg(windows)]
        if let Some(tex) = &self.select_bg {
            painter.image(
                tex.id(),
                rect,
                egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
        }
        painter.rect_filled(rect, 0.0, egui::Color32::from_black_alpha(70));

        let resp = ui.allocate_rect(rect, egui::Sense::click_and_drag());
        let pointer = ctx.input(|i| i.pointer.clone());

        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) || resp.secondary_clicked() {
            self.finish_select(ctx, None);
            return;
        }

        if pointer.any_down() {
            ctx.set_cursor_icon(egui::CursorIcon::Crosshair);
        }

        if pointer.primary_pressed() {
            self.marquee = pointer.interact_pos().map(|p| (p, p));
        } else if pointer.primary_down()
            && let (Some((_, cur)), Some(p)) = (&mut self.marquee, pointer.interact_pos())
        {
            *cur = p;
        }

        if let Some((a, b)) = self.marquee {
            let sel = egui::Rect::from_two_pos(a, b);
            painter.rect_filled(
                sel,
                0.0,
                egui::Color32::from_rgba_unmultiplied(0x53, 0x4A, 0xB7, 45),
            );
            painter.rect_stroke(
                sel,
                0.0,
                egui::Stroke::new(1.5_f32, egui::Color32::from_rgb(0x8A, 0x81, 0xE8)),
                egui::StrokeKind::Inside,
            );
            let label = format!("{:.0} x {:.0}", sel.width(), sel.height());
            let label_pos = egui::pos2(
                sel.left().clamp(rect.left() + 40.0, rect.right() - 40.0),
                (sel.top() - 12.0).max(rect.top() + 10.0),
            );
            painter.text(
                label_pos,
                egui::Align2::CENTER_BOTTOM,
                label,
                egui::FontId::proportional(13.0),
                TEXT_COLOR,
            );
        }

        if pointer.primary_released()
            && let Some((a, b)) = self.marquee.take()
        {
            self.finish_select(ctx, Some(egui::Rect::from_two_pos(a, b)));
        }
    }

    /// 结束框选：记录选区几何并切换到 Box 阶段（下一帧 spawn BOX 视口，创建即最终尺寸）；
    /// `None` 或面积过小则取默认 380x240（屏幕居中）。root 全屏窗在 BOX 显示后才隐藏。
    fn finish_select(&mut self, _ctx: &egui::Context, sel: Option<egui::Rect>) {
        self.marquee = None;
        let valid = sel.is_some_and(|r| r.width() >= MIN_SELECT && r.height() >= MIN_SELECT);
        self.sel_rect = Some(if valid {
            sel.unwrap_or(egui::Rect::ZERO)
        } else {
            let (sw, sh) = selinga_platform::primary_screen_points();
            egui::Rect::from_min_size(
                egui::pos2((sw - DEFAULT_SIZE[0]) * 0.5, (sh - DEFAULT_SIZE[1]) * 0.5),
                egui::Vec2::from(DEFAULT_SIZE),
            )
        });
        self.phase = Phase::Box;
        self.box_show_pending = true;
        self.box_pending_frames = 0;
        self.last_rect = None;
        self.stable_since = None;
        self.last_trigger = None;
        self.result = None;
        self.translate_error = None;
        self.busy = false;
        #[cfg(windows)]
        {
            self.select_bg = None;
        }
    }

    fn draw_box(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let rect = ui.max_rect();
        ui.painter().rect_stroke(
            rect,
            6.0,
            egui::Stroke::new(2.0_f32, egui::Color32::from_rgb(0x53, 0x4A, 0xB7)),
            egui::StrokeKind::Inside,
        );

        let resp = ui.allocate_rect(rect, egui::Sense::click_and_drag());
        let pointer = ctx.input(|i| i.pointer.clone());
        let pos = pointer.latest_pos();
        let resize_dir = resize_direction(rect, pos);
        let mut toolbar_hovered = false;
        if ui.rect_contains_pointer(rect) {
            toolbar_hovered = self.draw_toolbar(ui, rect);
        }
        self.draw_status_badge(ui, rect);

        if !pointer.any_down() {
            if let Some(dir) = resize_dir {
                if !toolbar_hovered {
                    ctx.set_cursor_icon(resize_cursor(dir));
                }
            } else if resp.hovered() {
                ctx.set_cursor_icon(if toolbar_hovered {
                    egui::CursorIcon::PointingHand
                } else {
                    egui::CursorIcon::Grab
                });
            }
        }

        if pointer.primary_pressed() && !toolbar_hovered {
            self.resizing = resize_dir;
            if let Some(dir) = resize_dir {
                ctx.send_viewport_cmd(egui::ViewportCommand::BeginResize(dir));
            }
        }
        if pointer.primary_released() {
            self.resizing = None;
        }

        if pointer.primary_pressed() && resize_dir.is_none() && resp.hovered() && !toolbar_hovered {
            self.dragging = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
        }
        if pointer.primary_released() {
            self.dragging = false;
        }
    }

    #[allow(clippy::too_many_lines)]
    fn draw_toolbar(&mut self, ui: &mut egui::Ui, box_rect: egui::Rect) -> bool {
        use ToolbarIcon::{Copy, Lang, Mode, Pause, Refresh, Resume};

        let lang_code = TARGET_LANGS[self.target_lang]
            .split('-')
            .next()
            .unwrap_or("EN")
            .to_uppercase();
        let mode_tip = match self.display_mode {
            DisplayMode::Translation => "switch to bilingual",
            DisplayMode::Bilingual => "switch to translation only",
        };
        let buttons: Vec<(ToolbarIcon, &str, ToolbarAction)> = vec![
            (
                Mode,
                mode_tip,
                Box::new(|app: &mut Self, ui: &egui::Ui| {
                    app.display_mode = app.display_mode.toggle();
                    app.persist_settings();
                    ui.ctx().request_repaint();
                }),
            ),
            (
                Copy,
                "copy result",
                Box::new(|app: &mut Self, ui: &egui::Ui| {
                    if let Some(Ok(output)) = &app.result {
                        let text = match app.display_mode {
                            DisplayMode::Translation => output.translated_text(),
                            DisplayMode::Bilingual => output.bilingual_text(),
                        };
                        ui.ctx().copy_text(text);
                        app.copied_at = Some(Instant::now());
                    }
                }),
            ),
            (
                Lang,
                "target language",
                Box::new(|app: &mut Self, _ui: &egui::Ui| {
                    app.target_lang = (app.target_lang + 1) % TARGET_LANGS.len();
                    let target = TARGET_LANGS[app.target_lang];
                    if let Ok(mut g) = app.translator.write() {
                        *g = build_translator(target, app.vlm.config().as_ref());
                    }
                    app.persist_settings();
                }),
            ),
            (
                Refresh,
                "re-recognize",
                Box::new(|app: &mut Self, ui: &egui::Ui| {
                    if !app.busy {
                        let ppp = ui.ctx().native_pixels_per_point().unwrap_or(1.0);
                        app.trigger(ui.ctx(), ui.max_rect(), ppp, true);
                    }
                }),
            ),
            (
                if self.paused { Resume } else { Pause },
                if self.paused { "resume" } else { "pause" },
                Box::new(|app: &mut Self, _ui: &egui::Ui| {
                    app.paused = !app.paused;
                    app.persist_settings();
                }),
            ),
        ];

        const ICON: f32 = 22.0;
        const GAP: f32 = 2.0;
        let n = buttons.len() as f32;
        let total_h = n * ICON + (n - 1.0) * GAP;
        let y0 = if total_h + 8.0 >= box_rect.height() {
            box_rect.top() + 4.0
        } else {
            box_rect.center().y - total_h * 0.5
        };
        let x_right = box_rect.right() - 4.0;
        let column_rect = egui::Rect::from_min_size(
            egui::pos2(x_right - ICON - 5.0, y0 - 5.0),
            egui::vec2(ICON + 10.0, total_h + 10.0),
        );
        let painter = ui.painter().clone();
        painter.rect_filled(column_rect, 8.0, egui::Color32::from_black_alpha(110));

        let pointer_pos = ui.input(|i| i.pointer.latest_pos());
        let mut any_hovered = false;
        for (idx, (icon, tip, action)) in buttons.iter().enumerate() {
            let btn_rect = egui::Rect::from_min_size(
                egui::pos2(x_right - ICON, y0 + idx as f32 * (ICON + GAP)),
                egui::vec2(ICON, ICON),
            );
            let hovered = pointer_pos.is_some_and(|p| btn_rect.contains(p));
            any_hovered |= hovered;
            let color = if hovered {
                TEXT_COLOR
            } else {
                TEXT_COLOR.gamma_multiply(0.72)
            };
            if hovered {
                painter.rect_filled(btn_rect, 6.0, egui::Color32::from_white_alpha(26));
            }
            let resp = ui.allocate_rect(btn_rect, egui::Sense::click());
            if resp.clicked() {
                action(self, ui);
            }
            if resp.hovered() {
                resp.on_hover_text(*tip);
            }
            let stroke = egui::Stroke::new(1.6_f32, color);
            match icon {
                ToolbarIcon::Mode => {
                    let (l, r, cy) = (
                        btn_rect.left() + 3.0,
                        btn_rect.right() - 3.0,
                        btn_rect.center().y,
                    );
                    painter.arrow(egui::pos2(l, cy - 3.5), egui::vec2(r - l, 0.0), stroke);
                    painter.arrow(egui::pos2(r, cy + 3.5), egui::vec2(l - r, 0.0), stroke);
                }
                ToolbarIcon::Copy => {
                    let back = egui::Rect::from_min_size(
                        egui::pos2(btn_rect.left() + 4.0, btn_rect.top() + 3.0),
                        egui::vec2(9.0, 9.0),
                    );
                    let front = egui::Rect::from_min_size(
                        egui::pos2(btn_rect.left() + 8.0, btn_rect.top() + 9.0),
                        egui::vec2(9.0, 9.0),
                    );
                    painter.rect_stroke(back, 2.0, stroke, egui::StrokeKind::Inside);
                    painter.rect_filled(front, 2.0, egui::Color32::from_rgb(0x22, 0x22, 0x2A));
                    painter.rect_stroke(front, 2.0, stroke, egui::StrokeKind::Inside);
                }
                ToolbarIcon::Lang => {
                    painter.text(
                        btn_rect.center(),
                        egui::Align2::CENTER_CENTER,
                        lang_code.as_str(),
                        egui::FontId::proportional(9.5),
                        color,
                    );
                }
                ToolbarIcon::Refresh => {
                    let c = btn_rect.center();
                    painter.circle_stroke(c, 5.5, stroke);
                    let a = 45_f32.to_radians();
                    let (dx, dy) = (a.cos(), a.sin());
                    let edge = egui::pos2(c.x + 5.5 * dx, c.y + 5.5 * dy);
                    let tip = egui::pos2(edge.x - 3.0 * dy, edge.y + 3.0 * dx);
                    let base = egui::pos2(edge.x + 3.0 * dy, edge.y - 3.0 * dx);
                    let inner = egui::pos2(c.x + 1.5 * dx, c.y + 1.5 * dy);
                    painter.add(egui::Shape::convex_polygon(
                        vec![tip, base, inner],
                        color,
                        egui::Stroke::NONE,
                    ));
                }
                ToolbarIcon::Pause => {
                    let c = btn_rect.center();
                    for x_off in [-4.5_f32, 1.5] {
                        painter.rect_filled(
                            egui::Rect::from_min_size(
                                egui::pos2(c.x + x_off, c.y - 4.5),
                                egui::vec2(3.0, 9.0),
                            ),
                            1.0,
                            color,
                        );
                    }
                }
                ToolbarIcon::Resume => {
                    let c = btn_rect.center();
                    painter.add(egui::Shape::convex_polygon(
                        vec![
                            egui::pos2(c.x - 3.5, c.y - 5.0),
                            egui::pos2(c.x - 3.5, c.y + 5.0),
                            egui::pos2(c.x + 5.0, c.y),
                        ],
                        color,
                        egui::Stroke::NONE,
                    ));
                }
            }
        }
        if self
            .copied_at
            .is_some_and(|t| t.elapsed() < Duration::from_secs(1))
        {
            painter.text(
                egui::pos2(column_rect.left() - 6.0, column_rect.center().y),
                egui::Align2::RIGHT_CENTER,
                "copied!",
                egui::FontId::proportional(12.0),
                TEXT_COLOR,
            );
        }
        any_hovered
    }

    fn draw_status_badge(&mut self, ui: &mut egui::Ui, box_rect: egui::Rect) {
        const WARN_COLOR: egui::Color32 = egui::Color32::from_rgb(0xE2, 0xA1, 0x4B);
        let (text, color): (String, egui::Color32) = if self.busy {
            ("recognizing...".to_owned(), SOURCE_COLOR)
        } else if self.paused {
            ("paused".to_owned(), SOURCE_COLOR)
        } else if let Some(err) = &self.translate_error {
            let short: String = format!("translate: {}", err.chars().take(56).collect::<String>());
            (short, WARN_COLOR)
        } else if let Some(Err(err)) = &self.result {
            let short: String = err.chars().take(64).collect();
            (short, egui::Color32::from_rgb(0xE2, 0x4B, 0x4A))
        } else {
            return;
        };
        ui.painter().text(
            egui::pos2(box_rect.left() + 8.0, box_rect.top() + 8.0),
            egui::Align2::LEFT_TOP,
            text,
            egui::FontId::proportional(12.0),
            color,
        );
    }

    fn draw_overlay_results(&mut self, ui: &mut egui::Ui, ppp: f32) {
        let Some(Ok(output)) = &self.result else {
            return;
        };
        if output.lines.is_empty() {
            return;
        }
        let mode = self.display_mode;
        let painter = ui.painter();
        let ctx = ui.ctx().clone();
        for pair in &output.lines {
            let Some(bbox) = pair.rect else {
                continue;
            };
            let line = egui::Rect::from_min_size(
                egui::pos2(bbox.x / ppp, bbox.y / ppp),
                egui::vec2(bbox.w / ppp, bbox.h / ppp),
            );
            let mask = line.expand(LINE_PAD);
            let avail_w = mask.width() - 2.0 * LINE_PAD;
            let font_size = match mode {
                DisplayMode::Translation => {
                    fit_font_size(&ctx, &pair.translated, avail_w, line.height())
                }
                DisplayMode::Bilingual => {
                    fit_font_size(&ctx, &pair.translated, avail_w, line.height() * 0.72)
                }
            };
            let mask_height = match mode {
                DisplayMode::Translation => mask.height(),
                DisplayMode::Bilingual => mask.height() * (1.0 + BILINGUAL_EXTRA),
            };
            let mask = egui::Rect::from_min_size(mask.min, egui::vec2(mask.width(), mask_height));
            painter.rect_filled(mask, 3.0, mask_color());
            let max_chars = (mask.width() / (font_size * 0.6)).max(4.0) as usize;
            match mode {
                DisplayMode::Translation => {
                    let translated = ellipsize(&pair.translated, max_chars);
                    painter.text(
                        mask.min + egui::vec2(LINE_PAD, LINE_PAD),
                        egui::Align2::LEFT_TOP,
                        &translated,
                        egui::FontId::proportional(font_size),
                        TEXT_COLOR,
                    );
                }
                DisplayMode::Bilingual => {
                    let source = ellipsize(&pair.source, max_chars);
                    painter.text(
                        mask.min + egui::vec2(LINE_PAD, LINE_PAD),
                        egui::Align2::LEFT_TOP,
                        &source,
                        egui::FontId::proportional(font_size * 0.62),
                        SOURCE_COLOR,
                    );
                    let translated = ellipsize(&pair.translated, max_chars);
                    painter.text(
                        mask.min + egui::vec2(LINE_PAD, LINE_PAD + mask.height() * 0.45),
                        egui::Align2::LEFT_TOP,
                        &translated,
                        egui::FontId::proportional(font_size),
                        TEXT_COLOR,
                    );
                }
            }
        }
    }

    #[cfg(windows)]
    fn trigger(&mut self, _ctx: &egui::Context, rect: egui::Rect, ppp: f32, force: bool) {
        self.busy = true;
        self.last_trigger = Some(Instant::now());
        self.stable_since = None;

        let Some(worker) = &self.worker else {
            self.busy = false;
            return;
        };
        let tx = worker.tx.clone();
        let cache = Arc::clone(&self.cache);
        let translator = Arc::clone(&self.translator);
        let region = selinga_capture::Region {
            x: (rect.min.x * ppp).round() as i32,
            y: (rect.min.y * ppp).round() as i32,
            w: (rect.width() * ppp).round() as u32,
            h: (rect.height() * ppp).round() as u32,
        };

        std::thread::spawn(move || {
            let capture = match selinga_capture::win32::GdiCapture::new() {
                Ok(c) => c,
                Err(e) => return send(&tx, WorkerMsg::Failed(e.to_string())),
            };
            let frame = match capture.capture_region(region) {
                Ok(f) => f,
                Err(e) => return send(&tx, WorkerMsg::Failed(format!("capture: {e}"))),
            };
            let hash = frame_hash(&frame);

            if !force
                && let Some(cached) = cache.lock().ok().and_then(|c| c.clone())
                && cached.hash == hash
            {
                return send(
                    &tx,
                    WorkerMsg::Done {
                        output: cached.output,
                        translate_error: None,
                    },
                );
            }

            let ocr = match selinga_ocr::win32::WinOcr::new(None) {
                Ok(o) => o,
                Err(e) => return send(&tx, WorkerMsg::Failed(format!("ocr engine: {e}"))),
            };
            let lines = match ocr.recognize(&frame) {
                Ok(l) => l,
                Err(e) => return send(&tx, WorkerMsg::Failed(format!("ocr: {e}"))),
            };
            let chain = match translator.read() {
                Ok(g) => g.clone(),
                Err(_) => return send(&tx, WorkerMsg::Failed("translator lock poisoned".into())),
            };
            let sources: Vec<String> = lines.iter().map(|l| l.text.clone()).collect();
            let (translated, translate_error) = match chain.translate_lines(&sources) {
                Ok(t) => (t, None),
                Err(e) => (sources.clone(), Some(e.to_string())),
            };
            let output = selinga_pipeline::PipelineOutput {
                lines: lines
                    .iter()
                    .zip(translated)
                    .map(|(l, t)| selinga_pipeline::LinePair {
                        source: l.text.clone(),
                        translated: t,
                        rect: l.rect,
                    })
                    .collect(),
            };
            if let Ok(mut c) = cache.lock() {
                *c = Some(FrameCache {
                    hash,
                    output: output.clone(),
                });
            }
            send(
                &tx,
                WorkerMsg::Done {
                    output,
                    translate_error,
                },
            );
        });
    }

    #[cfg(not(windows))]
    fn trigger(&mut self, _ctx: &egui::Context, _rect: egui::Rect, _ppp: f32, _force: bool) {
        self.busy = true;
        self.stable_since = None;
        self.last_trigger = Some(Instant::now());
        self.result = Some(Err(
            "pipeline not implemented on this platform yet".to_owned()
        ));
        self.busy = false;
    }

    #[cfg(windows)]
    fn drain_tray(&mut self, ctx: &egui::Context) {
        let commands = match &self.tray {
            Some(tray) => tray.drain(),
            None => return,
        };
        for cmd in commands {
            match cmd {
                TrayCommand::ToggleHidden => {
                    self.hidden = !self.hidden;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(!self.hidden));
                    if let Some(tray) = &self.tray {
                        tray.set_hidden_label(self.hidden);
                    }
                }
                TrayCommand::TogglePause => {
                    self.paused = !self.paused;
                    self.persist_settings();
                    if let Some(tray) = &self.tray {
                        tray.set_paused_label(self.paused);
                    }
                }
                TrayCommand::Refresh => {
                    if !self.busy
                        && let Some(rect) = self.last_rect
                    {
                        let ppp = ctx.native_pixels_per_point().unwrap_or(1.0);
                        self.trigger(ctx, rect, ppp, true);
                    }
                }
                TrayCommand::Quit => std::process::exit(0),
            }
        }
    }

    #[cfg(not(windows))]
    fn drain_tray(&mut self, _ctx: &egui::Context) {}

    #[cfg(windows)]
    fn drain_worker(&mut self, ctx: &egui::Context) {
        let Some(worker) = &self.worker else {
            return;
        };
        let mut done = false;
        while let Ok(msg) = worker.rx.try_recv() {
            match msg {
                WorkerMsg::Done {
                    output,
                    translate_error,
                } => {
                    self.result = Some(Ok(output));
                    self.translate_error = translate_error;
                }
                WorkerMsg::Failed(err) => {
                    self.result = Some(Err(err));
                    self.translate_error = None;
                }
            }
            done = true;
        }
        if done {
            self.busy = false;
            ctx.request_repaint();
        }
    }

    #[cfg(not(windows))]
    fn drain_worker(&mut self, _ctx: &egui::Context) {}
}

#[cfg(windows)]
fn send(tx: &std::sync::mpsc::Sender<WorkerMsg>, msg: WorkerMsg) {
    let _ = tx.send(msg);
}

#[cfg(windows)]
fn frame_hash(frame: &selinga_capture::Frame) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let step = (frame.bgra.len() / 8192).max(1);
    for byte in frame.bgra.iter().step_by(step) {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100_0000_01b3);
    }
    hash
}

fn fit_font_size(ctx: &egui::Context, text: &str, max_w: f32, max_h: f32) -> f32 {
    let fits = |size: f32| {
        let galley = ctx.fonts_mut(|f| {
            f.layout_no_wrap(
                text.to_owned(),
                egui::FontId::proportional(size),
                egui::Color32::WHITE,
            )
        });
        let size = galley.size();
        size.x <= max_w && size.y <= max_h
    };
    let (mut lo, mut hi) = (8.0_f32, 32.0_f32.min(max_h.max(8.0)));
    if !fits(lo) {
        return lo;
    }
    if fits(hi) {
        return hi;
    }
    for _ in 0..6 {
        let mid = (lo + hi) / 2.0;
        if fits(mid) {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    lo
}

fn ellipsize(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_owned();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

fn resize_direction(rect: egui::Rect, pos: Option<egui::Pos2>) -> Option<egui::ResizeDirection> {
    let pos = pos?;
    if !rect.expand(2.0).contains(pos) {
        return None;
    }
    let near_left = pos.x - rect.left() < HANDLE_MARGIN;
    let near_right = rect.right() - pos.x < HANDLE_MARGIN;
    let near_top = pos.y - rect.top() < HANDLE_MARGIN;
    let near_bottom = rect.bottom() - pos.y < HANDLE_MARGIN;
    let dir = match (near_left, near_right, near_top, near_bottom) {
        (true, false, false, false) => egui::ResizeDirection::West,
        (false, true, false, false) => egui::ResizeDirection::East,
        (false, false, true, false) => egui::ResizeDirection::North,
        (false, false, false, true) => egui::ResizeDirection::South,
        (true, false, true, false) => egui::ResizeDirection::NorthWest,
        (false, true, true, false) => egui::ResizeDirection::NorthEast,
        (true, false, false, true) => egui::ResizeDirection::SouthWest,
        (false, true, false, true) => egui::ResizeDirection::SouthEast,
        _ => return None,
    };
    if near_left || near_right || near_top || near_bottom {
        Some(dir)
    } else {
        None
    }
}

fn resize_cursor(dir: egui::ResizeDirection) -> egui::CursorIcon {
    match dir {
        egui::ResizeDirection::East | egui::ResizeDirection::West => {
            egui::CursorIcon::ResizeHorizontal
        }
        egui::ResizeDirection::North | egui::ResizeDirection::South => {
            egui::CursorIcon::ResizeVertical
        }
        egui::ResizeDirection::NorthWest | egui::ResizeDirection::SouthEast => {
            egui::CursorIcon::ResizeNwSe
        }
        egui::ResizeDirection::NorthEast | egui::ResizeDirection::SouthWest => {
            egui::CursorIcon::ResizeNeSw
        }
    }
}
