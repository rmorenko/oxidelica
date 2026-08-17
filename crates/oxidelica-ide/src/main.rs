//! Oxidelica IDE — the modeling environment: menu, code editor,
//! simulation, plots. EN/RU localization, dark/light themes, a
//! JetBrains-inspired look with icons and brand gradient strips.

mod highlight;
mod i18n;
mod settings;
mod style;
mod view3d;

use bevy::app::AppExit;
use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts, EguiPlugin};
use egui_phosphor::regular as icons;
use egui_plot::{Legend, Line, Plot, PlotPoints};
use i18n::Lang;
use settings::{Settings, Theme};
use std::path::PathBuf;
use std::sync::{mpsc, Mutex};
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

/// Which view occupies the central panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ViewMode {
    /// Time-series plots of all variables.
    Plots,
    /// Animated trajectory of two selected variables.
    Animation,
    /// Embedded Bevy 3D scene with bodies and trails.
    ThreeD,
}

/// Trajectory animation state.
struct Anim {
    /// Current animation time within the simulated interval.
    time: f64,
    playing: bool,
    /// Playback speed multiplier.
    speed: f64,
    /// Column index used as the X coordinate.
    x_col: usize,
    /// Column index used as the Y coordinate.
    y_col: usize,
    /// Draw a rod from the origin to the current point (pendulums).
    rod: bool,
}

impl Default for Anim {
    fn default() -> Self {
        Anim {
            time: 0.0,
            playing: true,
            speed: 1.0,
            x_col: 0,
            y_col: 1,
            rod: false,
        }
    }
}

/// One tunable value: a parameter or a state initial value.
struct TunerEntry {
    /// Variable name in the model.
    name: String,
    /// Current (possibly user-modified) value.
    value: f64,
    /// Model default, used by the reset buttons.
    default: f64,
}

/// The live parameter/initial-value tuner panel state.
#[derive(Default)]
struct Tuner {
    params: Vec<TunerEntry>,
    inits: Vec<TunerEntry>,
    /// Experiment settings (StopTime, Interval) as tunable entries.
    exp: Vec<TunerEntry>,
    /// A value changed and a re-simulation is pending.
    dirty: bool,
    /// When the last change happened (for debouncing).
    last_change: Option<Instant>,
    /// Whether the tuner has been populated at least once.
    initialized: bool,
    /// Integration method chosen for the next run.
    solver: oxidelica_sim::SolverMethod,
}

impl Tuner {
    /// Rebuild entries from a freshly compiled model, preserving the
    /// values the user has already modified (matched by name).
    fn refresh(&mut self, compiled: &oxidelica_sim::CompiledModel) {
        let carry = |old: &[TunerEntry], name: &str, default: f64| -> f64 {
            old.iter()
                .find(|e| e.name == name && e.value != e.default)
                .map(|e| e.value)
                .unwrap_or(default)
        };
        self.params = compiled
            .parameters
            .iter()
            .map(|(name, value)| TunerEntry {
                value: carry(&self.params, name, *value),
                default: *value,
                name: name.clone(),
            })
            .collect();
        self.inits = compiled
            .states
            .iter()
            .zip(&compiled.initial)
            .map(|(name, value)| TunerEntry {
                value: carry(&self.inits, name, *value),
                default: *value,
                name: name.clone(),
            })
            .collect();
        self.exp = [
            ("StopTime", compiled.stop_time),
            ("Interval", compiled.step),
        ]
        .into_iter()
        .map(|(name, value)| TunerEntry {
            value: carry(&self.exp, name, value),
            default: value,
            name: name.to_string(),
        })
        .collect();
        self.initialized = true;
    }

    /// Apply user overrides onto a compiled model before simulation.
    fn apply(&self, compiled: &mut oxidelica_sim::CompiledModel) {
        for entry in &self.params {
            if let Some(slot) = compiled
                .parameters
                .iter_mut()
                .find(|(n, _)| n == &entry.name)
            {
                slot.1 = entry.value;
            }
        }
        for entry in &self.inits {
            if let Some(index) = compiled.states.iter().position(|n| n == &entry.name) {
                compiled.initial[index] = entry.value;
            }
        }
        for entry in &self.exp {
            match entry.name.as_str() {
                "StopTime" => compiled.stop_time = entry.value.max(1e-6),
                "Interval" => compiled.step = entry.value.max(1e-9),
                _ => {}
            }
        }
    }
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
    /// Active central view.
    view: ViewMode,
    /// Trajectory animation state.
    anim: Anim,
    /// Live parameter/initial-value tuner.
    tuner: Tuner,
    /// The in-flight background simulation, if any.
    sim_job: Option<SimJob>,
}

