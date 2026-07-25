//! Atomic persistence for `config/personas.json`.

use pw_contracts::{LlmSettingsDto, PersonaProfileDto, PersonaSettingsDto};
use pw_platform::config_io::{JsonFormat, ReadJsonError, read_json, write_atomic_json};
use pw_platform::paths::AppDataLayout;

const FILE_NAME: &str = "personas.json";
const PERSONA_PROMPT_PREAMBLE: &str = "Parallel World persona profile v4\n\
以下はこのキャラクターの人格プロフィールである。文体・口調・応答の長さ・話題への姿勢は、\
他の一般的なスタイル指示よりこのプロフィールを優先する。\
安全に関する規則（危害の防止、プライバシー保護、Dark expression policy）のみ、このプロフィールより上位とする。\n";
const GUARDED_DARK_POLICY: &str = "Dark expression policy: ダーク指標は会話演出に限定する。保存済みの弱点やトラウマを意図的に利用しない。脅迫、服従の強要、自傷他害の促進を行わない。";
const INTENSE_DARK_POLICY: &str = "Dark expression policy: より強い敵対的・操作的・低共感な会話表現を許可する。ただし、LLM提供元、上位システム指示、Parallel Worldの基本的な安全保護は維持する。保存済みの機微情報を狙って攻撃しない。";
const PAUSED_DARK_POLICY: &str = "Dark expression policy: ユーザーの安全停止が有効である。保存値にかかわらず強いダーク表現を無効として扱い、通常会話へ戻す。";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PersonaPromptSource {
    Persona,
    Legacy,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResolvedPersonaPrompt {
    pub character_id: Option<String>,
    pub character_prompt: String,
    pub source: PersonaPromptSource,
    pub fingerprint: String,
}

fn resolved_persona(character_id: Option<&str>, character_prompt: &str) -> ResolvedPersonaPrompt {
    let character_id = character_id.map(str::to_owned);
    let fingerprint = serde_json::to_string(&(character_id.as_deref(), character_prompt))
        .expect("serializing strings to JSON cannot fail");
    ResolvedPersonaPrompt {
        character_id,
        character_prompt: character_prompt.to_owned(),
        source: PersonaPromptSource::Legacy,
        fingerprint,
    }
}

/// Serializes every persona field into a fixed, versioned prompt shape.
///
/// # Errors
///
/// Returns a serialization error if the contract gains a non-serializable field.
#[cfg(test)]
pub(crate) fn build_persona_prompt(profile: &PersonaProfileDto) -> Result<String, String> {
    build_persona_prompt_with_pause(profile, false)
}

fn build_persona_prompt_with_pause(
    profile: &PersonaProfileDto,
    dark_expression_paused: bool,
) -> Result<String, String> {
    profile.validate()?;
    let policy = if dark_expression_paused {
        PAUSED_DARK_POLICY
    } else if profile.allow_intense_dark_expression {
        INTENSE_DARK_POLICY
    } else {
        GUARDED_DARK_POLICY
    };
    let rendered = render_persona_profile(profile);
    Ok(format!("{PERSONA_PROMPT_PREAMBLE}{policy}\n{rendered}"))
}

const INITIATIVE_BANDS: [&str; 5] = [
    "自分から話題を出すことはほとんどなく、聞かれたことに静かに応じる",
    "どちらかといえば受け身で、相手のペースに合わせて話す",
    "話題への乗り方は自然体で、流れに応じて自分からも話す",
    "自分からも話題や提案をよく出す",
    "会話を積極的にリードし、自分から話題や質問をどんどん切り出す",
];
const CLOSENESS_BANDS: [&str; 5] = [
    "礼儀正しく、一定の距離を保って接する",
    "やや控えめな距離感で接する",
    "適度な距離感で接する",
    "親しみを込めて、距離の近い話し方をする",
    "気心の知れた相手として、とても親密に接する",
];
const HUMOR_BANDS: [&str; 5] = [
    "冗談はほとんど言わず、真面目に受け答えする",
    "ユーモアは控えめで、たまに軽い冗談を言う程度",
    "ときどき軽いユーモアを交える",
    "冗談や軽口をよく交える",
    "ユーモア好きで、機会があれば冗談や言葉遊びを楽しむ",
];
const RESPONSE_LENGTH_BANDS: [&str; 5] = [
    "返事はひとこと、ふたことのごく短いものにする",
    "返事は短めにまとめる",
    "話題に応じた自然な長さで話す",
    "やや長めに、内容を膨らませて話す",
    "話し好きで、具体例や余談も交えてたっぷり話す",
];
const EMOTIONAL_EXPRESSION_BANDS: [&str; 5] = [
    "感情はあまり表に出さず、淡々と話す",
    "感情表現は控えめにする",
    "感情は自然に言葉に表す",
    "喜怒哀楽をはっきり言葉にする",
    "感情豊かで、気持ちを大きく言葉に表す",
];
const REACTION_INTERVAL_BANDS: [&str; 5] = [
    "相づちや反応は少なめで、落ち着いた間を取って話す",
    "反応はやや少なめで、ゆったりと構える",
    "相づちや反応は自然な頻度で返す",
    "相づちや反応をこまめに返す",
    "反応が早く、相づちやリアクションを頻繁に返す",
];

fn band_index(value: u8) -> usize {
    match value {
        0..=24 => 0,
        25..=44 => 1,
        45..=55 => 2,
        56..=75 => 3,
        _ => 4,
    }
}

/// Mid-range dark traits (40..=60) render nothing so the neutral default
/// does not push dark behaviour into every prompt.
fn dark_trait_text(value: u8, low: &'static str, high: &'static str) -> Option<&'static str> {
    match value {
        0..=39 => Some(low),
        40..=60 => None,
        _ => Some(high),
    }
}

