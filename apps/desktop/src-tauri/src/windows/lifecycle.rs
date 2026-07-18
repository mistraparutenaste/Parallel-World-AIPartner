//! Pure lifecycle decisions for the desktop windows.

/// Returns whether closing `closing_label` should terminate the application.
///
/// `window_visibilities` contains every existing window and its visibility
/// before the close request is applied, including `closing_label` itself.
#[must_use]
pub fn should_exit_after_close<'a>(
    closing_label: &str,
    window_visibilities: impl IntoIterator<Item = (&'a str, bool)>,
) -> bool {
    let mut another_window_is_visible = false;
    let mut settings_window_exists = false;
    for (label, visible) in window_visibilities {
        settings_window_exists |= label == "settings";
        another_window_is_visible |= label != closing_label && visible;
    }

    match closing_label {
        "character" | "settings" => !another_window_is_visible,
        "chat" => !settings_window_exists && !another_window_is_visible,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::should_exit_after_close;

    #[test]
    fn exits_when_character_is_the_last_visible_window() {
        assert!(should_exit_after_close(
            "character",
            [("character", true), ("chat", false)]
        ));
    }

    #[test]
    fn exits_when_settings_is_the_last_visible_window() {
        assert!(should_exit_after_close(
            "settings",
            [("settings", true), ("chat", false)]
        ));
    }

    #[test]
    fn stays_running_while_another_window_is_visible() {
        assert!(!should_exit_after_close(
            "character",
            [("character", true), ("settings", true), ("chat", false)]
        ));
        assert!(!should_exit_after_close(
            "settings",
            [("settings", true), ("chat", true)]
        ));
    }

    #[test]
    fn chat_close_redocks_when_the_settings_window_still_exists() {
        assert!(!should_exit_after_close(
            "chat",
            [("chat", true), ("settings", false)]
        ));
    }

    #[test]
    fn chat_close_exits_when_it_is_last_visible_and_settings_was_destroyed() {
        assert!(should_exit_after_close("chat", [("chat", true)]));
    }

    #[test]
    fn chat_close_does_not_exit_while_character_is_still_visible() {
        assert!(!should_exit_after_close(
            "chat",
            [("chat", true), ("character", true)]
        ));
    }

    #[test]
    fn unknown_window_close_does_not_change_application_lifecycle() {
        assert!(!should_exit_after_close(
            "diagnostics",
            [("diagnostics", true)]
        ));
    }
}
