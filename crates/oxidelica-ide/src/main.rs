//! Oxidelica IDE — среда моделирования: меню, редактор кода,
//! симуляция, графики. Локализация EN/RU, тёмная/светлая темы.

mod i18n;
mod settings;
mod style;

use bevy::app::AppExit;
use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts, EguiPlugin};
use egui_plot::{Legend, Line, Plot, PlotPoints};
use i18n::Lang;
use settings::{Settings, Theme};
use std::path::PathBuf;
use std::time::Instant;

const DEFAULT_MODEL: &str = "model Demo \"Damped oscillator\"\n  parameter Real k = 4.0;\n  parameter Real d = 0.3;\n  Real x(start = 1.0);\n  Real v(start = 0.0);\nequation\n  der(x) = v;\n  der(v) = -k * x - d * v;\n  annotation(experiment(StopTime = 10.0, Interval = 0.001));\nend Demo;\n";

struct SimData {
    columns: Vec<String>,
    rows: Vec<Vec<f64>>,
    /// Видимость кривых (по одной на каждый столбец, кроме time).
    visible: Vec<bool>,
}

#[derive(Resource)]
struct Ide {
    source: String,
    file: Option<PathBuf>,
    examples: Vec<PathBuf>,
    log: String,
    result: Option<SimData>,
    settings: Settings,
    /// Какая тема сейчас реально применена к egui (None — ещё ни одна).
    applied_theme: Option<Theme>,
    show_about: bool,
}

fn main() {
    let settings = settings::load();
    let examples = list_examples();
    let (source, file) = match examples.first() {
        Some(path) => match std::fs::read_to_string(path) {
            Ok(text) => (text, Some(path.clone())),
            Err(_) => (DEFAULT_MODEL.to_string(), None),
        },
        None => (DEFAULT_MODEL.to_string(), None),
    };

    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "Oxidelica IDE".into(),
                resolution: (1440.0, 900.0).into(),
                ..default()
            }),
            ..default()
        }))
        .add_plugins(EguiPlugin {
            enable_multipass_for_primary_context: false,
        })
        .insert_resource(Ide {
            source,
            file,
            examples,
            log: settings.lang.strings().ready.into(),
            result: None,
            settings,
            applied_theme: None,
            show_about: false,
        })
        .add_systems(Update, ui_system)
        .run();
}

fn list_examples() -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = std::fs::read_dir("examples")
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|e| e == "mo"))
        .collect();
    found.sort();
    found
}

