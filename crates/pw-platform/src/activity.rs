//! User-scoped protection for persisted activity context.

use thiserror::Error;

/// Protects opaque activity payloads at the platform boundary.
pub trait DataProtector {
    /// Protects `plaintext` for the current operating-system user.
    ///
    /// # Errors
    /// Returns a stable platform error without including the input bytes.
    fn protect(&self, plaintext: &[u8]) -> Result<Vec<u8>, DataProtectionError>;

    /// Restores a payload protected for the current operating-system user.
    ///
    /// # Errors
    /// Returns a stable platform error without including the input bytes.
    fn unprotect(&self, protected: &[u8]) -> Result<Vec<u8>, DataProtectionError>;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct DpapiProtector;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum DataProtectionError {
    #[error("activity data protection is unsupported on this platform")]
    UnsupportedPlatform,
    #[error("activity data protection input is too large")]
    InputTooLarge,
    #[error("activity data protection failed with OS error code {code}")]
    ProtectFailed { code: u32 },
    #[error("activity data unprotection failed with OS error code {code}")]
    UnprotectFailed { code: u32 },
}

/// Minimal foreground context retained by the collector before encryption.
#[derive(Clone, PartialEq, Eq)]
pub struct ForegroundSnapshot {
    pub app_id: String,
    pub title: String,
    pub idle_seconds: u32,
    pub fullscreen: Option<bool>,
}

/// Testable platform boundary for foreground activity sampling.
pub trait ForegroundContextSource {
    /// Returns the current non-self foreground context, or `None` when no
    /// foreground window is available or the desktop itself is foreground.
    ///
    /// # Errors
    /// Returns a stable error which never contains a process path or title.
    fn snapshot(&mut self) -> Result<Option<ForegroundSnapshot>, ForegroundContextError>;
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ForegroundContextError {
    #[error("foreground activity is unsupported on this platform")]
    UnsupportedPlatform,
    #[error("foreground activity process query failed with OS error code {code}")]
    ProcessQueryFailed { code: u32 },
    #[error("foreground activity window changed during sampling")]
    WindowChanged,
}

/// Windows foreground source, with a fail-closed non-Windows implementation.
#[derive(Debug, Clone, Copy)]
pub struct SystemForegroundContextSource {
    self_process_id: u32,
}

impl Default for SystemForegroundContextSource {
    fn default() -> Self {
        Self {
            self_process_id: std::process::id(),
        }
    }
}

impl ForegroundContextSource for SystemForegroundContextSource {
    fn snapshot(&mut self) -> Result<Option<ForegroundSnapshot>, ForegroundContextError> {
        foreground_platform::snapshot(self.self_process_id)
    }
}

const MAX_TITLE_UTF16: usize = 512;

fn bounded_title(units: &[u16], reported_length: i32) -> String {
    let length = usize::try_from(reported_length)
        .unwrap_or(0)
        .min(units.len())
        .min(MAX_TITLE_UTF16);
    String::from_utf16_lossy(&units[..length])
}

const fn is_self_process(process_id: u32, self_process_id: u32) -> bool {
    process_id == self_process_id
}

const fn idle_seconds(current_tick: u32, last_input_tick: u32) -> u32 {
    current_tick.wrapping_sub(last_input_tick) / 1_000
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ScreenRect {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

impl ScreenRect {
    const fn new(left: i32, top: i32, right: i32, bottom: i32) -> Self {
        Self {
            left,
            top,
            right,
            bottom,
        }
    }
}

const fn fullscreen_from_rects(
    client: Option<ScreenRect>,
    monitor: Option<ScreenRect>,
) -> Option<bool> {
    match (client, monitor) {
        (Some(client), Some(monitor)) => Some(
            client.left == monitor.left
                && client.top == monitor.top
                && client.right == monitor.right
                && client.bottom == monitor.bottom,
        ),
        _ => None,
    }
}

#[cfg(windows)]
mod foreground_platform {
    use std::path::Path;
    use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, HANDLE, POINT, RECT};
    use windows_sys::Win32::Graphics::Gdi::{
        ClientToScreen, GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow,
    };
    use windows_sys::Win32::System::SystemInformation::GetTickCount;
    use windows_sys::Win32::System::Threading::{
        OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
    };
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetClientRect, GetForegroundWindow, GetWindowTextW, GetWindowThreadProcessId,
    };

    use super::{
        ForegroundContextError, ForegroundSnapshot, MAX_TITLE_UTF16, ScreenRect, bounded_title,
        fullscreen_from_rects, idle_seconds, is_self_process,
    };

    const MAX_PROCESS_PATH_UTF16: usize = 32_768;

    struct ProcessHandle(HANDLE);

    impl Drop for ProcessHandle {
        #[allow(unsafe_code)]
        fn drop(&mut self) {
            // SAFETY: The non-null handle is owned by this guard and closed once.
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }

    #[allow(unsafe_code)]
    pub(super) fn snapshot(
        self_process_id: u32,
    ) -> Result<Option<ForegroundSnapshot>, ForegroundContextError> {
        // SAFETY: This call has no parameters and may validly return null.
        let window = unsafe { GetForegroundWindow() };
        if window.is_null() {
            return Ok(None);
        }

        let mut first_process_id = 0;
        // SAFETY: `first_process_id` is a valid writable output pointer.
        if unsafe { GetWindowThreadProcessId(window, &raw mut first_process_id) } == 0 {
            return Ok(None);
        }
        if first_process_id == 0 || is_self_process(first_process_id, self_process_id) {
            return Ok(None);
        }

        let app_id = read_app_id(first_process_id)?;
        let title = read_title(window);
        let idle_seconds = read_idle_seconds();
        let fullscreen = read_fullscreen(window);

        let mut second_process_id = 0;
        // SAFETY: `second_process_id` is a valid writable output pointer.
        if unsafe { GetForegroundWindow() } != window
            || unsafe { GetWindowThreadProcessId(window, &raw mut second_process_id) } == 0
            || second_process_id != first_process_id
        {
            return Err(ForegroundContextError::WindowChanged);
        }

        Ok(Some(ForegroundSnapshot {
            app_id,
            title,
            idle_seconds,
            fullscreen,
        }))
    }

    #[allow(unsafe_code)]
    fn read_title(window: windows_sys::Win32::Foundation::HWND) -> String {
        let mut buffer = [0_u16; MAX_TITLE_UTF16 + 1];
        let capacity = i32::try_from(buffer.len()).expect("bounded title capacity fits i32");
        // SAFETY: The fixed buffer is writable for the supplied capacity.
        let length = unsafe { GetWindowTextW(window, buffer.as_mut_ptr(), capacity) };
        bounded_title(&buffer, length)
    }

    #[allow(unsafe_code)]
    fn read_app_id(process_id: u32) -> Result<String, ForegroundContextError> {
        // SAFETY: The requested access is query-limited, the handle is not inherited,
        // and `process_id` was returned by the foreground window query.
        let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, process_id) };
        if handle.is_null() {
            // SAFETY: This immediately follows the failed Win32 call.
            return Err(ForegroundContextError::ProcessQueryFailed {
                code: unsafe { GetLastError() },
            });
        }
        let handle = ProcessHandle(handle);
        let mut path = vec![0_u16; MAX_PROCESS_PATH_UTF16];
        let mut length = u32::try_from(path.len()).expect("process path capacity fits u32");
        // SAFETY: The process handle has query-limited access and the output
        // buffer/capacity pair remains valid for the call.
        if unsafe { QueryFullProcessImageNameW(handle.0, 0, path.as_mut_ptr(), &raw mut length) }
            == 0
        {
            // SAFETY: This immediately follows the failed Win32 call.
            return Err(ForegroundContextError::ProcessQueryFailed {
                code: unsafe { GetLastError() },
            });
        }
        let path = String::from_utf16_lossy(&path[..length as usize]);
        let app_id = Path::new(&path)
            .file_name()
            .and_then(|value| value.to_str())
            .filter(|value| !value.is_empty())
            .ok_or(ForegroundContextError::ProcessQueryFailed { code: 0 })?;
        Ok(app_id.to_ascii_lowercase())
    }

