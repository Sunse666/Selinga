#![deny(missing_docs)]

use selinga_capture::Frame;
use thiserror::Error;

pub mod win32;

#[derive(Debug, Clone, Copy)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

#[derive(Debug, Clone)]
pub struct OcrWord {
    pub text: String,
    pub rect: Rect,
}

#[derive(Debug, Clone)]
pub struct OcrLine {
    pub text: String,
    pub rect: Option<Rect>,
    pub words: Vec<OcrWord>,
}

#[derive(Debug, Error)]
pub enum OcrError {
    #[error("no OCR engine available for language {0:?}")]
    EngineUnavailable(Option<String>),
    #[error("region too large for OCR: {width}x{height} (max {max})")]
    TooLarge {
        width: u32,
        height: u32,
        max: u32,
    },
    #[error("OCR failed: {0}")]
    Platform(String),
}

pub trait Ocr {
    fn recognize(&self, frame: &Frame) -> Result<Vec<OcrLine>, OcrError>;
}
