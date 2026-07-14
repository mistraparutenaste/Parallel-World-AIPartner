//! Character model commands: manifest discovery and expression /
//! motion control routed to the character window.

use std::sync::Mutex;

use pw_contracts::{
    CHARACTER_MANIFEST_SCHEMA_VERSION, CHARACTER_SETTINGS_CHANGED_EVENT,
    CHARACTER_SETTINGS_SCHEMA_VERSION, CharacterManifestDto, CharacterRendererDto,
    CharacterSettingsChangedEventDto, CharacterSettingsDto, StaticExpressionDto,
};
use pw_platform::paths::AppDataLayout;
use tauri::{AppHandle, Emitter, EventTarget, Manager, Runtime, State};

use crate::character::{
    CharacterCapabilities, CharacterCatalog, ResolvedCharacter, ResolvedRenderer,
    load_character_settings, save_character_settings, with_expression_idle_timeout,
};

/// Event delivered to the character window when an expression is set.
pub const EXPRESSION_EVENT: &str = "character-expression";
/// Event delivered to the character window when a motion starts.
pub const MOTION_EVENT: &str = "character-motion";

/// Shared cache of the active character manifest.
#[derive(Default)]
pub struct CharacterState {
    manifest: Mutex<Option<ResolvedCharacter>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CharacterControlContext {
    pub renderer: &'static str,
    pub capabilities: CharacterCapabilities,
}

pub(crate) trait CharacterWindows {
    fn show_chat(&mut self) -> Result<(), String>;
    fn hide_character(&mut self) -> Result<(), String>;
    fn show_character(&mut self) -> Result<(), String>;
}

impl<R: Runtime> CharacterWindows for &AppHandle<R> {
    fn show_chat(&mut self) -> Result<(), String> {
        let layout = self.state::<AppDataLayout>();
        let placement = crate::ui::load_preferences(&layout).chat_placement;
        let label = match placement {
            pw_contracts::ChatPlacementDto::Docked => "settings",
            pw_contracts::ChatPlacementDto::Popped => "chat",
        };
        let window = self
            .get_webview_window(label)
            .ok_or_else(|| format!("{label} window is not available"))?;
        window.show().map_err(|error| error.to_string())?;
        window.set_focus().map_err(|error| error.to_string())?;
        if label == "settings" {
            let _ = self.emit_to(
                EventTarget::webview_window("settings"),
                "control-center-navigate",
                "conversation",
            );
        }
        Ok(())
    }
    fn hide_character(&mut self) -> Result<(), String> {
        self.get_webview_window("character")
            .ok_or_else(|| "character window is not available".to_owned())?
            .hide()
            .map_err(|error| error.to_string())
    }
    fn show_character(&mut self) -> Result<(), String> {
        self.get_webview_window("character")
            .ok_or_else(|| "character window is not available".to_owned())?
            .show()
            .map_err(|error| error.to_string())
    }
}

pub(crate) fn apply_character_renderer_window_mode(
    windows: &mut impl CharacterWindows,
    available: bool,
) -> Result<(), String> {
    if available {
        windows.show_character()
    } else {
        windows.show_chat()?;
        windows.hide_character()
    }
}

impl CharacterState {
    /// Expression and motion-group names of the loaded model, when a
    /// manifest has been fetched.
    #[must_use]
    pub fn manifest_summary(&self) -> Option<CharacterCapabilities> {
        let guard = self.manifest.lock().ok()?;
        Some(guard.as_ref()?.capabilities())
    }

    pub(crate) fn cache_manifest(&self, manifest: ResolvedCharacter) -> Result<(), String> {
        *self
            .manifest
            .lock()
            .map_err(|_| "character state is poisoned".to_owned())? = Some(manifest);
        Ok(())
    }

