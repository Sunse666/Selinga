use eframe::egui;
#[cfg(windows)]
use selinga_capture::ScreenCapture;
#[cfg(windows)]
use selinga_ocr::Ocr;
use selinga_pipeline::PipelineOutput;
use selinga_translate::Translator;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

mod settings;
use settings::Settings;

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

fn build_translator(target: &str) -> Arc<dyn Translator> {
    Arc::new(selinga_translate::CachedTranslator::new(
        selinga_translate::FallbackTranslator::new(vec![
            Box::new(selinga_translate::tencent::TransmartTranslator::new(target)),
            Box::new(selinga_translate::google::GoogleTranslator::new(target)),
            Box::new(selinga_translate::mymemory::MyMemoryTranslator::new(
                "en-US", target,
            )),
        ]),
        2048,
    ))
}

type ToolbarAction = Box<dyn Fn(&mut SelingaApp, &egui::Ui)>;

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
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Selinga")
            .with_decorations(false)
            .with_transparent(true)
            .with_resizable(true)
            .with_active(true)
            .with_inner_size([380.0, 240.0])
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
            translator: Arc::new(std::sync::RwLock::new(build_translator(
                TARGET_LANGS[target_lang],
            ))),
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
        }
        .save();
    }
}

impl eframe::App for SelingaApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();

        if self.last_exclude.elapsed() > EXCLUDE_INTERVAL {
            self.last_exclude = Instant::now();
            selinga_platform::exclude_self_from_capture();
        }

        let outer = ctx.input(|i| i.viewport().outer_rect);
        let ppp = ctx.native_pixels_per_point().unwrap_or(1.0);

        if outer != self.last_rect {
            self.last_rect = outer;
            self.stable_since = Some(Instant::now());
        }
        let stable_for = self
            .stable_since
            .map(|t| t.elapsed())
            .unwrap_or(Duration::ZERO);

        self.drain_worker(&ctx);
        self.draw_box(ui, &ctx);

        let pointer_idle = !ctx.input(|i| i.pointer.any_down());
        let debounce_due = self.stable_since.is_some() && stable_for >= DEBOUNCE;
        let poll_due = self.stable_since.is_none()
            && self
                .last_trigger
                .is_some_and(|t| t.elapsed() >= POLL_INTERVAL);
        if (debounce_due || poll_due)
            && pointer_idle
            && !self.paused
            && !self.busy
            && !self.dragging
            && self.resizing.is_none()
            && let Some(rect) = outer
        {
            self.trigger(&ctx, rect, ppp, false);
        }

        self.draw_overlay_results(ui, ppp);

        ctx.request_repaint_after(Duration::from_millis(100));
    }
}

impl SelingaApp {
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

    fn draw_toolbar(&mut self, ui: &mut egui::Ui, box_rect: egui::Rect) -> bool {
        let mode_label = match self.display_mode {
            DisplayMode::Translation => "mode: translation".to_owned(),
            DisplayMode::Bilingual => "mode: bilingual".to_owned(),
        };
        let lang = TARGET_LANGS[self.target_lang];
        let buttons: Vec<(f32, String, ToolbarAction)> = vec![
            (
                118.0,
                mode_label,
                Box::new(|app: &mut Self, ui: &egui::Ui| {
                    app.display_mode = app.display_mode.toggle();
                    app.persist_settings();
                    ui.ctx().request_repaint();
                }),
            ),
            (
                46.0,
                "copy".to_owned(),
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
                56.0,
                format!("to: {}", lang.split('-').next().unwrap_or(lang)),
                Box::new(|app: &mut Self, _ui: &egui::Ui| {
                    app.target_lang = (app.target_lang + 1) % TARGET_LANGS.len();
                    let target = TARGET_LANGS[app.target_lang];
                    if let Ok(mut g) = app.translator.write() {
                        *g = build_translator(target);
                    }
                    app.persist_settings();
                }),
            ),
            (
                58.0,
                "refresh".to_owned(),
                Box::new(|app: &mut Self, ui: &egui::Ui| {
                    if !app.busy {
                        let ppp = ui.ctx().native_pixels_per_point().unwrap_or(1.0);
                        app.trigger(ui.ctx(), ui.max_rect(), ppp, true);
                    }
                }),
            ),
            (
                62.0,
                if self.paused { "resume" } else { "pause" }.to_owned(),
                Box::new(|app: &mut Self, _ui: &egui::Ui| {
                    app.paused = !app.paused;
                    app.persist_settings();
                }),
            ),
        ];
        let mut x = box_rect.right() - 6.0;
        let mut any_hovered = false;
        for (w, label, action) in buttons.iter().rev() {
            x -= w;
            let btn_rect = egui::Rect::from_min_size(
                egui::pos2(x, box_rect.top() + 6.0),
                egui::vec2(*w, 20.0),
            );
            any_hovered |= ui
                .input(|i| i.pointer.latest_pos())
                .is_some_and(|p| btn_rect.contains(p));
            let resp = ui.put(btn_rect, egui::Button::new(label.clone()).small());
            if resp.clicked() {
                action(self, ui);
            }
        }
        if self
            .copied_at
            .is_some_and(|t| t.elapsed() < Duration::from_secs(1))
        {
            ui.painter().text(
                egui::pos2(x - 8.0, box_rect.top() + 16.0),
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
