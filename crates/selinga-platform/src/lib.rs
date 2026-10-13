#![deny(missing_docs)]

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
