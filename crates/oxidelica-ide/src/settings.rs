//! IDE settings: language and theme. Stored in
//! `~/.config/oxidelica/ide.conf` as `key=value` lines — no
//! dependencies, readable by eye.

use crate::i18n::Lang;
use std::path::PathBuf;

/// Color theme of the IDE.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Theme {
    /// Dark theme (default).
    #[default]
    Dark,
    /// Light theme.
    Light,
}

impl Theme {
    /// Identifier used in the settings file.
    pub fn code(self) -> &'static str {
        match self {
            Theme::Dark => "dark",
            Theme::Light => "light",
        }
    }

    /// Parse an identifier from the settings file.
    pub fn from_code(code: &str) -> Option<Theme> {
        match code {
            "dark" => Some(Theme::Dark),
            "light" => Some(Theme::Light),
            _ => None,
        }
    }
}

/// Persisted user preferences.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Settings {
    /// Interface language.
    pub lang: Lang,
    /// Color theme.
    pub theme: Theme,
}

fn config_path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(
        PathBuf::from(home)
            .join(".config")
            .join("oxidelica")
            .join("ide.conf"),
    )
}

/// Load settings from disk, falling back to defaults.
pub fn load() -> Settings {
    let mut settings = Settings::default();
    let Some(path) = config_path() else {
        return settings;
    };
    let Ok(text) = std::fs::read_to_string(path) else {
        return settings;
    };
    for line in text.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        match key.trim() {
            "lang" => {
                if let Some(lang) = Lang::from_code(value.trim()) {
                    settings.lang = lang;
                }
            }
            "theme" => {
                if let Some(theme) = Theme::from_code(value.trim()) {
                    settings.theme = theme;
                }
            }
            _ => {}
        }
    }
    settings
}

/// Persist settings to disk (best effort — errors are ignored).
pub fn save(settings: Settings) {
    let Some(path) = config_path() else { return };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let text = format!(
        "lang={}\ntheme={}\n",
        settings.lang.code(),
        settings.theme.code()
    );
    let _ = std::fs::write(path, text);
}
