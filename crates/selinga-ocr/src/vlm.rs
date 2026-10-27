use crate::{Frame, Ocr, OcrError, OcrLine, Rect};
use base64::Engine as _;
use serde_json::Value;
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(90);
pub const MAX_SIDE: u32 = 1568;

const OCR_MAX_TOKENS: u32 = 1024;
fn token_budget(expected_lines: usize) -> u32 {
    (96 + 40 * expected_lines as u32).clamp(384, 4096)
}

const OCR_PROMPT: &str = r#"You are a screen-text OCR engine. Read every visible text line in the image in reading order and report its box. Reply with JSON only, no fence, no commentary:
{"lines":[["<text>",[x,y,w,h]]]}
box = normalized [x,y,w,h] in 0..1, top-left corner. Keep text verbatim; never merge or split lines. If nothing is readable, reply {"lines":[]}."#;

const MERGED_PROMPT: &str = r#"You are a screen-text OCR and translation engine. Read every visible text line in the image in reading order, translate each into @TARGET@, and report where it sits. Reply with JSON only, no fence, no commentary:
{"lines":[["<original>","<translation>",[x,y,w,h]]]}
box = normalized [x,y,w,h] in 0..1, top-left corner. Keep original text verbatim; never merge or split lines. If nothing is readable, reply {"lines":[]}."#;

#[derive(Debug, Clone)]
pub struct VlmLine {
    pub text: String,
    pub translated: Option<String>,
    pub rect: Option<Rect>,
}

