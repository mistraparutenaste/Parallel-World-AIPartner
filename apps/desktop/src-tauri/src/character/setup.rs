use std::{
    collections::BTreeSet,
    fs,
    path::{Component, Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use pw_contracts::{
    CHARACTER_SETUP_SCHEMA_VERSION, CharacterRendererKindDto, CharacterSetupDto,
    CharacterSourceStatusDto,
};
use pw_platform::paths::AppDataLayout;

use super::{
    CharacterCatalog, CharacterProfileError, LEGACY_CHARACTER_ID, ResolvedCharacter,
    ResolvedRenderer, load_character_settings, save_character_settings, validate_profile_manifest,
};

const MANAGED_STATIC_PREFIX: &str = "managed-static-";
const MANAGED_LIVE2D_PREFIX: &str = "managed-live2d-";
static NEXT_GENERATION: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy)]
struct Live2dCopyLimits {
    files: usize,
    file_bytes: u64,
    total_bytes: u64,
}

impl Default for Live2dCopyLimits {
    fn default() -> Self {
        Self {
            files: 2048,
            file_bytes: 256 * 1024 * 1024,
            total_bytes: 1024 * 1024 * 1024,
        }
    }
}

fn has_reparse_attribute(attributes: u32) -> bool {
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

fn renderer_kind(character: &ResolvedCharacter) -> CharacterRendererKindDto {
    match character.renderer {
        ResolvedRenderer::Live2d { .. } => CharacterRendererKindDto::Live2d,
        ResolvedRenderer::StaticImage { .. } => CharacterRendererKindDto::StaticImage,
    }
}

fn file_name(character: &ResolvedCharacter) -> Option<String> {
    let path = match &character.renderer {
        ResolvedRenderer::Live2d { model_path, .. } => Some(model_path),
        ResolvedRenderer::StaticImage { expressions, .. } => {
            expressions.first().map(|expression| &expression.image_path)
        }
    }?;
    path.file_name()
        .and_then(|name| name.to_str())
        .map(str::to_owned)
}

fn setup_error(code: &str, message: impl std::fmt::Display) -> String {
    let safe = pw_domain::runtime_health::redact_diagnostic(&message.to_string());
    format!("character_setup_error:{code}:{safe}")
}

fn map_profile_error(error: CharacterProfileError) -> String {
    setup_error(error.stable_code(), error)
}

fn status(
    kind: CharacterRendererKindDto,
    character: Option<&ResolvedCharacter>,
    active_id: Option<&str>,
    import_enabled: bool,
) -> CharacterSourceStatusDto {
    CharacterSourceStatusDto {
        kind,
        configured: character.is_some(),
        display_name: character.map(|profile| profile.display_name.clone()),
        file_name: character.and_then(file_name),
        import_enabled,
        active: character.is_some_and(|profile| active_id == Some(profile.id.as_str())),
    }
}

pub(crate) fn discover_setup(
    layout: &AppDataLayout,
    live2d_import_enabled: bool,
) -> Result<CharacterSetupDto, String> {
    let settings = load_character_settings(layout);
    let catalog = match CharacterCatalog::discover(layout) {
        Ok(catalog) => Some(catalog),
        Err(CharacterProfileError::NoCharacterAvailable) => None,
        Err(error) => return Err(map_profile_error(error)),
    };
    let Some(catalog) = catalog else {
        return Ok(CharacterSetupDto {
            schema_version: CHARACTER_SETUP_SCHEMA_VERSION,
            active_renderer: None,
            live2d: status(
                CharacterRendererKindDto::Live2d,
                None,
                None,
                live2d_import_enabled,
            ),
            static_image: status(CharacterRendererKindDto::StaticImage, None, None, true),
        });
    };

    let explicit_active = settings
        .active_character_id
        .as_deref()
        .and_then(|id| catalog.profile_by_id(id));
    if settings.active_character_id.is_some() && explicit_active.is_none() {
        return Err(setup_error(
            "active_character_unavailable",
            "configured active character is unavailable",
        ));
    }
    let legacy_active =
        if settings.active_character_id.is_none() && !catalog.has_explicit_profiles() {
            catalog.profile_by_id(LEGACY_CHARACTER_ID)
        } else {
            None
        };
    let active = explicit_active.or(legacy_active);

    let remembered_live2d = settings
        .live2d_character_id
        .as_deref()
        .and_then(|id| catalog.profile_by_id(id))
        .filter(|profile| renderer_kind(profile) == CharacterRendererKindDto::Live2d);
    let remembered_static = settings
        .static_image_character_id
        .as_deref()
        .and_then(|id| catalog.profile_by_id(id))
        .filter(|profile| renderer_kind(profile) == CharacterRendererKindDto::StaticImage);
    let live2d = remembered_live2d.or_else(|| {
        settings.live2d_character_id.is_none().then(|| {
            active.filter(|profile| renderer_kind(profile) == CharacterRendererKindDto::Live2d)
        })?
    });
    let static_image = remembered_static.or_else(|| {
        settings.static_image_character_id.is_none().then(|| {
            active.filter(|profile| renderer_kind(profile) == CharacterRendererKindDto::StaticImage)
        })?
    });
    let active_id = active.map(|profile| profile.id.as_str());

    Ok(CharacterSetupDto {
        schema_version: CHARACTER_SETUP_SCHEMA_VERSION,
        active_renderer: active.map(renderer_kind),
        live2d: status(
            CharacterRendererKindDto::Live2d,
            live2d,
            active_id,
            live2d_import_enabled,
        ),
        static_image: status(
            CharacterRendererKindDto::StaticImage,
            static_image,
            active_id,
            true,
        ),
    })
}

pub(crate) fn select_active_renderer(
    layout: &AppDataLayout,
    kind: CharacterRendererKindDto,
) -> Result<CharacterSetupDto, String> {
    let mut settings = load_character_settings(layout);
    let catalog = CharacterCatalog::discover(layout).map_err(map_profile_error)?;
    let remembered_id = match kind {
        CharacterRendererKindDto::Live2d => settings.live2d_character_id.as_deref(),
        CharacterRendererKindDto::StaticImage => settings.static_image_character_id.as_deref(),
    };
    let active = settings
        .active_character_id
        .as_deref()
        .and_then(|id| catalog.profile_by_id(id));
    let selected = remembered_id
        .and_then(|id| catalog.profile_by_id(id))
        .filter(|profile| renderer_kind(profile) == kind)
        .or_else(|| {
            remembered_id
                .is_none()
                .then(|| active.filter(|profile| renderer_kind(profile) == kind))?
        });
    let Some(selected) = selected else {
        if kind == CharacterRendererKindDto::Live2d
            && settings.active_character_id.is_none()
            && !catalog.has_explicit_profiles()
            && catalog.profile_by_id(LEGACY_CHARACTER_ID).is_some()
        {
            return discover_setup(layout, cfg!(debug_assertions));
        }
        return Err(setup_error(
            "unconfigured_source",
            "requested renderer source is not configured",
        ));
    };
    settings.active_character_id = Some(selected.id.clone());
    save_character_settings(layout, &settings)
        .map_err(|error| setup_error("settings_save", error))?;
    discover_setup(layout, cfg!(debug_assertions))
}

pub(crate) fn import_character_source(
    layout: &AppDataLayout,
    kind: CharacterRendererKindDto,
    source_path: &Path,
    live2d_import_enabled: bool,
) -> Result<CharacterSetupDto, String> {
    import_character_source_with_saver(
        layout,
        kind,
        source_path,
        live2d_import_enabled,
        save_character_settings,
    )
}

fn import_character_source_with_saver(
    layout: &AppDataLayout,
    kind: CharacterRendererKindDto,
    source_path: &Path,
    live2d_import_enabled: bool,
    save: impl Fn(&AppDataLayout, &pw_contracts::CharacterSettingsDto) -> Result<(), String>,
) -> Result<CharacterSetupDto, String> {
    match kind {
        CharacterRendererKindDto::StaticImage => {
            import_static_source(layout, source_path, live2d_import_enabled, save)
        }
        CharacterRendererKindDto::Live2d => {
            if !live2d_import_enabled {
                return Err(setup_error(
                    "live2d_import_disabled",
                    "arbitrary Live2D import is disabled in release builds",
                ));
            }
            import_live2d_source(layout, source_path, live2d_import_enabled, save)
        }
    }
}

fn generation() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let sequence = NEXT_GENERATION.fetch_add(1, Ordering::Relaxed);
    format!("{nanos}-{}-{sequence}", std::process::id())
}