fn file_label(path: &std::path::Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn load_example(ide: &mut Ide, path: PathBuf) {
    let s = ide.settings.lang.strings();
    match std::fs::read_to_string(&path) {
        Ok(text) => {
            ide.source = text;
            ide.file = Some(path);
            ide.result = None;
            ide.log = s.file_loaded.into();
        }
        Err(e) => ide.log = format!("{} {}: {e}", s.open_error, path.display()),
    }
}

fn save_current(ide: &mut Ide) {
    let s = ide.settings.lang.strings();
    match &ide.file {
        Some(path) => match std::fs::write(path, &ide.source) {
            Ok(()) => ide.log = format!("{}: {}", s.saved, path.display()),
            Err(e) => ide.log = format!("{}: {e}", s.write_error),
        },
        None => ide.log = s.no_file_to_save.into(),
    }
}

fn ui_system(mut contexts: EguiContexts, mut ide: ResMut<Ide>, mut exit: EventWriter<AppExit>) {
    let ctx = contexts.ctx_mut();
    let ide = &mut *ide;

    // применяем тему при первом кадре и при каждой смене
    if ide.applied_theme != Some(ide.settings.theme) {
        style::apply(ctx, ide.settings.theme);
        ide.applied_theme = Some(ide.settings.theme);
    }

    let settings_before = ide.settings;
    let s = ide.settings.lang.strings();
    let accent = style::accent(ide.settings.theme);

    // --- строка меню ---
    egui::TopBottomPanel::top("menubar").show(ctx, |ui| {
        egui::menu::bar(ui, |ui| {
            ui.menu_button(s.menu_file, |ui| {
                ui.menu_button(s.menu_open_example, |ui| {
                    let examples = ide.examples.clone();
                    for path in examples {
                        if ui.button(file_label(&path)).clicked() {
                            load_example(ide, path);
                            ui.close_menu();
                        }
                    }
                });
                if ui.button(s.save).clicked() {
                    save_current(ide);
                    ui.close_menu();
                }
                ui.separator();
                if ui.button(s.menu_quit).clicked() {
                    exit.write(AppExit::Success);
                }
            });

            ui.menu_button(s.menu_simulation, |ui| {
                if ui.button(s.menu_run).clicked() {
                    run_simulation(ide);
                    ui.close_menu();
                }
            });

            ui.menu_button(s.menu_view, |ui| {
                ui.menu_button(s.menu_theme, |ui| {
                    if ui
                        .radio_value(&mut ide.settings.theme, Theme::Dark, s.theme_dark)
                        .clicked()
                        | ui.radio_value(&mut ide.settings.theme, Theme::Light, s.theme_light)
                            .clicked()
                    {
                        ui.close_menu();
                    }
                });
                ui.menu_button(s.menu_language, |ui| {
                    if ui
                        .radio_value(&mut ide.settings.lang, Lang::En, Lang::En.label())
                        .clicked()
                        | ui.radio_value(&mut ide.settings.lang, Lang::Ru, Lang::Ru.label())
                            .clicked()
                    {
                        ui.close_menu();
                    }
                });
            });

            ui.menu_button(s.menu_help, |ui| {
                if ui.button(s.menu_about).clicked() {
                    ide.show_about = true;
                    ui.close_menu();
                }
            });
        });
    });

    // --- тулбар: быстрый доступ ---
    egui::TopBottomPanel::top("toolbar").show(ctx, |ui| {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            let current = ide
                .file
                .as_ref()
                .map(|p| file_label(p))
                .unwrap_or_else(|| s.no_file.into());
            let mut selected: Option<PathBuf> = None;
            egui::ComboBox::from_id_salt("examples-combo")
                .selected_text(&current)
                .show_ui(ui, |ui| {
                    for path in &ide.examples {
                        if ui
                            .selectable_label(Some(path) == ide.file.as_ref(), file_label(path))
                            .clicked()
                        {
                            selected = Some(path.clone());
                        }
                    }
                });
            if let Some(path) = selected {
                load_example(ide, path);
            }

            let run =
                egui::Button::new(egui::RichText::new(s.simulate).color(egui::Color32::WHITE))
                    .fill(accent);
            if ui.add(run).clicked() {
                run_simulation(ide);
            }
        });
        ui.add_space(4.0);
    });

    // --- статусная строка ---
    egui::TopBottomPanel::bottom("log").show(ctx, |ui| {
        ui.add_space(2.0);
        ui.monospace(&ide.log);
        ui.add_space(2.0);
    });

    // --- редактор ---
    egui::SidePanel::left("editor")
        .resizable(true)
        .default_width(600.0)
        .show(ctx, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.add(
                    egui::TextEdit::multiline(&mut ide.source)
                        .font(egui::TextStyle::Monospace)
                        .code_editor()
                        .desired_rows(40)
                        .desired_width(f32::INFINITY),
                );
            });
        });

    // --- графики ---
    egui::CentralPanel::default().show(ctx, |ui| match &mut ide.result {
        None => {
            ui.centered_and_justified(|ui| {
                ui.label(egui::RichText::new(s.press_simulate).weak());
            });
        }
        Some(data) => {
            ui.horizontal_wrapped(|ui| {
                for (index, name) in data.columns.iter().skip(1).enumerate() {
                    ui.checkbox(&mut data.visible[index], name);
                }
            });
            Plot::new("sim-plot")
                .legend(Legend::default())
                .show(ui, |plot_ui| {
                    for (index, name) in data.columns.iter().enumerate().skip(1) {
                        if !data.visible[index - 1] {
                            continue;
                        }
                        let points: PlotPoints =
                            data.rows.iter().map(|row| [row[0], row[index]]).collect();
                        plot_ui.line(Line::new(points).name(name));
                    }
                });
        }
    });

    // --- о программе ---
    if ide.show_about {
        egui::Window::new(s.menu_about)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .open(&mut ide.show_about)
            .show(ctx, |ui| {
                ui.heading(format!("Oxidelica {}", env!("CARGO_PKG_VERSION")));
                ui.add_space(4.0);
                ui.label(s.about_text);
                ui.add_space(4.0);
                ui.hyperlink("https://github.com/romanmorenko/oxidelica");
            });
    }

    if settings_before != ide.settings {
        settings::save(ide.settings);
    }
}

fn run_simulation(ide: &mut Ide) {
    let s = ide.settings.lang.strings();
    let started = Instant::now();
    let model = match oxidelica_parser::parse_model(&ide.source) {
        Ok(model) => model,
        Err(e) => {
            ide.log = format!("{}: {e}", s.parse_error);
            return;
        }
    };
    let compiled = match oxidelica_sim::compile(&model) {
        Ok(compiled) => compiled,
        Err(e) => {
            ide.log = format!("{}: {e}", s.compile_error);
            return;
        }
    };
    match compiled.simulate() {
        Ok(result) => {
            ide.log = format!(
                "{}: {} {} {:.1?}; {}: {}",
                compiled.name,
                result.rows.len().saturating_sub(1),
                s.steps_in,
                started.elapsed(),
                s.variables,
                result.columns[1..].join(", ")
            );
            ide.result = Some(SimData {
                visible: vec![true; result.columns.len().saturating_sub(1)],
                columns: result.columns,
                rows: result.rows,
            });
        }
        Err(e) => ide.log = format!("{}: {e}", s.sim_error),
    }
}