impl VlmLine {
    fn into_ocr(self) -> OcrLine {
        OcrLine {
            text: self.text,
            rect: self.rect,
            words: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VlmEndpoint {
    pub base_url: String,
    pub api_key: Option<String>,
    pub model: String,
}

pub struct VlmOcr {
    endpoint: VlmEndpoint,
    agent: ureq::Agent,
}

impl VlmOcr {
    pub fn new(endpoint: VlmEndpoint) -> Self {
        let agent = ureq::AgentBuilder::new().timeout(TIMEOUT).build();
        Self { endpoint, agent }
    }

    fn chat(&self, body: &Value) -> Result<String, OcrError> {
        let url = format!(
            "{}/chat/completions",
            self.endpoint.base_url.trim_end_matches('/')
        );
        let mut req = self
            .agent
            .post(&url)
            .set("Content-Type", "application/json");
        if let Some(key) = &self.endpoint.api_key {
            req = req.set("Authorization", &format!("Bearer {key}"));
        }
        let json: Value = match req.send_string(&body.to_string()) {
            Ok(resp) => resp
                .into_json()
                .map_err(|e| OcrError::Platform(format!("vlm json: {e}")))?,
            Err(ureq::Error::Status(code, resp)) => {
                let detail = resp.into_string().unwrap_or_default();
                let detail: String = detail.chars().take(300).collect();
                return Err(OcrError::Platform(format!("vlm http {code}: {detail}")));
            }
            Err(e) => return Err(OcrError::Platform(format!("vlm http: {e}"))),
        };
        json.pointer("/choices/0/message/content")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| OcrError::Platform("vlm: unexpected response".into()))
    }
}

impl Ocr for VlmOcr {
    fn recognize(&self, frame: &Frame) -> Result<Vec<OcrLine>, OcrError> {
        let (w, h, rgba) = downscale(&frame.to_rgba(), frame.width, frame.height, MAX_SIDE);
        let png = encode_png(&rgba, w, h)?;
        let data_url = format!(
            "data:image/png;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(&png)
        );
        let body = serde_json::json!({
            "model": self.endpoint.model,
            "temperature": 0,
            "max_tokens": OCR_MAX_TOKENS,
            "messages": [
                {"role": "system", "content": OCR_PROMPT},
                {"role": "user", "content": [
                    {"type": "text", "text": "Transcribe the text in this image."},
                    {"type": "image_url", "image_url": {"url": data_url}},
                ]},
            ],
        });
        let content = self.chat(&body)?;
        Ok(parse_reply(&content, (w, h), (frame.width, frame.height)))
    }
}

impl VlmOcr {
    pub fn recognize_and_translate(
        &self,
        frame: &Frame,
        target: &str,
        expected_lines: usize,
    ) -> Result<Vec<VlmLine>, OcrError> {
        let (w, h, rgba) = downscale(&frame.to_rgba(), frame.width, frame.height, MAX_SIDE);
        let png = encode_png(&rgba, w, h)?;
        let data_url = format!(
            "data:image/png;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(&png)
        );
        let body = serde_json::json!({
            "model": self.endpoint.model,
            "temperature": 0,
            "max_tokens": token_budget(expected_lines),
            "messages": [
                {"role": "system", "content": MERGED_PROMPT.replace("@TARGET@", target)},
                {"role": "user", "content": [
                    {"type": "text", "text": "Read and translate."},
                    {"type": "image_url", "image_url": {"url": data_url}},
                ]},
            ],
        });
        let content = self.chat(&body)?;
        Ok(parse_merged(&content, (w, h), (frame.width, frame.height)))
    }
}

fn downscale(rgba: &[u8], w: u32, h: u32, max_side: u32) -> (u32, u32, Vec<u8>) {
    let longest = w.max(h);
    if longest <= max_side || w == 0 || h == 0 {
        return (w, h, rgba.to_vec());
    }
    let nw = ((u64::from(w) * u64::from(max_side)) / u64::from(longest)).max(1) as u32;
    let nh = ((u64::from(h) * u64::from(max_side)) / u64::from(longest)).max(1) as u32;
    let mut out = vec![0u8; (nw * nh * 4) as usize];
    for y in 0..nh {
        let sy0 = (u64::from(y) * u64::from(h)) / u64::from(nh);
        let sy1 = (((u64::from(y) + 1) * u64::from(h)) / u64::from(nh))
            .max(sy0 + 1)
            .min(u64::from(h));
        for x in 0..nw {
            let sx0 = (u64::from(x) * u64::from(w)) / u64::from(nw);
            let sx1 = (((u64::from(x) + 1) * u64::from(w)) / u64::from(nw))
                .max(sx0 + 1)
                .min(u64::from(w));
            let mut acc = [0u64; 3];
            let mut count = 0u64;
            for sy in sy0..sy1 {
                for sx in sx0..sx1 {
                    let px = ((sy * u64::from(w) + sx) * 4) as usize;
                    for c in 0..3 {
                        acc[c] += u64::from(rgba[px + c]);
                    }
                    count += 1;
                }
            }
            let o = ((y * nw + x) * 4) as usize;
            for c in 0..3 {
                out[o + c] = (acc[c] / count.max(1)) as u8;
            }
            out[o + 3] = 255;
        }
    }
    (nw, nh, out)
}

fn encode_png(rgba: &[u8], w: u32, h: u32) -> Result<Vec<u8>, OcrError> {
    let mut buf = Vec::new();
    let mut encoder = png::Encoder::new(&mut buf, w, h);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder
        .write_header()
        .map_err(|e| OcrError::Platform(format!("png header: {e}")))?;
    writer
        .write_image_data(rgba)
        .map_err(|e| OcrError::Platform(format!("png data: {e}")))?;
    writer
        .finish()
        .map_err(|e| OcrError::Platform(format!("png finish: {e}")))?;
    Ok(buf)
}

fn parse_reply(content: &str, sent: (u32, u32), original: (u32, u32)) -> Vec<OcrLine> {
    match parse_entries(content, sent, original) {
        Some(lines) => lines.into_iter().map(VlmLine::into_ocr).collect(),
        None => parse_plain_text(content),
    }
}

fn parse_merged(content: &str, sent: (u32, u32), original: (u32, u32)) -> Vec<VlmLine> {
    parse_entries(content, sent, original).unwrap_or_else(|| {
        parse_plain_text(content)
            .into_iter()
            .map(|line| VlmLine {
                text: line.text,
                translated: None,
                rect: None,
            })
            .collect()
    })
}

fn parse_entries(content: &str, sent: (u32, u32), original: (u32, u32)) -> Option<Vec<VlmLine>> {
    let body = extract_json(content)?;
    let value: Value = serde_json::from_str(body).ok()?;
    let entries = match &value {
        Value::Array(items) => items.clone(),
        Value::Object(map) => map.get("lines")?.as_array()?.clone(),
        _ => return None,
    };
    Some(
        entries
            .iter()
            .filter_map(|entry| parse_entry(entry, sent, original))
            .collect(),
    )
}

fn parse_entry(entry: &Value, sent: (u32, u32), original: (u32, u32)) -> Option<VlmLine> {
    match entry {
        Value::Object(map) => {
            let text = map.get("text").and_then(Value::as_str)?.trim();
            if text.is_empty() {
                return None;
            }
            let translated = map
                .get("translation")
                .or_else(|| map.get("tr"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|t| !t.is_empty())
                .map(str::to_owned);
            let rect = map
                .get("box")
                .or_else(|| map.get("bbox"))
                .and_then(parse_box)
                .and_then(|b| box_to_rect(b, sent, original));
            Some(VlmLine {
                text: text.to_owned(),
                translated,
                rect,
            })
        }
        Value::Array(items) => {
            let text = items.first().and_then(Value::as_str)?.trim();
            if text.is_empty() {
                return None;
            }
            let mut translated: Option<String> = None;
            let mut rect: Option<Rect> = None;
            for item in items.iter().skip(1) {
                match item {
                    Value::Array(_) => {
                        if rect.is_none() {
                            rect = parse_box(item).and_then(|b| box_to_rect(b, sent, original));
                        }
                    }
                    Value::String(raw) => {
                        let trimmed = raw.trim();
                        if translated.is_none() && !trimmed.is_empty() {
                            translated = Some(trimmed.to_owned());
                        }
                    }
                    _ => {}
                }
            }
            Some(VlmLine {
                text: text.to_owned(),
                translated,
                rect,
            })
        }
        _ => None,
    }
}

fn extract_json(content: &str) -> Option<&str> {
    let start = content.find(['{', '['])?;
    let end = content.rfind(['}', ']'])?;
    (end > start).then(|| &content[start..=end])
}

fn parse_box(value: &Value) -> Option<[f32; 4]> {
    let number = |v: &Value| v.as_f64().map(|n| n as f32);
    let numbers = match value {
        Value::Array(items) if items.len() == 4 => [
            number(&items[0])?,
            number(&items[1])?,
            number(&items[2])?,
            number(&items[3])?,
        ],
        Value::Object(map) => [
            number(map.get("x")?)?,
            number(map.get("y")?)?,
            number(map.get("w")?)?,
            number(map.get("h")?)?,
        ],
        _ => return None,
    };
    numbers.iter().all(|n| n.is_finite()).then_some(numbers)
}

fn box_to_rect(b: [f32; 4], sent: (u32, u32), original: (u32, u32)) -> Option<Rect> {
    let (ow, oh) = (original.0 as f32, original.1 as f32);
    if ow <= 0.0 || oh <= 0.0 {
        return None;
    }
    let largest = b.iter().copied().fold(0.0_f32, f32::max);
    let sent_max = sent.0.max(sent.1).max(1) as f32;
    let (sx, sy) = if largest <= 1.0001 {
        (ow, oh)
    } else if largest <= sent_max * 1.05 {
        (ow / sent.0.max(1) as f32, oh / sent.1.max(1) as f32)
    } else if largest <= 1000.5 {
        (ow / 1000.0, oh / 1000.0)
    } else {
        (ow / sent.0.max(1) as f32, oh / sent.1.max(1) as f32)
    };
    let (x, y) = (b[0] * sx, b[1] * sy);
    let (mut w, mut h) = (b[2] * sx, b[3] * sy);
    if x + w > ow * 1.02 || y + h > oh * 1.02 {
        let (corner_w, corner_h) = ((b[2] - b[0]) * sx, (b[3] - b[1]) * sy);
        if corner_w > 1.0
            && corner_h > 1.0
            && x + corner_w <= ow * 1.02
            && y + corner_h <= oh * 1.02
        {
            w = corner_w;
            h = corner_h;
        }
    }
    if x >= ow || y >= oh || w <= 1.0 || h <= 1.0 {
        return None;
    }
    let x = x.max(0.0);
    let y = y.max(0.0);
    let w = w.min(ow - x);
    let h = h.min(oh - y);
    (w > 1.0 && h > 1.0).then_some(Rect { x, y, w, h })
}

fn parse_plain_text(content: &str) -> Vec<OcrLine> {
    content
        .split('\n')
        .map(str::trim)
        .filter(|l| !l.is_empty() && *l != "-")
        .map(|text| OcrLine {
            text: text.to_owned(),
            rect: None,
            words: Vec::new(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_fallback_drops_noise() {
        let lines = parse_plain_text("hello world\n\n  \n- \nsecond\n-\n");
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].text, "hello world");
        assert_eq!(lines[1].text, "second");
        assert!(lines.iter().all(|l| l.rect.is_none()));
    }

    #[test]
    fn plain_text_fallback_empty_reply_yields_empty() {
        assert!(parse_plain_text("-").is_empty());
        assert!(parse_plain_text("").is_empty());
    }

    #[test]
    fn box_accepts_the_zero_to_thousand_space() {
        let rect = box_to_rect([500.0, 200.0, 300.0, 150.0], (200, 100), (1000, 500)).unwrap();
        assert!((rect.x - 500.0).abs() < 0.5);
        assert!((rect.y - 100.0).abs() < 0.5);
        assert!((rect.w - 300.0).abs() < 0.5);
        assert!((rect.h - 75.0).abs() < 0.5);
    }

    #[test]
    fn overflowing_box_is_retried_as_corners() {
        let rect = box_to_rect([200.0, 100.0, 900.0, 400.0], (1000, 500), (1000, 500)).unwrap();
        assert!((rect.w - 700.0).abs() < 0.5);
        assert!((rect.h - 300.0).abs() < 0.5);
    }

    #[test]
    fn merged_token_budget_scales_with_lines() {
        assert!(token_budget(1) < token_budget(40));
        assert_eq!(token_budget(0), 384);
        assert_eq!(token_budget(10_000), 4096);
    }

    #[test]
    fn json_reply_carries_normalized_boxes() {
        let reply = "{\"lines\":[{\"text\":\"hello\",\"box\":[0.1,0.2,0.5,0.1]}]}";
        let lines = parse_reply(reply, (1568, 784), (1000, 500));
        let rect = lines[0].rect.unwrap();
        assert!((rect.x - 100.0).abs() < 0.5);
        assert!((rect.y - 100.0).abs() < 0.5);
        assert!((rect.w - 500.0).abs() < 0.5);
        assert!((rect.h - 50.0).abs() < 0.5);
    }

    #[test]
    fn json_reply_scales_pixel_boxes_from_the_sent_image() {
        let reply = "{\"lines\":[{\"text\":\"hi\",\"box\":[100,50,200,40]}]}";
        let lines = parse_reply(reply, (500, 250), (1000, 500));
        let rect = lines[0].rect.unwrap();
        assert!((rect.x - 200.0).abs() < 0.5);
        assert!((rect.y - 100.0).abs() < 0.5);
        assert!((rect.w - 400.0).abs() < 0.5);
        assert!((rect.h - 80.0).abs() < 0.5);
    }

    #[test]
    fn json_reply_tolerates_code_fence_and_bare_array() {
        let fenced = "```json\n[{\"text\":\"a\",\"box\":[0.0,0.0,0.2,0.1]}]\n```";
        let lines = parse_reply(fenced, (100, 100), (100, 100));
        assert_eq!(lines.len(), 1);
        assert!(lines[0].rect.is_some());
    }

    #[test]
    fn json_reply_drops_out_of_range_boxes_but_keeps_text() {
        let reply = "{\"lines\":[{\"text\":\"far\",\"box\":[2.0,2.0,0.5,0.5]}]}";
        let lines = parse_reply(reply, (100, 100), (100, 100));
        assert_eq!(lines[0].text, "far");
        assert!(lines[0].rect.is_none());
    }

    #[test]
    fn array_entries_parse_without_translation() {
        let reply = r#"{"lines":[["hello",[0.1,0.2,0.5,0.1]]]}"#;
        let lines = parse_reply(reply, (1000, 500), (1000, 500));
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].text, "hello");
        assert!(lines[0].rect.is_some());
    }

    #[test]
    fn merged_entries_carry_translation_and_box() {
        let reply =
            r#"{"lines":[["hello","你好",[0.1,0.2,0.5,0.1]],["world","世界",[0.1,0.4,0.5,0.1]]]}"#;
        let lines = parse_merged(reply, (1000, 500), (1000, 500));
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].translated.as_deref(), Some("你好"));
        assert_eq!(lines[1].translated.as_deref(), Some("世界"));
        assert!(lines.iter().all(|l| l.rect.is_some()));
    }

