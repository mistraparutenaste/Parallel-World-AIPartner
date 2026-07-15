//! Fail-closed discovery and validation of character profiles.

use std::{
    collections::HashSet,
    fs,
    io::Cursor,
    path::{Component, Path, PathBuf},
};

use image::{GenericImageView, ImageFormat, ImageReader, Limits};
use pw_contracts::{CharacterSettingsDto, MotionGroupDto};
use pw_platform::paths::AppDataLayout;
use serde::Deserialize;

use super::{CharacterManifest, find_first_model3, parse_model3_json};

const PROFILE_SCHEMA_VERSION: u16 = 1;
const MAX_NAME_SCALARS: usize = 128;
const MAX_EXPRESSIONS: usize = 32;
const MAX_IMAGE_DIMENSION: u32 = 4096;
const MAX_IMAGE_FILE_BYTES: u64 = 32 * 1024 * 1024;
pub(crate) const MAX_TOTAL_DECODED_BYTES: u64 = 256 * 1024 * 1024;
const MAX_DECODER_ALLOC_BYTES: u64 = 128 * 1024 * 1024;

/// Stable identity of the virtual fallback profile; explicit manifests may not use it.
pub const LEGACY_CHARACTER_ID: &str = "legacy-live2d";

#[derive(Debug, thiserror::Error)]
pub enum CharacterProfileError {
    #[error("failed to access {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("invalid character profile {path}: {message}")]
    InvalidProfile { path: PathBuf, message: String },
    #[error("character profile ID is duplicated: {0}")]
    DuplicateId(String),
    #[error("character selection is required")]
    SelectionRequired,
    #[error("active character is unavailable: {0}")]
    ActiveCharacterUnavailable(String),
    #[error("no character profile or legacy Live2D model is available")]
    NoCharacterAvailable,
    #[error("path escapes the characters root: {0}")]
    PathEscape(PathBuf),
    #[error("invalid character name: {0}")]
    InvalidName(String),
    #[error("duplicate static expression: {0}")]
    DuplicateExpression(String),
    #[error("default expression is unavailable: {0}")]
    DefaultExpressionUnavailable(String),
    #[error("too many static expressions: {0}")]
    TooManyExpressions(usize),
    #[error("image file exceeds 32 MiB ({bytes} bytes): {path}")]
    ImageFileTooLarge { path: PathBuf, bytes: u64 },
    #[error("invalid image {0}")]
    InvalidImage(PathBuf),
    #[error("animated WebP is unsupported: {0}")]
    AnimatedWebp(PathBuf),
    #[error("image must contain an alpha channel: {0}")]
    AlphaRequired(PathBuf),
    #[error("image dimensions exceed limits or are empty ({width}x{height}): {path}")]
    ImageDimensions {
        path: PathBuf,
        width: u32,
        height: u32,
    },
    #[error(
        "static expression dimensions do not match: expected {expected_width}x{expected_height}, got {actual_width}x{actual_height} at {path}"
    )]
    DimensionMismatch {
        path: PathBuf,
        expected_width: u32,
        expected_height: u32,
        actual_width: u32,
        actual_height: u32,
    },
    #[error("decoded RGBA total exceeds 256 MiB: {bytes} bytes")]
    DecodedImageLimit { bytes: u64 },
}

