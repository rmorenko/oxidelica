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

/// State of the diagram editor.
#[derive(Default)]
pub struct Diagram {
    /// Components on the canvas.
    pub components: Vec<Placed>,
    /// Wires between ports.
    pub wires: Vec<Wire>,
    /// Port picked as the start of a wire being drawn.
    pending: Option<(usize, String)>,
    /// Component whose parameters are being edited.
    selected: Option<usize>,
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
const BOX_SIZE: egui::Vec2 = egui::vec2(120.0, 64.0);

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
        self.selected = None;
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
        self.selected = None;
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
            ui.label("\u{1F50D}");
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
        let (response, painter) =
            ui.allocate_painter(ui.available_size(), egui::Sense::click_and_drag());
        let origin = response.rect.min.to_vec2();
        let visuals = ui.visuals().clone();

        // Wires first, so boxes draw over them.
        for wire in &self.wires {
            let (Some(a), Some(b)) = (
                self.port_position(wire.from.0, &wire.from.1),
                self.port_position(wire.to.0, &wire.to.1),
            ) else {
                continue;
            };
            painter.line_segment(
                [a + origin, b + origin],
                egui::Stroke::new(2.0, accent.gamma_multiply(0.9)),
            );
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
                let delta = box_response.drag_delta();
                self.components[index].position += delta;
                changed = true;
            }
            if box_response.clicked() {
                self.selected = Some(index);
            }
            if box_response.secondary_clicked() {
                to_remove = Some(index);
            }

            let placed = &self.components[index];
            let selected = self.selected == Some(index);
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
            painter.text(
                rect.center() - egui::vec2(0.0, 8.0),
                egui::Align2::CENTER_CENTER,
                &placed.name,
                egui::FontId::proportional(14.0),
                visuals.text_color(),
            );
            painter.text(
                rect.center() + egui::vec2(0.0, 10.0),
                egui::Align2::CENTER_CENTER,
                short,
                egui::FontId::proportional(11.0),
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
            // A click on empty canvas cancels a half-drawn wire.
            self.pending = None;
            self.selected = None;
        }
        changed
    }

    /// Parameter editor for the selected component; returns true on any
    /// edit.
    pub fn inspector_ui(&mut self, ui: &mut egui::Ui) -> bool {
        let Some(index) = self.selected else {
            ui.label(egui::RichText::new("Select a component").weak());
            return false;
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