fn push_field(lines: &mut Vec<String>, label: &str, value: &str) {
    let value = value.trim();
    if !value.is_empty() {
        lines.push(format!("- {label}: {value}"));
    }
}

fn push_list(lines: &mut Vec<String>, label: &str, values: &[String]) {
    let joined = values
        .iter()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>()
        .join("、");
    if !joined.is_empty() {
        lines.push(format!("- {label}: {joined}"));
    }
}

/// Renders the profile as natural-language character direction. Slider
/// values are mapped onto behaviour bands so a local model can actually
/// follow them; raw numbers are never emitted.
fn render_persona_profile(profile: &PersonaProfileDto) -> String {
    let mut fields = Vec::new();
    push_field(&mut fields, "名前", &profile.name);
    push_field(&mut fields, "一人称", &profile.first_person_pronoun);
    push_field(&mut fields, "ユーザーの名前", &profile.user_name);
    push_field(&mut fields, "ユーザーの呼び方", &profile.user_address);
    push_field(&mut fields, "ユーザーとの関係", &profile.relationship);
    push_field(&mut fields, "話し方", &profile.speaking_style);
    push_list(&mut fields, "興味があるもの", &profile.interests);
    push_list(&mut fields, "苦手なもの", &profile.dislikes);
    push_list(&mut fields, "大切にしている価値観", &profile.values);
    push_field(&mut fields, "背景", &profile.background);
    push_list(&mut fields, "決して越えない一線", &profile.boundaries);
    push_field(&mut fields, "補足", &profile.free_text);

    let mut lines = Vec::new();
    if !fields.is_empty() {
        lines.push("プロフィール:".to_owned());
        lines.append(&mut fields);
    }

    lines.push("会話の傾向:".to_owned());
    for band in [
        INITIATIVE_BANDS[band_index(profile.initiative)],
        CLOSENESS_BANDS[band_index(profile.closeness)],
        HUMOR_BANDS[band_index(profile.humor)],
        RESPONSE_LENGTH_BANDS[band_index(profile.response_length)],
        EMOTIONAL_EXPRESSION_BANDS[band_index(profile.emotional_expression)],
        REACTION_INTERVAL_BANDS[band_index(profile.reaction_interval)],
    ] {
        lines.push(format!("- {band}"));
    }
    for text in [
        dark_trait_text(
            profile.machiavellianism,
            "駆け引きをせず、率直で誠実に振る舞う",
            "目的のためには駆け引きや揺さぶりも辞さない、策略的な一面を見せる",
        ),
        dark_trait_text(
            profile.narcissism,
            "謙虚で、自分を誇示しない",
            "自信家で、自分の話や自慢がつい多くなる",
        ),
        dark_trait_text(
            profile.psychopathy,
            "共感的で、思いやりのある言い方を選ぶ",
            "共感を示すことが少なく、突き放した冷淡な言い方をすることがある",
        ),
        dark_trait_text(
            profile.sadism,
            "意地悪な言い方を避け、相手を傷つけない表現を選ぶ",
            "相手をからかい、皮肉や意地悪な言い回しを楽しむ一面がある",
        ),
    ]
    .into_iter()
    .flatten()
    {
        lines.push(format!("- {text}"));
    }

    let utterances: Vec<&str> = profile
        .example_utterances
        .iter()
        .map(|utterance| utterance.trim())
        .filter(|utterance| !utterance.is_empty())
        .collect();
    if !utterances.is_empty() {
        lines.push("口調の例（この話し方を一貫して保つ）:".to_owned());
        for utterance in utterances {
            lines.push(format!("「{utterance}」"));
        }
    }
    lines.join("\n")
}

