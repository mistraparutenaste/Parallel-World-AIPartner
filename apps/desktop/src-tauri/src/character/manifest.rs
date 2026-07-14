//! model3.json parsing into a character manifest.

use std::path::{Path, PathBuf};

/// Parsed subset of a model3.json needed to load and control a model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CharacterManifest {
    pub model_path: PathBuf,
    pub expressions: Vec<String>,
    /// Motion group names with their motion counts, in file order.
    pub motion_groups: Vec<(String, u32)>,
}

#[derive(Debug, thiserror::Error)]
pub enum ManifestError {
    #[error("model3.json is not valid JSON: {0}")]
    InvalidJson(#[from] serde_json::Error),
}

/// Parses the given model3.json content.
///
/// # Errors
///
/// Returns [`ManifestError::InvalidJson`] when the content is not
/// valid JSON.
pub fn parse_model3_json(
    model_path: &Path,
    content: &str,
) -> Result<CharacterManifest, ManifestError> {
    let json: serde_json::Value = serde_json::from_str(content)?;
    let file_references = &json["FileReferences"];

    let expressions = file_references["Expressions"]
        .as_array()
        .map(|entries| {
            entries
                .iter()
                .filter_map(|entry| entry["Name"].as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();

    let motion_groups = file_references["Motions"]
        .as_object()
        .map(|groups| {
            groups
                .iter()
                .map(|(name, motions)| {
                    let count = motions.as_array().map_or(0, Vec::len);
                    (name.clone(), u32::try_from(count).unwrap_or(u32::MAX))
                })
                .collect()
        })
        .unwrap_or_default();

    Ok(CharacterManifest {
        model_path: model_path.to_path_buf(),
        expressions,
        motion_groups,
    })
}

/// Finds the first `*.model3.json` under the directory, walking
/// subdirectories in sorted order.
#[must_use]
pub fn find_first_model3(dir: &Path) -> Option<PathBuf> {
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .collect();
    entries.sort();

    for path in &entries {
        if path.is_file()
            && path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.ends_with(".model3.json"))
        {
            return Some(path.clone());
        }
    }
    for path in &entries {
        if path.is_dir()
            && let Some(found) = find_first_model3(path)
        {
            return Some(found);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::{CharacterManifest, find_first_model3, parse_model3_json};

    const MODEL3_JSON: &str = r#"{
        "Version": 3,
        "FileReferences": {
            "Moc": "Epsilon.moc3",
            "Textures": ["Epsilon.2048/texture_00.png"],
            "Physics": "Epsilon.physics3.json",
            "Expressions": [
                { "Name": "Normal", "File": "expressions/Normal.exp3.json" },
                { "Name": "Smile", "File": "expressions/Smile.exp3.json" }
            ],
            "Motions": {
                "Idle": [ { "File": "motion/idle_01.motion3.json" } ],
                "Tap": [
                    { "File": "motion/tap_01.motion3.json" },
                    { "File": "motion/tap_02.motion3.json" }
                ]
            }
        }
    }"#;

    #[test]
    fn parses_expressions_and_motion_groups() {
        let manifest: CharacterManifest = parse_model3_json(
            Path::new("C:/data/characters/eps/Epsilon.model3.json"),
            MODEL3_JSON,
        )
        .unwrap();
        assert_eq!(
            manifest.model_path,
            PathBuf::from("C:/data/characters/eps/Epsilon.model3.json")
        );
        assert_eq!(manifest.expressions, ["Normal", "Smile"]);
        assert_eq!(
            manifest.motion_groups,
            [("Idle".to_owned(), 1), ("Tap".to_owned(), 2)]
        );
    }

    #[test]
    fn parses_models_without_expressions_or_motions() {
        let manifest = parse_model3_json(
            Path::new("m.model3.json"),
            r#"{"FileReferences":{"Moc":"m.moc3"}}"#,
        )
        .unwrap();
        assert!(manifest.expressions.is_empty());
        assert!(manifest.motion_groups.is_empty());
    }

    #[test]
    fn rejects_invalid_json() {
        assert!(parse_model3_json(Path::new("m.model3.json"), "not json").is_err());
    }

    #[test]
    fn finds_the_first_model3_recursively() {
        let root = std::env::temp_dir().join(format!("pw-manifest-test-{}", std::process::id()));
        let nested = root.join("epsilon").join("runtime");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(nested.join("Epsilon.model3.json"), "{}").unwrap();
        std::fs::write(root.join("readme.txt"), "").unwrap();

        let found = find_first_model3(&root).unwrap();
        assert_eq!(found, nested.join("Epsilon.model3.json"));

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn returns_none_when_no_model_exists() {
        let root = std::env::temp_dir().join(format!("pw-manifest-empty-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        assert!(find_first_model3(&root).is_none());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn preserves_recursive_sorted_legacy_discovery_order() {
        let root =
            std::env::temp_dir().join(format!("pw-manifest-order-test-{}", std::process::id()));
        let first = root.join("a").join("First.model3.json");
        let second = root.join("z").join("Second.model3.json");
        std::fs::create_dir_all(first.parent().unwrap()).unwrap();
        std::fs::create_dir_all(second.parent().unwrap()).unwrap();
        std::fs::write(&first, "{}").unwrap();
        std::fs::write(&second, "{}").unwrap();

        assert_eq!(find_first_model3(&root), Some(first));

        std::fs::remove_dir_all(&root).unwrap();
    }
}
