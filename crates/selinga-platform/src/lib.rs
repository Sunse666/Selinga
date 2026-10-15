pub fn exclude_self_from_capture() -> usize {
    #[cfg(windows)]
    {
        use windows::Win32::Foundation::{HWND, LPARAM};
        use windows::Win32::System::Threading::GetCurrentProcessId;
        use windows::Win32::UI::WindowsAndMessaging::{
            EnumWindows, GetWindowThreadProcessId, SetWindowDisplayAffinity, WDA_EXCLUDEFROMCAPTURE,
        };

        unsafe extern "system" fn on_window(hwnd: HWND, lparam: LPARAM) -> windows::core::BOOL {
            unsafe {
                let mut pid = 0u32;
                GetWindowThreadProcessId(hwnd, Some(&mut pid));
                if pid == GetCurrentProcessId() {
                    let _ = SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE);
                    let counter = lparam.0 as *mut usize;
                    *counter += 1;
                }
                true.into()
            }
        }

        unsafe {
            let mut count = 0usize;
            let lparam = LPARAM(&mut count as *mut usize as isize);
            let _ = EnumWindows(Some(on_window), lparam);
            count
        }
    }

    #[cfg(not(windows))]
    {
        0
    }
}

pub fn enable_utf8_console() {
    #[cfg(windows)]
    {
        use windows::Win32::System::Console::{GetConsoleOutputCP, SetConsoleOutputCP};
        const CP_UTF8: u32 = 65001;
        unsafe {
            if GetConsoleOutputCP() != CP_UTF8 {
                let _ = SetConsoleOutputCP(CP_UTF8);
            }
        }
    }
}

pub fn primary_screen_points() -> (f32, f32) {
    #[cfg(windows)]
    unsafe {
        use windows::Win32::UI::HiDpi::GetDpiForSystem;
        use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXSCREEN, SM_CYSCREEN};
        let scale = (GetDpiForSystem() as f32 / 96.0).max(1.0);
        (
            GetSystemMetrics(SM_CXSCREEN) as f32 / scale,
            GetSystemMetrics(SM_CYSCREEN) as f32 / scale,
        )
    }

    #[cfg(not(windows))]
    {
        (1280.0, 720.0)
    }
}

pub fn primary_screen_px() -> (u32, u32) {
    #[cfg(windows)]
    unsafe {
        use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXSCREEN, SM_CYSCREEN};
        (
            GetSystemMetrics(SM_CXSCREEN).max(0) as u32,
            GetSystemMetrics(SM_CYSCREEN).max(0) as u32,
        )
    }

    #[cfg(not(windows))]
    {
        (1280, 720)
    }
}