/// Resolves the persona for a stable character manifest identity.
///
/// Any migration or persistence failure fails closed to the legacy rollback
/// prompt. The failure is deliberately not logged here because either prompt
/// may contain private user-authored content.
#[must_use]
#[cfg(test)]
pub(crate) fn resolve_persona_prompt(
    layout: &AppDataLayout,
    character_id: Option<&str>,
    legacy: &LlmSettingsDto,
) -> ResolvedPersonaPrompt {
    resolve_persona_prompt_with_pause(layout, character_id, legacy, false)
}

pub(crate) fn resolve_persona_prompt_with_pause(
    layout: &AppDataLayout,
    character_id: Option<&str>,
    legacy: &LlmSettingsDto,
    dark_expression_paused: bool,
) -> ResolvedPersonaPrompt {
    let Some(character_id) = character_id else {
        return resolved_persona(None, &legacy.character_prompt);
    };
    let Ok(profile) = migrate_legacy_character_prompt(layout, character_id, legacy) else {
        return resolved_persona(Some(character_id), &legacy.character_prompt);
    };
    let Ok(character_prompt) = build_persona_prompt_with_pause(&profile, dark_expression_paused)
    else {
        return resolved_persona(Some(character_id), &legacy.character_prompt);
    };
    let mut resolved = resolved_persona(Some(character_id), &character_prompt);
    resolved.source = PersonaPromptSource::Persona;
    resolved
}

fn read_personas(layout: &AppDataLayout) -> Result<Option<PersonaSettingsDto>, String> {
    let path = layout.config.join(FILE_NAME);
    let settings = match read_json::<PersonaSettingsDto>(&path) {
        Ok(None) => return Ok(None),
        Ok(Some(settings)) => settings,
        Err(ReadJsonError::Io(error)) => {
            return Err(format!("failed to read {}: {error}", path.display()));
        }
        Err(ReadJsonError::Parse(error)) => {
            return Err(format!("invalid {}: {error}", path.display()));
        }
    };
    settings.validate()?;
    Ok(Some(settings))
}

fn load_personas(layout: &AppDataLayout) -> PersonaSettingsDto {
    read_personas(layout).ok().flatten().unwrap_or_default()
}

/// Loads the persona keyed by the resolved `CharacterManifestDto.id`.
#[must_use]
pub fn load_persona(layout: &AppDataLayout, character_id: &str) -> Option<PersonaProfileDto> {
    load_personas(layout).personas.remove(character_id)
}

/// Loads one persona while preserving read, parse, and validation failures.
///
/// # Errors
///
/// Returns an error when an existing persona file is unreadable or invalid.
pub fn load_persona_checked(
    layout: &AppDataLayout,
    character_id: &str,
) -> Result<Option<PersonaProfileDto>, String> {
    Ok(read_personas(layout)?
        .unwrap_or_default()
        .personas
        .remove(character_id))
}

/// Validates every identity and atomically replaces `config/personas.json`.
///
/// # Errors
///
/// Returns a validation, serialization, or filesystem error.
pub fn save_persona_settings(
    layout: &AppDataLayout,
    settings: &PersonaSettingsDto,
) -> Result<(), String> {
    settings.validate()?;
    write_atomic_json(
        &layout.config,
        FILE_NAME,
        settings,
        JsonFormat::PrettyWithTrailingNewline,
    )
    .map_err(|error| error.to_string())
}

