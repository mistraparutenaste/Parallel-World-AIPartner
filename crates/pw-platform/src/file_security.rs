//! Handle-based file opening for security-sensitive imports.

use std::{fs::File, io, path::Path};

/// Opens one regular file below `root` without following a final reparse point.
///
/// On Windows the returned file is the same handle whose normalized final path
/// was checked. Sharing is restricted to reads while the handle remains open.
///
/// # Errors
///
/// Returns an I/O error when either handle cannot be opened, the source is not
/// a regular non-reparse file, or its final path is outside `root`.
pub fn open_contained_read(root: &Path, path: &Path) -> io::Result<File> {
    imp::open_contained_read(root, path)
}

#[cfg(not(windows))]
mod imp {
    use std::{fs, fs::File, io, path::Path};

    pub(super) fn open_contained_read(root: &Path, path: &Path) -> io::Result<File> {
        let root = root.canonicalize()?;
        let metadata = fs::symlink_metadata(path)?;
        if metadata.file_type().is_symlink() || !metadata.file_type().is_file() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "source is not a regular file",
            ));
        }
        let canonical = path.canonicalize()?;
        if !canonical.starts_with(&root) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "source escapes root",
            ));
        }
        File::open(canonical)
    }
}

#[cfg(windows)]
mod imp {
    use std::{
        fs::{File, OpenOptions},
        io,
        os::windows::{
            fs::{MetadataExt, OpenOptionsExt},
            io::AsRawHandle,
        },
        path::{Path, PathBuf},
    };
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
        FILE_NAME_NORMALIZED, FILE_SHARE_READ, GetFinalPathNameByHandleW, VOLUME_NAME_DOS,
    };

    pub(super) fn open_contained_read(root: &Path, path: &Path) -> io::Result<File> {
        let root_handle = OpenOptions::new()
            .access_mode(0)
            .share_mode(FILE_SHARE_READ)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS)
            .open(root)?;
        let file = OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
            .open(path)?;
        let metadata = file.metadata()?;
        if !metadata.file_type().is_file()
            || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "source is not a regular non-reparse file",
            ));
        }
        let root_final = final_path(&root_handle)?;
        let file_final = final_path(&file)?;
        if !file_final.starts_with(&root_final) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "source escapes root",
            ));
        }
        Ok(file)
    }

    #[allow(unsafe_code)]
    fn final_path(file: &File) -> io::Result<PathBuf> {
        let mut buffer = vec![0_u16; 512];
        loop {
            let length = unsafe {
                GetFinalPathNameByHandleW(
                    file.as_raw_handle(),
                    buffer.as_mut_ptr(),
                    u32::try_from(buffer.len()).unwrap_or(u32::MAX),
                    FILE_NAME_NORMALIZED | VOLUME_NAME_DOS,
                )
            };
            if length == 0 {
                return Err(io::Error::last_os_error());
            }
            let length = usize::try_from(length).map_err(|_| io::Error::other("path too long"))?;
            if length < buffer.len() {
                return Ok(PathBuf::from(String::from_utf16_lossy(&buffer[..length])));
            }
            buffer.resize(length.saturating_add(1), 0);
        }
    }
}