    #[test]
    fn merged_plain_text_reply_has_no_translation() {
        let lines = parse_merged("hello\nworld", (10, 10), (10, 10));
        assert_eq!(lines.len(), 2);
        assert!(lines.iter().all(|l| l.translated.is_none()));
    }

    #[test]
    fn malformed_json_falls_back_to_plain_text() {
        let reply = "{\"lines\": [ not json";
        let lines = parse_reply(reply, (100, 100), (100, 100));
        assert!(!lines.is_empty());
        assert!(lines.iter().all(|l| l.rect.is_none()));
    }

    #[test]
    fn downscale_noop_below_limit() {
        let rgba = vec![1u8; 4 * 10 * 5];
        let (w, h, out) = downscale(&rgba, 10, 5, 100);
        assert_eq!((w, h), (10, 5));
        assert_eq!(out, rgba);
    }

    #[test]
    fn downscale_averages_neighbors() {
        let mut rgba = vec![0u8; 4 * 2];
        rgba[0..3].copy_from_slice(&[255, 0, 0]);
        rgba[3] = 255;
        rgba[4..7].copy_from_slice(&[0, 255, 0]);
        rgba[7] = 255;
        let (w, h, out) = downscale(&rgba, 2, 1, 1);
        assert_eq!((w, h), (1, 1));
        assert_eq!(out[0], 127);
        assert_eq!(out[1], 127);
        assert_eq!(out[2], 0);
        assert_eq!(out[3], 255);
    }

    #[test]
    fn downscale_keeps_block_colors_pure() {
        let mut rgba = vec![0u8; 4 * 4 * 2];
        for x in 0..4u32 {
            let c = if x < 2 { [255u8, 0, 0] } else { [0, 255, 0] };
            for y in 0..2u32 {
                let o = ((y * 4 + x) * 4) as usize;
                rgba[o..o + 3].copy_from_slice(&c);
                rgba[o + 3] = 255;
            }
        }
        let (w, h, out) = downscale(&rgba, 4, 2, 2);
        assert_eq!((w, h), (2, 1));
        assert_eq!(&out[0..4], &[255, 0, 0, 255]);
        assert_eq!(&out[4..8], &[0, 255, 0, 255]);
    }

    #[test]
    fn encode_png_produces_png_magic() {
        let rgba = vec![128u8; 4 * 3 * 2];
        let png = encode_png(&rgba, 3, 2).unwrap();
        assert!(png.starts_with(&[0x89, b'P', b'N', b'G']));
    }
}
