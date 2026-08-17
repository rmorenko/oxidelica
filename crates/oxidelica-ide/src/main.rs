//! Oxidelica IDE — the modeling environment: menu, code editor,
//! simulation, plots. EN/RU localization, dark/light themes, a
//! JetBrains-inspired look with icons and brand gradient strips.

mod i18n;
mod settings;
mod style;

use bevy::app::AppExit;
use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts, EguiPlugin};
use egui_phosphor::regular as icons;
use egui_plot::{Legend, Line, Plot, PlotPoints};
use i18n::Lang;
use settings::{Settings, Theme};
use std::path::PathBuf;
use std::time::Instant;

/// Model shown when no example files are found.
const DEFAULT_MODEL: &str = "model Demo \"Damped oscillator\"\n  parameter Real k = 4.0;\n  parameter Real d = 0.3;\n  Real x(start = 1.0);\n  Real v(start = 0.0);\nequation\n  der(x) = v;\n  der(v) = -k * x - d * v;\n  annotation(experiment(StopTime = 10.0, Interval = 0.001));\nend Demo;\n";

/// Simulation output prepared for plotting.
struct SimData {
    columns: Vec<String>,
    rows: Vec<Vec<f64>>,
    /// Curve visibility, one flag per column except time.
    visible: Vec<bool>,
}

/// The whole IDE state, stored as a Bevy resource.
#[derive(Resource)]
struct Ide {
    source: String,
    file: Option<PathBuf>,
    examples: Vec<PathBuf>,
    log: String,
    /// Whether the last logged action succeeded (drives the status icon).
    log_ok: bool,
    result: Option<SimData>,
    settings: Settings,
    /// The theme currently applied to egui (None — none yet).
    applied_theme: Option<Theme>,
    /// Whether the icon font has been installed into the egui context.
    fonts_installed: bool,
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
            log_ok: true,
            result: None,
            settings,
            applied_theme: None,
            fonts_installed: false,
            show_about: false,
        })
        .add_systems(Update, ui_system)
        .run();
}

/// Collect the `.mo` files from the `examples` directory.
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

