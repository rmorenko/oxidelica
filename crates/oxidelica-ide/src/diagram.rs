//! The diagram editor: place library components on a canvas, wire their
//! connectors together and generate a Modelica model from the result.
//!
//! The diagram is the source of truth for what it generates: pressing
//! "Generate code" writes a model into the editor, which then follows
//! the ordinary path through the compiler. An existing model can also be
//! imported — its components of library types and its `connect`
//! statements become boxes and wires.

use bevy_egui::egui;
use oxidelica_parser::{class_info, ClassDef, ClassInfo};
use std::collections::HashMap;

/// Strings and colors the inspector needs, passed in so this module
/// stays independent of the localization tables.
pub struct Labels<'a> {
    /// Heading shown for a selected wire.
    pub wire: &'a str,
    /// Label of the delete button.
    pub delete: &'a str,
    /// Icon placed before the delete label.
    pub delete_icon: &'a str,
    /// Placeholder shown when nothing is selected.
    pub nothing_selected: &'a str,
    /// Color for destructive actions.
    pub danger: egui::Color32,
}

/// A component placed on the canvas.
pub struct Placed {
    /// Instance name used in the generated code.
    pub name: String,
    /// Fully qualified class name.
    pub class: String,
    /// Position of the box centre, in canvas coordinates.
    pub position: egui::Pos2,
    /// Parameter overrides as typed by the user.
    pub parameters: Vec<(String, String)>,
}

/// A wire between two connector ports.
pub struct Wire {
    /// Index of the source component and its port name.
    pub from: (usize, String),
    /// Index of the target component and its port name.
    pub to: (usize, String),
}

/// What the user currently has selected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Selection {
    /// Nothing selected.
    #[default]
    Nothing,
    /// A component, by index.
    Component(usize),
    /// A wire, by index.
    Wire(usize),
}

/// State of the diagram editor.
#[derive(Default)]
pub struct Diagram {
    /// Components on the canvas.
    pub components: Vec<Placed>,
    /// Wires between ports.
    pub wires: Vec<Wire>,
    /// Port picked as the start of a wire being drawn.
    pending: Option<(usize, String)>,
    /// Component or wire currently selected.
    selection: Selection,
    /// Class summaries, keyed by qualified class name.
    catalog: HashMap<String, ClassInfo>,
    /// Instantiable classes offered in the palette, sorted.
    palette: Vec<String>,
    /// Simulation settings written into the generated model.
    pub stop_time: f64,
    /// Output interval written into the generated model.
    pub interval: f64,
}

/// Size of a component box on the canvas.
const BOX_SIZE: egui::Vec2 = egui::vec2(120.0, 72.0);

/// The scrollable canvas is larger than the viewport, so components
/// never have to overlap and nothing is clipped away.
const CANVAS_SIZE: egui::Vec2 = egui::vec2(2400.0, 1600.0);

/// Grid step used when placing a new component in a free slot.
const GRID: egui::Vec2 = egui::vec2(170.0, 120.0);

