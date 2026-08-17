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
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Settings {
    /// Interface language.
    pub lang: Lang,
    /// Color theme.
    pub theme: Theme,
    /// File open when the IDE was last closed.
    pub last_file: Option<String>,
    /// Tab that was active, by its identifier.
    pub last_view: Option<String>,
}

/// Where the settings file lives, which is not the same place on every
/// desktop: `%APPDATA%\oxidelica\ide.conf` on Windows, where `HOME` is
/// usually not set at all, and `~/.config/oxidelica/ide.conf` elsewhere.
fn config_path() -> Option<PathBuf> {
    let (base, folder) = if cfg!(windows) {
        let base = std::env::var_os("APPDATA").or_else(|| std::env::var_os("USERPROFILE"))?;
        (base, None)
    } else {
        (std::env::var_os("HOME")?, Some(".config"))
    };
    let mut path = PathBuf::from(base);
    if let Some(folder) = folder {
        path.push(folder);
    }
    path.push("oxidelica");
    path.push("ide.conf");
    Some(path)
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
            "file" => settings.last_file = Some(value.trim().to_string()),
            "view" => settings.last_view = Some(value.trim().to_string()),
            _ => {}
        }
    }
    settings
}

/// Persist settings to disk (best effort — errors are ignored).
pub fn save(settings: &Settings) {
    let Some(path) = config_path() else { return };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let mut text = format!(
        "lang={}\ntheme={}\n",
        settings.lang.code(),
        settings.theme.code()
    );
    if let Some(file) = &settings.last_file {
        text.push_str(&format!("file={file}\n"));
    }
    if let Some(view) = &settings.last_view {
        text.push_str(&format!("view={view}\n"));
    }
    let _ = std::fs::write(path, text);
}