    #[allow(unsafe_code)]
    fn read_idle_seconds() -> u32 {
        let mut input = LASTINPUTINFO {
            cbSize: u32::try_from(std::mem::size_of::<LASTINPUTINFO>())
                .expect("LASTINPUTINFO size fits u32"),
            dwTime: 0,
        };
        // SAFETY: `input` has the required size and is writable.
        if unsafe { GetLastInputInfo(&raw mut input) } == 0 {
            return 0;
        }
        // SAFETY: GetTickCount takes no parameters.
        idle_seconds(unsafe { GetTickCount() }, input.dwTime)
    }

    #[allow(unsafe_code)]
    fn read_fullscreen(window: windows_sys::Win32::Foundation::HWND) -> Option<bool> {
        let client = client_screen_rect(window);
        // SAFETY: The window handle was returned by GetForegroundWindow.
        let monitor = unsafe { MonitorFromWindow(window, MONITOR_DEFAULTTONEAREST) };
        let monitor = if monitor.is_null() {
            None
        } else {
            let mut info = MONITORINFO {
                cbSize: u32::try_from(std::mem::size_of::<MONITORINFO>())
                    .expect("MONITORINFO size fits u32"),
                rcMonitor: RECT::default(),
                rcWork: RECT::default(),
                dwFlags: 0,
            };
            // SAFETY: `info` has the required size and is writable.
            (unsafe { GetMonitorInfoW(monitor, &raw mut info) } != 0).then(|| {
                ScreenRect::new(
                    info.rcMonitor.left,
                    info.rcMonitor.top,
                    info.rcMonitor.right,
                    info.rcMonitor.bottom,
                )
            })
        };
        fullscreen_from_rects(client, monitor)
    }

