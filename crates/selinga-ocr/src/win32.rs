use selinga_capture::Frame;
use windows::Globalization::Language;
use windows::Graphics::Imaging::{BitmapPixelFormat, SoftwareBitmap};
use windows::Media::Ocr::OcrEngine;
use windows::Security::Cryptography::CryptographicBuffer;

use super::{Ocr, OcrError, OcrLine, OcrWord, Rect};

const ENHANCE_CONTRAST: f32 = 1.3;
const PAD_PX: u32 = 16;
const DARK_MEAN_THRESHOLD: f32 = 100.0;
const RETRY_MIN_WEIGHT: usize = 6;
const RADICAL_FRAGMENTS: &[char] = &[
    '彳', '亍', '氵', '讠', '钅', '忄', '丬', '宀', '屮', '彡', '冫', '厶', '丿', '丨', '丶', '亠',
    '冂', '匚', '卩', '刂', '阝', '廴', '犭', '彐', '乛', '丷',
];

const FRAGMENT_PENALTY: usize = 4;
const MERGE_MAX_GAP: f32 = 0.35;
const POLARITY_TILE: u32 = 64;
const DARK_TILE_LUMA: f32 = 100.0;
const LIGHT_TILE_LUMA: f32 = 150.0;
const MIXED_RATIO: f32 = 0.2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tone {
    Contrast,
    Binarize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Polarity {
    Auto,
    Normal,
    Invert,
}

struct Prepared {
    frame: Frame,
    scale: f32,
    pad: f32,
}

fn upscale(frame: &Frame, factor: u32) -> Frame {
    let (w, h) = (frame.width, frame.height);
    let (nw, nh) = (w * factor, h * factor);
    let mut bgra = Vec::with_capacity((nw * nh * 4) as usize);
    for sy in 0..h {
        let row = &frame.bgra[(sy * w * 4) as usize..((sy + 1) * w * 4) as usize];
        let mut line = Vec::with_capacity((nw * 4) as usize);
        for px in row.chunks_exact(4) {
            let pixel: Vec<u8> = px
                .iter()
                .cycle()
                .take(factor as usize * 4)
                .copied()
                .collect();
            line.extend_from_slice(&pixel);
        }
        for _ in 0..factor {
            bgra.extend_from_slice(&line);
        }
    }
    Frame {
        width: nw,
        height: nh,
        bgra,
    }
}

fn mean_luma(frame: &Frame) -> f32 {
    let pixels = frame.bgra.len() / 4;
    if pixels == 0 {
        return 255.0;
    }
    let step = (pixels / 16384).max(1);
    let mut sum = 0u64;
    let mut count = 0u64;
    for i in (0..pixels).step_by(step) {
        let b = u32::from(frame.bgra[i * 4]);
        let g = u32::from(frame.bgra[i * 4 + 1]);
        let r = u32::from(frame.bgra[i * 4 + 2]);
        sum += u64::from((r * 299 + g * 587 + b * 114) / 1000);
        count += 1;
    }
    if count == 0 {
        255.0
    } else {
        #[allow(clippy::cast_precision_loss)]
        let mean = sum as f32 / count as f32;
        mean
    }
}

fn invert(frame: &mut Frame) {
    for px in frame.bgra.chunks_exact_mut(4) {
        for c in &mut px[..3] {
            *c = 255 - *c;
        }
    }
}

fn apply_contrast(frame: &mut Frame) {
    for px in frame.bgra.chunks_exact_mut(4) {
        for c in &mut px[..3] {
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let boosted = ((f32::from(*c) - 128.0) * ENHANCE_CONTRAST + 128.0).clamp(0.0, 255.0);
            *c = boosted as u8;
        }
    }
}

fn otsu_threshold(frame: &Frame) -> u8 {
    let mut hist = [0u32; 256];
    let pixels = frame.bgra.len() / 4;
    let step = (pixels / 16384).max(1);
    for i in (0..pixels).step_by(step) {
        let b = u32::from(frame.bgra[i * 4]);
        let g = u32::from(frame.bgra[i * 4 + 1]);
        let r = u32::from(frame.bgra[i * 4 + 2]);
        let y = ((r * 299 + g * 587 + b * 114) / 1000) as usize;
        hist[y] += 1;
    }
    let total: u32 = hist.iter().sum();
    if total == 0 {
        return 128;
    }
    let mut sum_all = 0u64;
    for (i, &h) in hist.iter().enumerate() {
        sum_all += (i as u64) * u64::from(h);
    }
    let mut sum_b = 0u64;
    let mut w_b = 0u32;
    let mut best_var = 0f64;
    let mut thr = 128u8;
    for (i, &h) in hist.iter().enumerate() {
        w_b += h;
        if w_b == 0 {
            continue;
        }
        let w_f = total - w_b;
        if w_f == 0 {
            break;
        }
        sum_b += (i as u64) * u64::from(h);
        #[allow(clippy::cast_precision_loss)]
        {
            let m_b = sum_b as f64 / f64::from(w_b);
            let m_f = (sum_all - sum_b) as f64 / f64::from(w_f);
            let var = f64::from(w_b) * f64::from(w_f) * (m_b - m_f).powi(2);
            if var > best_var {
                best_var = var;
                thr = i as u8;
            }
        }
    }
    thr
}

fn apply_binarize(frame: &mut Frame) {
    let thr = otsu_threshold(frame);
    let mut dark = 0u64;
    let mut total = 0u64;
    for px in frame.bgra.chunks_exact_mut(4) {
        for c in &mut px[..3] {
            *c = if u32::from(*c) >= u32::from(thr) {
                255
            } else {
                0
            };
        }
        total += 1;
        if px[1] == 0 {
            dark += 1;
        }
    }
    if dark * 2 > total {
        invert(frame);
    }
}

fn border_color(frame: &Frame) -> [u8; 3] {
    let w = frame.width as usize;
    let h = frame.height as usize;
    let mut sum = [0u64; 3];
    let mut count = 0u64;
    let mut sample = |x: usize, y: usize| {
        if let Some(px) = frame.bgra.get((y * w + x) * 4..(y * w + x) * 4 + 4) {
            for c in 0..3 {
                sum[c] += u64::from(px[c]);
            }
            count += 1;
        }
    };
    if w > 0 && h > 0 {
        for x in (0..w).step_by((w / 64).max(1)) {
            sample(x, 0);
            sample(x, h - 1);
        }
        for y in (0..h).step_by((h / 64).max(1)) {
            sample(0, y);
            sample(w - 1, y);
        }
    }
    if count == 0 {
        return [255, 255, 255];
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let avg = |s: u64| (s as f32 / count as f32).round().clamp(0.0, 255.0) as u8;
    [avg(sum[0]), avg(sum[1]), avg(sum[2])]
}

fn pad_edges(frame: &Frame, pad: u32) -> Frame {
    if pad == 0 {
        return frame.clone();
    }
    let (w, h) = (frame.width, frame.height);
    let (nw, nh) = (w + pad * 2, h + pad * 2);
    let color = border_color(frame);
    let mut bgra = vec![0u8; (nw * nh * 4) as usize];
    for px in bgra.chunks_exact_mut(4) {
        px.copy_from_slice(&[color[0], color[1], color[2], 255]);
    }
    for (row, src_row) in frame.bgra.chunks_exact((w * 4) as usize).enumerate() {
        let dst_off = (((pad + row as u32) * nw + pad) * 4) as usize;
        bgra[dst_off..dst_off + src_row.len()].copy_from_slice(src_row);
    }
    Frame {
        width: nw,
        height: nh,
        bgra,
    }
}

fn prepare(
    frame: &Frame,
    factor: u32,
    tone: Tone,
    polarity: Polarity,
    max: Option<u32>,
) -> Option<Prepared> {
    if let Some(max) = max {
        let final_w = frame.width * factor + PAD_PX * 2;
        let final_h = frame.height * factor + PAD_PX * 2;
        if final_w > max || final_h > max {
            return None;
        }
    }
    let mut out = upscale(frame, factor);
    match tone {
        Tone::Contrast => {
            match polarity {
                Polarity::Auto => {
                    if mean_luma(&out) < DARK_MEAN_THRESHOLD {
                        invert(&mut out);
                    }
                }
                Polarity::Normal => {}
                Polarity::Invert => invert(&mut out),
            }
            apply_contrast(&mut out);
        }
        Tone::Binarize => apply_binarize(&mut out),
    }
    let out = pad_edges(&out, PAD_PX * factor);
    Some(Prepared {
        frame: out,
        scale: factor as f32,
        pad: PAD_PX as f32,
    })
}

fn map_back(rect: &mut Rect, prep: &Prepared) {
    rect.x = rect.x / prep.scale - prep.pad;
    rect.y = rect.y / prep.scale - prep.pad;
    rect.w /= prep.scale;
    rect.h /= prep.scale;
}

fn is_cjk(c: char) -> bool {
    matches!(c as u32,
        0x3000..=0x303F
        | 0x3040..=0x30FF
        | 0x3400..=0x4DBF
        | 0x4E00..=0x9FFF
        | 0xF900..=0xFAFF
        | 0xFF00..=0xFFEF
        | 0x20000..=0x2FA1F)
}

fn clean_cjk_spaces(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() < 3 {
        return s.to_owned();
    }
    let mut out = String::with_capacity(s.len());
    for (i, &c) in chars.iter().enumerate() {
        if c == ' ' {
            let prev = out.chars().last();
            let next = chars[i + 1..].iter().find(|&&x| x != ' ').copied();
            if prev.is_some_and(is_cjk) && next.is_some_and(is_cjk) {
                continue;
            }
        }
        out.push(c);
    }
    out
}

fn result_weight(lines: &[OcrLine]) -> usize {
    lines
        .iter()
        .map(|l| {
            l.text
                .chars()
                .map(|c| if is_cjk(c) { 2 } else { 1 })
                .sum::<usize>()
        })
        .sum()
}

fn fragment_count(lines: &[OcrLine]) -> usize {
    lines
        .iter()
        .map(|l| {
            l.text
                .chars()
                .filter(|c| RADICAL_FRAGMENTS.contains(c))
                .count()
        })
        .sum()
}

fn score(lines: &[OcrLine]) -> usize {
    result_weight(lines).saturating_sub(FRAGMENT_PENALTY * fragment_count(lines))
}

fn median_f32(mut values: Vec<f32>) -> f32 {
    if values.is_empty() {
        return 0.0;
    }
    values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let mid = values.len() / 2;
    if values.len().is_multiple_of(2) {
        (values[mid - 1] + values[mid]) / 2.0
    } else {
        values[mid]
    }
}

fn splittable_pair(a: &OcrWord, b: &OcrWord, med_h: f32) -> bool {
    let both_cjk = !a.text.is_empty()
        && a.text.chars().all(is_cjk)
        && !b.text.is_empty()
        && b.text.chars().all(is_cjk);
    if !both_cjk {
        return false;
    }
    let gap = b.rect.x - (a.rect.x + a.rect.w);
    if gap < 0.0 || gap > MERGE_MAX_GAP * med_h {
        return false;
    }
    let ov_top = a.rect.y.max(b.rect.y);
    let ov_bot = (a.rect.y + a.rect.h).min(b.rect.y + b.rect.h);
    if ov_bot - ov_top < 0.4 * a.rect.h.min(b.rect.h) {
        return false;
    }
    let union_w = (a.rect.x + a.rect.w).max(b.rect.x + b.rect.w) - a.rect.x.min(b.rect.x);
    union_w <= 1.4 * med_h
}

fn merge_split_words(words: Vec<OcrWord>) -> Vec<OcrWord> {
    if words.len() < 2 {
        return words;
    }
    let med_h = median_f32(words.iter().map(|w| w.rect.h).collect());
    if med_h <= 0.0 {
        return words;
    }
    let mut sorted = words;
    sorted.sort_by(|a, b| {
        a.rect
            .x
            .partial_cmp(&b.rect.x)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let mut out: Vec<OcrWord> = Vec::with_capacity(sorted.len());
    for w in sorted {
        let merged = match out.last_mut() {
            Some(prev) if splittable_pair(prev, &w, med_h) => {
                prev.text.push_str(&w.text);
                prev.rect = prev.rect.cover(w.rect);
                true
            }
            _ => false,
        };
        if !merged {
            out.push(w);
        }
    }
    out
}

fn polarity_profile(frame: &Frame) -> (f32, f32) {
    let (w, h) = (frame.width, frame.height);
    if w == 0 || h == 0 {
        return (0.0, 0.0);
    }
    let cols = w.div_ceil(POLARITY_TILE);
    let rows = h.div_ceil(POLARITY_TILE);
    let tiles = (cols * rows) as usize;
    let mut sums = vec![0u64; tiles];
    let mut counts = vec![0u64; tiles];

    let pixels = frame.bgra.len() / 4;
    let step = (pixels / 16384).max(1);
    for i in (0..pixels).step_by(step) {
        let x = (i % w as usize) as u32;
        let y = (i / w as usize) as u32;
        let b = u32::from(frame.bgra[i * 4]);
        let g = u32::from(frame.bgra[i * 4 + 1]);
        let r = u32::from(frame.bgra[i * 4 + 2]);
        let idx = (y / POLARITY_TILE) * cols + (x / POLARITY_TILE);
        sums[idx as usize] += u64::from((r * 299 + g * 587 + b * 114) / 1000);
        counts[idx as usize] += 1;
    }

    let mut dark = 0u32;
    let mut light = 0u32;
    let mut total = 0u32;
    for (&s, &c) in sums.iter().zip(&counts) {
        if c == 0 {
            continue;
        }
        #[allow(clippy::cast_precision_loss)]
        let mean = s as f32 / c as f32;
        total += 1;
        if mean < DARK_TILE_LUMA {
            dark += 1;
        } else if mean >= LIGHT_TILE_LUMA {
            light += 1;
        }
    }
    if total == 0 {
        return (0.0, 0.0);
    }
    #[allow(clippy::cast_precision_loss)]
    let ratio = |n: u32| n as f32 / total as f32;
    (ratio(dark), ratio(light))
}

fn is_mixed_polarity(frame: &Frame) -> bool {
    let (dark, light) = polarity_profile(frame);
    dark >= MIXED_RATIO && light >= MIXED_RATIO
}

fn same_text_row(a: &OcrLine, b: &OcrLine) -> bool {
    match (a.rect, b.rect) {
        (Some(ra), Some(rb)) => {
            let top = ra.y.max(rb.y);
            let bot = (ra.y + ra.h).min(rb.y + rb.h);
            bot - top > 0.5 * ra.h.min(rb.h).max(1.0)
        }
        _ => false,
    }
}

fn merge_polarity_lines(mut lines: Vec<OcrLine>) -> Vec<OcrLine> {
    if lines.len() < 2 {
        return lines;
    }
    lines.sort_by(|a, b| {
        let ya = a.rect.map_or(0.0, |r| r.y);
        let yb = b.rect.map_or(0.0, |r| r.y);
        ya.partial_cmp(&yb).unwrap_or(std::cmp::Ordering::Equal)
    });
    let mut kept: Vec<OcrLine> = Vec::new();
    for line in lines {
        match kept.iter().position(|k| same_text_row(k, &line)) {
            Some(i) => {
                if result_weight(std::slice::from_ref(&line))
                    > result_weight(std::slice::from_ref(&kept[i]))
                {
                    kept[i] = line;
                }
            }
            None => kept.push(line),
        }
    }
    kept
}

pub struct WinOcr {
    engine: OcrEngine,
}

impl WinOcr {
    pub fn new(lang: Option<&str>) -> Result<Self, OcrError> {
        if let Some(tag) = lang {
            let language = Language::CreateLanguage(&tag.into())
                .map_err(|e| OcrError::Platform(e.to_string()))?;
            let engine = OcrEngine::TryCreateFromLanguage(&language)
                .map_err(|_| OcrError::EngineUnavailable(Some(tag.to_owned())))?;
            return Ok(Self { engine });
        }
        if let Ok(engine) = OcrEngine::TryCreateFromUserProfileLanguages() {
            return Ok(Self { engine });
        }
        for tag in ["zh-CN", "en-US"] {
            let language = Language::CreateLanguage(&tag.into())
                .map_err(|e| OcrError::Platform(e.to_string()))?;
            if let Ok(engine) = OcrEngine::TryCreateFromLanguage(&language) {
                return Ok(Self { engine });
            }
        }
        Err(OcrError::EngineUnavailable(None))
    }

    pub fn language_tag(&self) -> Result<String, OcrError> {
        Ok(self
            .engine
            .RecognizerLanguage()
            .map_err(|e| OcrError::Platform(e.to_string()))?
            .LanguageTag()
            .map_err(|e| OcrError::Platform(e.to_string()))?
            .to_string())
    }

    fn run_variant(&self, prep: &Prepared) -> Result<Vec<OcrLine>, OcrError> {
        let buffer = CryptographicBuffer::CreateFromByteArray(&prep.frame.bgra)
            .map_err(|e| OcrError::Platform(e.to_string()))?;
        let bitmap = SoftwareBitmap::CreateCopyFromBuffer(
            &buffer,
            BitmapPixelFormat::Bgra8,
            prep.frame.width as i32,
            prep.frame.height as i32,
        )
        .map_err(|e| OcrError::Platform(e.to_string()))?;

        let result = self
            .engine
            .RecognizeAsync(&bitmap)
            .map_err(|e| OcrError::Platform(e.to_string()))?
            .get()
            .map_err(|e| OcrError::Platform(e.to_string()))?;

        let mut lines = Vec::new();
        for line in result
            .Lines()
            .map_err(|e| OcrError::Platform(e.to_string()))?
        {
            let text = line.Text().map_err(|e| OcrError::Platform(e.to_string()))?;
            let mut words = Vec::new();
            for word in line
                .Words()
                .map_err(|e| OcrError::Platform(e.to_string()))?
            {
                let r = word
                    .BoundingRect()
                    .map_err(|e| OcrError::Platform(e.to_string()))?;
                let mut rect = Rect {
                    x: r.X,
                    y: r.Y,
                    w: r.Width,
                    h: r.Height,
                };
                map_back(&mut rect, prep);
                words.push(OcrWord {
                    text: word
                        .Text()
                        .map_err(|e| OcrError::Platform(e.to_string()))?
                        .to_string(),
                    rect,
                });
            }
            let words = merge_split_words(words);
            let line_rect = union_rect(words.iter().map(|w| &w.rect));
            lines.push(OcrLine {
                text: clean_cjk_spaces(&text.to_string_lossy()),
                rect: line_rect,
                words,
            });
        }
        Ok(lines)
    }
}

fn union_rect<'a>(rects: impl Iterator<Item = &'a Rect>) -> Option<Rect> {
    let mut rects = rects;
    let first = *rects.next()?;
    Some(rects.fold(first, |acc, r| acc.cover(*r)))
}

impl Rect {
    fn cover(self, other: Rect) -> Rect {
        let x0 = self.x.min(other.x);
        let y0 = self.y.min(other.y);
        let x1 = (self.x + self.w).max(other.x + other.w);
        let y1 = (self.y + self.h).max(other.y + other.h);
        Rect {
            x: x0,
            y: y0,
            w: x1 - x0,
            h: y1 - y0,
        }
    }
}

impl Ocr for WinOcr {
    fn recognize(&self, frame: &Frame) -> Result<Vec<OcrLine>, OcrError> {
        let max = OcrEngine::MaxImageDimension().ok();
        if let Some(max) = max
            && (frame.width > max || frame.height > max)
        {
            return Err(OcrError::TooLarge {
                width: frame.width,
                height: frame.height,
                max,
            });
        }

        let mut best: Option<Vec<OcrLine>> = None;
        let mut best_score = 0usize;
        let mut first_err: Option<OcrError> = None;
        let consider =
            |lines: Vec<OcrLine>, best: &mut Option<Vec<OcrLine>>, best_score: &mut usize| {
                let s = score(&lines);
                if s > *best_score {
                    *best_score = s;
                    *best = Some(lines);
                }
            };

        if is_mixed_polarity(frame) {
            for factor in [2, 3] {
                if best_score >= RETRY_MIN_WEIGHT {
                    break;
                }
                let mut pair: Vec<OcrLine> = Vec::new();
                for polarity in [Polarity::Normal, Polarity::Invert] {
                    let Some(prep) = prepare(frame, factor, Tone::Contrast, polarity, max) else {
                        continue;
                    };
                    match self.run_variant(&prep) {
                        Ok(lines) => pair.extend(lines),
                        Err(err) if first_err.is_none() => first_err = Some(err),
                        Err(_) => {}
                    }
                }
                consider(merge_polarity_lines(pair), &mut best, &mut best_score);
            }
            if best_score < RETRY_MIN_WEIGHT
                && let Some(prep) = prepare(frame, 3, Tone::Binarize, Polarity::Auto, max)
            {
                match self.run_variant(&prep) {
                    Ok(lines) => consider(lines, &mut best, &mut best_score),
                    Err(err) if first_err.is_none() => first_err = Some(err),
                    Err(_) => {}
                }
            }
        } else {
            let plans: [(u32, Tone); 4] = [
                (2, Tone::Contrast),
                (3, Tone::Contrast),
                (4, Tone::Contrast),
                (3, Tone::Binarize),
            ];
            for (i, (factor, tone)) in plans.into_iter().enumerate() {
                if i > 0
                    && best_score >= RETRY_MIN_WEIGHT
                    && best.as_ref().is_some_and(|b| fragment_count(b) == 0)
                {
                    break;
                }
                let mut prep = prepare(frame, factor, tone, Polarity::Auto, max);
                if i == 0 && prep.is_none() {
                    prep = prepare(frame, 1, tone, Polarity::Auto, max);
                }
                let Some(prep) = prep else {
                    continue;
                };
                match self.run_variant(&prep) {
                    Ok(lines) => consider(lines, &mut best, &mut best_score),
                    Err(err) if first_err.is_none() => first_err = Some(err),
                    Err(_) => {}
                }
            }
        }

        match (best, first_err) {
            (Some(lines), _) => Ok(lines),
            (None, Some(err)) => Err(err),
            (None, None) => Ok(Vec::new()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cjk_spaces_collapse_between_ideographs() {
        assert_eq!(clean_cjk_spaces("光 是 像 以 前"), "光是像以前");
        assert_eq!(clean_cjk_spaces("用 脚 投票 其实"), "用脚投票其实");
        assert_eq!(clean_cjk_spaces("hello world"), "hello world");
        assert_eq!(clean_cjk_spaces("使 用 Rust 写 的"), "使用 Rust 写的");
    }

    #[test]
    fn weight_ranks_cjk_higher() {
        assert!(result_weight(&[]) == 0);
        let lines = vec![OcrLine {
            text: "中文".to_owned(),
            rect: None,
            words: Vec::new(),
        }];
        assert_eq!(result_weight(&lines), 4);
    }

    fn word(text: &str, x: f32, y: f32, w: f32, h: f32) -> OcrWord {
        OcrWord {
            text: text.to_owned(),
            rect: Rect { x, y, w, h },
        }
    }

    fn line(text: &str, y: f32, h: f32) -> OcrLine {
        OcrLine {
            text: text.to_owned(),
            rect: Some(Rect {
                x: 0.0,
                y,
                w: 100.0,
                h,
            }),
            words: Vec::new(),
        }
    }

    #[test]
    fn fragments_detected_and_penalized() {
        let split = vec![line("彳亍", 0.0, 20.0)];
        assert_eq!(fragment_count(&split), 2);
        assert_eq!(score(&split), 0);
        let clean = vec![line("行动", 0.0, 20.0)];
        assert_eq!(fragment_count(&clean), 0);
        assert_eq!(score(&clean), 4);
    }

    #[test]
    fn split_glyph_words_merge_back() {
        let words = vec![
            word("彳", 0.0, 0.0, 12.0, 24.0),
            word("亍", 14.0, 0.0, 12.0, 24.0),
            word("中", 60.0, 0.0, 24.0, 24.0),
            word("文", 200.0, 0.0, 24.0, 24.0),
        ];
        let merged = merge_split_words(words);
        assert_eq!(merged.len(), 3);
        assert_eq!(merged[0].text, "彳亍");
        assert_eq!(merged[0].rect.w, 26.0);
        assert_eq!(merged[1].text, "中");
        assert_eq!(merged[2].text, "文");
    }

    #[test]
    fn latin_and_wide_gaps_never_merge() {
        let words = vec![
            word("hello", 0.0, 0.0, 40.0, 20.0),
            word("world", 45.0, 0.0, 40.0, 20.0),
        ];
        assert_eq!(merge_split_words(words).len(), 2);
        let words = vec![
            word("中", 0.0, 0.0, 24.0, 24.0),
            word("文", 60.0, 0.0, 24.0, 24.0),
        ];
        assert_eq!(merge_split_words(words).len(), 2);
    }

    #[test]
    fn polarity_profile_detects_mixed_frame() {
        let mut bgra = Vec::with_capacity(128 * 128 * 4);
        for _y in 0..128u32 {
            for x in 0..128u32 {
                let v = if x < 64 { 0u8 } else { 255u8 };
                bgra.extend_from_slice(&[v, v, v, 255]);
            }
        }
        let frame = Frame {
            width: 128,
            height: 128,
            bgra,
        };
        assert!(is_mixed_polarity(&frame));

        let bgra = vec![255u8; 128 * 128 * 4];
        let frame = Frame {
            width: 128,
            height: 128,
            bgra,
        };
        assert!(!is_mixed_polarity(&frame));
    }

    #[test]
    fn polarity_lines_dedupe_and_keep_heavier() {
        let lines = vec![line("轻", 0.0, 20.0), line("较重的译文行", 2.0, 20.0)];
        let merged = merge_polarity_lines(lines);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].text, "较重的译文行");

        let lines = vec![line("下", 40.0, 20.0), line("上", 0.0, 20.0)];
        let merged = merge_polarity_lines(lines);
        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].text, "上");
        assert_eq!(merged[1].text, "下");
    }
}