fn ensure_regular_source(path: &Path) -> Result<(), String> {
    let metadata =
        fs::symlink_metadata(path).map_err(|error| setup_error("invalid_source", error))?;
    if metadata.file_type().is_symlink() || !metadata.file_type().is_file() {
        return Err(setup_error(
            "invalid_source",
            "source must be a regular non-symlink file",
        ));
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if has_reparse_attribute(metadata.file_attributes()) {
            return Err(setup_error(
                "invalid_source",
                "source must not be a reparse point",
            ));
        }
    }
    Ok(())
}

fn static_extension(path: &Path) -> Result<String, String> {
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .map(str::to_ascii_lowercase)
        .ok_or_else(|| setup_error("invalid_source", "static source extension is missing"))?;
    if matches!(extension.as_str(), "png" | "webp") {
        Ok(extension)
    } else {
        Err(setup_error(
            "invalid_source",
            "static source must use .png or .webp",
        ))
    }
}

fn source_display_name(path: &Path, fallback: &str) -> String {
    path.file_stem()
        .and_then(|value| value.to_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(fallback)
        .to_owned()
}

fn import_static_source(
    layout: &AppDataLayout,
    source_path: &Path,
    live2d_import_enabled: bool,
    save: impl Fn(&AppDataLayout, &pw_contracts::CharacterSettingsDto) -> Result<(), String>,
) -> Result<CharacterSetupDto, String> {
    ensure_regular_source(source_path)?;
    let extension = static_extension(source_path)?;
    let initial_setup = discover_setup(layout, live2d_import_enabled)?;
    let initial_settings = load_character_settings(layout);
    if initial_setup.active_renderer == Some(CharacterRendererKindDto::Live2d)
        && initial_settings.active_character_id.is_none()
        && initial_settings.live2d_character_id.is_none()
    {
        let catalog = CharacterCatalog::discover(layout).map_err(map_profile_error)?;
        let legacy = catalog
            .profile_by_id(LEGACY_CHARACTER_ID)
            .and_then(ResolvedCharacter::live2d_model_path)
            .ok_or_else(|| setup_error("missing_asset", "legacy Live2D model is unavailable"))?
            .to_path_buf();
        import_live2d_source(
            layout,
            &legacy,
            live2d_import_enabled,
            save_character_settings,
        )?;
    }
    let before_setup = discover_setup(layout, live2d_import_enabled)?;
    let original = load_character_settings(layout);
    let generation = generation();
    let id = format!("{MANAGED_STATIC_PREFIX}{generation}");
    let staging = layout.characters.join(".staging").join(&generation);
    let final_dir = layout.characters.join(&id);
    let image_name = format!("neutral.{extension}");

    let staged_result = (|| -> Result<ResolvedCharacter, String> {
        fs::create_dir_all(&staging).map_err(|error| setup_error("asset_write", error))?;
        fs::copy(source_path, staging.join(&image_name))
            .map_err(|error| setup_error("asset_write", error))?;
        let manifest = serde_json::json!({
            "schema_version": 1,
            "id": id,
            "display_name": source_display_name(source_path, "Static Image"),
            "renderer": {
                "kind": "static_image",
                "default_expression": "neutral",
                "expressions": [{"name": "neutral", "file": image_name}]
            }
        });
        fs::write(
            staging.join("character.json"),
            serde_json::to_vec_pretty(&manifest)
                .map_err(|error| setup_error("manifest_write", error))?,
        )
        .map_err(|error| setup_error("manifest_write", error))?;
        validate_profile_manifest(layout, &staging.join("character.json"))
            .map_err(map_profile_error)
    })();
    let validated = match staged_result {
        Ok(profile) => profile,
        Err(error) => {
            let _ = fs::remove_dir_all(&staging);
            return Err(error);
        }
    };
    if renderer_kind(&validated) != CharacterRendererKindDto::StaticImage {
        let _ = fs::remove_dir_all(&staging);
        return Err(setup_error("invalid_manifest", "renderer kind mismatch"));
    }
    if let Err(error) = fs::rename(&staging, &final_dir) {
        let _ = fs::remove_dir_all(&staging);
        return Err(setup_error("asset_commit", error));
    }

    let previous_id = original.static_image_character_id.clone().or_else(|| {
        (before_setup.active_renderer == Some(CharacterRendererKindDto::StaticImage))
            .then(|| original.active_character_id.clone())
            .flatten()
    });
    let mut updated = original.clone();
    updated.static_image_character_id = Some(id.clone());
    if before_setup.active_renderer.is_none()
        || before_setup.active_renderer == Some(CharacterRendererKindDto::StaticImage)
    {
        updated.active_character_id = Some(id.clone());
    }
    if let Err(error) = save(layout, &updated) {
        let _ = fs::remove_dir_all(&final_dir);
        return Err(setup_error("settings_save", error));
    }

    if let Some(previous_id) = previous_id
        && previous_id.starts_with(MANAGED_STATIC_PREFIX)
        && previous_id != id
    {
        let _ = fs::remove_dir_all(layout.characters.join(previous_id));
    }
    discover_setup(layout, live2d_import_enabled)
}

fn copy_live2d_references(
    model_path: &Path,
    destination: &Path,
    limits: Live2dCopyLimits,
) -> Result<(), String> {
    ensure_regular_source(model_path)?;
    let file_name = model_path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| setup_error("invalid_source", "Live2D filename is not valid UTF-8"))?;
    if !file_name.to_ascii_lowercase().ends_with(".model3.json") {
        return Err(setup_error(
            "invalid_source",
            "Live2D source must end with .model3.json",
        ));
    }
    let source_root = model_path
        .parent()
        .ok_or_else(|| setup_error("invalid_source", "Live2D source has no parent"))?;
    ensure_no_reparse_components(source_root)?;
    let canonical_root = source_root
        .canonicalize()
        .map_err(|error| setup_error("invalid_source", error))?;
    let raw =
        fs::read_to_string(model_path).map_err(|error| setup_error("invalid_model", error))?;
    let json: serde_json::Value =
        serde_json::from_str(&raw).map_err(|error| setup_error("invalid_model", error))?;
    let references = collect_live2d_references(&json, file_name)?;
    if references.len() > limits.files {
        return Err(setup_error(
            "file_limit",
            format!("Live2D import contains {} files", references.len()),
        ));
    }

    let mut total = 0_u64;
    for relative in references {
        validate_live2d_relative_path(&relative)?;
        let source = source_root.join(&relative);
        ensure_no_reparse_components(&source)?;
        let metadata =
            fs::symlink_metadata(&source).map_err(|error| setup_error("missing_asset", error))?;
        if metadata.file_type().is_symlink() || !metadata.file_type().is_file() {
            return Err(setup_error(
                "unsafe_reference",
                "Live2D reference is not a regular non-symlink file",
            ));
        }
        #[cfg(windows)]
        ensure_not_reparse(&metadata)?;
        let canonical = source
            .canonicalize()
            .map_err(|error| setup_error("missing_asset", error))?;
        if !canonical.starts_with(&canonical_root) {
            return Err(setup_error(
                "unsafe_reference",
                "Live2D reference escapes the selected model directory",
            ));
        }
        if metadata.len() > limits.file_bytes {
            return Err(setup_error(
                "file_too_large",
                format!("Live2D reference is {} bytes", metadata.len()),
            ));
        }
        total = total
            .checked_add(metadata.len())
            .ok_or_else(|| setup_error("total_limit", "Live2D total size overflow"))?;
        if total > limits.total_bytes {
            return Err(setup_error(
                "total_limit",
                format!("Live2D import totals {total} bytes"),
            ));
        }
        let target = destination.join(&relative);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|error| setup_error("asset_write", error))?;
        }
        fs::copy(&canonical, &target).map_err(|error| setup_error("asset_write", error))?;
    }
    Ok(())
}

