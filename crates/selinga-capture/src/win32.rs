use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BitBlt, CreateCompatibleBitmap, CreateCompatibleDC,
    DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC, GetDIBits, ReleaseDC, SRCCOPY, SelectObject,
};
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext,
};

use super::{CaptureError, Frame, Region, ScreenCapture};

pub struct GdiCapture;

impl GdiCapture {
    pub fn new() -> Result<Self, CaptureError> {
        unsafe {
            let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        }
        Ok(Self)
    }
}

impl ScreenCapture for GdiCapture {
    fn capture_region(&self, region: Region) -> Result<Frame, CaptureError> {
        if region.w == 0 || region.h == 0 {
            return Err(CaptureError::InvalidRegion(region));
        }
        unsafe {
            let hdc_screen = GetDC(None);
            if hdc_screen.is_invalid() {
                return Err(CaptureError::Platform("GetDC failed".to_owned()));
            }

            let result = self.blit_and_read(hdc_screen, region);

            ReleaseDC(None, hdc_screen);
            result
        }
    }
}

impl GdiCapture {
    fn blit_and_read(
        &self,
        hdc_screen: windows::Win32::Graphics::Gdi::HDC,
        region: Region,
    ) -> Result<Frame, CaptureError> {
        unsafe {
            let hdc_mem = CreateCompatibleDC(Some(hdc_screen));
            if hdc_mem.is_invalid() {
                return Err(CaptureError::Platform(
                    "CreateCompatibleDC failed".to_owned(),
                ));
            }

            let read_result = (|| {
                let hbmp = CreateCompatibleBitmap(hdc_screen, region.w as i32, region.h as i32);
                if hbmp.is_invalid() {
                    return Err(CaptureError::Platform(
                        "CreateCompatibleBitmap failed".to_owned(),
                    ));
                }

                let old = SelectObject(hdc_mem, hbmp.into());
                let blitted = BitBlt(
                    hdc_mem,
                    0,
                    0,
                    region.w as i32,
                    region.h as i32,
                    Some(hdc_screen),
                    region.x,
                    region.y,
                    SRCCOPY,
                );
                let restore = SelectObject(hdc_mem, old);

                blitted.map_err(|e| CaptureError::Platform(format!("BitBlt failed: {e}")))?;

                let mut buf = vec![0u8; (region.w as usize) * (region.h as usize) * 4];
                let mut bmi = BITMAPINFO {
                    bmiHeader: BITMAPINFOHEADER {
                        biSize: size_of::<BITMAPINFOHEADER>() as u32,
                        biWidth: region.w as i32,
                        biHeight: -(region.h as i32),
                        biPlanes: 1,
                        biBitCount: 32,
                        biCompression: BI_RGB.0,
                        ..Default::default()
                    },
                    ..Default::default()
                };
                let lines = GetDIBits(
                    hdc_mem,
                    hbmp,
                    0,
                    region.h,
                    Some(buf.as_mut_ptr().cast()),
                    &mut bmi,
                    DIB_RGB_COLORS,
                );
                if lines == 0 {
                    return Err(CaptureError::Platform("GetDIBits failed".to_owned()));
                }

                for px in buf.chunks_exact_mut(4) {
                    px[3] = 0xff;
                }

                let _ = restore;
                let _ = DeleteObject(hbmp.into());
                Ok(Frame {
                    width: region.w,
                    height: region.h,
                    bgra: buf,
                })
            })();

            let _ = DeleteDC(hdc_mem);
            read_result
        }
    }
}
