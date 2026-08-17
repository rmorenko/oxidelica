//! Локализация IDE. Строки — поля структуры, поэтому «забыть перевести»
//! невозможно: новый язык не соберётся без всех строк.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Lang {
    #[default]
    En,
    Ru,
}

impl Lang {
    pub fn code(self) -> &'static str {
        match self {
            Lang::En => "en",
            Lang::Ru => "ru",
        }
    }

    pub fn from_code(code: &str) -> Option<Lang> {
        match code {
            "en" => Some(Lang::En),
            "ru" => Some(Lang::Ru),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Lang::En => "English",
            Lang::Ru => "Русский",
        }
    }

    pub fn strings(self) -> &'static Strings {
        match self {
            Lang::En => &EN,
            Lang::Ru => &RU,
        }
    }
}

pub struct Strings {
    pub menu_file: &'static str,
    pub menu_open_example: &'static str,
    pub menu_quit: &'static str,
    pub menu_simulation: &'static str,
    pub menu_run: &'static str,
    pub menu_view: &'static str,
    pub menu_theme: &'static str,
    pub theme_dark: &'static str,
    pub theme_light: &'static str,
    pub menu_language: &'static str,
    pub menu_help: &'static str,
    pub menu_about: &'static str,
    pub about_text: &'static str,
    pub simulate: &'static str,
    pub save: &'static str,
    pub ready: &'static str,
    pub file_loaded: &'static str,
    pub saved: &'static str,
    pub write_error: &'static str,
    pub open_error: &'static str,
    pub no_file_to_save: &'static str,
    pub no_file: &'static str,
    pub parse_error: &'static str,
    pub compile_error: &'static str,
    pub sim_error: &'static str,
    pub steps_in: &'static str,
    pub variables: &'static str,
    pub press_simulate: &'static str,
}

pub static EN: Strings = Strings {
    menu_file: "File",
    menu_open_example: "Open Example",
    menu_quit: "Quit",
    menu_simulation: "Simulation",
    menu_run: "Run",
    menu_view: "View",
    menu_theme: "Theme",
    theme_dark: "Dark",
    theme_light: "Light",
    menu_language: "Language",
    menu_help: "Help",
    menu_about: "About Oxidelica",
    about_text: "A modern cross-platform Modelica environment in Rust.",
    simulate: "▶ Simulate",
    save: "Save",
    ready: "ready",
    file_loaded: "file loaded",
    saved: "saved",
    write_error: "write error",
    open_error: "failed to open",
    no_file_to_save: "no file: pick an example or create a .mo",
    no_file: "<no file>",
    parse_error: "parse error",
    compile_error: "compile error",
    sim_error: "simulation error",
    steps_in: "steps in",
    variables: "variables",
    press_simulate: "Press \"Simulate\" to see the plots",
};

pub static RU: Strings = Strings {
    menu_file: "Файл",
    menu_open_example: "Открыть пример",
    menu_quit: "Выход",
    menu_simulation: "Симуляция",
    menu_run: "Запустить",
    menu_view: "Вид",
    menu_theme: "Тема",
    theme_dark: "Тёмная",
    theme_light: "Светлая",
    menu_language: "Язык",
    menu_help: "Справка",
    menu_about: "Об Oxidelica",
    about_text: "Современная кроссплатформенная среда Modelica на Rust.",
    simulate: "▶ Симулировать",
    save: "Сохранить",
    ready: "готов",
    file_loaded: "файл загружен",
    saved: "сохранено",
    write_error: "ошибка записи",
    open_error: "не удалось открыть",
    no_file_to_save: "нет файла: выберите пример или создайте .mo",
    no_file: "<без файла>",
    parse_error: "ошибка разбора",
    compile_error: "ошибка компиляции",
    sim_error: "ошибка симуляции",
    steps_in: "шагов за",
    variables: "переменные",
    press_simulate: "Нажмите «Симулировать», чтобы увидеть графики",
};
