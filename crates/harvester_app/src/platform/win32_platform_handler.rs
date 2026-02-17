use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;

use engine_logging::engine_warn;
use harvester_io::PlatformEffectHandler;
use windows::core::PCWSTR;
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

/// Windows-specific platform handler that uses ShellExecuteW to open URLs in the default browser.
pub struct Win32PlatformHandler;

impl PlatformEffectHandler for Win32PlatformHandler {
    fn open_url(&self, url: &str) {
        engine_logging::engine_info!("[browser] Opening URL: {}", url);

        let operation: Vec<u16> = OsStr::new("open").encode_wide().chain(Some(0)).collect();
        let url_wide: Vec<u16> = OsStr::new(url).encode_wide().chain(Some(0)).collect();

        let result = unsafe {
            ShellExecuteW(
                None,
                PCWSTR(operation.as_ptr()),
                PCWSTR(url_wide.as_ptr()),
                None,
                None,
                SW_SHOWNORMAL,
            )
        };

        if result.0 as isize <= 32 {
            engine_warn!(
                "[browser] ShellExecuteW failed for URL '{}', error code: {}",
                url,
                result.0 as isize
            );
        }
    }
}