fn collect_live2d_references(
    json: &serde_json::Value,
    model_file_name: &str,
) -> Result<BTreeSet<PathBuf>, String> {
    let file_references = &json["FileReferences"];
    let mut references = BTreeSet::new();
    references.insert(PathBuf::from(model_file_name));
    for key in ["Moc", "Physics", "Pose", "UserData", "DisplayInfo"] {
        insert_optional_reference(&mut references, &file_references[key], key)?;
    }
    if let Some(textures) = file_references["Textures"].as_array() {
        for value in textures {
            insert_required_reference(&mut references, value, "Textures")?;
        }
    } else if !file_references["Textures"].is_null() {
        return Err(setup_error("invalid_model", "Textures must be an array"));
    }
    if let Some(expressions) = file_references["Expressions"].as_array() {
        for expression in expressions {
            insert_required_reference(&mut references, &expression["File"], "Expressions.File")?;
        }
    } else if !file_references["Expressions"].is_null() {
        return Err(setup_error("invalid_model", "Expressions must be an array"));
    }
    if let Some(motions) = file_references["Motions"].as_object() {
        for values in motions.values() {
            let values = values
                .as_array()
                .ok_or_else(|| setup_error("invalid_model", "motion group must be an array"))?;
            for motion in values {
                insert_required_reference(&mut references, &motion["File"], "Motions.File")?;
                insert_optional_reference(&mut references, &motion["Sound"], "Motions.Sound")?;
            }
        }
    } else if !file_references["Motions"].is_null() {
        return Err(setup_error("invalid_model", "Motions must be an object"));
    }
    Ok(references)
}

