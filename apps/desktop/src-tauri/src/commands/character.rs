//! Character model commands: manifest discovery and expression /
//! motion control routed to the character window.

use std::sync::Mutex;

use pw_contracts::{CharacterManifestDto, MotionGroupDto, SCHEMA_VERSION};
use pw_platform::paths::AppDataLayout;
use tauri::{AppHandle, Emitter, EventTarget, Manager, Runtime, State};

use crate::character::{CharacterManifest, find_first_model3, parse_model3_json};

/// Event delivered to the character window when an expression is set.
pub const EXPRESSION_EVENT: &str = "character-expression";
/// Event delivered to the character window when a motion starts.
pub const MOTION_EVENT: &str = "character-motion";

/// Shared cache of the active character manifest.
#[derive(Default)]
pub struct CharacterState {
    manifest: Mutex<Option<CharacterManifest>>,
}

fn load_manifest(layout: &AppDataLayout) -> Result<CharacterManifest, String> {
    let model_path = find_first_model3(&layout.characters).ok_or_else(|| {
        format!(
            "no character model (*.model3.json) found under {}",
            layout.characters.display()
        )
    })?;
    let content = std::fs::read_to_string(&model_path)
        .map_err(|error| format!("failed to read {}: {error}", model_path.display()))?;
    parse_model3_json(&model_path, &content).map_err(|error| error.to_string())
}

fn to_dto(manifest: &CharacterManifest) -> CharacterManifestDto {
    CharacterManifestDto {
        schema_version: SCHEMA_VERSION,
        model_path: manifest.model_path.to_string_lossy().into_owned(),
        expressions: manifest.expressions.clone(),
        motion_groups: manifest
            .motion_groups
            .iter()
            .map(|(name, motion_count)| MotionGroupDto {
                name: name.clone(),
                motion_count: *motion_count,
            })
            .collect(),
    }
}

fn validate_expression(manifest: &CharacterManifest, name: &str) -> Result<(), String> {
    if manifest.expressions.iter().any(|known| known == name) {
        Ok(())
    } else {
        Err(format!("unknown expression: {name}"))
    }
}

fn validate_motion_group(manifest: &CharacterManifest, group: &str) -> Result<(), String> {
    if manifest
        .motion_groups
        .iter()
        .any(|(known, _)| known == group)
    {
        Ok(())
    } else {
        Err(format!("unknown motion group: {group}"))
    }
}

/// Discovers the active character model and returns its manifest.
///
/// # Errors
///
/// Returns an error message when no model exists or it cannot be read.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)] // tauri commands take owned args
pub fn get_character_manifest(
    layout: State<'_, AppDataLayout>,
    state: State<'_, CharacterState>,
) -> Result<CharacterManifestDto, String> {
    let manifest = load_manifest(&layout)?;
    let dto = to_dto(&manifest);
    *state
        .manifest
        .lock()
        .map_err(|_| "character state is poisoned".to_owned())? = Some(manifest);
    Ok(dto)
}

/// Validates and forwards an expression change to the character window.
///
/// # Errors
///
/// Returns an error message for unknown expressions or when the
/// manifest has not been loaded yet.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)] // tauri commands take owned args
pub fn set_expression<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, CharacterState>,
    name: String,
) -> Result<(), String> {
    let guard = state
        .manifest
        .lock()
        .map_err(|_| "character state is poisoned".to_owned())?;
    let manifest = guard
        .as_ref()
        .ok_or_else(|| "character manifest is not loaded".to_owned())?;
    validate_expression(manifest, &name)?;
    app.emit_to(
        EventTarget::webview_window("character"),
        EXPRESSION_EVENT,
        &name,
    )
    .map_err(|error| error.to_string())
}

/// Validates and forwards a motion request to the character window.
///
/// # Errors
///
/// Returns an error message for unknown motion groups or when the
/// manifest has not been loaded yet.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)] // tauri commands take owned args
pub fn start_motion<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, CharacterState>,
    group: String,
) -> Result<(), String> {
    let guard = state
        .manifest
        .lock()
        .map_err(|_| "character state is poisoned".to_owned())?;
    let manifest = guard
        .as_ref()
        .ok_or_else(|| "character manifest is not loaded".to_owned())?;
    validate_motion_group(manifest, &group)?;
    app.emit_to(
        EventTarget::webview_window("character"),
        MOTION_EVENT,
        &group,
    )
    .map_err(|error| error.to_string())
}

/// Lets clicks pass through the character window (or restores input).
///
/// # Errors
///
/// Returns an error message when the character window is missing or
/// the OS call fails.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)] // tauri commands take owned args
pub fn set_click_through<R: Runtime>(app: AppHandle<R>, enabled: bool) -> Result<(), String> {
    let window = app
        .get_webview_window("character")
        .ok_or_else(|| "character window is not available".to_owned())?;
    window
        .set_ignore_cursor_events(enabled)
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{load_manifest, to_dto, validate_expression, validate_motion_group};
    use crate::character::CharacterManifest;
    use pw_platform::paths::AppDataLayout;

    fn manifest() -> CharacterManifest {
        CharacterManifest {
            model_path: PathBuf::from("C:/data/characters/eps/Epsilon.model3.json"),
            expressions: vec!["Normal".into(), "Smile".into()],
            motion_groups: vec![("Idle".into(), 1), ("Tap".into(), 4)],
        }
    }

    #[test]
    fn maps_manifest_to_versioned_dto() {
        let dto = to_dto(&manifest());
        assert_eq!(dto.schema_version, 1);
        assert_eq!(dto.expressions, ["Normal", "Smile"]);
        assert_eq!(dto.motion_groups[1].name, "Tap");
        assert_eq!(dto.motion_groups[1].motion_count, 4);
    }

    #[test]
    fn rejects_unknown_expression_and_motion_names() {
        let manifest = manifest();
        assert!(validate_expression(&manifest, "Smile").is_ok());
        assert!(validate_expression(&manifest, "Rage").is_err());
        assert!(validate_motion_group(&manifest, "Idle").is_ok());
        assert!(validate_motion_group(&manifest, "Dance").is_err());
    }

    #[test]
    fn load_manifest_reports_missing_models() {
        let root =
            std::env::temp_dir().join(format!("pw-cmd-manifest-test-{}", std::process::id()));
        let layout = AppDataLayout::under(root.clone());
        layout.create_all().unwrap();

        let error = load_manifest(&layout).unwrap_err();
        assert!(error.contains("no character model"));

        std::fs::remove_dir_all(&root).unwrap();
    }
}
