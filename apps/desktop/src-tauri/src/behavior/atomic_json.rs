use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use pw_platform::diagnostics::atomic_replace;
use serde::Serialize;

static NEXT_TEMP_FILE: AtomicU64 = AtomicU64::new(0);

pub(super) fn write_atomic_json<T: Serialize>(
    config_dir: &Path,
    file_name: &str,
    value: &T,
) -> Result<(), String> {
    fs::create_dir_all(config_dir).map_err(|error| error.to_string())?;
    let destination = config_dir.join(file_name);
    let sequence = NEXT_TEMP_FILE.fetch_add(1, Ordering::Relaxed);
    let temporary = config_dir.join(format!(
        ".{file_name}.{}.{sequence}.tmp",
        std::process::id()
    ));
    let serialized = serde_json::to_vec_pretty(value).map_err(|error| error.to_string())?;

    let result = (|| -> Result<(), String> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(|error| error.to_string())?;
        file.write_all(&serialized)
            .map_err(|error| error.to_string())?;
        file.write_all(b"\n").map_err(|error| error.to_string())?;
        file.sync_all().map_err(|error| error.to_string())?;
        drop(file);
        atomic_replace(&temporary, &destination).map_err(|error| error.to_string())
    })();

    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}