/// Draw a schematic symbol for a class inside `area`.
///
/// The symbols are recognizable rather than standard-compliant: enough
/// to tell a resistor from a capacitor at a glance. Anything unknown
/// falls back to a plain block.
fn draw_symbol(painter: &egui::Painter, area: egui::Rect, class: &str, color: egui::Color32) {
    let stroke = egui::Stroke::new(1.8, color);
    let (left, right) = (
        egui::pos2(area.left(), area.center().y),
        egui::pos2(area.right(), area.center().y),
    );
    let leads = |from: f32, to: f32| {
        painter.line_segment([left, egui::pos2(from, area.center().y)], stroke);
        painter.line_segment([egui::pos2(to, area.center().y), right], stroke);
    };
    let short = class.rsplit('.').next().unwrap_or(class);
    let (w, h) = (area.width(), area.height());
    let centre = area.center();

    match short {
        "Resistor" => {
            let body = egui::Rect::from_center_size(centre, egui::vec2(w * 0.5, h * 0.5));
            leads(body.left(), body.right());
            painter.rect_stroke(body, 1.0, stroke, egui::StrokeKind::Inside);
        }
        "Capacitor" => {
            let gap = w * 0.07;
            leads(centre.x - gap, centre.x + gap);
            for x in [centre.x - gap, centre.x + gap] {
                painter.line_segment(
                    [
                        egui::pos2(x, centre.y - h * 0.32),
                        egui::pos2(x, centre.y + h * 0.32),
                    ],
                    stroke,
                );
            }
        }
        "Inductor" => {
            let body = egui::Rect::from_center_size(centre, egui::vec2(w * 0.5, h * 0.4));
            leads(body.left(), body.right());
            for i in 0..3 {
                let cx = body.left() + body.width() * (0.17 + 0.33 * i as f32);
                painter.circle_stroke(egui::pos2(cx, body.center().y), body.width() * 0.16, stroke);
            }
        }
        "Ground" => {
            painter.line_segment(
                [
                    egui::pos2(centre.x, area.top()),
                    egui::pos2(centre.x, centre.y),
                ],
                stroke,
            );
            for (i, width) in [0.34f32, 0.22, 0.10].into_iter().enumerate() {
                let y = centre.y + i as f32 * h * 0.16;
                painter.line_segment(
                    [
                        egui::pos2(centre.x - w * width, y),
                        egui::pos2(centre.x + w * width, y),
                    ],
                    stroke,
                );
            }
        }
        "ConstantVoltage" | "StepVoltage" | "SineVoltage" => {
            let radius = h * 0.38;
            leads(centre.x - radius, centre.x + radius);
            painter.circle_stroke(centre, radius, stroke);
            // The mark inside the circle is drawn, not typed: a font
            // without the glyph would otherwise show a stray symbol.
            let r = radius * 0.55;
            match short {
                "SineVoltage" => {
                    let points: Vec<egui::Pos2> = (0..=12)
                        .map(|i| {
                            let phase = i as f32 / 12.0;
                            egui::pos2(
                                centre.x - r + 2.0 * r * phase,
                                centre.y - r * 0.7 * (phase * std::f32::consts::TAU).sin(),
                            )
                        })
                        .collect();
                    painter.add(egui::Shape::line(points, stroke));
                }
                "StepVoltage" => {
                    painter.add(egui::Shape::line(
                        vec![
                            egui::pos2(centre.x - r, centre.y + r * 0.6),
                            egui::pos2(centre.x, centre.y + r * 0.6),
                            egui::pos2(centre.x, centre.y - r * 0.6),
                            egui::pos2(centre.x + r, centre.y - r * 0.6),
                        ],
                        stroke,
                    ));
                }
                _ => {
                    // Battery symbol: a long plate and a short one.
                    painter.line_segment(
                        [
                            egui::pos2(centre.x - r * 0.4, centre.y - r),
                            egui::pos2(centre.x - r * 0.4, centre.y + r),
                        ],
                        stroke,
                    );
                    painter.line_segment(
                        [
                            egui::pos2(centre.x + r * 0.4, centre.y - r * 0.5),
                            egui::pos2(centre.x + r * 0.4, centre.y + r * 0.5),
                        ],
                        stroke,
                    );
                }
            }
        }
        "EMF" => {
            let radius = h * 0.38;
            leads(centre.x - radius, centre.x + radius);
            painter.circle_stroke(centre, radius, stroke);
            painter.text(
                centre,
                egui::Align2::CENTER_CENTER,
                "M",
                egui::FontId::proportional(13.0),
                color,
            );
        }
        "Inertia" => {
            let body = egui::Rect::from_center_size(centre, egui::vec2(w * 0.42, h * 0.66));
            leads(body.left(), body.right());
            painter.rect_filled(body, 2.0, color.gamma_multiply(0.35));
            painter.rect_stroke(body, 2.0, stroke, egui::StrokeKind::Inside);
        }
        "Spring" => {
            let (x0, x1) = (centre.x - w * 0.25, centre.x + w * 0.25);
            leads(x0, x1);
            let mut points = vec![egui::pos2(x0, centre.y)];
            for i in 0..6 {
                let x = x0 + (x1 - x0) * (i as f32 + 0.5) / 6.0;
                let y = centre.y + if i % 2 == 0 { -h * 0.28 } else { h * 0.28 };
                points.push(egui::pos2(x, y));
            }
            points.push(egui::pos2(x1, centre.y));
            painter.add(egui::Shape::line(points, stroke));
        }
        "Damper" | "ViscousFriction" => {
            let body = egui::Rect::from_center_size(centre, egui::vec2(w * 0.4, h * 0.5));
            leads(body.left(), body.right());
            painter.rect_stroke(body, 1.0, stroke, egui::StrokeKind::Inside);
            painter.line_segment(
                [
                    egui::pos2(body.center().x, body.top()),
                    egui::pos2(body.center().x, body.bottom()),
                ],
                egui::Stroke::new(3.0, color),
            );
        }
        "Fixed" => {
            painter.line_segment(
                [
                    egui::pos2(centre.x, centre.y - h * 0.4),
                    egui::pos2(centre.x, centre.y + h * 0.4),
                ],
                stroke,
            );
            for i in 0..4 {
                let y = centre.y - h * 0.3 + i as f32 * h * 0.2;
                painter.line_segment(
                    [
                        egui::pos2(centre.x, y),
                        egui::pos2(centre.x - w * 0.16, y + h * 0.12),
                    ],
                    stroke,
                );
            }
            painter.line_segment([egui::pos2(centre.x, centre.y), right], stroke);
        }
        _ => {
            let body = egui::Rect::from_center_size(centre, egui::vec2(w * 0.62, h * 0.6));
            painter.rect_stroke(body, 3.0, stroke, egui::StrokeKind::Inside);
        }
    }
}