    #[must_use]
    pub(crate) fn control_context(&self) -> Option<CharacterControlContext> {
        let guard = self.manifest.lock().ok()?;
        let manifest = guard.as_ref()?;
        let renderer = match manifest.renderer {
            ResolvedRenderer::Live2d { .. } => "live2d",
            ResolvedRenderer::StaticImage { .. } => "static_image",
        };
        Some(CharacterControlContext {
            renderer,
            capabilities: manifest.capabilities(),
        })
    }
}

fn load_manifest(layout: &AppDataLayout) -> Result<ResolvedCharacter, String> {
    let settings = load_character_settings(layout);
    CharacterCatalog::discover(layout)
        .and_then(|catalog| catalog.resolve(&settings))
        .map_err(|error| error.to_ipc_error())
}

fn to_dto(manifest: &ResolvedCharacter) -> CharacterManifestDto {
    CharacterManifestDto {
        schema_version: CHARACTER_MANIFEST_SCHEMA_VERSION,
        id: manifest.id.clone(),
        display_name: manifest.display_name.clone(),
        renderer: match &manifest.renderer {
            ResolvedRenderer::Live2d {
                model_path,
                default_expression,
                expressions,
                motion_groups,
            } => CharacterRendererDto::Live2d {
                model_path: model_path.to_string_lossy().into_owned(),
                default_expression: default_expression.clone(),
                expressions: expressions.clone(),
                motion_groups: motion_groups.clone(),
            },
            ResolvedRenderer::StaticImage {
                default_expression,
                expressions,
                width,
                height,
            } => CharacterRendererDto::StaticImage {
                default_expression: default_expression.clone(),
                expressions: expressions
                    .iter()
                    .map(|expression| StaticExpressionDto {
                        name: expression.name.clone(),
                        image_path: expression.image_path.to_string_lossy().into_owned(),
                    })
                    .collect(),
                width: *width,
                height: *height,
            },
        },
    }
}

fn validate_expression(manifest: &ResolvedCharacter, name: &str) -> Result<(), String> {
    if manifest
        .capabilities()
        .expressions
        .iter()
        .any(|known| known == name)
    {
        Ok(())
    } else {
        Err(format!("unknown expression: {name}"))
    }
}

fn validate_motion_group(manifest: &ResolvedCharacter, group: &str) -> Result<(), String> {
    match &manifest.renderer {
        ResolvedRenderer::StaticImage { .. } => {
            Err("motion is unsupported for the static_image renderer".into())
        }
        ResolvedRenderer::Live2d { motion_groups, .. } => {
            if motion_groups.iter().any(|known| known.name == group) {
                Ok(())
            } else {
                Err(format!("unknown motion group: {group}"))
            }
        }
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
    state.cache_manifest(manifest)?;
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

/// Returns global character behavior settings.
#[tauri::command]
#[must_use]
#[allow(clippy::needless_pass_by_value)] // tauri commands take owned args
pub fn get_character_settings(layout: State<'_, AppDataLayout>) -> CharacterSettingsDto {
    load_character_settings(&layout)
}

/// Updates the global expression idle timeout and notifies only the
/// character `WebView`.
///
/// # Errors
///
/// Returns a validation, persistence, or event-delivery error.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)] // tauri commands take owned args
pub fn set_expression_idle_timeout<R: Runtime>(
    app: AppHandle<R>,
    layout: State<'_, AppDataLayout>,
    timeout_seconds: Option<u32>,
) -> Result<CharacterSettingsDto, String> {
    let current = load_character_settings(&layout);
    let settings = with_expression_idle_timeout(current, timeout_seconds)?;
    save_character_settings(&layout, &settings)?;
    app.emit_to(
        EventTarget::webview_window("character"),
        CHARACTER_SETTINGS_CHANGED_EVENT,
        CharacterSettingsChangedEventDto {
            schema_version: CHARACTER_SETTINGS_SCHEMA_VERSION,
            settings: settings.clone(),
        },
    )
    .map_err(|error| error.to_string())?;
    Ok(settings)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::Mutex;

    use super::{load_manifest, to_dto, validate_expression, validate_motion_group};
    use crate::character::{ResolvedCharacter, ResolvedRenderer, ResolvedStaticExpression};
    use pw_contracts::{
        CHARACTER_SETTINGS_SCHEMA_VERSION, CharacterRendererDto, CharacterSettingsDto,
        MotionGroupDto,
    };
    use pw_platform::paths::AppDataLayout;

    #[derive(Default)]
    struct FakeWindows {
        chat_visible: bool,
        character_visible: bool,
    }

    impl super::CharacterWindows for FakeWindows {
        fn show_chat(&mut self) -> Result<(), String> {
            self.chat_visible = true;
            Ok(())
        }
        fn hide_character(&mut self) -> Result<(), String> {
            self.character_visible = false;
            Ok(())
        }
        fn show_character(&mut self) -> Result<(), String> {
            self.character_visible = true;
            Ok(())
        }
    }

    #[test]
    fn renderer_failure_shows_normal_chat_and_hides_character_surface() {
        let mut windows = FakeWindows {
            character_visible: true,
            ..Default::default()
        };
        super::apply_character_renderer_window_mode(&mut windows, false).unwrap();
        assert!(windows.chat_visible);
        assert!(!windows.character_visible);
    }

    #[test]
    fn renderer_recovery_restores_character_surface() {
        let mut windows = FakeWindows::default();
        super::apply_character_renderer_window_mode(&mut windows, true).unwrap();
        assert!(windows.character_visible);
    }

    fn live2d_character() -> ResolvedCharacter {
        ResolvedCharacter {
            id: "legacy-live2d".into(),
            display_name: "Legacy Live2D".into(),
            profile_root: PathBuf::from("C:/data/characters/eps"),
            renderer: ResolvedRenderer::Live2d {
                model_path: PathBuf::from("C:/data/characters/eps/Epsilon.model3.json"),
                default_expression: Some("Normal".into()),
                expressions: vec!["Normal".into(), "Smile".into()],
                motion_groups: vec![
                    MotionGroupDto {
                        name: "Idle".into(),
                        motion_count: 1,
                    },
                    MotionGroupDto {
                        name: "Tap".into(),
                        motion_count: 4,
                    },
                ],
            },
        }
    }

    #[test]
    fn maps_manifest_to_versioned_dto() {
        let dto = to_dto(&live2d_character());
        assert_eq!(dto.schema_version, 2);
        let CharacterRendererDto::Live2d {
            expressions,
            motion_groups,
            ..
        } = dto.renderer
        else {
            panic!("expected Live2D renderer")
        };
        assert_eq!(expressions, ["Normal", "Smile"]);
        assert_eq!(motion_groups[1].name, "Tap");
        assert_eq!(motion_groups[1].motion_count, 4);
    }

    #[test]
    fn rejects_unknown_expression_and_motion_names() {
        let manifest = live2d_character();
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
        assert!(error.starts_with("character_profile_error:missing_asset:"));

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn load_manifest_uses_persisted_active_id_with_multiple_profiles() {
        let root = std::env::temp_dir().join(format!(
            "pw-cmd-manifest-selection-test-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let layout = AppDataLayout::under(root.clone());
        layout.create_all().unwrap();
        for id in ["alpha", "beta"] {
            let profile = layout.characters.join(id);
            std::fs::create_dir_all(&profile).unwrap();
            std::fs::write(
                profile.join(format!("{id}.model3.json")),
                r#"{"FileReferences":{"Expressions":[{"Name":"Normal"}]}}"#,
            )
            .unwrap();
            std::fs::write(
                profile.join("character.json"),
                serde_json::to_vec(&serde_json::json!({
                    "schema_version": 1,
                    "id": id,
                    "display_name": id,
                    "renderer": {
                        "kind": "live2d",
                        "model": format!("{id}.model3.json"),
                        "default_expression": "Normal"
                    }
                }))
                .unwrap(),
            )
            .unwrap();
        }
        crate::character::save_character_settings(
            &layout,
            &CharacterSettingsDto {
                schema_version: CHARACTER_SETTINGS_SCHEMA_VERSION,
                active_character_id: Some("beta".into()),
                expression_idle_timeout_seconds: Some(20),
            },
        )
        .unwrap();

        let manifest = load_manifest(&layout).unwrap();

        assert_eq!(manifest.id, "beta");
        std::fs::remove_dir_all(&root).unwrap();
    }

    fn static_character() -> ResolvedCharacter {
        ResolvedCharacter {
            id: "epsilon-static".into(),
            display_name: "Epsilon Static".into(),
            profile_root: PathBuf::from("C:/data/characters/epsilon-static"),
            renderer: ResolvedRenderer::StaticImage {
                default_expression: "neutral".into(),
                expressions: vec![
                    ResolvedStaticExpression {
                        name: "neutral".into(),
                        image_path: PathBuf::from(
                            "C:/data/characters/epsilon-static/expressions/neutral.png",
                        ),
                    },
                    ResolvedStaticExpression {
                        name: "happy".into(),
                        image_path: PathBuf::from(
                            "C:/data/characters/epsilon-static/expressions/happy.webp",
                        ),
                    },
                ],
                width: 1024,
                height: 2048,
            },
        }
    }

    #[test]
    fn maps_static_character_to_task_one_dto_with_absolute_paths() {
        let dto = to_dto(&static_character());
        assert_eq!(dto.id, "epsilon-static");
        assert_eq!(dto.display_name, "Epsilon Static");
        let CharacterRendererDto::StaticImage {
            default_expression,
            expressions,
            width,
            height,
        } = dto.renderer
        else {
            panic!("expected static renderer")
        };
        assert_eq!(default_expression, "neutral");
        assert_eq!(expressions.len(), 2);
        assert!(PathBuf::from(&expressions[0].image_path).is_absolute());
        assert_eq!((width, height), (1024, 2048));
    }

    #[test]
    fn static_motion_is_rejected_without_corrupting_cached_capabilities() {
        let character = static_character();
        let state = super::CharacterState {
            manifest: Mutex::new(Some(character.clone())),
        };
        let before = state.manifest_summary().unwrap();

        let error = validate_motion_group(&character, "Idle").unwrap_err();

        assert!(error.contains("unsupported"));
        assert_eq!(state.manifest_summary().unwrap(), before);
        assert_eq!(before.expressions, ["neutral", "happy"]);
        assert!(before.motions.is_empty());
        assert_eq!(*state.manifest.lock().unwrap(), Some(character));
    }

    #[test]
    fn control_context_identifies_each_renderer_kind() {
        let live2d_state = super::CharacterState::default();
        live2d_state.cache_manifest(live2d_character()).unwrap();
        let live2d = live2d_state.control_context().unwrap();
        let static_state = super::CharacterState::default();
        static_state.cache_manifest(static_character()).unwrap();
        let static_image = static_state.control_context().unwrap();

        assert_eq!(live2d.renderer, "live2d");
        assert_eq!(live2d.capabilities.motions, ["Idle", "Tap"]);
        assert_eq!(static_image.renderer, "static_image");
        assert!(static_image.capabilities.motions.is_empty());
    }
}
