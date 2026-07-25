//! 設定ファイル(JSON)を耐障害性のある方法で永続化するための共有ヘルパー。
//!
//! 「一時ファイルへ書く → `fsync` する → [`crate::diagnostics::atomic_replace`]
//! で置き換える → 失敗時は一時ファイルを削除する」という手順を1箇所にまとめ、
//! アプリ側に散らばっていた同種の実装を統合するためのモジュール。
//! アトミックな置換そのものは `diagnostics` モジュールが公開する
//! `atomic_replace`(Windows では監査済みの `MoveFileExW` 呼び出し)を再利用する。

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::diagnostics::atomic_replace;

static NEXT_TEMP_FILE: AtomicU64 = AtomicU64::new(0);

/// 書き込み時のJSON整形方針。
///
/// 呼び出し元ごとにディスク上の表現が異なるため、バリアントとして明示する。
/// 既存ファイルのオンディスク表現(pretty/非pretty・末尾改行の有無)を変えない
/// ことが本モジュール導入の前提条件。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JsonFormat {
    /// `serde_json::to_vec_pretty` の出力に末尾改行を1つ付加する。
    PrettyWithTrailingNewline,
    /// `serde_json::to_vec_pretty` の出力をそのまま書く(末尾改行なし)。
    Pretty,
    /// `serde_json::to_vec`(非pretty)の出力をそのまま書く(末尾改行なし)。
    Compact,
}

impl JsonFormat {
    fn serialize<T: Serialize>(self, value: &T) -> io::Result<Vec<u8>> {
        let mut bytes = match self {
            Self::PrettyWithTrailingNewline | Self::Pretty => {
                serde_json::to_vec_pretty(value).map_err(io::Error::other)?
            }
            Self::Compact => serde_json::to_vec(value).map_err(io::Error::other)?,
        };
        if self == Self::PrettyWithTrailingNewline {
            bytes.push(b'\n');
        }
        Ok(bytes)
    }
}

/// JSON設定ファイルの読み込みエラー。`_checked` 系の呼び出し元が
/// I/O失敗とパース失敗を区別してマッピングできるように分離する。
#[derive(Debug)]
pub enum ReadJsonError {
    Io(io::Error),
    Parse(serde_json::Error),
}

impl std::fmt::Display for ReadJsonError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => error.fmt(f),
            Self::Parse(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for ReadJsonError {}

/// `path` をJSONとして読み込む。ファイルが存在しない場合は `Ok(None)`。
///
/// # Errors
///
/// 読み込みに失敗した場合は [`ReadJsonError::Io`]、JSONとして不正な場合は
/// [`ReadJsonError::Parse`] を返す。
pub fn read_json<T: DeserializeOwned>(path: &Path) -> Result<Option<T>, ReadJsonError> {
    let raw = match fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(ReadJsonError::Io(error)),
    };
    serde_json::from_str(&raw)
        .map(Some)
        .map_err(ReadJsonError::Parse)
}

/// 「読めなければデフォルトへフォールバック」する呼び出し元向けの寛容版。
/// ファイルの欠落は静かに、読解失敗は警告ログを残して `None` を返す。
#[must_use]
pub fn read_json_lenient<T: DeserializeOwned>(path: &Path) -> Option<T> {
    match read_json(path) {
        Ok(value) => value,
        Err(error) => {
            tracing::warn!(%error, path = %path.display(), "invalid config file; using defaults");
            None
        }
    }
}

/// `directory` を作成した上で `directory/file_name` を `value` の内容で
/// アトミックに置き換える。
///
/// 手順: `.{file_name}.{pid}.{連番}.tmp` という衝突しない名前の一時ファイルを
/// `create_new` で作成し、書き込み後に `fsync` してから
/// [`atomic_replace`] で宛先へ置き換える。途中で失敗した場合は一時ファイルを
/// 削除し、宛先は変更しない。
///
/// # Errors
///
/// ディレクトリの作成、シリアライズ、一時ファイルへの書き込み、または
/// アトミックな置換のいずれかに失敗した場合、I/Oエラーを返す。
pub fn write_atomic_json<T: Serialize>(
    directory: &Path,
    file_name: &str,
    value: &T,
    format: JsonFormat,
) -> io::Result<()> {
    fs::create_dir_all(directory)?;
    write_atomic_json_at(&directory.join(file_name), value, format)
}

