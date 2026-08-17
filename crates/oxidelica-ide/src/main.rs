//! Oxidelica IDE v0 — примитивная среда: редактор кода модели,
//! кнопка «Симулировать», графики результатов, локализация (EN/RU)
//! и две темы (тёмная/светлая) с сохранением выбора в конфиг.

mod i18n;
mod settings;

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

fn ui_system(mut contexts: EguiContexts, mut ide: ResMut<Ide>) {
    let ctx = contexts.ctx_mut();
    let ide = &mut *ide;

    // применяем тему при первом кадре и при каждой смене
    if ide.applied_theme != Some(ide.settings.theme) {
        ctx.set_visuals(match ide.settings.theme {
            Theme::Dark => egui::Visuals::dark(),
            Theme::Light => egui::Visuals::light(),
        });
        ide.applied_theme = Some(ide.settings.theme);
    }

    let s = ide.settings.lang.strings();

    egui::TopBottomPanel::top("toolbar").show(ctx, |ui| {
        ui.horizontal(|ui| {
            ui.heading("Oxidelica");
            ui.separator();

            let current = ide
                .file
                .as_ref()
                .and_then(|p| p.file_name())
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| s.no_file.into());
            let mut selected: Option<PathBuf> = None;
            egui::ComboBox::from_id_salt("examples-combo")
                .selected_text(&current)
                .show_ui(ui, |ui| {
                    for path in &ide.examples {
                        let name = path
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_default();
                        if ui
                            .selectable_label(Some(path) == ide.file.as_ref(), name)
                            .clicked()
                        {
                            selected = Some(path.clone());
                        }
                    }
                });
            if let Some(path) = selected {
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

            if ui.button(s.save).clicked() {
                match &ide.file {
                    Some(path) => match std::fs::write(path, &ide.source) {
                        Ok(()) => ide.log = format!("{}: {}", s.saved, path.display()),
                        Err(e) => ide.log = format!("{}: {e}", s.write_error),
                    },
                    None => ide.log = s.no_file_to_save.into(),
                }
            }

            if ui.button(s.simulate).clicked() {
                run_simulation(ide);
            }

            // правый край: тема и язык
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let theme_icon = match ide.settings.theme {
                    Theme::Dark => "☀",
                    Theme::Light => "🌙",
                };
                if ui
                    .button(theme_icon)
                    .on_hover_text(s.theme_tooltip)
                    .clicked()
                {
                    ide.settings.theme = match ide.settings.theme {
                        Theme::Dark => Theme::Light,
                        Theme::Light => Theme::Dark,
                    };
                    settings::save(ide.settings);
                }

                let mut lang = ide.settings.lang;
                egui::ComboBox::from_id_salt("lang-combo")
                    .selected_text(lang.label())
                    .show_ui(ui, |ui| {
                        for candidate in Lang::ALL {
                            ui.selectable_value(&mut lang, candidate, candidate.label());
                        }
                    })
                    .response
                    .on_hover_text(s.language_tooltip);
                if lang != ide.settings.lang {
                    ide.settings.lang = lang;
                    settings::save(ide.settings);
                }
            });
        });
    });

    egui::TopBottomPanel::bottom("log").show(ctx, |ui| {
        ui.monospace(&ide.log);
    });

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

    egui::CentralPanel::default().show(ctx, |ui| match &mut ide.result {
        None => {
            ui.centered_and_justified(|ui| {
                ui.label(s.press_simulate);
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