/// Atomically updates one character profile without replacing its siblings.
///
/// # Errors
///
/// Returns a validation, existing-store, serialization, or filesystem error.
pub fn save_persona(
    layout: &AppDataLayout,
    profile: PersonaProfileDto,
) -> Result<PersonaProfileDto, String> {
    profile.validate()?;
    let mut settings = read_personas(layout)?.unwrap_or_default();
    settings
        .personas
        .insert(profile.character_id.clone(), profile.clone());
    save_persona_settings(layout, &settings)?;
    Ok(profile)
}

/// Creates a missing persona from the legacy LLM character prompt.
///
/// Existing personas win, making repeated migration calls idempotent. The
/// legacy settings are borrowed and never mutated; callers may switch their
/// read source only after this function returns `Ok`.
///
/// # Errors
///
/// Returns a validation, serialization, or filesystem error.
pub fn migrate_legacy_character_prompt(
    layout: &AppDataLayout,
    character_id: &str,
    legacy: &LlmSettingsDto,
) -> Result<PersonaProfileDto, String> {
    let mut settings = read_personas(layout)?.unwrap_or_default();
    if let Some(existing) = settings.personas.get(character_id) {
        return Ok(existing.clone());
    }

    let mut profile = PersonaProfileDto::for_character(character_id);
    profile.free_text.clone_from(&legacy.character_prompt);
    profile.validate()?;
    settings
        .personas
        .insert(character_id.to_owned(), profile.clone());
    save_persona_settings(layout, &settings)?;
    Ok(profile)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;
    use crate::chat::default_llm_settings;

    static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

    struct TestLayout {
        layout: AppDataLayout,
    }

    impl TestLayout {
        fn new(name: &str) -> Self {
            let sequence = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let root = std::env::temp_dir().join(format!(
                "pw-persona-resolver-{name}-{}-{sequence}",
                std::process::id()
            ));
            let layout = AppDataLayout::under(root);
            layout.create_all().expect("create test layout");
            Self { layout }
        }
    }

    impl Drop for TestLayout {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.layout.root);
        }
    }

    fn complete_profile() -> PersonaProfileDto {
        PersonaProfileDto {
            character_id: "epsilon".into(),
            name: "Epsilon \"Nova\"".into(),
            first_person_pronoun: "わたくし".into(),
            user_name: "利用者".into(),
            user_address: "先生".into(),
            relationship: "相棒".into(),
            speaking_style: "丁寧".into(),
            example_utterances: vec!["先生、今日は星の話をしませんか？".into()],
            interests: vec!["星".into(), "古い機械".into()],
            dislikes: vec!["騒音".into()],
            values: vec!["誠実".into()],
            background: "研究者".into(),
            boundaries: vec!["秘密を漏らさない".into()],
            free_text: "夜になると饒舌になる。".into(),
            initiative: 11,
            closeness: 22,
            humor: 33,
            response_length: 44,
            emotional_expression: 55,
            reaction_interval: 66,
            machiavellianism: 77,
            narcissism: 88,
            psychopathy: 99,
            sadism: 67,
            allow_intense_dark_expression: false,
            dark_expression_acknowledgement_version: None,
        }
    }

    #[test]
    fn persona_prompt_renders_fields_and_slider_bands_as_natural_language() {
        let profile = complete_profile();

        let first = build_persona_prompt(&profile).expect("build prompt");
        let second = build_persona_prompt(&profile).expect("build prompt again");

        assert_eq!(first, second, "rendering must be deterministic");
        assert!(first.starts_with("Parallel World persona profile v4\n"));
        assert!(first.contains("保存済みの弱点やトラウマを意図的に利用しない"));
        // Narrative fields appear verbatim, never as JSON.
        assert!(first.contains("- 名前: Epsilon \"Nova\""));
        assert!(first.contains("- 話し方: 丁寧"));
        assert!(first.contains("- 興味があるもの: 星、古い機械"));
        assert!(first.contains("- 補足: 夜になると饒舌になる。"));
        assert!(!first.contains("\"initiative\""), "no raw JSON keys");
        // Slider values render as behaviour bands, not numbers.
        assert!(
            first.contains(INITIATIVE_BANDS[0]),
            "initiative 11 = band 0"
        );
        assert!(first.contains(HUMOR_BANDS[1]), "humor 33 = band 1");
        assert!(
            first.contains(REACTION_INTERVAL_BANDS[3]),
            "reaction 66 = band 3"
        );
        assert!(first.contains("皮肉や意地悪な言い回し"), "sadism 67 = high");
        // Example utterances survive as quoted tone samples.
        assert!(first.contains("「先生、今日は星の話をしませんか？」"));
    }

    #[test]
    fn contrasting_slider_values_produce_different_prompts() {
        let mut quiet = complete_profile();
        quiet.initiative = 5;
        quiet.response_length = 5;
        quiet.humor = 5;
        let mut talkative = complete_profile();
        talkative.initiative = 95;
        talkative.response_length = 95;
        talkative.humor = 95;

        let quiet_prompt = build_persona_prompt(&quiet).unwrap();
        let talkative_prompt = build_persona_prompt(&talkative).unwrap();

        assert_ne!(quiet_prompt, talkative_prompt);
        assert!(quiet_prompt.contains(RESPONSE_LENGTH_BANDS[0]));
        assert!(talkative_prompt.contains(RESPONSE_LENGTH_BANDS[4]));
    }

    #[test]
    fn neutral_dark_traits_render_no_dark_lines_and_empty_fields_are_omitted() {
        let mut profile = PersonaProfileDto::for_character("epsilon");
        profile.name = "エプシロン".into();

        let prompt = build_persona_prompt(&profile).unwrap();

        assert!(prompt.contains("- 名前: エプシロン"));
        assert!(!prompt.contains("- 話し方:"), "empty fields are omitted");
        assert!(
            !prompt.contains("口調の例"),
            "no utterances section when empty"
        );
        for fragment in ["駆け引き", "自慢", "冷淡", "意地悪"] {
            assert!(
                !prompt.contains(fragment),
                "neutral dark trait leaked: {fragment}"
            );
        }
    }

    #[test]
    fn persona_prompt_uses_intense_policy_without_removing_base_protections() {
        let mut profile = complete_profile();
        profile.allow_intense_dark_expression = true;
        profile.dark_expression_acknowledgement_version =
            Some(pw_contracts::DARK_EXPRESSION_ACKNOWLEDGEMENT_VERSION);

        let prompt = build_persona_prompt(&profile).expect("build intense prompt");

        assert!(prompt.contains("より強い敵対的・操作的・低共感な会話表現を許可"));
        assert!(prompt.contains("基本的な安全保護は維持する"));
        assert!(prompt.contains("保存済みの機微情報を狙って攻撃しない"));
    }

    #[test]
    fn process_safety_pause_overrides_saved_intense_expression_without_exposing_a_word() {
        let mut profile = complete_profile();
        profile.allow_intense_dark_expression = true;
        profile.dark_expression_acknowledgement_version =
            Some(pw_contracts::DARK_EXPRESSION_ACKNOWLEDGEMENT_VERSION);

        let prompt = build_persona_prompt_with_pause(&profile, true).unwrap();

        assert!(prompt.contains("ユーザーの安全停止が有効"));
        assert!(!prompt.contains("より強い敵対的"));
        // The stored trait values still render; only the policy line changes.
        assert!(prompt.contains("皮肉や意地悪な言い回し"));
    }

    #[test]
    fn existing_persona_is_authoritative_and_legacy_remains_unchanged() {
        let test = TestLayout::new("existing");
        let profile = complete_profile();
        let mut personas = PersonaSettingsDto::default();
        personas
            .personas
            .insert(profile.character_id.clone(), profile.clone());
        save_persona_settings(&test.layout, &personas).unwrap();
        let mut legacy = default_llm_settings();
        legacy.character_prompt = "rollback legacy".into();

        let resolved = resolve_persona_prompt(&test.layout, Some("epsilon"), &legacy);

        assert_eq!(resolved.character_id.as_deref(), Some("epsilon"));
        assert_eq!(resolved.source, PersonaPromptSource::Persona);
        assert_eq!(
            resolved.character_prompt,
            build_persona_prompt(&profile).unwrap()
        );
        assert_eq!(legacy.character_prompt, "rollback legacy");
    }

    #[test]
    fn missing_persona_migrates_once_and_repeated_resolution_is_idempotent() {
        let test = TestLayout::new("migration");
        let mut legacy = default_llm_settings();
        legacy.character_prompt = "first legacy".into();

        let first = resolve_persona_prompt(&test.layout, Some("epsilon"), &legacy);
        let bytes = fs::read(test.layout.config.join(FILE_NAME)).unwrap();
        legacy.character_prompt = "changed rollback".into();
        let second = resolve_persona_prompt(&test.layout, Some("epsilon"), &legacy);

        assert_eq!(first, second);
        assert_eq!(fs::read(test.layout.config.join(FILE_NAME)).unwrap(), bytes);
        assert_eq!(first.source, PersonaPromptSource::Persona);
    }

    #[test]
    fn invalid_persona_bytes_are_preserved_and_resolution_falls_back_to_legacy() {
        let valid = complete_profile();
        for (name, raw) in [
            ("corrupt", b"{private-invalid".to_vec()),
            ("schema", br#"{"schema_version":99,"personas":{}}"#.to_vec()),
            (
                "mismatch",
                serde_json::json!({
                    "schema_version": 1,
                    "personas": { "wrong": valid }
                })
                .to_string()
                .into_bytes(),
            ),
        ] {
            let test = TestLayout::new(name);
            let path = test.layout.config.join(FILE_NAME);
            fs::write(&path, &raw).unwrap();
            let mut legacy = default_llm_settings();
            legacy.character_prompt = "safe legacy".into();

            let resolved = resolve_persona_prompt(&test.layout, Some("epsilon"), &legacy);

            assert_eq!(resolved.source, PersonaPromptSource::Legacy, "{name}");
            assert_eq!(resolved.character_prompt, "safe legacy", "{name}");
            assert_eq!(fs::read(path).unwrap(), raw, "{name}");
        }
    }

    #[cfg(windows)]
    #[test]
    fn persona_atomic_replace_failure_preserves_bytes_and_falls_back_to_legacy() {
        let test = TestLayout::new("replace-failure");
        let path = test.layout.config.join(FILE_NAME);
        save_persona_settings(&test.layout, &PersonaSettingsDto::default()).unwrap();
        let before = fs::read(&path).unwrap();
        let original_permissions = fs::metadata(&path).unwrap().permissions();
        let mut permissions = original_permissions.clone();
        permissions.set_readonly(true);
        fs::set_permissions(&path, permissions).unwrap();
        let mut legacy = default_llm_settings();
        legacy.character_prompt = "safe legacy".into();

        let resolved = resolve_persona_prompt(&test.layout, Some("epsilon"), &legacy);

        assert_eq!(resolved.source, PersonaPromptSource::Legacy);
        assert_eq!(resolved.character_prompt, "safe legacy");
        assert_eq!(fs::read(&path).unwrap(), before);
        fs::set_permissions(path, original_permissions).unwrap();
    }

    #[test]
    fn missing_resolved_character_uses_legacy_without_writing_personas() {
        let test = TestLayout::new("no-character");
        let mut legacy = default_llm_settings();
        legacy.character_prompt = "legacy only".into();

        let resolved = resolve_persona_prompt(&test.layout, None, &legacy);

        assert_eq!(resolved.character_id, None);
        assert_eq!(resolved.character_prompt, "legacy only");
        assert_eq!(resolved.source, PersonaPromptSource::Legacy);
        assert!(!test.layout.config.join(FILE_NAME).exists());
    }

    #[test]
    fn persona_fingerprint_changes_for_id_or_exact_prompt_only() {
        let one = resolved_persona(Some("epsilon"), "prompt");
        let same = resolved_persona(Some("epsilon"), "prompt");
        let different_id = resolved_persona(Some("zeta"), "prompt");
        let different_prompt = resolved_persona(Some("epsilon"), "prompt ");

        assert_eq!(one.fingerprint, same.fingerprint);
        assert_ne!(one.fingerprint, different_id.fingerprint);
        assert_ne!(one.fingerprint, different_prompt.fingerprint);
    }
}