impl CharacterProfileError {
    #[must_use]
    pub(crate) fn stable_code(&self) -> &'static str {
        match self {
            Self::SelectionRequired => "selection_required",
            Self::ActiveCharacterUnavailable(_) => "active_character_unavailable",
            Self::NoCharacterAvailable => "missing_asset",
            Self::Io { source, .. } if source.kind() == std::io::ErrorKind::NotFound => {
                "missing_asset"
            }
            Self::Io { .. } => "transient_asset_read",
            Self::InvalidProfile { .. }
            | Self::DuplicateId(_)
            | Self::PathEscape(_)
            | Self::InvalidName(_)
            | Self::DuplicateExpression(_)
            | Self::DefaultExpressionUnavailable(_)
            | Self::TooManyExpressions(_) => "invalid_manifest",
            Self::ImageFileTooLarge { .. }
            | Self::InvalidImage(_)
            | Self::AnimatedWebp(_)
            | Self::AlphaRequired(_)
            | Self::ImageDimensions { .. }
            | Self::DimensionMismatch { .. }
            | Self::DecodedImageLimit { .. } => "invalid_image",
        }
    }

    #[must_use]
    pub(crate) fn to_ipc_error(&self) -> String {
        let safe_message = pw_domain::runtime_health::redact_diagnostic(&self.to_string());
        format!(
            "character_profile_error:{}:{safe_message}",
            self.stable_code()
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CharacterCapabilities {
    pub expressions: Vec<String>,
    pub motions: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedStaticExpression {
    pub name: String,
    pub image_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolvedRenderer {
    Live2d {
        model_path: PathBuf,
        default_expression: Option<String>,
        expressions: Vec<String>,
        motion_groups: Vec<MotionGroupDto>,
    },
    StaticImage {
        default_expression: String,
        expressions: Vec<ResolvedStaticExpression>,
        width: u32,
        height: u32,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedCharacter {
    pub id: String,
    pub display_name: String,
    pub profile_root: PathBuf,
    pub renderer: ResolvedRenderer,
}

impl ResolvedCharacter {
    #[must_use]
    pub fn capabilities(&self) -> CharacterCapabilities {
        match &self.renderer {
            ResolvedRenderer::Live2d {
                expressions,
                motion_groups,
                ..
            } => CharacterCapabilities {
                expressions: expressions.clone(),
                motions: motion_groups
                    .iter()
                    .map(|group| group.name.clone())
                    .collect(),
            },
            ResolvedRenderer::StaticImage { expressions, .. } => CharacterCapabilities {
                expressions: expressions
                    .iter()
                    .map(|expression| expression.name.clone())
                    .collect(),
                motions: Vec::new(),
            },
        }
    }

    #[must_use]
    pub fn live2d_model_path(&self) -> Option<&Path> {
        match &self.renderer {
            ResolvedRenderer::Live2d { model_path, .. } => Some(model_path),
            ResolvedRenderer::StaticImage { .. } => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct CharacterCatalog {
    profiles: Vec<ResolvedCharacter>,
    explicit_profile_count: usize,
}

impl CharacterCatalog {
    /// Discovers and validates all explicit profiles, or the legacy `Live2D` model when none exist.
    ///
    /// # Errors
    ///
    /// Fails closed when discovery, parsing, path validation, or asset validation fails.
    pub fn discover(layout: &AppDataLayout) -> Result<Self, CharacterProfileError> {
        let characters_root = canonicalize(&layout.characters)?;
        let explicit = discover_profile_files(&layout.characters)?;
        if explicit.is_empty() {
            let model_path = find_first_model3(&layout.characters)
                .ok_or(CharacterProfileError::NoCharacterAvailable)?;
            return Ok(Self {
                profiles: vec![resolve_legacy(&characters_root, &model_path)?],
                explicit_profile_count: 0,
            });
        }

        let explicit_profile_count = explicit.len();
        let mut profiles = Vec::with_capacity(explicit.len());
        let mut ids = HashSet::with_capacity(explicit.len());
        for path in explicit {
            let profile = parse_profile(&characters_root, &path)?;
            if !ids.insert(profile.id.clone()) {
                return Err(CharacterProfileError::DuplicateId(profile.id));
            }
            profiles.push(profile);
        }
        Ok(Self {
            profiles,
            explicit_profile_count,
        })
    }

    /// True only for one disk-backed explicit profile, never for the virtual legacy profile.
    #[must_use]
    pub const fn has_single_explicit_profile(&self) -> bool {
        self.explicit_profile_count == 1
    }

    #[must_use]
    pub(crate) const fn has_explicit_profiles(&self) -> bool {
        self.explicit_profile_count != 0
    }

    #[must_use]
    pub(crate) fn profile_by_id(&self, id: &str) -> Option<&ResolvedCharacter> {
        self.profiles.iter().find(|profile| profile.id == id)
    }

    /// Resolves an exact configured ID, or the sole available profile.
    ///
    /// # Errors
    ///
    /// Returns selection and availability errors without falling through to another identity.
    pub fn resolve(
        &self,
        settings: &CharacterSettingsDto,
    ) -> Result<ResolvedCharacter, CharacterProfileError> {
        if let Some(active_id) = settings.active_character_id.as_deref() {
            if self.explicit_profile_count == 0 && active_id == LEGACY_CHARACTER_ID {
                return Err(CharacterProfileError::ActiveCharacterUnavailable(
                    active_id.to_owned(),
                ));
            }
            return self
                .profiles
                .iter()
                .find(|profile| profile.id == active_id)
                .cloned()
                .ok_or_else(|| {
                    CharacterProfileError::ActiveCharacterUnavailable(active_id.to_owned())
                });
        }
        match self.profiles.as_slice() {
            [profile] => Ok(profile.clone()),
            [] => Err(CharacterProfileError::NoCharacterAvailable),
            _ => Err(CharacterProfileError::SelectionRequired),
        }
    }
}

pub(crate) fn validate_profile_manifest(
    layout: &AppDataLayout,
    manifest_path: &Path,
) -> Result<ResolvedCharacter, CharacterProfileError> {
    let characters_root = canonicalize(&layout.characters)?;
    parse_profile(&characters_root, manifest_path)
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DiskProfile {
    schema_version: u16,
    id: String,
    display_name: String,
    renderer: DiskRenderer,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum DiskRenderer {
    Live2d {
        model: PathBuf,
        default_expression: Option<String>,
    },
    StaticImage {
        default_expression: String,
        expressions: Vec<DiskStaticExpression>,
    },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DiskStaticExpression {
    name: String,
    file: PathBuf,
}

fn discover_profile_files(characters: &Path) -> Result<Vec<PathBuf>, CharacterProfileError> {
    let entries = fs::read_dir(characters).map_err(|source| CharacterProfileError::Io {
        path: characters.to_path_buf(),
        source,
    })?;
    let mut paths = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|source| CharacterProfileError::Io {
            path: characters.to_path_buf(),
            source,
        })?;
        let entry_path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|source| CharacterProfileError::Io {
                path: entry_path.clone(),
                source,
            })?;
        if !file_type.is_dir() && !file_type.is_symlink() {
            continue;
        }
        let entry_metadata =
            fs::metadata(&entry_path).map_err(|source| CharacterProfileError::Io {
                path: entry_path.clone(),
                source,
            })?;
        if !entry_metadata.is_dir() {
            continue;
        }
        let manifest_path = entry_path.join("character.json");
        match fs::symlink_metadata(&manifest_path) {
            Ok(_) => paths.push(manifest_path),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(source) => {
                return Err(CharacterProfileError::Io {
                    path: manifest_path,
                    source,
                });
            }
        }
    }
    paths.sort();
    Ok(paths)
}

fn parse_profile(
    characters_root: &Path,
    manifest_path: &Path,
) -> Result<ResolvedCharacter, CharacterProfileError> {
    let profile_root = manifest_path
        .parent()
        .ok_or_else(|| CharacterProfileError::PathEscape(manifest_path.to_path_buf()))?;
    let profile_root = canonicalize_under(characters_root, profile_root)?;
    let manifest_path = canonical_regular_file(characters_root, manifest_path)?;
    let content =
        fs::read_to_string(&manifest_path).map_err(|source| CharacterProfileError::Io {
            path: manifest_path.clone(),
            source,
        })?;
    let disk: DiskProfile =
        serde_json::from_str(&content).map_err(|error| CharacterProfileError::InvalidProfile {
            path: manifest_path.clone(),
            message: error.to_string(),
        })?;
    if disk.schema_version != PROFILE_SCHEMA_VERSION {
        return Err(CharacterProfileError::InvalidProfile {
            path: manifest_path,
            message: format!("unsupported schema version {}", disk.schema_version),
        });
    }
    validate_name(&disk.id)?;
    if disk.id == LEGACY_CHARACTER_ID {
        return Err(CharacterProfileError::InvalidProfile {
            path: manifest_path,
            message: format!("character ID is reserved: {LEGACY_CHARACTER_ID}"),
        });
    }
    validate_name(&disk.display_name)?;

    let renderer = match disk.renderer {
        DiskRenderer::Live2d {
            model,
            default_expression,
        } => resolve_live2d(characters_root, &profile_root, &model, default_expression)?,
        DiskRenderer::StaticImage {
            default_expression,
            expressions,
        } => resolve_static(
            characters_root,
            &profile_root,
            default_expression,
            expressions,
        )?,
    };

    Ok(ResolvedCharacter {
        id: disk.id,
        display_name: disk.display_name,
        profile_root,
        renderer,
    })
}

fn resolve_live2d(
    characters_root: &Path,
    profile_root: &Path,
    model: &Path,
    explicit_default: Option<String>,
) -> Result<ResolvedRenderer, CharacterProfileError> {
    let model_path = resolve_asset_path(characters_root, profile_root, model)?;
    let content = fs::read_to_string(&model_path).map_err(|source| CharacterProfileError::Io {
        path: model_path.clone(),
        source,
    })?;
    let manifest = parse_model3_json(&model_path, &content).map_err(|error| {
        CharacterProfileError::InvalidProfile {
            path: model_path.clone(),
            message: error.to_string(),
        }
    })?;
    let default_expression = resolve_live2d_default(&manifest, explicit_default)?;
    Ok(live2d_renderer(manifest, default_expression))
}

fn resolve_legacy(
    characters_root: &Path,
    model_path: &Path,
) -> Result<ResolvedCharacter, CharacterProfileError> {
    let model_path = canonical_regular_file(characters_root, model_path)?;
    let content = fs::read_to_string(&model_path).map_err(|source| CharacterProfileError::Io {
        path: model_path.clone(),
        source,
    })?;
    let manifest = parse_model3_json(&model_path, &content).map_err(|error| {
        CharacterProfileError::InvalidProfile {
            path: model_path.clone(),
            message: error.to_string(),
        }
    })?;
    let profile_root = model_path
        .parent()
        .map_or_else(|| characters_root.to_path_buf(), Path::to_path_buf);
    let default_expression = resolve_live2d_default(&manifest, None)?;
    Ok(ResolvedCharacter {
        id: LEGACY_CHARACTER_ID.into(),
        display_name: "Legacy Live2D".into(),
        profile_root,
        renderer: live2d_renderer(manifest, default_expression),
    })
}

fn resolve_live2d_default(
    manifest: &CharacterManifest,
    explicit: Option<String>,
) -> Result<Option<String>, CharacterProfileError> {
    if let Some(name) = explicit {
        validate_name(&name)?;
        if !manifest.expressions.iter().any(|known| known == &name) {
            return Err(CharacterProfileError::DefaultExpressionUnavailable(name));
        }
        return Ok(Some(name));
    }
    Ok(manifest
        .expressions
        .iter()
        .find(|name| name.as_str() == "Normal")
        .or_else(|| manifest.expressions.first())
        .cloned())
}

fn live2d_renderer(
    manifest: CharacterManifest,
    default_expression: Option<String>,
) -> ResolvedRenderer {
    ResolvedRenderer::Live2d {
        model_path: manifest.model_path,
        default_expression,
        expressions: manifest.expressions,
        motion_groups: manifest
            .motion_groups
            .into_iter()
            .map(|(name, motion_count)| MotionGroupDto { name, motion_count })
            .collect(),
    }
}

fn resolve_static(
    characters_root: &Path,
    profile_root: &Path,
    default_expression: String,
    expressions: Vec<DiskStaticExpression>,
) -> Result<ResolvedRenderer, CharacterProfileError> {
    if expressions.len() > MAX_EXPRESSIONS {
        return Err(CharacterProfileError::TooManyExpressions(expressions.len()));
    }
    if expressions.is_empty() {
        return Err(CharacterProfileError::DefaultExpressionUnavailable(
            default_expression,
        ));
    }
    validate_name(&default_expression)?;
    let mut names = HashSet::with_capacity(expressions.len());
    for expression in &expressions {
        validate_name(&expression.name)?;
        if !names.insert(expression.name.clone()) {
            return Err(CharacterProfileError::DuplicateExpression(
                expression.name.clone(),
            ));
        }
    }
    if !names.contains(&default_expression) {
        return Err(CharacterProfileError::DefaultExpressionUnavailable(
            default_expression,
        ));
    }

    let mut total_decoded = 0_u64;
    let mut expected_dimensions = None;
    let mut resolved = Vec::with_capacity(expressions.len());
    for expression in expressions {
        let image_path = resolve_asset_path(characters_root, profile_root, &expression.file)?;
        let (width, height, decoded_bytes) = validate_static_image(&image_path)?;
        if let Some((expected_width, expected_height)) = expected_dimensions {
            if (width, height) != (expected_width, expected_height) {
                return Err(CharacterProfileError::DimensionMismatch {
                    path: image_path,
                    expected_width,
                    expected_height,
                    actual_width: width,
                    actual_height: height,
                });
            }
        } else {
            expected_dimensions = Some((width, height));
        }
        add_decoded_bytes(&mut total_decoded, decoded_bytes)?;
        resolved.push(ResolvedStaticExpression {
            name: expression.name,
            image_path,
        });
    }
    let (width, height) = expected_dimensions.ok_or_else(|| {
        CharacterProfileError::DefaultExpressionUnavailable(default_expression.clone())
    })?;
    Ok(ResolvedRenderer::StaticImage {
        default_expression,
        expressions: resolved,
        width,
        height,
    })
}

fn validate_static_image(path: &Path) -> Result<(u32, u32, u64), CharacterProfileError> {
    let metadata = fs::metadata(path).map_err(|source| CharacterProfileError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    if metadata.len() > MAX_IMAGE_FILE_BYTES {
        return Err(CharacterProfileError::ImageFileTooLarge {
            path: path.to_path_buf(),
            bytes: metadata.len(),
        });
    }
    let bytes = fs::read(path).map_err(|source| CharacterProfileError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    if bytes.len() as u64 > MAX_IMAGE_FILE_BYTES {
        return Err(CharacterProfileError::ImageFileTooLarge {
            path: path.to_path_buf(),
            bytes: bytes.len() as u64,
        });
    }
    let guessed = image::guess_format(&bytes)
        .map_err(|_| CharacterProfileError::InvalidImage(path.to_path_buf()))?;
    let expected = image_format_from_extension(path)
        .ok_or_else(|| CharacterProfileError::InvalidImage(path.to_path_buf()))?;
    if guessed != expected || !matches!(guessed, ImageFormat::Png | ImageFormat::WebP) {
        return Err(CharacterProfileError::InvalidImage(path.to_path_buf()));
    }
    if guessed == ImageFormat::WebP && webp_contains_animation(&bytes) {
        return Err(CharacterProfileError::AnimatedWebp(path.to_path_buf()));
    }

    let dimensions = ImageReader::with_format(Cursor::new(bytes.as_slice()), guessed)
        .into_dimensions()
        .map_err(|_| CharacterProfileError::InvalidImage(path.to_path_buf()))?;
    let (width, height) = dimensions;
    if width == 0 || height == 0 || width > MAX_IMAGE_DIMENSION || height > MAX_IMAGE_DIMENSION {
        return Err(CharacterProfileError::ImageDimensions {
            path: path.to_path_buf(),
            width,
            height,
        });
    }

    let mut reader = ImageReader::with_format(Cursor::new(bytes), guessed);
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_IMAGE_DIMENSION);
    limits.max_image_height = Some(MAX_IMAGE_DIMENSION);
    limits.max_alloc = Some(MAX_DECODER_ALLOC_BYTES);
    reader.limits(limits);
    let decoded = reader
        .decode()
        .map_err(|_| CharacterProfileError::InvalidImage(path.to_path_buf()))?;
    if !decoded.color().has_alpha() {
        return Err(CharacterProfileError::AlphaRequired(path.to_path_buf()));
    }
    let decoded_dimensions = decoded.dimensions();
    if decoded_dimensions != dimensions {
        return Err(CharacterProfileError::InvalidImage(path.to_path_buf()));
    }
    let decoded_bytes = u64::from(width)
        .checked_mul(u64::from(height))
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or(CharacterProfileError::DecodedImageLimit { bytes: u64::MAX })?;
    Ok((width, height, decoded_bytes))
}

fn image_format_from_extension(path: &Path) -> Option<ImageFormat> {
    match path
        .extension()
        .and_then(|extension| extension.to_str())?
        .to_ascii_lowercase()
        .as_str()
    {
        "png" => Some(ImageFormat::Png),
        "webp" => Some(ImageFormat::WebP),
        _ => None,
    }
}

fn webp_contains_animation(bytes: &[u8]) -> bool {
    if bytes.len() < 12 || &bytes[..4] != b"RIFF" || &bytes[8..12] != b"WEBP" {
        return false;
    }
    let mut offset = 12_usize;
    while offset.checked_add(8).is_some_and(|end| end <= bytes.len()) {
        if &bytes[offset..offset + 4] == b"ANIM" {
            return true;
        }
        let size = u32::from_le_bytes([
            bytes[offset + 4],
            bytes[offset + 5],
            bytes[offset + 6],
            bytes[offset + 7],
        ]) as usize;
        let padded = size.saturating_add(size & 1);
        let Some(next) = offset
            .checked_add(8)
            .and_then(|value| value.checked_add(padded))
        else {
            return false;
        };
        if next > bytes.len() {
            return false;
        }
        offset = next;
    }
    false
}

fn validate_name(name: &str) -> Result<(), CharacterProfileError> {
    if name.is_empty() || name.chars().count() > MAX_NAME_SCALARS {
        return Err(CharacterProfileError::InvalidName(name.to_owned()));
    }
    Ok(())
}

fn resolve_asset_path(
    characters_root: &Path,
    profile_root: &Path,
    relative: &Path,
) -> Result<PathBuf, CharacterProfileError> {
    if relative.as_os_str().is_empty()
        || relative.is_absolute()
        || relative.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(CharacterProfileError::PathEscape(relative.to_path_buf()));
    }
    canonical_regular_file(characters_root, &profile_root.join(relative))
}

fn canonicalize(path: &Path) -> Result<PathBuf, CharacterProfileError> {
    path.canonicalize()
        .map_err(|source| CharacterProfileError::Io {
            path: path.to_path_buf(),
            source,
        })
}

fn canonicalize_under(
    characters_root: &Path,
    path: &Path,
) -> Result<PathBuf, CharacterProfileError> {
    let canonical = canonicalize(path)?;
    if !canonical.starts_with(characters_root) {
        return Err(CharacterProfileError::PathEscape(canonical));
    }
    Ok(canonical)
}

fn canonical_regular_file(
    characters_root: &Path,
    path: &Path,
) -> Result<PathBuf, CharacterProfileError> {
    let canonical = canonicalize_under(characters_root, path)?;
    let metadata = fs::metadata(&canonical).map_err(|source| CharacterProfileError::Io {
        path: canonical.clone(),
        source,
    })?;
    if !metadata.is_file() {
        return Err(CharacterProfileError::InvalidProfile {
            path: canonical,
            message: "asset is not a regular file".into(),
        });
    }
    Ok(canonical)
}

fn ensure_total_decoded_limit(bytes: u64) -> Result<(), CharacterProfileError> {
    if bytes > MAX_TOTAL_DECODED_BYTES {
        Err(CharacterProfileError::DecodedImageLimit { bytes })
    } else {
        Ok(())
    }
}

fn add_decoded_bytes(total: &mut u64, decoded_bytes: u64) -> Result<(), CharacterProfileError> {
    let updated = total.checked_add(decoded_bytes).unwrap_or(u64::MAX);
    ensure_total_decoded_limit(updated)?;
    *total = updated;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{
        fs::File,
        io::BufWriter,
        path::{Path, PathBuf},
        sync::atomic::{AtomicU64, Ordering},
    };

    use image::{
        ExtendedColorType, ImageEncoder,
        codecs::{png::PngEncoder, webp::WebPEncoder},
    };
    use pw_contracts::CharacterSettingsDto;
    use pw_platform::paths::AppDataLayout;

    use super::{
        CharacterCatalog, CharacterProfileError, MAX_TOTAL_DECODED_BYTES, add_decoded_bytes,
    };

    static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);

    struct Fixture {
        layout: AppDataLayout,
        root: PathBuf,
    }

    impl Fixture {
        fn new(tag: &str) -> Self {
            let serial = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
            let root = std::env::temp_dir().join(format!(
                "pw-character-catalog-{tag}-{}-{serial}",
                std::process::id()
            ));
            let layout = AppDataLayout::under(root.clone());
            layout.create_all().unwrap();
            Self { layout, root }
        }

        fn profile_dir(&self, directory: &str) -> PathBuf {
            let path = self.layout.characters.join(directory);
            std::fs::create_dir_all(&path).unwrap();
            path
        }

        fn add_static_profile(&self, directory: &str, id: &str, expressions: &[(&str, &str)]) {
            let profile = self.profile_dir(directory);
            let entries: Vec<_> = expressions
                .iter()
                .map(|(name, file)| serde_json::json!({ "name": name, "file": file }))
                .collect();
            let manifest = serde_json::json!({
                "schema_version": 1,
                "id": id,
                "display_name": format!("{id} display"),
                "renderer": {
                    "kind": "static_image",
                    "default_expression": expressions[0].0,
                    "expressions": entries,
                }
            });
            std::fs::write(
                profile.join("character.json"),
                serde_json::to_vec_pretty(&manifest).unwrap(),
            )
            .unwrap();
        }

        fn add_valid_static_profile(&self, directory: &str, id: &str) -> PathBuf {
            let profile = self.profile_dir(directory);
            write_png(&profile.join("neutral.png"), 2, 2, true);
            self.add_static_profile(directory, id, &[("neutral", "neutral.png")]);
            profile
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    fn write_png(path: &Path, width: u32, height: u32, alpha: bool) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let channels = if alpha { 4 } else { 3 };
        let mut pixels = vec![255; width as usize * height as usize * channels];
        if alpha {
            for pixel in pixels.chunks_exact_mut(4) {
                pixel[3] = 128;
            }
        }
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

    fn write_webp(path: &Path, width: u32, height: u32) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let pixels = vec![128; width as usize * height as usize * 4];
        let file = BufWriter::new(File::create(path).unwrap());
        WebPEncoder::new_lossless(file)
            .encode(&pixels, width, height, ExtendedColorType::Rgba8)
            .unwrap();
    }

    fn default_settings() -> CharacterSettingsDto {
        CharacterSettingsDto::default()
    }

    #[test]
    #[allow(clippy::too_many_lines)] // Exhaustively lists every public error variant in one table.
    fn every_profile_error_has_a_stable_ipc_code() {
        let errors = vec![
            (
                CharacterProfileError::Io {
                    path: PathBuf::from("missing"),
                    source: std::io::Error::from(std::io::ErrorKind::NotFound),
                },
                "missing_asset",
            ),
            (
                CharacterProfileError::Io {
                    path: PathBuf::from("busy"),
                    source: std::io::Error::from(std::io::ErrorKind::PermissionDenied),
                },
                "transient_asset_read",
            ),
            (
                CharacterProfileError::InvalidProfile {
                    path: PathBuf::new(),
                    message: String::new(),
                },
                "invalid_manifest",
            ),
            (
                CharacterProfileError::DuplicateId(String::new()),
                "invalid_manifest",
            ),
            (
                CharacterProfileError::SelectionRequired,
                "selection_required",
            ),
            (
                CharacterProfileError::ActiveCharacterUnavailable(String::new()),
                "active_character_unavailable",
            ),
            (CharacterProfileError::NoCharacterAvailable, "missing_asset"),
            (
                CharacterProfileError::PathEscape(PathBuf::new()),
                "invalid_manifest",
            ),
            (
                CharacterProfileError::InvalidName(String::new()),
                "invalid_manifest",
            ),
            (
                CharacterProfileError::DuplicateExpression(String::new()),
                "invalid_manifest",
            ),
            (
                CharacterProfileError::DefaultExpressionUnavailable(String::new()),
                "invalid_manifest",
            ),
            (
                CharacterProfileError::TooManyExpressions(33),
                "invalid_manifest",
            ),
            (
                CharacterProfileError::ImageFileTooLarge {
                    path: PathBuf::new(),
                    bytes: 1,
                },
                "invalid_image",
            ),
            (
                CharacterProfileError::InvalidImage(PathBuf::new()),
                "invalid_image",
            ),
            (
                CharacterProfileError::AnimatedWebp(PathBuf::new()),
                "invalid_image",
            ),
            (
                CharacterProfileError::AlphaRequired(PathBuf::new()),
                "invalid_image",
            ),
            (
                CharacterProfileError::ImageDimensions {
                    path: PathBuf::new(),
                    width: 0,
                    height: 0,
                },
                "invalid_image",
            ),
            (
                CharacterProfileError::DimensionMismatch {
                    path: PathBuf::new(),
                    expected_width: 1,
                    expected_height: 1,
                    actual_width: 2,
                    actual_height: 2,
                },
                "invalid_image",
            ),
            (
                CharacterProfileError::DecodedImageLimit { bytes: 1 },
                "invalid_image",
            ),
        ];

        for (error, expected) in errors {
            assert_eq!(error.stable_code(), expected, "{error}");
            assert!(
                error
                    .to_ipc_error()
                    .starts_with(&format!("character_profile_error:{expected}:"))
            );
        }
    }

    #[cfg(windows)]
    fn create_symlink_or_report(target: &Path, link: &Path) -> bool {
        use std::os::windows::fs::symlink_file;

        match symlink_file(target, link) {
            Ok(()) => true,
            Err(error) if std::env::var_os("CI").is_some() => {
                panic!("CI must support the symlink escape regression test: {error}")
            }
            Err(error) => {
                eprintln!(
                    "SKIP symlink escape regression: cannot create {} -> {}: {error}",
                    link.display(),
                    target.display()
                );
                false
            }
        }
    }

    #[test]
    fn one_explicit_profile_is_selected_without_existing_setting() {
        let fixture = Fixture::new("one-profile");
        let profile = fixture.add_valid_static_profile("epsilon", "epsilon-static");

        let catalog = CharacterCatalog::discover(&fixture.layout).unwrap();
        let resolved = catalog.resolve(&default_settings()).unwrap();

        assert_eq!(resolved.id, "epsilon-static");
        assert!(
            resolved
                .profile_root
                .starts_with(profile.canonicalize().unwrap())
        );
    }

    #[test]
    fn reserved_virtual_legacy_id_is_rejected_for_an_explicit_profile() {
        let fixture = Fixture::new("reserved-explicit-id");
        fixture.add_valid_static_profile("reserved", "legacy-live2d");

        let error = CharacterCatalog::discover(&fixture.layout).unwrap_err();

        assert_eq!(error.stable_code(), "invalid_manifest");
    }

    #[test]
    fn reserved_explicit_profile_never_falls_back_to_a_legacy_model() {
        let fixture = Fixture::new("reserved-explicit-with-legacy");
        fixture.add_valid_static_profile("reserved", "legacy-live2d");
        std::fs::write(
            fixture.layout.characters.join("fallback.model3.json"),
            r#"{"FileReferences":{}}"#,
        )
        .unwrap();

        let error = CharacterCatalog::discover(&fixture.layout).unwrap_err();

        assert_eq!(error.stable_code(), "invalid_manifest");
    }

    #[test]
    fn normal_explicit_profile_id_remains_selectable() {
        let fixture = Fixture::new("normal-explicit-id");
        fixture.add_valid_static_profile("epsilon", "epsilon-static");

        let resolved = CharacterCatalog::discover(&fixture.layout)
            .unwrap()
            .resolve(&default_settings())
            .unwrap();

        assert_eq!(resolved.id, "epsilon-static");
    }

    #[test]
    fn persisted_reserved_id_does_not_select_the_virtual_legacy_profile() {
        let fixture = Fixture::new("persisted-reserved-legacy");
        std::fs::write(
            fixture.layout.characters.join("legacy.model3.json"),
            r#"{"FileReferences":{}}"#,
        )
        .unwrap();
        let mut settings = default_settings();
        settings.active_character_id = Some("legacy-live2d".into());
        let catalog = CharacterCatalog::discover(&fixture.layout).unwrap();

        assert!(matches!(
            catalog.resolve(&settings),
            Err(CharacterProfileError::ActiveCharacterUnavailable(id)) if id == "legacy-live2d"
        ));
    }

    #[test]
    fn non_animated_webp_profile_is_accepted() {
        let fixture = Fixture::new("valid-webp");
        let profile = fixture.profile_dir("webp");
        write_webp(&profile.join("neutral.webp"), 2, 2);
        fixture.add_static_profile("webp", "webp", &[("neutral", "neutral.webp")]);

        let catalog = CharacterCatalog::discover(&fixture.layout).unwrap();
        let resolved = catalog.resolve(&default_settings()).unwrap();

        assert_eq!(resolved.id, "webp");
    }

    #[test]
    fn explicit_live2d_profile_resolves_canonical_model_and_default_expression() {
        let fixture = Fixture::new("explicit-live2d");
        let profile = fixture.profile_dir("live2d");
        let model = profile.join("model").join("Epsilon.model3.json");
        std::fs::create_dir_all(model.parent().unwrap()).unwrap();
        std::fs::write(
            &model,
            r#"{"FileReferences":{"Expressions":[{"Name":"Normal"},{"Name":"Smile"}]}}"#,
        )
        .unwrap();
        let manifest = serde_json::json!({
            "schema_version": 1,
            "id": "live2d",
            "display_name": "Live2D",
            "renderer": {
                "kind": "live2d",
                "model": "model/Epsilon.model3.json",
                "default_expression": "Smile"
            }
        });
        std::fs::write(
            profile.join("character.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();

        let catalog = CharacterCatalog::discover(&fixture.layout).unwrap();
        let resolved = catalog.resolve(&default_settings()).unwrap();

        assert_eq!(
            resolved.live2d_model_path(),
            Some(model.canonicalize().unwrap().as_path())
        );
        let super::ResolvedRenderer::Live2d {
            default_expression, ..
        } = resolved.renderer
        else {
            panic!("expected Live2D")
        };
        assert_eq!(default_expression.as_deref(), Some("Smile"));
    }

    #[test]
    fn multiple_profiles_without_active_id_require_selection() {
        let fixture = Fixture::new("selection-required");
        fixture.add_valid_static_profile("one", "one");
        fixture.add_valid_static_profile("two", "two");

        let catalog = CharacterCatalog::discover(&fixture.layout).unwrap();

        assert!(matches!(
            catalog.resolve(&default_settings()),
            Err(CharacterProfileError::SelectionRequired)
        ));
    }

    #[test]
    fn active_id_is_exact_and_missing_id_never_switches_identity() {
        let fixture = Fixture::new("active-id");
        fixture.add_valid_static_profile("one", "one");
        fixture.add_valid_static_profile("two", "two");
        let catalog = CharacterCatalog::discover(&fixture.layout).unwrap();

        let selected = catalog
            .resolve(&CharacterSettingsDto {
                active_character_id: Some("two".into()),
                ..default_settings()
            })
            .unwrap();
        assert_eq!(selected.id, "two");
        assert!(matches!(
            catalog.resolve(&CharacterSettingsDto {
                active_character_id: Some("missing".into()),
                ..default_settings()
            }),
            Err(CharacterProfileError::ActiveCharacterUnavailable(id)) if id == "missing"
        ));
    }

    #[test]
    fn duplicate_ids_are_rejected() {
        let fixture = Fixture::new("duplicate-id");
        fixture.add_valid_static_profile("one", "same");
        fixture.add_valid_static_profile("two", "same");

        assert!(matches!(
            CharacterCatalog::discover(&fixture.layout),
            Err(CharacterProfileError::DuplicateId(id)) if id == "same"
        ));
    }

    #[test]
    fn rejects_parent_and_absolute_asset_paths() {
        let fixture = Fixture::new("unsafe-paths");
        write_png(&fixture.layout.characters.join("outside.png"), 1, 1, true);
        fixture.add_static_profile("parent", "parent", &[("neutral", "../outside.png")]);
        assert!(matches!(
            CharacterCatalog::discover(&fixture.layout),
            Err(CharacterProfileError::PathEscape(_))
        ));

        std::fs::remove_dir_all(fixture.layout.characters.join("parent")).unwrap();
        let absolute = fixture.layout.characters.join("outside.png");
        fixture.add_static_profile(
            "absolute",
            "absolute",
            &[("neutral", absolute.to_str().unwrap())],
        );
        assert!(matches!(
            CharacterCatalog::discover(&fixture.layout),
            Err(CharacterProfileError::PathEscape(_))
        ));
    }

    #[test]
    fn character_json_directory_blocks_legacy_fallback() {
        let fixture = Fixture::new("manifest-directory");
        let invalid_profile = fixture.profile_dir("invalid-profile");
        std::fs::create_dir_all(invalid_profile.join("character.json")).unwrap();
        let legacy = fixture.profile_dir("legacy").join("Legacy.model3.json");
        std::fs::write(&legacy, "{}").unwrap();

        assert!(matches!(
            CharacterCatalog::discover(&fixture.layout),
            Err(CharacterProfileError::InvalidProfile { .. })
        ));
    }

    #[cfg(windows)]
    #[test]
    fn broken_character_json_symlink_blocks_legacy_fallback_when_symlinks_are_available() {
        let fixture = Fixture::new("broken-manifest-link");
        let invalid_profile = fixture.profile_dir("invalid-profile");
        let missing_target = fixture.root.join("missing-character.json");
        if !create_symlink_or_report(&missing_target, &invalid_profile.join("character.json")) {
            return;
        }
        let legacy = fixture.profile_dir("legacy").join("Legacy.model3.json");
        std::fs::write(&legacy, "{}").unwrap();

        assert!(matches!(
            CharacterCatalog::discover(&fixture.layout),
            Err(CharacterProfileError::Io { .. })
        ));
    }

    #[cfg(windows)]
    #[test]
    fn rejects_symlink_escape_when_symlinks_are_available() {
        let fixture = Fixture::new("symlink-escape");
        let outside = fixture.root.join("outside.png");
        write_png(&outside, 1, 1, true);
        let profile = fixture.profile_dir("linked");
        let link = profile.join("neutral.png");
        if !create_symlink_or_report(&outside, &link) {
            return;
        }
        fixture.add_static_profile("linked", "linked", &[("neutral", "neutral.png")]);

        assert!(matches!(
            CharacterCatalog::discover(&fixture.layout),
            Err(CharacterProfileError::PathEscape(_))
        ));
    }

    #[test]
    fn rejects_false_extension_corrupt_and_animated_webp_assets() {
        let fixture = Fixture::new("invalid-images");
        let profile = fixture.profile_dir("false-extension");
        write_png(&profile.join("neutral.webp"), 1, 1, true);
        fixture.add_static_profile(
            "false-extension",
            "false-extension",
            &[("neutral", "neutral.webp")],
        );
        assert!(matches!(
            CharacterCatalog::discover(&fixture.layout),
            Err(CharacterProfileError::InvalidImage(_))
        ));

        std::fs::remove_dir_all(&fixture.layout.characters).unwrap();
        std::fs::create_dir_all(&fixture.layout.characters).unwrap();
        let corrupt = fixture.profile_dir("corrupt");
        std::fs::write(corrupt.join("neutral.png"), b"\x89PNG\r\n\x1a\ntruncated").unwrap();
        fixture.add_static_profile("corrupt", "corrupt", &[("neutral", "neutral.png")]);
        assert!(matches!(
            CharacterCatalog::discover(&fixture.layout),
            Err(CharacterProfileError::InvalidImage(_))
        ));

        std::fs::remove_dir_all(&fixture.layout.characters).unwrap();
        std::fs::create_dir_all(&fixture.layout.characters).unwrap();
        let animated = fixture.profile_dir("animated");
        std::fs::write(
            animated.join("neutral.webp"),
            b"RIFF\x0c\x00\x00\x00WEBPANIM\x00\x00\x00\x00",
        )
        .unwrap();
        fixture.add_static_profile("animated", "animated", &[("neutral", "neutral.webp")]);
        assert!(matches!(
            CharacterCatalog::discover(&fixture.layout),
            Err(CharacterProfileError::AnimatedWebp(_))
        ));
    }

    #[test]
    fn rejects_non_alpha_and_mismatched_dimensions() {
        let fixture = Fixture::new("pixel-contract");
        let profile = fixture.profile_dir("no-alpha");
        write_png(&profile.join("neutral.png"), 2, 2, false);
        fixture.add_static_profile("no-alpha", "no-alpha", &[("neutral", "neutral.png")]);
        assert!(matches!(
            CharacterCatalog::discover(&fixture.layout),
            Err(CharacterProfileError::AlphaRequired(_))
        ));

        std::fs::remove_dir_all(&fixture.layout.characters).unwrap();
        std::fs::create_dir_all(&fixture.layout.characters).unwrap();
        let mismatch = fixture.profile_dir("mismatch");
        write_png(&mismatch.join("one.png"), 2, 2, true);
        write_png(&mismatch.join("two.png"), 3, 2, true);
        fixture.add_static_profile(
            "mismatch",
            "mismatch",
            &[("one", "one.png"), ("two", "two.png")],
        );
        assert!(matches!(
            CharacterCatalog::discover(&fixture.layout),
            Err(CharacterProfileError::DimensionMismatch { .. })
        ));
    }

    #[test]
    fn rejects_expression_count_dimensions_file_bytes_and_decoded_total_limits() {
        let fixture = Fixture::new("limits");
        let profile = fixture.profile_dir("count");
        write_png(&profile.join("same.png"), 1, 1, true);
        let entries: Vec<_> = (0..33)
            .map(|index| (format!("expression-{index}"), "same.png".to_owned()))
            .collect();
        let manifest = serde_json::json!({
            "schema_version": 1,
            "id": "count",
            "display_name": "count",
            "renderer": {
                "kind": "static_image",
                "default_expression": "expression-0",
                "expressions": entries.iter().map(|(name, file)| serde_json::json!({"name": name, "file": file})).collect::<Vec<_>>()
            }
        });
        std::fs::write(
            profile.join("character.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        assert!(matches!(
            CharacterCatalog::discover(&fixture.layout),
            Err(CharacterProfileError::TooManyExpressions(33))
        ));

        std::fs::remove_dir_all(&fixture.layout.characters).unwrap();
        std::fs::create_dir_all(&fixture.layout.characters).unwrap();
        let dimensions = fixture.profile_dir("dimensions");
        write_png(&dimensions.join("large.png"), 4097, 1, true);
        fixture.add_static_profile("dimensions", "dimensions", &[("large", "large.png")]);
        assert!(matches!(
            CharacterCatalog::discover(&fixture.layout),
            Err(CharacterProfileError::ImageDimensions { .. })
        ));

        std::fs::remove_dir_all(&fixture.layout.characters).unwrap();
        std::fs::create_dir_all(&fixture.layout.characters).unwrap();
        let bytes = fixture.profile_dir("bytes");
        File::create(bytes.join("large.png"))
            .unwrap()
            .set_len(32 * 1024 * 1024 + 1)
            .unwrap();
        fixture.add_static_profile("bytes", "bytes", &[("large", "large.png")]);
        assert!(matches!(
            CharacterCatalog::discover(&fixture.layout),
            Err(CharacterProfileError::ImageFileTooLarge { .. })
        ));

        let mut decoded_total = 0;
        for _ in 0..4 {
            add_decoded_bytes(&mut decoded_total, MAX_TOTAL_DECODED_BYTES / 4).unwrap();
        }
        assert_eq!(decoded_total, MAX_TOTAL_DECODED_BYTES);
        assert!(matches!(
            add_decoded_bytes(&mut decoded_total, 1),
            Err(CharacterProfileError::DecodedImageLimit { .. })
        ));
        assert_eq!(decoded_total, MAX_TOTAL_DECODED_BYTES);
    }

    #[test]
    fn rejects_duplicate_empty_and_missing_default_expression_names() {
        let fixture = Fixture::new("expression-names");
        let profile = fixture.profile_dir("duplicate");
        write_png(&profile.join("same.png"), 1, 1, true);
        fixture.add_static_profile(
            "duplicate",
            "duplicate",
            &[("same", "same.png"), ("same", "same.png")],
        );
        assert!(matches!(
            CharacterCatalog::discover(&fixture.layout),
            Err(CharacterProfileError::DuplicateExpression(_))
        ));

        std::fs::remove_dir_all(&fixture.layout.characters).unwrap();
        std::fs::create_dir_all(&fixture.layout.characters).unwrap();
        let profile = fixture.profile_dir("empty");
        write_png(&profile.join("same.png"), 1, 1, true);
        fixture.add_static_profile("empty", "empty", &[("", "same.png")]);
        assert!(matches!(
            CharacterCatalog::discover(&fixture.layout),
            Err(CharacterProfileError::InvalidName(_))
        ));

        std::fs::remove_dir_all(&fixture.layout.characters).unwrap();
        std::fs::create_dir_all(&fixture.layout.characters).unwrap();
        let profile = fixture.profile_dir("default");
        write_png(&profile.join("same.png"), 1, 1, true);
        let manifest = serde_json::json!({
            "schema_version": 1,
            "id": "default",
            "display_name": "default",
            "renderer": {
                "kind": "static_image",
                "default_expression": "missing",
                "expressions": [{"name": "same", "file": "same.png"}]
            }
        });
        std::fs::write(
            profile.join("character.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        assert!(matches!(
            CharacterCatalog::discover(&fixture.layout),
            Err(CharacterProfileError::DefaultExpressionUnavailable(_))
        ));
    }

    #[test]
    fn unknown_manifest_fields_are_rejected() {
        let fixture = Fixture::new("unknown-field");
        let profile = fixture.profile_dir("unknown");
        write_png(&profile.join("neutral.png"), 1, 1, true);
        let manifest = serde_json::json!({
            "schema_version": 1,
            "id": "unknown",
            "display_name": "unknown",
            "unexpected": true,
            "renderer": {
                "kind": "static_image",
                "default_expression": "neutral",
                "expressions": [{"name": "neutral", "file": "neutral.png"}]
            }
        });
        std::fs::write(
            profile.join("character.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();

        assert!(matches!(
            CharacterCatalog::discover(&fixture.layout),
            Err(CharacterProfileError::InvalidProfile { .. })
        ));
    }

    #[test]
    fn legacy_discovery_remains_sorted_and_only_applies_without_explicit_profiles() {
        let fixture = Fixture::new("legacy");
        let first = fixture.layout.characters.join("a").join("A.model3.json");
        let second = fixture.layout.characters.join("z").join("Z.model3.json");
        std::fs::create_dir_all(first.parent().unwrap()).unwrap();
        std::fs::create_dir_all(second.parent().unwrap()).unwrap();
        std::fs::write(
            &first,
            r#"{"FileReferences":{"Expressions":[{"Name":"Normal"}],"Motions":{"Idle":[{}]}}}"#,
        )
        .unwrap();
        std::fs::write(&second, "{}").unwrap();

        let catalog = CharacterCatalog::discover(&fixture.layout).unwrap();
        let resolved = catalog.resolve(&default_settings()).unwrap();

        assert_eq!(resolved.id, "legacy-live2d");
        assert_eq!(resolved.capabilities().expressions, ["Normal"]);
        assert_eq!(resolved.capabilities().motions, ["Idle"]);
        assert_eq!(
            resolved.live2d_model_path(),
            Some(first.canonicalize().unwrap().as_path())
        );
    }
}
