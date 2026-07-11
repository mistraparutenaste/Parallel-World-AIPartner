#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowDefinition {
    pub label: &'static str,
    pub title: &'static str,
    pub url: &'static str,
    pub transparent: bool,
    pub decorations: bool,
}

pub const WINDOWS: [WindowDefinition; 3] = [
    WindowDefinition {
        label: "character",
        title: "Parallel World Character",
        url: "character.html",
        transparent: true,
        decorations: false,
    },
    WindowDefinition {
        label: "chat",
        title: "Parallel World",
        url: "chat.html",
        transparent: false,
        decorations: true,
    },
    WindowDefinition {
        label: "settings",
        title: "設定",
        url: "settings.html",
        transparent: false,
        decorations: true,
    },
];

#[cfg(test)]
mod tests {
    #[test]
    fn defines_exactly_three_unique_window_labels() {
        let labels: Vec<_> = super::WINDOWS.iter().map(|window| window.label).collect();
        assert_eq!(labels, ["character", "chat", "settings"]);
    }

    #[test]
    fn character_is_the_only_transparent_undecorated_window() {
        let character = super::WINDOWS[0];
        assert!(character.transparent);
        assert!(!character.decorations);
        assert!(
            super::WINDOWS[1..]
                .iter()
                .all(|window| { !window.transparent && window.decorations })
        );
    }
}
