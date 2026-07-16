use std::fs;
use std::sync::atomic::{AtomicU64, Ordering};

use parallel_world_desktop::behavior::{
    DarkExpressionSafetyLoadError, load_dark_expression_safety,
    load_dark_expression_safety_checked, safe_word_matches, sanitize_safe_word,
    save_dark_expression_safety,
};
use pw_contracts::DarkExpressionSafetySettingsDto;
use pw_platform::paths::AppDataLayout;

static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

struct TestLayout {
    layout: AppDataLayout,
}

impl TestLayout {
    fn new(name: &str) -> Self {
        let sequence = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "pw-dark-expression-safety-{name}-{}-{sequence}",
            std::process::id()
        ));
        let layout = AppDataLayout::under(root);
        layout.create_all().unwrap();
        Self { layout }
    }
}

impl Drop for TestLayout {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.layout.root);
    }
}

#[test]
fn missing_safety_file_returns_unpaused_defaults() {
    let test = TestLayout::new("missing");

    let settings = load_dark_expression_safety_checked(&test.layout).unwrap();

    assert_eq!(settings, DarkExpressionSafetySettingsDto::default());
    assert!(
        !test
            .layout
            .config
            .join("dark-expression-safety.json")
            .exists()
    );
}

#[test]
fn safe_word_is_trimmed_and_empty_input_clears_it() {
    assert_eq!(
        sanitize_safe_word(Some("  Stop Now  ".to_owned())).unwrap(),
        Some("Stop Now".to_owned())
    );
    assert_eq!(sanitize_safe_word(Some("　 ".to_owned())).unwrap(), None);
    assert_eq!(sanitize_safe_word(None).unwrap(), None);
}

#[test]
fn matching_uses_nfkc_full_case_fold_and_trailing_punctuation_only() {
    let safe_word = "ＳＴＲＡＳＳＥ";

    assert!(safe_word_matches(Some(safe_word), "  strasse！？  "));
    assert!(safe_word_matches(Some("STOP"), "ｓｔｏｐ。"));
    assert!(!safe_word_matches(Some("stop"), "please stop"));
    assert!(!safe_word_matches(Some("stop now"), "stop  now"));
    assert!(!safe_word_matches(None, "stop"));
}

#[test]
fn safety_settings_round_trip_without_leaking_into_other_files() {
    let test = TestLayout::new("round-trip");
    let settings = DarkExpressionSafetySettingsDto {
        safe_word: Some("停止".to_owned()),
        dark_expression_paused: true,
        ..DarkExpressionSafetySettingsDto::default()
    };

    save_dark_expression_safety(&test.layout, &settings).unwrap();

    assert_eq!(load_dark_expression_safety(&test.layout), settings);
    assert!(!test.layout.config.join("personas.json").exists());
    assert!(!test.layout.config.join("behavior.json").exists());
}

#[test]
fn corrupt_safety_file_is_preserved_and_checked_load_fails_closed() {
    let test = TestLayout::new("corrupt");
    let path = test.layout.config.join("dark-expression-safety.json");
    let bytes = b"{private-invalid";
    fs::write(&path, bytes).unwrap();

    assert_eq!(
        load_dark_expression_safety_checked(&test.layout),
        Err(DarkExpressionSafetyLoadError::Invalid)
    );
    let fallback = load_dark_expression_safety(&test.layout);
    assert!(fallback.dark_expression_paused);
    assert_eq!(fallback.safe_word, None);
    assert_eq!(fs::read(path).unwrap(), bytes);
}
