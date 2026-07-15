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