/// 完全な宛先パスを受け取る版。呼び出し元がディレクトリ名とファイル名を
/// 分けて持っていない場合(完全パスしか持たない場合)に使う。
///
/// 親ディレクトリの作成は行わない。存在確認は呼び出し元の責務とする。
///
/// # Errors
///
/// シリアライズ、一時ファイルへの書き込み、またはアトミックな置換に
/// 失敗した場合、I/Oエラーを返す。
pub fn write_atomic_json_at<T: Serialize>(
    destination: &Path,
    value: &T,
    format: JsonFormat,
) -> io::Result<()> {
    let serialized = format.serialize(value)?;
    let temporary = temp_path(destination);

    let result = write_and_replace(&temporary, destination, &serialized);
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn write_and_replace(temporary: &Path, destination: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(temporary)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    atomic_replace(temporary, destination)
}

/// `.{file_name}.{pid}.{連番}.tmp` という、プロセスIDとプロセス内アトミック
/// カウンタで衝突を避ける一時ファイル名を組み立てる。
fn temp_path(destination: &Path) -> PathBuf {
    let file_name = destination
        .file_name()
        .map_or_else(String::new, |name| name.to_string_lossy().into_owned());
    let sequence = NEXT_TEMP_FILE.fetch_add(1, Ordering::Relaxed);
    destination.with_file_name(format!(
        ".{file_name}.{}.{sequence}.tmp",
        std::process::id()
    ))
}

#[cfg(test)]
mod tests {
    use super::{
        JsonFormat, ReadJsonError, read_json, read_json_lenient, write_atomic_json,
        write_atomic_json_at,
    };
    use serde_json::json;

    fn temp_root(name: &str) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!("pw-config-io-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn read_json_distinguishes_missing_invalid_and_valid_files() {
        let root = temp_root("read-json");
        let path = root.join("value.json");

        assert!(matches!(read_json::<serde_json::Value>(&path), Ok(None)));

        std::fs::write(&path, "{not json").unwrap();
        assert!(matches!(
            read_json::<serde_json::Value>(&path),
            Err(ReadJsonError::Parse(_))
        ));
        assert_eq!(read_json_lenient::<serde_json::Value>(&path), None);

        std::fs::write(&path, r#"{"a":1}"#).unwrap();
        assert_eq!(
            read_json::<serde_json::Value>(&path).unwrap(),
            Some(json!({"a": 1}))
        );
        assert_eq!(
            read_json_lenient::<serde_json::Value>(&path),
            Some(json!({"a": 1}))
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn pretty_with_trailing_newline_matches_the_legacy_on_disk_shape() {
        let root = temp_root("format-pretty-newline");
        write_atomic_json(
            &root,
            "value.json",
            &json!({"a": 1}),
            JsonFormat::PrettyWithTrailingNewline,
        )
        .unwrap();

        let bytes = std::fs::read(root.join("value.json")).unwrap();
        let mut expected = serde_json::to_vec_pretty(&json!({"a": 1})).unwrap();
        expected.push(b'\n');
        assert_eq!(bytes, expected);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn pretty_without_trailing_newline_matches_to_string_pretty() {
        let root = temp_root("format-pretty");
        write_atomic_json(&root, "value.json", &json!({"a": 1}), JsonFormat::Pretty).unwrap();

        let bytes = std::fs::read(root.join("value.json")).unwrap();
        assert_eq!(bytes, serde_json::to_vec_pretty(&json!({"a": 1})).unwrap());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn compact_has_no_trailing_newline_and_no_indentation() {
        let root = temp_root("format-compact");
        write_atomic_json(&root, "value.json", &json!({"a": 1}), JsonFormat::Compact).unwrap();

        let bytes = std::fs::read(root.join("value.json")).unwrap();
        assert_eq!(bytes, serde_json::to_vec(&json!({"a": 1})).unwrap());
        assert!(!bytes.ends_with(b"\n"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn overwriting_an_existing_file_fully_replaces_its_content() {
        let root = temp_root("overwrite");
        let large = json!({"payload": "x".repeat(4096), "tag": "first"});
        write_atomic_json(&root, "value.json", &large, JsonFormat::Pretty).unwrap();

        let small = json!({"tag": "second"});
        write_atomic_json(&root, "value.json", &small, JsonFormat::Pretty).unwrap();

        let bytes = std::fs::read(root.join("value.json")).unwrap();
        assert_eq!(bytes, serde_json::to_vec_pretty(&small).unwrap());
        let decoded: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(decoded, small);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn failed_write_leaves_no_temporary_file_behind() {
        let root = temp_root("write-failure");
        // 宛先をディレクトリにしておくと、アトミックな置換(ファイルへの
        // rename/MoveFileExW)は必ず失敗する。
        let destination = root.join("destination-is-a-directory.json");
        std::fs::create_dir_all(&destination).unwrap();

        let result = write_atomic_json_at(&destination, &json!({"a": 1}), JsonFormat::Pretty);

        assert!(result.is_err());
        let leftover_tmp = std::fs::read_dir(&root)
            .unwrap()
            .filter_map(Result::ok)
            .any(|entry| entry.file_name().to_string_lossy().contains(".tmp"));
        assert!(!leftover_tmp, "failed write must not leave a temp file");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn concurrent_writes_to_the_same_destination_never_collide_on_temp_names() {
        // 検証したいのは「一時ファイル名が衝突しないこと」であって、
        // 「同一の宛先ファイルへの並行な `atomic_replace` が必ず成功すること」
        // ではない。後者はWindowsのファイルシステムの都合で、置換の瞬間に
        // 別スレッドの置換と鉢合わせると一時的に `ACCESS_DENIED` になり得る
        // (これは本ヘルパーの一時ファイル命名とは無関係な、OS側の競合)。
        // 一時ファイル名が衝突した場合は `create_new` が
        // `ErrorKind::AlreadyExists` を返すので、それが起きていないことを
        // 確認する。
        let root = std::sync::Arc::new(temp_root("concurrent-same-file"));
        let threads = (0..32)
            .map(|index| {
                let root = std::sync::Arc::clone(&root);
                std::thread::spawn(move || {
                    write_atomic_json(
                        &root,
                        "shared.json",
                        &json!({"writer": index}),
                        JsonFormat::Pretty,
                    )
                })
            })
            .collect::<Vec<_>>();

        let mut successes = 0_u32;
        for thread in threads {
            match thread.join().unwrap() {
                Ok(()) => successes += 1,
                Err(error) => assert_ne!(
                    error.kind(),
                    std::io::ErrorKind::AlreadyExists,
                    "a temporary-file name must never collide with another writer's"
                ),
            }
        }
        assert!(successes > 0, "at least one concurrent writer must win");

        let leftover_tmp = std::fs::read_dir(root.as_path())
            .unwrap()
            .filter_map(Result::ok)
            .any(|entry| entry.file_name().to_string_lossy().contains(".tmp"));
        assert!(!leftover_tmp, "a failed writer must clean up its temp file");
        let decoded: serde_json::Value =
            serde_json::from_slice(&std::fs::read(root.join("shared.json")).unwrap()).unwrap();
        assert!(decoded["writer"].is_number());
        let _ = std::fs::remove_dir_all(root.as_path());
    }

    #[test]
    fn concurrent_writes_to_different_destinations_all_succeed_independently() {
        let root = std::sync::Arc::new(temp_root("concurrent-different-files"));
        let threads = (0..16)
            .map(|index| {
                let root = std::sync::Arc::clone(&root);
                std::thread::spawn(move || {
                    write_atomic_json(
                        &root,
                        &format!("file-{index}.json"),
                        &json!({"writer": index}),
                        JsonFormat::Pretty,
                    )
                })
            })
            .collect::<Vec<_>>();

        for thread in threads {
            thread.join().unwrap().expect("write must succeed");
        }

        for index in 0..16 {
            let decoded: serde_json::Value = serde_json::from_slice(
                &std::fs::read(root.join(format!("file-{index}.json"))).unwrap(),
            )
            .unwrap();
            assert_eq!(decoded["writer"], index);
        }
        let _ = std::fs::remove_dir_all(root.as_path());
    }
}