impl Diagram {
    /// Build the palette from the library classes.
    pub fn with_catalog(classes: &[ClassDef]) -> Diagram {
        let mut catalog = HashMap::new();
        let mut palette = Vec::new();
        for class in classes {
            let Some(info) = class_info(classes, &class.name) else {
                continue;
            };
            // Only classes with connectors can be wired up.
            if info.instantiable && !info.ports.is_empty() {
                palette.push(class.name.clone());
            }
            catalog.insert(class.name.clone(), info);
        }
        palette.sort();
        Diagram {
            catalog,
            palette,
            stop_time: 1.0,
            interval: 0.001,
            ..Diagram::default()
        }
    }

    /// Ports of a placed component's class.
    fn ports(&self, class: &str) -> &[String] {
        self.catalog
            .get(class)
            .map(|info| info.ports.as_slice())
            .unwrap_or_default()
    }

    /// Position of a port on the boundary of its component box: ports
    /// are spread down the left and right edges, alternating.
    fn port_position(&self, index: usize, port: &str) -> Option<egui::Pos2> {
        let placed = self.components.get(index)?;
        let ports = self.ports(&placed.class);
        let slot = ports.iter().position(|p| p == port)?;
        let left = slot % 2 == 0;
        let column: Vec<usize> = (0..ports.len()).filter(|i| (i % 2 == 0) == left).collect();
        let row = column.iter().position(|i| *i == slot).unwrap_or(0);
        let spacing = BOX_SIZE.y / (column.len() as f32 + 1.0);
        Some(egui::pos2(
            placed.position.x
                + if left {
                    -BOX_SIZE.x / 2.0
                } else {
                    BOX_SIZE.x / 2.0
                },
            placed.position.y - BOX_SIZE.y / 2.0 + spacing * (row as f32 + 1.0),
        ))
    }

    /// A fresh instance name derived from the class name.
    fn fresh_name(&self, class: &str) -> String {
        let stem: String = class
            .rsplit('.')
            .next()
            .unwrap_or(class)
            .chars()
            .take(1)
            .flat_map(|c| c.to_lowercase())
            .chain(
                class
                    .rsplit('.')
                    .next()
                    .unwrap_or(class)
                    .chars()
                    .skip(1)
                    .filter(|c| c.is_alphanumeric()),
            )
            .collect();
        let mut candidate = stem.clone();
        let mut counter = 1;
        while self.components.iter().any(|c| c.name == candidate) {
            counter += 1;
            candidate = format!("{stem}{counter}");
        }
        candidate
    }

    /// The first grid slot far enough from every placed component, so
    /// repeated additions spread out instead of piling up.
    pub fn free_slot(&self) -> egui::Pos2 {
        let columns = (CANVAS_SIZE.x / GRID.x) as i32;
        for index in 0.. {
            let candidate = egui::pos2(
                90.0 + (index % columns) as f32 * GRID.x,
                80.0 + (index / columns) as f32 * GRID.y,
            );
            let taken = self
                .components
                .iter()
                .any(|placed| (placed.position - candidate).abs().max_elem() < GRID.x * 0.5);
            if !taken {
                return candidate;
            }
        }
        egui::pos2(90.0, 80.0)
    }

    /// Lay every component out on a fresh grid.
    pub fn auto_layout(&mut self) {
        let columns = 4;
        for (index, placed) in self.components.iter_mut().enumerate() {
            placed.position = egui::pos2(
                110.0 + (index % columns) as f32 * GRID.x * 1.1,
                90.0 + (index / columns) as f32 * GRID.y * 1.15,
            );
        }
    }

    /// What is currently selected.
    pub fn selection(&self) -> Selection {
        self.selection
    }

    /// Select a component by index.
    pub fn select(&mut self, index: usize) {
        if index < self.components.len() {
            self.selection = Selection::Component(index);
        }
    }

    /// Select a wire by index.
    pub fn select_wire(&mut self, index: usize) {
        if index < self.wires.len() {
            self.selection = Selection::Wire(index);
        }
    }

    /// Delete whatever is selected: a component takes its wires with
    /// it, a wire goes on its own.
    pub fn delete_selected(&mut self) -> bool {
        match self.selection {
            Selection::Component(index) if index < self.components.len() => {
                self.remove(index);
                true
            }
            Selection::Wire(index) if index < self.wires.len() => {
                self.wires.remove(index);
                self.selection = Selection::Nothing;
                true
            }
            _ => false,
        }
    }

