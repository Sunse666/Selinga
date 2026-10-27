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

    pub fn upscaled(&self, factor: u32) -> Frame {
        let (w, h) = (self.width, self.height);
        let (nw, nh) = (w * factor, h * factor);
        let mut bgra = Vec::with_capacity((nw * nh * 4) as usize);
        if w == 0 || h == 0 || factor == 0 {
            return Frame {
                width: nw,
                height: nh,
                bgra,
            };
        }
        let mut line = Vec::with_capacity((nw * 4) as usize);
        for row in self.bgra.chunks_exact((w * 4) as usize) {
            line.clear();
            for px in row.chunks_exact(4) {
                for _ in 0..factor {
                    line.extend_from_slice(px);
                }
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
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reference(frame: &Frame, factor: u32) -> Frame {
        let (w, h) = (frame.width, frame.height);
        let (nw, nh) = (w * factor, h * factor);
        let mut bgra = Vec::with_capacity((nw * nh * 4) as usize);
        for sy in 0..h {
            let row = &frame.bgra[(sy * w * 4) as usize..((sy + 1) * w * 4) as usize];
            let mut line = Vec::with_capacity((nw * 4) as usize);
            for px in row.chunks_exact(4) {
                for _ in 0..factor {
                    line.extend_from_slice(px);
                }
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

    #[test]
    fn upscaled_matches_block_reference() {
        let (w, h) = (7u32, 5u32);
        let mut bgra = Vec::with_capacity((w * h * 4) as usize);
        for i in 0..(w * h) {
            bgra.extend_from_slice(&[(i % 251) as u8, (i % 7) as u8, (i * 3 % 253) as u8, 255]);
        }
        let frame = Frame {
            width: w,
            height: h,
            bgra,
        };
        for factor in [1u32, 2, 3, 4] {
            let fast = frame.upscaled(factor);
            let expected = reference(&frame, factor);
            assert_eq!(fast.width, expected.width);
            assert_eq!(fast.height, expected.height);
            assert_eq!(fast.bgra, expected.bgra, "factor {factor} differs");
        }
    }

    #[test]
    fn upscaled_handles_empty_frame() {
        let frame = Frame {
            width: 0,
            height: 0,
            bgra: Vec::new(),
        };
        assert!(frame.upscaled(2).bgra.is_empty());
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
