use thiserror::Error;

#[cfg(windows)]
pub mod win32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Region {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
}

#[derive(Debug, Clone)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub bgra: Vec<u8>,
}

impl Frame {
    pub fn to_rgba(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.bgra.len());
        for px in self.bgra.chunks_exact(4) {
            out.extend_from_slice(&[px[2], px[1], px[0], 0xff]);
        }
        out
    }
}

#[derive(Debug, Error)]
pub enum CaptureError {
    #[error("invalid region {0:?}")]
    InvalidRegion(Region),
    #[error("capture failed: {0}")]
    Platform(String),
}

pub trait ScreenCapture {
    fn capture_region(&self, region: Region) -> Result<Frame, CaptureError>;
}