fn insert_optional_reference(
    references: &mut BTreeSet<PathBuf>,
    value: &serde_json::Value,
    label: &str,
) -> Result<(), String> {
    if value.is_null() {
        Ok(())
    } else {
        insert_required_reference(references, value, label)
    }
}

fn insert_required_reference(
    references: &mut BTreeSet<PathBuf>,
    value: &serde_json::Value,
    label: &str,
) -> Result<(), String> {
    let value = value
        .as_str()
        .ok_or_else(|| setup_error("invalid_model", format!("{label} must be a string")))?;
    references.insert(PathBuf::from(value));
    Ok(())
}

fn validate_live2d_relative_path(path: &Path) -> Result<(), String> {
    if path.as_os_str().is_empty()
        || path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        Err(setup_error(
            "unsafe_reference",
            "Live2D reference must be a non-empty child path",
        ))
    } else {
        Ok(())
    }
}

fn ensure_no_reparse_components(path: &Path) -> Result<(), String> {
    let mut current = PathBuf::new();
    for component in path.components() {
        current.push(component.as_os_str());
        if matches!(component, Component::Prefix(_) | Component::RootDir) {
            continue;
        }
        let metadata =
            fs::symlink_metadata(&current).map_err(|error| setup_error("missing_asset", error))?;
        if metadata.file_type().is_symlink() {
            return Err(setup_error(
                "unsafe_reference",
                "Live2D path contains a symlink",
            ));
        }
        #[cfg(windows)]
        ensure_not_reparse(&metadata)?;
    }
    Ok(())
}

#[cfg(windows)]
fn ensure_not_reparse(metadata: &fs::Metadata) -> Result<(), String> {
    use std::os::windows::fs::MetadataExt;
    if has_reparse_attribute(metadata.file_attributes()) {
        Err(setup_error(
            "unsafe_reference",
            "Live2D path contains a reparse point",
        ))
    } else {
        Ok(())
    }
}