/// A background simulation running on a worker thread.
struct SimJob {
    /// Wrapped in a Mutex because Bevy resources must be `Sync`.
    #[allow(clippy::type_complexity)]
    receiver: Mutex<
        mpsc::Receiver<(
            String,
            Result<oxidelica_sim::SimResult, oxidelica_sim::SimError>,
        )>,
    >,
    started: Instant,
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
            view: ViewMode::Plots,
            anim: Anim::default(),
            tuner: Tuner::default(),
            sim_job: None,
        })
        .add_systems(Startup, view3d::setup)
        .add_systems(Update, (ui_system, view3d::sync_scene).chain())
        .run();
}

/// Library sources from the `lib` directory, available to every model.
fn load_libraries() -> Vec<String> {
    let mut paths: Vec<PathBuf> = std::fs::read_dir("lib")
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|e| e == "mo"))
        .collect();
    paths.sort();
    paths
        .iter()
        .filter_map(|path| std::fs::read_to_string(path).ok())
        .collect()
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

/// Re-parse the editor buffer and refresh the tuner entries
/// (silently: editor errors are reported on the next explicit run).
fn refresh_tuner(ide: &mut Ide) {
    if let Ok(model) = oxidelica_parser::parse_model_with_libraries(&load_libraries(), &ide.source)
    {
        if let Ok(compiled) = oxidelica_sim::compile(&model) {
            ide.tuner.refresh(&compiled);
        }
    }
}