    /// Add a component of the given class at a canvas position.
    pub fn add(&mut self, class: &str, position: egui::Pos2) {
        let parameters = self
            .catalog
            .get(class)
            .map(|info| {
                info.parameters
                    .iter()
                    .map(|(name, _)| (name.clone(), String::new()))
                    .collect()
            })
            .unwrap_or_default();
        self.components.push(Placed {
            name: self.fresh_name(class),
            class: class.to_string(),
            position,
            parameters,
        });
    }

    /// Remove a component and every wire attached to it.
    fn remove(&mut self, index: usize) {
        self.components.remove(index);
        self.wires.retain(|w| w.from.0 != index && w.to.0 != index);
        for wire in &mut self.wires {
            if wire.from.0 > index {
                wire.from.0 -= 1;
            }
            if wire.to.0 > index {
                wire.to.0 -= 1;
            }
        }
        self.selection = Selection::Nothing;
        self.pending = None;
    }

    /// Generate the Modelica source for the diagram.
    pub fn to_source(&self, model_name: &str) -> String {
        let mut out = format!("model {model_name} \"Assembled in the diagram editor\"\n");
        for placed in &self.components {
            let overrides: Vec<String> = placed
                .parameters
                .iter()
                .filter(|(_, value)| !value.trim().is_empty())
                .map(|(name, value)| format!("{name} = {}", value.trim()))
                .collect();
            let modifier = if overrides.is_empty() {
                String::new()
            } else {
                format!("({})", overrides.join(", "))
            };
            out.push_str(&format!(
                "  {} {}{};\n",
                placed.class, placed.name, modifier
            ));
        }
        out.push_str("equation\n");
        for wire in &self.wires {
            let (Some(from), Some(to)) = (
                self.components.get(wire.from.0),
                self.components.get(wire.to.0),
            ) else {
                continue;
            };
            out.push_str(&format!(
                "  connect({}.{}, {}.{});\n",
                from.name, wire.from.1, to.name, wire.to.1
            ));
        }
        out.push_str(&format!(
            "  annotation(experiment(StopTime = {}, Interval = {}));\n",
            self.stop_time, self.interval
        ));
        out.push_str(&format!("end {model_name};\n"));
        out
    }

    /// Rebuild the diagram from a model's own source: components of
    /// library types become boxes, `connect` statements become wires,
    /// and the layout is a simple grid.
    pub fn import(&mut self, classes: &[ClassDef], model_name: &str) -> Result<usize, String> {
        let class = classes
            .iter()
            .rev()
            .find(|c| c.name == model_name)
            .ok_or_else(|| format!("model `{model_name}` not found"))?;
        self.components.clear();
        self.wires.clear();
        self.selection = Selection::Nothing;
        self.pending = None;

        let mut index_of: HashMap<String, usize> = HashMap::new();
        for component in &class.components {
            // Resolve the declared type against the catalog, allowing
            // for the import aliases of the model.
            let Some(qualified) = self.resolve_class(&component.type_name, &class.imports) else {
                continue;
            };
            let position = egui::pos2(
                160.0 + (self.components.len() % 4) as f32 * 190.0,
                120.0 + (self.components.len() / 4) as f32 * 140.0,
            );
            index_of.insert(component.name.clone(), self.components.len());
            let parameters = self.catalog[&qualified]
                .parameters
                .iter()
                .map(|(name, _)| {
                    let value = component
                        .modifiers
                        .iter()
                        .find(|(m, _)| m == name)
                        .map(|(_, expr)| expression_text(expr))
                        .unwrap_or_default();
                    (name.clone(), value)
                })
                .collect();
            self.components.push(Placed {
                name: component.name.clone(),
                class: qualified,
                position,
                parameters,
            });
        }
        for (a, b) in &class.connects {
            let split = |path: &str| -> Option<(usize, String)> {
                let (instance, port) = path.split_once('.')?;
                Some((*index_of.get(instance)?, port.to_string()))
            };
            if let (Some(from), Some(to)) = (split(a), split(b)) {
                self.wires.push(Wire { from, to });
            }
        }
        Ok(self.components.len())
    }

    /// Match a declared type name against the catalog, honouring the
    /// model's import aliases.
    fn resolve_class(&self, type_name: &str, imports: &[(String, String)]) -> Option<String> {
        if self.catalog.contains_key(type_name) {
            return Some(type_name.to_string());
        }
        let (head, rest) = type_name.split_once('.')?;
        let (_, target) = imports.iter().find(|(local, _)| local == head)?;
        let qualified = format!("{target}.{rest}");
        self.catalog.contains_key(&qualified).then_some(qualified)
    }