fn import_live2d_source(
    layout: &AppDataLayout,
    source_path: &Path,
    live2d_import_enabled: bool,
    save: impl Fn(&AppDataLayout, &pw_contracts::CharacterSettingsDto) -> Result<(), String>,
) -> Result<CharacterSetupDto, String> {
    let before_setup = discover_setup(layout, live2d_import_enabled)?;
    let original = load_character_settings(layout);
    let generation = generation();
    let id = format!("{MANAGED_LIVE2D_PREFIX}{generation}");
    let staging = layout.characters.join(".staging").join(&generation);
    let final_dir = layout.characters.join(&id);
    let model_name = source_path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| setup_error("invalid_source", "Live2D filename is not valid UTF-8"))?;
    let lower_name = model_name.to_ascii_lowercase();
    let display_name = lower_name
        .strip_suffix(".model3.json")
        .map(|_| &model_name[..model_name.len() - ".model3.json".len()])
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .unwrap_or("Live2D")
        .to_owned();

    let staged_result = (|| -> Result<ResolvedCharacter, String> {
        fs::create_dir_all(&staging).map_err(|error| setup_error("asset_write", error))?;
        copy_live2d_references(source_path, &staging, Live2dCopyLimits::default())?;
        let manifest = serde_json::json!({
            "schema_version": 1,
            "id": id,
            "display_name": display_name,
            "renderer": {"kind": "live2d", "model": model_name, "default_expression": null}
        });
        fs::write(
            staging.join("character.json"),
            serde_json::to_vec_pretty(&manifest)
                .map_err(|error| setup_error("manifest_write", error))?,
        )
        .map_err(|error| setup_error("manifest_write", error))?;
        validate_profile_manifest(layout, &staging.join("character.json"))
            .map_err(map_profile_error)
    })();
    let validated = match staged_result {
        Ok(profile) => profile,
        Err(error) => {
            let _ = fs::remove_dir_all(&staging);
            return Err(error);
        }
    };
    if renderer_kind(&validated) != CharacterRendererKindDto::Live2d {
        let _ = fs::remove_dir_all(&staging);
        return Err(setup_error("invalid_manifest", "renderer kind mismatch"));
    }
    if let Err(error) = fs::rename(&staging, &final_dir) {
        let _ = fs::remove_dir_all(&staging);
        return Err(setup_error("asset_commit", error));
    }

    let previous_id = original.live2d_character_id.clone().or_else(|| {
        (before_setup.active_renderer == Some(CharacterRendererKindDto::Live2d))
            .then(|| original.active_character_id.clone())
            .flatten()
    });
    let mut updated = original.clone();
    updated.live2d_character_id = Some(id.clone());
    if before_setup.active_renderer.is_none()
        || before_setup.active_renderer == Some(CharacterRendererKindDto::Live2d)
    {
        updated.active_character_id = Some(id.clone());
    }
    if let Err(error) = save(layout, &updated) {
        let _ = fs::remove_dir_all(&final_dir);
        return Err(setup_error("settings_save", error));
    }
    if let Some(previous_id) = previous_id
        && previous_id.starts_with(MANAGED_LIVE2D_PREFIX)
        && previous_id != id
    {
        let _ = fs::remove_dir_all(layout.characters.join(previous_id));
    }
    discover_setup(layout, live2d_import_enabled)
}

#[cfg(test)]
mod tests {
    use std::{
        fs::File,
        io::BufWriter,
        path::{Path, PathBuf},
        sync::atomic::{AtomicU64, Ordering},
    };

    use image::{ExtendedColorType, ImageEncoder, codecs::png::PngEncoder};
    use pw_contracts::{
        CHARACTER_SETTINGS_SCHEMA_VERSION, CharacterRendererKindDto, CharacterSettingsDto,
    };
    use pw_platform::paths::AppDataLayout;

    use super::{
        Live2dCopyLimits, copy_live2d_references, discover_setup, has_reparse_attribute,
        import_character_source, import_character_source_with_saver, select_active_renderer,
    };