    #[allow(unsafe_code)]
    fn client_screen_rect(window: windows_sys::Win32::Foundation::HWND) -> Option<ScreenRect> {
        let mut rect = RECT::default();
        // SAFETY: `rect` is writable and `window` is the sampled foreground handle.
        if unsafe { GetClientRect(window, &raw mut rect) } == 0 {
            return None;
        }
        let mut top_left = POINT {
            x: rect.left,
            y: rect.top,
        };
        let mut bottom_right = POINT {
            x: rect.right,
            y: rect.bottom,
        };
        // SAFETY: Both points are valid writable values for this window.
        if unsafe { ClientToScreen(window, &raw mut top_left) } == 0
            || unsafe { ClientToScreen(window, &raw mut bottom_right) } == 0
        {
            return None;
        }
        Some(ScreenRect::new(
            top_left.x,
            top_left.y,
            bottom_right.x,
            bottom_right.y,
        ))
    }
}

#[cfg(not(windows))]
mod foreground_platform {
    use super::{ForegroundContextError, ForegroundSnapshot};

    pub(super) fn snapshot(
        _self_process_id: u32,
    ) -> Result<Option<ForegroundSnapshot>, ForegroundContextError> {
        Err(ForegroundContextError::UnsupportedPlatform)
    }
}

impl DataProtector for DpapiProtector {
    fn protect(&self, plaintext: &[u8]) -> Result<Vec<u8>, DataProtectionError> {
        platform::protect(plaintext)
    }

    fn unprotect(&self, protected: &[u8]) -> Result<Vec<u8>, DataProtectionError> {
        platform::unprotect(protected)
    }
}

#[cfg(windows)]
mod platform {
    use std::ptr;
    use std::sync::atomic::{Ordering, compiler_fence};

    use windows_sys::Win32::Foundation::{GetLastError, LocalFree};
    use windows_sys::Win32::Security::Cryptography::{
        CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData,
    };

    use super::DataProtectionError;

    static EMPTY_DPAPI_INPUT: u8 = 0;

    struct DpapiBuffer {
        pointer: *mut u8,
        length: usize,
    }

    impl DpapiBuffer {
        fn from_blob(blob: CRYPT_INTEGER_BLOB) -> Self {
            Self {
                pointer: blob.pbData,
                length: blob.cbData as usize,
            }
        }

        #[allow(unsafe_code)]
        fn copy_to_vec(&self) -> Vec<u8> {
            if self.length == 0 {
                return Vec::new();
            }
            // SAFETY: A successful DPAPI call returns a buffer valid for exactly
            // `cbData` bytes. This guard owns it until `Drop` releases it.
            unsafe { std::slice::from_raw_parts(self.pointer, self.length).to_vec() }
        }
    }

    impl Drop for DpapiBuffer {
        fn drop(&mut self) {
            zero_and_local_free(self.pointer, self.length);
        }
    }

    /// Overwrites a DPAPI-owned buffer with volatile stores before releasing it.
    ///
    /// The compiler fence prevents the stores from moving past `LocalFree`.
    #[allow(unsafe_code)]
    fn zero_and_local_free(pointer: *mut u8, length: usize) {
        if pointer.is_null() {
            return;
        }
        // SAFETY: `pointer` and `length` come directly from a DPAPI output blob.
        // DPAPI allocates this region with LocalAlloc and transfers ownership to
        // the caller, which must release it exactly once with LocalFree.
        unsafe {
            for offset in 0..length {
                ptr::write_volatile(pointer.add(offset), 0);
            }
            compiler_fence(Ordering::SeqCst);
            let _ = LocalFree(pointer.cast());
        }
    }

