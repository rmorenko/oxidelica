//! Настройки IDE: язык и тема. Хранятся в `~/.config/oxidelica/ide.conf`
//! в формате `ключ=значение` — без зависимостей, читаемо глазами.

use crate::i18n::Lang;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Theme {
    Dark,
    Light,
}

impl Default for Theme {
    fn default() -> Self {
        Theme::Dark
    }
}

impl Theme {
    pub fn code(self) -> &'static str {
        match self {
            Theme::Dark => "dark",
            Theme::Light => "light",
        }
    }

    pub fn from_code(code: &str) -> Option<Theme> {
        match code {
            "dark" => Some(Theme::Dark),
            "light" => Some(Theme::Light),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Settings {
    pub lang: Lang,
    pub theme: Theme,
}

fn config_path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join(".config").join("oxidelica").join("ide.conf"))
}

pub fn load() -> Settings {
    let mut settings = Settings::default();
    let Some(path) = config_path() else { return settings };
    let Ok(text) = std::fs::read_to_string(path) else { return settings };
    for line in text.lines() {
        let Some((key, value)) = line.split_once('=') else { continue };
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

pub fn save(settings: Settings) {
    let Some(path) = config_path() else { return };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let text = format!("lang={}\ntheme={}\n", settings.lang.code(), settings.theme.code());
    let _ = std::fs::write(path, text);
}