    /// Draw the palette; returns the class the user asked to place.
    pub fn palette_ui(&self, ui: &mut egui::Ui, filter: &mut String) -> Option<String> {
        let mut chosen = None;
        ui.horizontal(|ui| {
            ui.label(egui_phosphor::regular::MAGNIFYING_GLASS);
            ui.add(
                egui::TextEdit::singleline(filter)
                    .desired_width(f32::INFINITY)
                    .hint_text("filter"),
            );
        });
        ui.separator();
        egui::ScrollArea::vertical().show(ui, |ui| {
            let needle = filter.to_lowercase();
            for class in &self.palette {
                if !needle.is_empty() && !class.to_lowercase().contains(&needle) {
                    continue;
                }
                let short = class.rsplit('.').next().unwrap_or(class);
                let response = ui.selectable_label(false, short);
                let tooltip = self
                    .catalog
                    .get(class)
                    .and_then(|info| info.description.clone())
                    .unwrap_or_else(|| class.clone());
                if response
                    .on_hover_text(format!("{class}\n{tooltip}"))
                    .clicked()
                {
                    chosen = Some(class.clone());
                }
            }
        });
        chosen
    }

    /// Draw the canvas and handle interaction. Returns true when the
    /// diagram changed and the generated code is stale.
    pub fn canvas_ui(&mut self, ui: &mut egui::Ui, accent: egui::Color32) -> bool {
        let mut changed = false;
        let (response, painter) = ui.allocate_painter(CANVAS_SIZE, egui::Sense::click_and_drag());
        let origin = response.rect.min.to_vec2();
        let visuals = ui.visuals().clone();

        // A faint grid gives the canvas a sense of place while scrolling.
        let grid_stroke = egui::Stroke::new(1.0, visuals.weak_text_color().gamma_multiply(0.25));
        let mut x = 0.0;
        while x <= CANVAS_SIZE.x {
            painter.line_segment(
                [
                    egui::pos2(x, 0.0) + origin,
                    egui::pos2(x, CANVAS_SIZE.y) + origin,
                ],
                grid_stroke,
            );
            x += GRID.x;
        }
        let mut y = 0.0;
        while y <= CANVAS_SIZE.y {
            painter.line_segment(
                [
                    egui::pos2(0.0, y) + origin,
                    egui::pos2(CANVAS_SIZE.x, y) + origin,
                ],
                grid_stroke,
            );
            y += GRID.y;
        }

        // Wires first, so boxes draw over them. A wire is pickable
        // along its length and shows a handle at its midpoint.
        let pointer = ui.ctx().pointer_latest_pos();
        let mut hovered_wire = None;
        for (index, wire) in self.wires.iter().enumerate() {
            let (Some(a), Some(b)) = (
                self.port_position(wire.from.0, &wire.from.1),
                self.port_position(wire.to.0, &wire.to.1),
            ) else {
                continue;
            };
            let (a, b) = (a + origin, b + origin);
            if pointer.is_some_and(|p| distance_to_segment(p, a, b) < 6.0) {
                hovered_wire = Some(index);
            }
            let selected = self.selection == Selection::Wire(index);
            let near = hovered_wire == Some(index);
            let (width, color) = match (selected, near) {
                (true, _) => (3.5, egui::Color32::from_rgb(219, 92, 92)),
                (_, true) => (3.0, accent),
                _ => (2.0, accent.gamma_multiply(0.9)),
            };
            painter.line_segment([a, b], egui::Stroke::new(width, color));
            if selected || near {
                let middle = a + (b - a) * 0.5;
                painter.circle_filled(middle, 5.5, color);
                let arm = 2.6;
                let cross = egui::Stroke::new(1.6, visuals.extreme_bg_color);
                painter.line_segment(
                    [
                        middle + egui::vec2(-arm, -arm),
                        middle + egui::vec2(arm, arm),
                    ],
                    cross,
                );
                painter.line_segment(
                    [
                        middle + egui::vec2(-arm, arm),
                        middle + egui::vec2(arm, -arm),
                    ],
                    cross,
                );
            }
        }

        // A wire being drawn follows the pointer.
        if let Some((index, port)) = &self.pending {
            if let (Some(start), Some(pointer)) = (
                self.port_position(*index, port),
                ui.ctx().pointer_latest_pos(),
            ) {
                painter.line_segment(
                    [start + origin, pointer],
                    egui::Stroke::new(1.5, accent.gamma_multiply(0.5)),
                );
            }
        }

        let mut clicked_port: Option<(usize, String)> = None;
        let mut to_remove: Option<usize> = None;

        for index in 0..self.components.len() {
            let placed = &self.components[index];
            let rect = egui::Rect::from_center_size(placed.position + origin, BOX_SIZE);
            let id = ui.id().with(("diagram-box", index));
            let box_response = ui.interact(rect, id, egui::Sense::click_and_drag());

            if box_response.dragged() {
                let moved = self.components[index].position + box_response.drag_delta();
                self.components[index].position = egui::pos2(
                    moved.x.clamp(BOX_SIZE.x, CANVAS_SIZE.x - BOX_SIZE.x),
                    moved.y.clamp(BOX_SIZE.y, CANVAS_SIZE.y - BOX_SIZE.y),
                );
                changed = true;
            }
            if box_response.clicked() {
                self.select(index);
            }
            if box_response.secondary_clicked() {
                to_remove = Some(index);
            }

            let placed = &self.components[index];
            let selected = self.selection == Selection::Component(index);
            painter.rect(
                rect,
                6.0,
                if selected {
                    visuals.widgets.hovered.weak_bg_fill
                } else {
                    visuals.widgets.inactive.weak_bg_fill
                },
                egui::Stroke::new(if selected { 2.0 } else { 1.0 }, accent),
                egui::StrokeKind::Inside,
            );
            let short = placed.class.rsplit('.').next().unwrap_or(&placed.class);
            // The symbol occupies the middle of the box; the instance
            // name sits above it and the class below.
            draw_symbol(
                &painter,
                egui::Rect::from_center_size(
                    rect.center() + egui::vec2(0.0, 2.0),
                    egui::vec2(BOX_SIZE.x * 0.62, BOX_SIZE.y * 0.44),
                ),
                &placed.class,
                visuals.text_color(),
            );
            painter.text(
                egui::pos2(rect.center().x, rect.top() + 11.0),
                egui::Align2::CENTER_CENTER,
                &placed.name,
                egui::FontId::proportional(13.0),
                visuals.text_color(),
            );
            painter.text(
                egui::pos2(rect.center().x, rect.bottom() - 9.0),
                egui::Align2::CENTER_CENTER,
                short,
                egui::FontId::proportional(10.0),
                visuals.weak_text_color(),
            );

            // Ports.
            for port in self.ports(&placed.class).to_vec() {
                let Some(at) = self.port_position(index, &port) else {
                    continue;
                };
                let at = at + origin;
                let port_rect = egui::Rect::from_center_size(at, egui::vec2(11.0, 11.0));
                let port_id = ui.id().with(("diagram-port", index, port.clone()));
                let port_response = ui.interact(port_rect, port_id, egui::Sense::click());
                let hovered = port_response.hovered();
                painter.circle(
                    at,
                    if hovered { 6.0 } else { 4.5 },
                    if hovered { accent } else { visuals.panel_fill },
                    egui::Stroke::new(1.5, accent),
                );
                if hovered {
                    painter.text(
                        at + egui::vec2(0.0, -12.0),
                        egui::Align2::CENTER_CENTER,
                        &port,
                        egui::FontId::proportional(11.0),
                        visuals.text_color(),
                    );
                }
                if port_response.clicked() {
                    clicked_port = Some((index, port.clone()));
                }
            }
        }

        if let Some(index) = to_remove {
            self.remove(index);
            changed = true;
        }
        if let Some((index, port)) = clicked_port {
            match self.pending.take() {
                Some((other_index, other_port)) if other_index != index || other_port != port => {
                    self.wires.push(Wire {
                        from: (other_index, other_port),
                        to: (index, port),
                    });
                    changed = true;
                }
                Some(_) => {}
                None => self.pending = Some((index, port)),
            }
        } else if response.clicked() {
            match hovered_wire {
                // Clicking a wire selects it; clicking the selected one
                // again removes it.
                Some(index) if self.selection == Selection::Wire(index) => {
                    self.wires.remove(index);
                    self.selection = Selection::Nothing;
                    changed = true;
                }
                Some(index) => self.select_wire(index),
                None => {
                    // A click on empty canvas cancels a half-drawn wire.
                    self.pending = None;
                    self.selection = Selection::Nothing;
                }
            }
        }
        if hovered_wire.is_some() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        changed
    }

