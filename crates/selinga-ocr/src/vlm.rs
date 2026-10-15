//! Vision-LLM OCR backend speaking the OpenAI-compatible chat/completions
//! protocol (works with any provider exposing that shape: SiliconFlow,
//! Qwen-VL, Gemini-compatible endpoints, llama.cpp / Ollama /v1, ...).
//!
//! The crop is downscaled, PNG-encoded and sent as a base64 data URL; the
//! model transcribes the visible text line by line. Per-line geometry is not
//! requested from the model: lines come back with `rect: None` and the
//! caller synthesizes layout.

use crate::{Frame, Ocr, OcrError, OcrLine};
use base64::Engine as _;
use serde_json::Value;
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(90);
/// VLM vision encoders degrade past this side; downscale anything larger.
const MAX_SIDE: u32 = 1568;

const SYSTEM_PROMPT: &str = "You are an OCR engine. Transcribe ALL text visible in the \
image exactly as written, in natural reading order. Output one source line per text \
line. Do not translate, do not number, do not merge lines, do not add commentary. \
If no text is visible, reply with a single dash.";

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
            "messages": [
                {"role": "system", "content": SYSTEM_PROMPT},
                {"role": "user", "content": [
                    {"type": "text", "text": "Transcribe the text in this image."},
                    {"type": "image_url", "image_url": {"url": data_url}},
                ]},
            ],
        });
        let content = self.chat(&body)?;
        Ok(parse_lines(&content))
    }
}

/// Box-filter downscale preserving aspect ratio so the longest side fits
/// `max_side`. Returns `(width, height, rgba)`; a no-op when already small.
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

fn parse_lines(content: &str) -> Vec<OcrLine> {
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
    fn parse_lines_drops_noise() {
        let lines = parse_lines("hello world\n\n  \n- \nsecond\n-\n");
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].text, "hello world");
        assert_eq!(lines[1].text, "second");
        assert!(lines.iter().all(|l| l.rect.is_none()));
    }

    #[test]
    fn parse_lines_empty_reply_yields_empty() {
        assert!(parse_lines("-").is_empty());
        assert!(parse_lines("").is_empty());
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
        // 2x1 red|green -> 1x1, the single target pixel averages both.
        let mut rgba = vec![0u8; 4 * 2];
        rgba[0..3].copy_from_slice(&[255, 0, 0]);
        rgba[3] = 255;
        rgba[4..7].copy_from_slice(&[0, 255, 0]);
        rgba[7] = 255;
        let (w, h, out) = downscale(&rgba, 2, 1, 1);
        assert_eq!((w, h), (1, 1));
        assert_eq!(out[0], 127); // (255+0)/2, integer floor
        assert_eq!(out[1], 127);
        assert_eq!(out[2], 0);
        assert_eq!(out[3], 255);
    }

    #[test]
    fn downscale_keeps_block_colors_pure() {
        // 4x2 with red|red|green|green columns -> 2x1 must stay pure.
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