    fn input_blob(input: &[u8]) -> Result<CRYPT_INTEGER_BLOB, DataProtectionError> {
        let cb_data = u32::try_from(input.len()).map_err(|_| DataProtectionError::InputTooLarge)?;
        Ok(CRYPT_INTEGER_BLOB {
            cbData: cb_data,
            pbData: if input.is_empty() {
                (&raw const EMPTY_DPAPI_INPUT).cast_mut()
            } else {
                input.as_ptr().cast_mut()
            },
        })
    }

    #[allow(unsafe_code)]
    pub(super) fn protect(plaintext: &[u8]) -> Result<Vec<u8>, DataProtectionError> {
        let input = input_blob(plaintext)?;
        let mut output = CRYPT_INTEGER_BLOB::default();
        // SAFETY: All input pointers remain valid for the duration of the call,
        // optional parameters are null as permitted, and `output` is writable.
        // Only UI-forbidden is set, so protection remains current-user scoped.
        let succeeded = unsafe {
            CryptProtectData(
                &raw const input,
                ptr::null(),
                ptr::null(),
                ptr::null(),
                ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &raw mut output,
            )
        };
        let error_code = if succeeded == 0 {
            // SAFETY: This immediately follows the failed Win32 call.
            Some(unsafe { GetLastError() })
        } else {
            None
        };
        let output = DpapiBuffer::from_blob(output);
        if let Some(code) = error_code {
            return Err(DataProtectionError::ProtectFailed { code });
        }
        Ok(output.copy_to_vec())
    }

    #[allow(unsafe_code)]
    pub(super) fn unprotect(protected: &[u8]) -> Result<Vec<u8>, DataProtectionError> {
        let input = input_blob(protected)?;
        let mut output = CRYPT_INTEGER_BLOB::default();
        // SAFETY: All input pointers remain valid for the duration of the call,
        // no description is requested, optional parameters are null as
        // permitted, and `output` is writable.
        let succeeded = unsafe {
            CryptUnprotectData(
                &raw const input,
                ptr::null_mut(),
                ptr::null(),
                ptr::null(),
                ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &raw mut output,
            )
        };
        let error_code = if succeeded == 0 {
            // SAFETY: This immediately follows the failed Win32 call.
            Some(unsafe { GetLastError() })
        } else {
            None
        };
        let output = DpapiBuffer::from_blob(output);
        if let Some(code) = error_code {
            return Err(DataProtectionError::UnprotectFailed { code });
        }
        Ok(output.copy_to_vec())
    }
}

#[cfg(test)]
mod foreground_helper_tests {
    use super::{ScreenRect, bounded_title, fullscreen_from_rects, idle_seconds, is_self_process};

    #[test]
    fn activity_title_conversion_is_bounded_to_512_utf16_units() {
        let title = vec![u16::from(b'a'); 600];
        assert_eq!(bounded_title(&title, 600).encode_utf16().count(), 512);
        assert_eq!(bounded_title(&title, -1), "");
    }

    #[test]
    fn activity_pid_self_exclusion_is_explicit() {
        assert!(is_self_process(42, 42));
        assert!(!is_self_process(42, 43));
    }

    #[test]
    fn activity_idle_math_wraps_with_the_win32_tick_counter() {
        assert_eq!(idle_seconds(3_000, 1_000), 2);
        assert_eq!(idle_seconds(500, u32::MAX - 499), 1);
    }

    #[test]
    fn activity_fullscreen_is_unknown_when_any_required_rectangle_is_missing() {
        let rect = ScreenRect::new(0, 0, 1920, 1080);
        assert_eq!(fullscreen_from_rects(None, Some(rect)), None);
        assert_eq!(fullscreen_from_rects(Some(rect), None), None);
        assert_eq!(fullscreen_from_rects(Some(rect), Some(rect)), Some(true));
        assert_eq!(
            fullscreen_from_rects(Some(ScreenRect::new(0, 0, 1280, 720)), Some(rect)),
            Some(false)
        );
    }
}

#[cfg(not(windows))]
mod platform {
    use super::DataProtectionError;

    pub(super) fn protect(_plaintext: &[u8]) -> Result<Vec<u8>, DataProtectionError> {
        Err(DataProtectionError::UnsupportedPlatform)
    }

    pub(super) fn unprotect(_protected: &[u8]) -> Result<Vec<u8>, DataProtectionError> {
        Err(DataProtectionError::UnsupportedPlatform)
    }
}