    /// Inspector for the current selection: parameters of a component,
    /// endpoints of a wire. Returns true on any edit.
    pub fn inspector_ui(&mut self, ui: &mut egui::Ui, labels: &Labels) -> bool {
        let index = match self.selection {
            Selection::Component(index) => index,
            Selection::Wire(index) => {
                let Some(wire) = self.wires.get(index) else {
                    return false;
                };
                let endpoint = |(component, port): &(usize, String)| {
                    self.components
                        .get(*component)
                        .map(|placed| format!("{}.{port}", placed.name))
                        .unwrap_or_else(|| port.clone())
                };
                ui.strong(labels.wire);
                ui.label(endpoint(&wire.from));
                ui.label(endpoint(&wire.to));
                ui.separator();
                if ui
                    .button(
                        egui::RichText::new(format!("{} {}", labels.delete_icon, labels.delete))
                            .color(labels.danger),
                    )
                    .clicked()
                {
                    self.wires.remove(index);
                    self.selection = Selection::Nothing;
                    return true;
                }
                return false;
            }
            Selection::Nothing => {
                ui.label(egui::RichText::new(labels.nothing_selected).weak());
                return false;
            }
        };
        let Some(placed) = self.components.get_mut(index) else {
            return false;
        };
        let mut changed = false;
        ui.horizontal(|ui| {
            ui.label("name");
            if ui
                .add(egui::TextEdit::singleline(&mut placed.name).desired_width(120.0))
                .changed()
            {
                changed = true;
            }
        });
        ui.label(egui::RichText::new(&placed.class).weak().small());
        ui.separator();
        let delete = ui
            .button(
                egui::RichText::new(format!("{} {}", labels.delete_icon, labels.delete))
                    .color(labels.danger),
            )
            .clicked();
        if delete {
            self.remove(index);
            return true;
        }
        let Some(placed) = self.components.get_mut(index) else {
            return false;
        };
        for (name, value) in &mut placed.parameters {
            ui.horizontal(|ui| {
                ui.label(name.as_str());
                if ui
                    .add(
                        egui::TextEdit::singleline(value)
                            .desired_width(90.0)
                            .hint_text("default"),
                    )
                    .changed()
                {
                    changed = true;
                }
            });
        }
        changed
    }
}