/// The single per-frame UI system: menus, panels, plots, dialogs.
fn ui_system(
    mut contexts: EguiContexts,
    mut ide: ResMut<Ide>,
    mut scene: ResMut<view3d::Scene3d>,
    mut exit: EventWriter<AppExit>,
) {
    // Register the 3D render target with egui once.
    if scene.texture.is_none() {
        scene.texture = Some(contexts.add_image(scene.target.clone_weak()));
    }
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

    if !ide.tuner.initialized {
        refresh_tuner(ide);
    }

    // Collect a finished background simulation.
    if let Some(job) = &ide.sim_job {
        let received = job.receiver.lock().map(|rx| rx.try_recv());
        match received.unwrap_or(Err(mpsc::TryRecvError::Disconnected)) {
            Ok((name, outcome)) => {
                let elapsed = job.started.elapsed();
                ide.sim_job = None;
                apply_sim_outcome(ide, name, outcome, elapsed);
            }
            Err(mpsc::TryRecvError::Empty) => {}
            Err(mpsc::TryRecvError::Disconnected) => {
                ide.sim_job = None;
                ide.log = s.sim_error.into();
                ide.log_ok = false;
            }
        }
    }

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
            if let Some(job) = &ide.sim_job {
                let spinner = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
                let index = (job.started.elapsed().as_millis() / 90) as usize % spinner.len();
                ui.label(
                    egui::RichText::new(spinner[index])
                        .color(p.accent)
                        .size(15.0),
                );
                ui.monospace(format!("{}… {:.1?}", s.simulating, job.started.elapsed()));
            } else {
                let (icon, color) = if ide.log_ok {
                    (icons::CHECK_CIRCLE, p.ok_green)
                } else {
                    (icons::X_CIRCLE, p.error_red)
                };
                ui.label(egui::RichText::new(icon).color(color).size(15.0));
                ui.monospace(&ide.log);
            }
        });
        ui.add_space(2.0);
    });

    // --- editor with Modelica syntax highlighting ---
    let mut source_edited = false;
    let editor_theme = ide.settings.theme;
    let mut layouter = |ui: &egui::Ui, text: &str, wrap_width: f32| {
        let font = egui::TextStyle::Monospace.resolve(ui.style());
        let mut job = highlight::highlight(text, editor_theme, font);
        job.wrap.max_width = wrap_width;
        ui.fonts(|fonts| fonts.layout_job(job))
    };
    egui::SidePanel::left("editor")
        .resizable(true)
        .default_width(600.0)
        .show(ctx, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                let response = ui.add(
                    egui::TextEdit::multiline(&mut ide.source)
                        .font(egui::TextStyle::Monospace)
                        .code_editor()
                        .desired_rows(40)
                        .desired_width(f32::INFINITY)
                        .layouter(&mut layouter),
                );
                if response.changed() {
                    source_edited = true;
                }
            });
        });
    if source_edited {
        refresh_tuner(ide);
    }

    // --- tuner: parameters and initial values ---
    if ide.tuner.initialized {
        egui::SidePanel::right("tuner")
            .resizable(true)
            .default_width(220.0)
            .show(ctx, |ui| {
                let tuner = &mut ide.tuner;
                let mut changed = false;
                egui::ScrollArea::vertical().show(ui, |ui| {
                    for (title, entries) in [
                        (s.params_title, &mut tuner.params),
                        (s.inits_title, &mut tuner.inits),
                        (s.menu_simulation, &mut tuner.exp),
                    ] {
                        if entries.is_empty() {
                            continue;
                        }
                        ui.add_space(4.0);
                        ui.strong(title);
                        ui.separator();
                        for entry in entries.iter_mut() {
                            ui.horizontal(|ui| {
                                ui.label(&entry.name);
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        let modified = entry.value != entry.default;
                                        if ui
                                            .add_visible(
                                                modified,
                                                egui::Button::new(icons::ARROW_COUNTER_CLOCKWISE)
                                                    .small(),
                                            )
                                            .clicked()
                                        {
                                            entry.value = entry.default;
                                            changed = true;
                                        }
                                        let speed = (entry.default.abs() * 0.01).max(0.001);
                                        if ui
                                            .add(
                                                egui::DragValue::new(&mut entry.value)
                                                    .speed(speed)
                                                    .max_decimals(6),
                                            )
                                            .changed()
                                        {
                                            changed = true;
                                        }
                                    },
                                );
                            });
                        }
                    }
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        ui.label(s.solver_label);
                        let before = tuner.solver;
                        egui::ComboBox::from_id_salt("solver-combo")
                            .selected_text(tuner.solver.name())
                            .show_ui(ui, |ui| {
                                for method in [
                                    oxidelica_sim::SolverMethod::Dopri45,
                                    oxidelica_sim::SolverMethod::Bdf,
                                    oxidelica_sim::SolverMethod::Rk4,
                                ] {
                                    ui.selectable_value(&mut tuner.solver, method, method.name());
                                }
                            });
                        if tuner.solver != before {
                            changed = true;
                        }
                    });
                    ui.add_space(8.0);
                    if ui
                        .button(format!(
                            "{} {}",
                            icons::ARROW_COUNTER_CLOCKWISE,
                            s.reset_all
                        ))
                        .clicked()
                    {
                        let all = tuner
                            .params
                            .iter_mut()
                            .chain(tuner.inits.iter_mut())
                            .chain(tuner.exp.iter_mut());
                        for entry in all {
                            entry.value = entry.default;
                        }
                        changed = true;
                    }
                });
                if changed {
                    tuner.dirty = true;
                    tuner.last_change = Some(Instant::now());
                }
            });
    }

    // Debounced live re-simulation after tuner changes.
    if ide.tuner.dirty
        && ide.sim_job.is_none()
        && ide
            .tuner
            .last_change
            .is_some_and(|at| at.elapsed().as_millis() > 300)
    {
        ide.tuner.dirty = false;
        run_simulation(ide);
    }

    // Advance the shared animation clock for both animated views.
    if let Some(data) = &ide.result {
        if ide.anim.playing && matches!(ide.view, ViewMode::Animation | ViewMode::ThreeD) {
            let stop = data.rows.last().map(|row| row[0]).unwrap_or(1.0).max(1e-9);
            ide.anim.time += ctx.input(|i| i.stable_dt) as f64 * ide.anim.speed;
            if ide.anim.time > stop {
                ide.anim.time = 0.0;
            }
        }
    }

    // --- central panel: plots / animation / 3D tabs ---
    egui::CentralPanel::default().show(ctx, |ui| {
        ui.horizontal(|ui| {
            ui.selectable_value(
                &mut ide.view,
                ViewMode::Plots,
                format!("{} {}", icons::CHART_LINE, s.tab_plots),
            );
            ui.selectable_value(
                &mut ide.view,
                ViewMode::Animation,
                format!("{} {}", icons::PLAY, s.tab_animation),
            );
            ui.selectable_value(
                &mut ide.view,
                ViewMode::ThreeD,
                format!("{} {}", icons::CUBE, s.tab_3d),
            );
        });
        ui.separator();

        let Ide {
            result, anim, view, ..
        } = &mut *ide;
        match result {
            None => {
                ui.centered_and_justified(|ui| {
                    ui.label(
                        egui::RichText::new(format!("{}  {}", icons::CHART_LINE, s.press_simulate))
                            .weak(),
                    );
                });
            }
            Some(data) => match view {
                ViewMode::Plots => {
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
                ViewMode::Animation => animation_ui(ui, data, anim, s, &p),
                ViewMode::ThreeD => {
                    playback_controls(ui, data, anim, s);
                    view3d::tab_ui(ui, &mut scene, view3d::has_bodies(data), s.anim_no_bodies);
                }
            },
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

/// Shared playback controls: play/pause, the time slider and speed.
fn playback_controls(ui: &mut egui::Ui, data: &SimData, anim: &mut Anim, s: &i18n::Strings) {
    let stop = data.rows.last().map(|row| row[0]).unwrap_or(1.0).max(1e-9);
    ui.horizontal(|ui| {
        let icon = if anim.playing {
            icons::PAUSE
        } else {
            icons::PLAY
        };
        if ui.button(egui::RichText::new(icon).size(16.0)).clicked() {
            anim.playing = !anim.playing;
        }
        ui.add(
            egui::Slider::new(&mut anim.time, 0.0..=stop)
                .show_value(false)
                .trailing_fill(true),
        );
        ui.monospace(format!("t = {:6.2}", anim.time));
        ui.separator();
        ui.label(s.anim_speed);
        ui.add(
            egui::Slider::new(&mut anim.speed, 0.1..=10.0)
                .logarithmic(true)
                .show_value(false),
        );
    });
}

/// The trajectory animation view: playback controls, X/Y variable
/// selection and the animated plot (marker, optional rod, trail).
fn animation_ui(
    ui: &mut egui::Ui,
    data: &SimData,
    anim: &mut Anim,
    s: &i18n::Strings,
    p: &style::Palette,
) {
    anim.x_col = anim.x_col.min(data.columns.len() - 1);
    anim.y_col = anim.y_col.min(data.columns.len() - 1);

    playback_controls(ui, data, anim, s);
    ui.horizontal(|ui| {
        for (label, col) in [("X", &mut anim.x_col), ("Y", &mut anim.y_col)] {
            egui::ComboBox::from_id_salt(label)
                .selected_text(format!("{label}: {}", data.columns[*col]))
                .show_ui(ui, |ui| {
                    for (index, name) in data.columns.iter().enumerate() {
                        ui.selectable_value(col, index, name);
                    }
                });
        }
        ui.checkbox(&mut anim.rod, s.anim_rod);
    });

    // Current sample and a decimated trail up to it.
    let idx = data
        .rows
        .partition_point(|row| row[0] < anim.time)
        .min(data.rows.len() - 1);
    let step = (idx / 3000).max(1);
    let trail: PlotPoints = data.rows[..=idx]
        .iter()
        .step_by(step)
        .map(|row| [row[anim.x_col], row[anim.y_col]])
        .collect();
    let current = [data.rows[idx][anim.x_col], data.rows[idx][anim.y_col]];

    // Fixed bounds over the whole trajectory so the view does not jump.
    let (mut min_x, mut max_x, mut min_y, mut max_y) = (
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
    );
    for row in &data.rows {
        min_x = min_x.min(row[anim.x_col]);
        max_x = max_x.max(row[anim.x_col]);
        min_y = min_y.min(row[anim.y_col]);
        max_y = max_y.max(row[anim.y_col]);
    }
    if anim.rod {
        min_x = min_x.min(0.0);
        max_x = max_x.max(0.0);
        min_y = min_y.min(0.0);
        max_y = max_y.max(0.0);
    }
    let margin_x = ((max_x - min_x) * 0.1).max(0.1);
    let margin_y = ((max_y - min_y) * 0.1).max(0.1);

    let mut plot = Plot::new("anim-plot");
    if anim.rod {
        plot = plot.data_aspect(1.0);
    }
    plot.show(ui, |plot_ui| {
        plot_ui.set_plot_bounds(egui_plot::PlotBounds::from_min_max(
            [min_x - margin_x, min_y - margin_y],
            [max_x + margin_x, max_y + margin_y],
        ));
        plot_ui.line(
            Line::new(trail)
                .color(p.accent.gamma_multiply(0.4))
                .width(1.5),
        );
        if anim.rod {
            plot_ui.line(
                Line::new(PlotPoints::from(vec![[0.0, 0.0], current]))
                    .color(p.accent)
                    .width(3.0),
            );
        }
        plot_ui.points(
            egui_plot::Points::new(vec![current])
                .radius(6.0)
                .color(p.accent),
        );
    });
}

/// Parse and compile the editor buffer, then launch the simulation on
/// a worker thread; the UI stays responsive and the result is picked
/// up by `ui_system` on a later frame.
fn run_simulation(ide: &mut Ide) {
    let s = ide.settings.lang.strings();
    ide.log_ok = false;
    let model = match oxidelica_parser::parse_model_with_libraries(&load_libraries(), &ide.source) {
        Ok(model) => model,
        Err(e) => {
            ide.log = format!("{}: {e}", s.parse_error);
            return;
        }
    };
    let mut compiled = match oxidelica_sim::compile(&model) {
        Ok(compiled) => compiled,
        Err(e) => {
            ide.log = format!("{}: {e}", s.compile_error);
            return;
        }
    };
    ide.tuner.refresh(&compiled);
    ide.tuner.apply(&mut compiled);
    compiled.method = ide.tuner.solver;

    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let outcome = compiled.simulate();
        let _ = sender.send((compiled.name.clone(), outcome));
    });
    ide.sim_job = Some(SimJob {
        receiver: Mutex::new(receiver),
        started: Instant::now(),
    });
}

/// Apply a finished background simulation to the IDE state.
fn apply_sim_outcome(
    ide: &mut Ide,
    name: String,
    outcome: Result<oxidelica_sim::SimResult, oxidelica_sim::SimError>,
    elapsed: std::time::Duration,
) {
    let s = ide.settings.lang.strings();
    match outcome {
        Ok(result) => {
            ide.log = match &result.terminated {
                Some(message) => format!("{name}: {message}"),
                None => format!(
                    "{name}: {} {} {:.1?}; {}: {}",
                    result.rows.len().saturating_sub(1),
                    s.steps_in,
                    elapsed,
                    s.variables,
                    result.columns[1..].join(", ")
                ),
            };
            ide.log_ok = true;
            // Preserve playback and curve choices when the variable set
            // is unchanged (live parameter tuning); reset otherwise.
            let same_columns = ide
                .result
                .as_ref()
                .is_some_and(|old| old.columns == result.columns);
            let visible = match ide.result.take() {
                Some(old) if same_columns => old.visible,
                _ => vec![true; result.columns.len().saturating_sub(1)],
            };
            if !same_columns {
                // Animation defaults: literal x/y columns turn on
                // pendulum mode (rod from the origin, equal aspect).
                let find = |name: &str| result.columns.iter().position(|c| c == name);
                let (x_col, y_col) = match (find("x"), find("y")) {
                    (Some(x), Some(y)) => (x, y),
                    _ => (0, 1.min(result.columns.len() - 1)),
                };
                ide.anim = Anim {
                    rod: find("x").is_some() && find("y").is_some(),
                    x_col,
                    y_col,
                    ..Anim::default()
                };
            }
            ide.result = Some(SimData {
                visible,
                columns: result.columns,
                rows: result.rows,
            });
        }
        Err(e) => {
            ide.log = format!("{}: {e}", s.sim_error);
            ide.log_ok = false;
        }
    }
}