    static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);

    struct Fixture {
        layout: AppDataLayout,
        root: PathBuf,
    }

    impl Fixture {
        fn new(tag: &str) -> Self {
            let sequence = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
            let root = std::env::temp_dir().join(format!(
                "pw-character-setup-{tag}-{}-{sequence}",
                std::process::id()
            ));
            let layout = AppDataLayout::under(root.clone());
            layout.create_all().unwrap();
            Self { layout, root }
        }

        fn add_static(&self, id: &str) {
            let profile = self.layout.characters.join(id);
            std::fs::create_dir_all(&profile).unwrap();
            write_png(&profile.join("neutral.png"), 2, 2, true);
            std::fs::write(
                profile.join("character.json"),
                serde_json::to_vec(&serde_json::json!({
                    "schema_version": 1,
                    "id": id,
                    "display_name": "Static Friend",
                    "renderer": {
                        "kind": "static_image",
                        "default_expression": "neutral",
                        "expressions": [{"name": "neutral", "file": "neutral.png"}]
                    }
                }))
                .unwrap(),
            )
            .unwrap();
        }

        fn add_live2d(&self, id: &str) {
            let profile = self.layout.characters.join(id);
            std::fs::create_dir_all(&profile).unwrap();
            std::fs::write(
                profile.join("friend.model3.json"),
                r#"{"FileReferences":{}}"#,
            )
            .unwrap();
            std::fs::write(
                profile.join("character.json"),
                serde_json::to_vec(&serde_json::json!({
                    "schema_version": 1,
                    "id": id,
                    "display_name": "Live Friend",
                    "renderer": {"kind": "live2d", "model": "friend.model3.json", "default_expression": null}
                }))
                .unwrap(),
            )
            .unwrap();
        }

        fn save(&self, settings: &CharacterSettingsDto) {
            crate::character::save_character_settings(&self.layout, settings).unwrap();
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    fn write_png(path: &Path, width: u32, height: u32, alpha: bool) {
        let channels = if alpha { 4 } else { 3 };
        let pixels = vec![128; width as usize * height as usize * channels];
        let file = BufWriter::new(File::create(path).unwrap());
        PngEncoder::new(file)
            .write_image(
                &pixels,
                width,
                height,
                if alpha {
                    ExtendedColorType::Rgba8
                } else {
                    ExtendedColorType::Rgb8
                },
            )
            .unwrap();
    }

    #[test]
    fn empty_setup_succeeds_with_both_sources_unconfigured() {
        let fixture = Fixture::new("empty");

        let setup = discover_setup(&fixture.layout, true).unwrap();

        assert_eq!(setup.active_renderer, None);
        assert!(!setup.live2d.configured);
        assert!(!setup.static_image.configured);
    }

    #[test]
    fn active_explicit_v1_selection_reconciles_the_matching_source() {
        let fixture = Fixture::new("v1-active");
        fixture.add_static("static-v1");
        fixture.save(&CharacterSettingsDto {
            schema_version: CHARACTER_SETTINGS_SCHEMA_VERSION,
            active_character_id: Some("static-v1".into()),
            ..CharacterSettingsDto::default()
        });

        let setup = discover_setup(&fixture.layout, true).unwrap();

        assert_eq!(
            setup.active_renderer,
            Some(CharacterRendererKindDto::StaticImage)
        );
        assert!(setup.static_image.configured);
        assert!(setup.static_image.active);
        assert_eq!(
            setup.static_image.display_name.as_deref(),
            Some("Static Friend")
        );
        assert_eq!(setup.static_image.file_name.as_deref(), Some("neutral.png"));
    }

    #[test]
    fn virtual_legacy_is_reported_without_persisting_its_reserved_id() {
        let fixture = Fixture::new("legacy");
        std::fs::write(
            fixture.layout.characters.join("legacy.model3.json"),
            r#"{"FileReferences":{}}"#,
        )
        .unwrap();

        let setup = discover_setup(&fixture.layout, true).unwrap();

        assert_eq!(
            setup.active_renderer,
            Some(CharacterRendererKindDto::Live2d)
        );
        assert!(setup.live2d.configured);
        assert!(setup.live2d.active);
        let settings = crate::character::load_character_settings(&fixture.layout);
        assert_eq!(settings.active_character_id, None);
        assert_eq!(settings.live2d_character_id, None);
    }

    #[test]
    fn remembered_id_is_configured_only_for_an_exact_kind_match() {
        let fixture = Fixture::new("kind-mismatch");
        fixture.add_live2d("live-source");
        fixture.save(&CharacterSettingsDto {
            schema_version: CHARACTER_SETTINGS_SCHEMA_VERSION,
            active_character_id: Some("live-source".into()),
            static_image_character_id: Some("live-source".into()),
            ..CharacterSettingsDto::default()
        });

        let setup = discover_setup(&fixture.layout, true).unwrap();

        assert!(!setup.static_image.configured);
        let error = select_active_renderer(&fixture.layout, CharacterRendererKindDto::StaticImage)
            .unwrap_err();
        assert!(error.starts_with("character_setup_error:unconfigured_source:"));
    }

    #[test]
    fn invalid_non_null_remembered_id_does_not_fall_back_to_same_kind_active_profile() {
        let fixture = Fixture::new("invalid-remembered-no-fallback");
        fixture.add_live2d("active-live");
        fixture.save(&CharacterSettingsDto {
            schema_version: CHARACTER_SETTINGS_SCHEMA_VERSION,
            active_character_id: Some("active-live".into()),
            live2d_character_id: Some("missing-live".into()),
            ..CharacterSettingsDto::default()
        });

        let setup = discover_setup(&fixture.layout, true).unwrap();

        assert!(!setup.live2d.configured);
        let error =
            select_active_renderer(&fixture.layout, CharacterRendererKindDto::Live2d).unwrap_err();
        assert!(error.starts_with("character_setup_error:unconfigured_source:"));
    }

    #[test]
    fn valid_static_import_creates_and_activates_one_managed_generation() {
        let fixture = Fixture::new("static-import");
        let source = fixture.root.join("Static Friend.PNG");
        write_png(&source, 2, 2, true);

        let setup = import_character_source(
            &fixture.layout,
            CharacterRendererKindDto::StaticImage,
            &source,
            true,
        )
        .unwrap();

        assert_eq!(
            setup.active_renderer,
            Some(CharacterRendererKindDto::StaticImage)
        );
        assert_eq!(
            setup.static_image.display_name.as_deref(),
            Some("Static Friend")
        );
        let settings = crate::character::load_character_settings(&fixture.layout);
        let id = settings.static_image_character_id.unwrap();
        assert!(id.starts_with("managed-static-"));
        assert_eq!(settings.active_character_id.as_deref(), Some(id.as_str()));
        assert!(
            fixture
                .layout
                .characters
                .join(&id)
                .join("character.json")
                .is_file()
        );
        assert!(source.is_file(), "selected source original must remain");
    }

    #[test]
    fn static_import_rejects_disguised_non_alpha_and_oversize_images() {
        for (tag, extension, width, height, alpha) in [
            ("disguised", "webp", 2, 2, true),
            ("no-alpha", "png", 2, 2, false),
            ("oversize", "png", 4097, 1, true),
        ] {
            let fixture = Fixture::new(tag);
            let source = fixture.root.join(format!("source.{extension}"));
            write_png(&source, width, height, alpha);

            let error = import_character_source(
                &fixture.layout,
                CharacterRendererKindDto::StaticImage,
                &source,
                true,
            )
            .unwrap_err();

            assert!(
                error.starts_with("character_setup_error:invalid_image:"),
                "{error}"
            );
        }
    }

    #[test]
    fn settings_save_failure_rolls_back_new_static_generation() {
        let fixture = Fixture::new("static-rollback");
        let first = fixture.root.join("first.png");
        write_png(&first, 2, 2, true);
        import_character_source(
            &fixture.layout,
            CharacterRendererKindDto::StaticImage,
            &first,
            true,
        )
        .unwrap();
        let original = crate::character::load_character_settings(&fixture.layout);
        let original_id = original.static_image_character_id.clone().unwrap();
        let second = fixture.root.join("second.png");
        write_png(&second, 2, 2, true);

        let error = import_character_source_with_saver(
            &fixture.layout,
            CharacterRendererKindDto::StaticImage,
            &second,
            true,
            |_layout, _settings| Err("injected save failure token=secret-value".into()),
        )
        .unwrap_err();

        assert!(error.starts_with("character_setup_error:settings_save:"));
        assert!(!error.contains("secret-value"));
        assert_eq!(
            crate::character::load_character_settings(&fixture.layout),
            original
        );
        let managed: Vec<_> = std::fs::read_dir(&fixture.layout.characters)
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.starts_with("managed-static-"))
            .collect();
        assert_eq!(managed, [original_id]);
    }

    fn write_live2d_source(root: &Path) -> PathBuf {
        std::fs::create_dir_all(root.join("textures")).unwrap();
        std::fs::create_dir_all(root.join("expressions")).unwrap();
        std::fs::create_dir_all(root.join("motions")).unwrap();
        for (relative, bytes) in [
            ("friend.moc3", b"moc".as_slice()),
            ("textures/face.png", b"texture".as_slice()),
            ("physics.json", b"physics".as_slice()),
            ("pose.json", b"pose".as_slice()),
            ("userdata.json", b"user".as_slice()),
            ("display.json", b"display".as_slice()),
            ("expressions/smile.json", b"expression".as_slice()),
            ("motions/idle.json", b"motion".as_slice()),
            ("motions/idle.wav", b"sound".as_slice()),
        ] {
            std::fs::write(root.join(relative), bytes).unwrap();
        }
        let model = root.join("Friend.model3.json");
        std::fs::write(
            &model,
            serde_json::to_vec(&serde_json::json!({
                "Version": 3,
                "FileReferences": {
                    "Moc": "friend.moc3",
                    "Textures": ["textures/face.png", "textures/face.png"],
                    "Physics": "physics.json",
                    "Pose": "pose.json",
                    "UserData": "userdata.json",
                    "DisplayInfo": "display.json",
                    "Expressions": [{"Name": "Smile", "File": "expressions/smile.json"}],
                    "Motions": {"Idle": [{"File": "motions/idle.json", "Sound": "motions/idle.wav"}]}
                }
            }))
            .unwrap(),
        )
        .unwrap();
        model
    }

    #[test]
    fn valid_live2d_import_copies_all_unique_references_and_activates() {
        let fixture = Fixture::new("live2d-import");
        let source_root = fixture.root.join("selected-live2d");
        let model = write_live2d_source(&source_root);

        let setup = import_character_source(
            &fixture.layout,
            CharacterRendererKindDto::Live2d,
            &model,
            true,
        )
        .unwrap();

        assert_eq!(
            setup.active_renderer,
            Some(CharacterRendererKindDto::Live2d)
        );
        let settings = crate::character::load_character_settings(&fixture.layout);
        let id = settings.live2d_character_id.unwrap();
        assert!(id.starts_with("managed-live2d-"));
        let managed = fixture.layout.characters.join(&id);
        for relative in [
            "Friend.model3.json",
            "friend.moc3",
            "textures/face.png",
            "physics.json",
            "pose.json",
            "userdata.json",
            "display.json",
            "expressions/smile.json",
            "motions/idle.json",
            "motions/idle.wav",
        ] {
            assert!(managed.join(relative).is_file(), "missing {relative}");
        }
        assert!(model.is_file(), "selected source original must remain");
    }

    #[test]
    fn live2d_copy_rejects_missing_absolute_and_parent_references() {
        for (tag, reference, expected_code) in [
            ("missing", "missing.moc3", "missing_asset"),
            ("absolute", "C:/outside.moc3", "unsafe_reference"),
            ("parent", "../outside.moc3", "unsafe_reference"),
        ] {
            let fixture = Fixture::new(tag);
            let source = fixture.root.join("source");
            std::fs::create_dir_all(&source).unwrap();
            let model = source.join("unsafe.model3.json");
            std::fs::write(
                &model,
                serde_json::to_vec(&serde_json::json!({"FileReferences":{"Moc": reference}}))
                    .unwrap(),
            )
            .unwrap();

            let error = copy_live2d_references(
                &model,
                &fixture.root.join("copied"),
                Live2dCopyLimits::default(),
            )
            .unwrap_err();

            assert!(
                error.starts_with(&format!("character_setup_error:{expected_code}:")),
                "{error}"
            );
        }
    }

    #[test]
    fn live2d_copy_enforces_file_count_file_size_and_total_limits() {
        let fixture = Fixture::new("live2d-limits");
        let source = fixture.root.join("source");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::write(source.join("one.bin"), b"1111").unwrap();
        std::fs::write(source.join("two.bin"), b"2222").unwrap();
        let model = source.join("limit.model3.json");
        std::fs::write(
            &model,
            r#"{"FileReferences":{"Moc":"one.bin","Textures":["two.bin"]}}"#,
        )
        .unwrap();
        let model_bytes = std::fs::metadata(&model).unwrap().len();

        let count = copy_live2d_references(
            &model,
            &fixture.root.join("count"),
            Live2dCopyLimits {
                files: 2,
                ..Live2dCopyLimits::default()
            },
        )
        .unwrap_err();
        assert!(
            count.starts_with("character_setup_error:file_limit:"),
            "{count}"
        );

        let file = copy_live2d_references(
            &model,
            &fixture.root.join("file"),
            Live2dCopyLimits {
                file_bytes: 3,
                ..Live2dCopyLimits::default()
            },
        )
        .unwrap_err();
        assert!(
            file.starts_with("character_setup_error:file_too_large:"),
            "{file}"
        );

        let total = copy_live2d_references(
            &model,
            &fixture.root.join("total"),
            Live2dCopyLimits {
                total_bytes: model_bytes + 4,
                ..Live2dCopyLimits::default()
            },
        )
        .unwrap_err();
        assert!(
            total.starts_with("character_setup_error:total_limit:"),
            "{total}"
        );
    }

    #[test]
    fn live2d_import_policy_rejects_arbitrary_import_when_disabled() {
        let fixture = Fixture::new("live2d-policy");
        let model = write_live2d_source(&fixture.root.join("source"));

        let error = import_character_source(
            &fixture.layout,
            CharacterRendererKindDto::Live2d,
            &model,
            false,
        )
        .unwrap_err();

        assert!(error.starts_with("character_setup_error:live2d_import_disabled:"));
    }

    #[test]
    fn windows_reparse_attribute_bit_is_detected_deterministically() {
        assert!(!has_reparse_attribute(0));
        assert!(!has_reparse_attribute(0x20));
        assert!(has_reparse_attribute(0x400));
        assert!(has_reparse_attribute(0x420));
    }

    #[test]
    fn inactive_import_preserves_active_and_active_kind_reimport_applies_immediately() {
        let fixture = Fixture::new("active-inactive-import");
        let first_static = fixture.root.join("first-static.png");
        write_png(&first_static, 2, 2, true);
        import_character_source(
            &fixture.layout,
            CharacterRendererKindDto::StaticImage,
            &first_static,
            true,
        )
        .unwrap();
        let first_static_id = crate::character::load_character_settings(&fixture.layout)
            .static_image_character_id
            .unwrap();
        let model = write_live2d_source(&fixture.root.join("inactive-live2d"));

        import_character_source(
            &fixture.layout,
            CharacterRendererKindDto::Live2d,
            &model,
            true,
        )
        .unwrap();
        let after_inactive = crate::character::load_character_settings(&fixture.layout);
        assert_eq!(
            after_inactive.active_character_id.as_deref(),
            Some(first_static_id.as_str())
        );

        let second_static = fixture.root.join("second-static.png");
        write_png(&second_static, 2, 2, true);
        import_character_source(
            &fixture.layout,
            CharacterRendererKindDto::StaticImage,
            &second_static,
            true,
        )
        .unwrap();
        let after_reimport = crate::character::load_character_settings(&fixture.layout);
        assert_eq!(
            after_reimport.active_character_id,
            after_reimport.static_image_character_id
        );
        assert!(!fixture.layout.characters.join(first_static_id).exists());
    }

    #[test]
    fn legacy_plus_static_import_materializes_legacy_and_switches_both_ways() {
        let fixture = Fixture::new("legacy-static-migration");
        std::fs::write(
            fixture.layout.characters.join("Legacy.model3.json"),
            r#"{"FileReferences":{}}"#,
        )
        .unwrap();
        let source = fixture.root.join("static.png");
        write_png(&source, 2, 2, true);

        let setup = import_character_source(
            &fixture.layout,
            CharacterRendererKindDto::StaticImage,
            &source,
            true,
        )
        .unwrap();

        assert!(setup.live2d.configured);
        assert!(setup.static_image.configured);
        assert_eq!(
            setup.active_renderer,
            Some(CharacterRendererKindDto::Live2d)
        );
        let migrated = crate::character::load_character_settings(&fixture.layout);
        let live2d_id = migrated.live2d_character_id.clone().unwrap();
        assert!(live2d_id.starts_with("managed-live2d-"));
        assert_eq!(
            migrated.active_character_id.as_deref(),
            Some(live2d_id.as_str())
        );

        let static_setup =
            select_active_renderer(&fixture.layout, CharacterRendererKindDto::StaticImage).unwrap();
        assert_eq!(
            static_setup.active_renderer,
            Some(CharacterRendererKindDto::StaticImage)
        );
        let live_setup =
            select_active_renderer(&fixture.layout, CharacterRendererKindDto::Live2d).unwrap();
        assert_eq!(
            live_setup.active_renderer,
            Some(CharacterRendererKindDto::Live2d)
        );
    }

    #[cfg(windows)]
    #[test]
    fn live2d_copy_rejects_reparse_escape_when_symlinks_are_available() {
        use std::os::windows::fs::symlink_file;

        let fixture = Fixture::new("live2d-reparse");
        let source = fixture.root.join("source");
        std::fs::create_dir_all(&source).unwrap();
        let outside = fixture.root.join("outside.moc3");
        std::fs::write(&outside, b"outside").unwrap();
        let link = source.join("linked.moc3");
        if let Err(error) = symlink_file(&outside, &link) {
            eprintln!("SKIP Live2D reparse regression: {error}");
            return;
        }
        let model = source.join("linked.model3.json");
        std::fs::write(&model, r#"{"FileReferences":{"Moc":"linked.moc3"}}"#).unwrap();

        let error = copy_live2d_references(
            &model,
            &fixture.root.join("copied"),
            Live2dCopyLimits::default(),
        )
        .unwrap_err();

        assert!(error.starts_with("character_setup_error:unsafe_reference:"));
    }
}