/// Distance from a point to a segment, for wire picking.
fn distance_to_segment(point: egui::Pos2, a: egui::Pos2, b: egui::Pos2) -> f32 {
    let segment = b - a;
    let length_squared = segment.length_sq();
    if length_squared <= f32::EPSILON {
        return (point - a).length();
    }
    let t = ((point - a).dot(segment) / length_squared).clamp(0.0, 1.0);
    (point - (a + segment * t)).length()
}

/// Render an expression back to source text for the inspector.
fn expression_text(expr: &oxidelica_parser::Expr) -> String {
    use oxidelica_parser::Expr;
    match expr {
        Expr::Number(n) => format!("{n}"),
        Expr::Bool(b) => b.to_string(),
        Expr::Ref(name) => name.clone(),
        Expr::Time => "time".to_string(),
        Expr::Neg(inner) => format!("-{}", expression_text(inner)),
        Expr::Bin(op, l, r) => {
            let symbol = match op {
                oxidelica_parser::BinOp::Add => "+",
                oxidelica_parser::BinOp::Sub => "-",
                oxidelica_parser::BinOp::Mul => "*",
                oxidelica_parser::BinOp::Div => "/",
                oxidelica_parser::BinOp::Pow => "^",
            };
            format!("{} {symbol} {}", expression_text(l), expression_text(r))
        }
        other => format!("{other:?}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn library() -> Vec<ClassDef> {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let source = std::fs::read_to_string(root.join("lib/Oxidelica.mo")).unwrap();
        oxidelica_parser::parse_file(&source).unwrap()
    }

    #[test]
    fn palette_offers_only_wireable_classes() {
        let diagram = Diagram::with_catalog(&library());
        assert!(diagram
            .palette
            .contains(&"Oxidelica.Electrical.Analog.Basic.Resistor".to_string()));
        // Partial bases and classes without connectors stay out.
        assert!(!diagram
            .palette
            .iter()
            .any(|c| c.ends_with("Interfaces.OnePort")));
        assert!(!diagram
            .palette
            .iter()
            .any(|c| c == "Oxidelica.Blocks.Math.Gain"));
    }

    #[test]
    fn a_mouse_built_circuit_generates_a_model_that_compiles() {
        let classes = library();
        let mut diagram = Diagram::with_catalog(&classes);
        // Place a source, a resistor, a capacitor and a ground, the way
        // the palette would.
        for class in [
            "Oxidelica.Electrical.Analog.Sources.ConstantVoltage",
            "Oxidelica.Electrical.Analog.Basic.Resistor",
            "Oxidelica.Electrical.Analog.Basic.Capacitor",
            "Oxidelica.Electrical.Analog.Basic.Ground",
        ] {
            diagram.add(class, egui::pos2(0.0, 0.0));
        }
        for (name, value) in [("R", "100"), ("C", "0.001"), ("V", "1")] {
            for placed in &mut diagram.components {
                if let Some(slot) = placed.parameters.iter_mut().find(|(p, _)| p == name) {
                    slot.1 = value.to_string();
                }
            }
        }
        // Wire it into an RC circuit.
        diagram.wires.push(Wire {
            from: (0, "p".into()),
            to: (1, "p".into()),
        });
        diagram.wires.push(Wire {
            from: (1, "n".into()),
            to: (2, "p".into()),
        });
        diagram.wires.push(Wire {
            from: (2, "n".into()),
            to: (0, "n".into()),
        });
        diagram.wires.push(Wire {
            from: (0, "n".into()),
            to: (3, "p".into()),
        });
        diagram.stop_time = 0.5;

        let source = diagram.to_source("MouseBuilt");
        let library_source = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../lib/Oxidelica.mo"),
        )
        .unwrap();
        let model =
            oxidelica_parser::parse_model_with_libraries(&[library_source], &source).unwrap();
        let result = oxidelica_sim::compile(&model).unwrap().simulate().unwrap();

        // The capacitor charges toward the source voltage.
        let cv = result
            .columns
            .iter()
            .position(|c| c.ends_with(".v") && c.starts_with("capacitor"))
            .expect("a capacitor voltage column");
        let last = result.rows.last().unwrap()[cv];
        let analytic = 1.0 - (-0.5f64 / (100.0 * 0.001)).exp();
        assert!(
            (last - analytic).abs() < 1e-6,
            "capacitor at {last}, analytic {analytic}"
        );
    }

    #[test]
    fn repeated_additions_spread_across_the_grid() {
        let mut diagram = Diagram::with_catalog(&library());
        let class = "Oxidelica.Electrical.Analog.Basic.Resistor";
        for _ in 0..6 {
            let slot = diagram.free_slot();
            diagram.add(class, slot);
        }
        // Every component landed somewhere of its own.
        for (i, a) in diagram.components.iter().enumerate() {
            for b in &diagram.components[i + 1..] {
                assert!(
                    (a.position - b.position).abs().max_elem() >= GRID.x * 0.5,
                    "components overlap at {:?}",
                    a.position
                );
            }
            // ... and inside the canvas.
            assert!(a.position.x < CANVAS_SIZE.x && a.position.y < CANVAS_SIZE.y);
            // Names are unique, so the generated code compiles.
            assert_eq!(
                diagram
                    .components
                    .iter()
                    .filter(|other| other.name == a.name)
                    .count(),
                1
            );
        }
        // Arranging keeps them apart too.
        diagram.auto_layout();
        for (i, a) in diagram.components.iter().enumerate() {
            for b in &diagram.components[i + 1..] {
                assert!((a.position - b.position).abs().max_elem() > 20.0);
            }
        }
    }

    #[test]
    fn deleting_a_component_drops_its_wires_and_reindexes() {
        let mut diagram = Diagram::with_catalog(&library());
        for class in [
            "Oxidelica.Electrical.Analog.Sources.ConstantVoltage",
            "Oxidelica.Electrical.Analog.Basic.Resistor",
            "Oxidelica.Electrical.Analog.Basic.Ground",
        ] {
            let slot = diagram.free_slot();
            diagram.add(class, slot);
        }
        diagram.wires.push(Wire {
            from: (0, "p".into()),
            to: (1, "p".into()),
        });
        diagram.wires.push(Wire {
            from: (1, "n".into()),
            to: (2, "p".into()),
        });

        // Nothing selected yet, so there is nothing to delete.
        assert!(!diagram.delete_selected());

        // A wire can be removed on its own, leaving the components.
        diagram.select_wire(1);
        assert!(diagram.delete_selected());
        assert_eq!(diagram.wires.len(), 1);
        assert_eq!(diagram.components.len(), 3);
        assert_eq!(diagram.selection(), Selection::Nothing);
        diagram.wires.push(Wire {
            from: (1, "n".into()),
            to: (2, "p".into()),
        });

        diagram.select(1);
        assert!(diagram.delete_selected());
        assert_eq!(diagram.components.len(), 2);
        // Both wires touched the resistor, so both are gone.
        assert!(diagram.wires.is_empty());
        // The ground moved down one index and the generated code follows.
        assert!(diagram.components[1].class.ends_with("Ground"));
        assert_eq!(diagram.selection(), Selection::Nothing);
    }

    #[test]
    fn a_model_imports_back_into_a_diagram() {
        let mut classes = library();
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let source = std::fs::read_to_string(root.join("examples/dc_motor.mo")).unwrap();
        classes.extend(oxidelica_parser::parse_file(&source).unwrap());

        let mut diagram = Diagram::with_catalog(&classes);
        let count = diagram.import(&classes, "DCMotor").unwrap();
        assert_eq!(count, 7, "every library component landed on the canvas");
        assert_eq!(
            diagram.wires.len(),
            7,
            "every connect statement became a wire"
        );
        // Import keeps parameter overrides and resolves import aliases.
        let supply = diagram
            .components
            .iter()
            .find(|c| c.name == "supply")
            .expect("the supply component");
        assert!(supply.class.ends_with("Sources.StepVoltage"));
        assert!(supply
            .parameters
            .iter()
            .any(|(name, value)| name == "startTime" && value == "0.1"));
    }
}