/// File name for UI labels.
fn file_label(path: &std::path::Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Load an example file into the editor.
fn load_example(ide: &mut Ide, path: PathBuf) {
    let s = ide.settings.lang.strings();
    match std::fs::read_to_string(&path) {
        Ok(text) => {
            ide.source = text;
            ide.file = Some(path);
            ide.result = None;
            ide.log = s.file_loaded.into();
            ide.log_ok = true;
        }
        Err(e) => {
            ide.log = format!("{} {}: {e}", s.open_error, path.display());
            ide.log_ok = false;
        }
    }
}

/// Save the editor buffer to the current file.
fn save_current(ide: &mut Ide) {
    let s = ide.settings.lang.strings();
    match &ide.file {
        Some(path) => match std::fs::write(path, &ide.source) {
            Ok(()) => {
                ide.log = format!("{}: {}", s.saved, path.display());
                ide.log_ok = true;
            }
            Err(e) => {
                ide.log = format!("{}: {e}", s.write_error);
                ide.log_ok = false;
            }
        },
        None => {
            ide.log = s.no_file_to_save.into();
            ide.log_ok = false;
        }
    }
}

/// The single per-frame UI system: menus, panels, plots, dialogs.
fn ui_system(mut contexts: EguiContexts, mut ide: ResMut<Ide>, mut exit: EventWriter<AppExit>) {
    let ctx = contexts.ctx_mut();
    let ide = &mut *ide;

    // Install the Phosphor icon font once.
    if !ide.fonts_installed {
        let mut fonts = egui::FontDefinitions::default();
        egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
        ctx.set_fonts(fonts);
        ide.fonts_installed = true;
    }

    // Apply the theme on the first frame and on every change.
    if ide.applied_theme != Some(ide.settings.theme) {
        style::apply(ctx, ide.settings.theme);
        ide.applied_theme = Some(ide.settings.theme);
    }

    let settings_before = ide.settings;
    let s = ide.settings.lang.strings();
    let p = style::palette(ide.settings.theme);

    // --- menu bar ---
    egui::TopBottomPanel::top("menubar").show(ctx, |ui| {
        egui::menu::bar(ui, |ui| {
            ui.menu_button(s.menu_file, |ui| {
                ui.menu_button(
                    format!("{} {}", icons::FOLDER_OPEN, s.menu_open_example),
                    |ui| {
                        let examples = ide.examples.clone();
                        for path in examples {
                            if ui
                                .button(format!("{} {}", icons::FILE_CODE, file_label(&path)))
                                .clicked()
                            {
                                load_example(ide, path);
                                ui.close_menu();
                            }
                        }
                    },
                );
                if ui
                    .button(format!("{} {}", icons::FLOPPY_DISK, s.save))
                    .clicked()
                {
                    save_current(ide);
                    ui.close_menu();
                }
                ui.separator();
                if ui
                    .button(format!("{} {}", icons::SIGN_OUT, s.menu_quit))
                    .clicked()
                {
                    exit.write(AppExit::Success);
                }
            });

            ui.menu_button(s.menu_simulation, |ui| {
                if ui
                    .button(format!("{} {}", icons::PLAY, s.menu_run))
                    .clicked()
                {
                    run_simulation(ide);
                    ui.close_menu();
                }
            });

            ui.menu_button(s.menu_view, |ui| {
                ui.menu_button(format!("{} {}", icons::PAINT_BRUSH, s.menu_theme), |ui| {
                    if ui
                        .radio_value(&mut ide.settings.theme, Theme::Dark, s.theme_dark)
                        .clicked()
                        | ui.radio_value(&mut ide.settings.theme, Theme::Light, s.theme_light)
                            .clicked()
                    {
                        ui.close_menu();
                    }
                });
                ui.menu_button(format!("{} {}", icons::TRANSLATE, s.menu_language), |ui| {
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
                if ui
                    .button(format!("{} {}", icons::INFO, s.menu_about))
                    .clicked()
                {
                    ide.show_about = true;
                    ui.close_menu();
                }
            });
        });
    });

    // --- toolbar: quick access ---
    egui::TopBottomPanel::top("toolbar").show(ctx, |ui| {
        ui.add_space(5.0);
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(icons::TREE_STRUCTURE).color(p.accent));
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

            if ui
                .button(egui::RichText::new(icons::FLOPPY_DISK).size(16.0))
                .on_hover_text(s.save)
                .clicked()
            {
                save_current(ide);
            }

            ui.separator();

            let run_label = format!("{} {}", icons::PLAY_CIRCLE, s.simulate);
            let run = egui::Button::new(
                egui::RichText::new(run_label)
                    .color(egui::Color32::WHITE)
                    .strong(),
            )
            .fill(p.run_green);
            if ui.add(run).clicked() {
                run_simulation(ide);
            }
        });
        ui.add_space(5.0);
    });

    // --- brand gradient strip under the toolbar ---
    egui::TopBottomPanel::top("gradient")
        .frame(egui::Frame::NONE)
        .exact_height(3.0)
        .show(ctx, |ui| {
            style::gradient_strip(ui, 3.0);
        });

    // --- status line ---
    egui::TopBottomPanel::bottom("log").show(ctx, |ui| {
        ui.add_space(2.0);
        ui.horizontal(|ui| {
            let (icon, color) = if ide.log_ok {
                (icons::CHECK_CIRCLE, p.ok_green)
            } else {
                (icons::X_CIRCLE, p.error_red)
            };
            ui.label(egui::RichText::new(icon).color(color).size(15.0));
            ui.monospace(&ide.log);
        });
        ui.add_space(2.0);
    });

    // --- editor ---
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

    // --- plots ---
    egui::CentralPanel::default().show(ctx, |ui| match &mut ide.result {
        None => {
            ui.centered_and_justified(|ui| {
                ui.label(
                    egui::RichText::new(format!("{}  {}", icons::CHART_LINE, s.press_simulate))
                        .weak(),
                );
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

    // --- about dialog ---
    if ide.show_about {
        egui::Window::new(s.menu_about)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .open(&mut ide.show_about)
            .show(ctx, |ui| {
                ui.heading(format!("Oxidelica {}", env!("CARGO_PKG_VERSION")));
                style::gradient_strip(ui, 3.0);
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

/// Parse, compile and simulate the editor buffer; report to the log line.
fn run_simulation(ide: &mut Ide) {
    let s = ide.settings.lang.strings();
    let started = Instant::now();
    ide.log_ok = false;
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
            ide.log_ok = true;
            ide.result = Some(SimData {
                visible: vec![true; result.columns.len().saturating_sub(1)],
                columns: result.columns,
                rows: result.rows,
            });
        }
        Err(e) => ide.log = format!("{}: {e}", s.sim_error),
    }
}
