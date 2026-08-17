//! IDE look and feel, modeled after the JetBrains "Int UI" design
//! language: calm flat panels, a blue accent, a green run button and
//! brand gradient strips. A single source of truth for both themes.

use crate::settings::Theme;
use bevy_egui::egui::{
    self, Color32, CornerRadius, FontFamily, FontId, Mesh, Rect, Stroke, TextStyle, Visuals,
};

/// The color palette of a theme.
pub struct Palette {
    /// Editor background (the deepest surface).
    pub editor_bg: Color32,
    /// Panel background (toolbars, side panels).
    pub panel: Color32,
    /// Slightly raised surface (buttons, inputs).
    pub surface: Color32,
    /// Hover highlight.
    pub hover: Color32,
    /// Hairline borders between panels.
    pub border: Color32,
    /// Accent color (selection, links, focus).
    pub accent: Color32,
    /// The green of the Run button.
    pub run_green: Color32,
    /// Error red for the status line.
    pub error_red: Color32,
    /// Success green for the status line.
    pub ok_green: Color32,
}

/// Palette for the given theme (values follow JetBrains Int UI).
pub fn palette(theme: Theme) -> Palette {
    match theme {
        Theme::Dark => Palette {
            editor_bg: Color32::from_rgb(30, 31, 34),  // #1E1F22
            panel: Color32::from_rgb(43, 45, 48),      // #2B2D30
            surface: Color32::from_rgb(52, 54, 58),    // #34363A
            hover: Color32::from_rgb(57, 59, 64),      // #393B40
            border: Color32::from_rgb(30, 31, 34),     // #1E1F22
            accent: Color32::from_rgb(53, 116, 240),   // #3574F0
            run_green: Color32::from_rgb(73, 156, 84), // #499C54
            error_red: Color32::from_rgb(219, 92, 92), // #DB5C5C
            ok_green: Color32::from_rgb(87, 150, 92),  // #57965C
        },
        Theme::Light => Palette {
            editor_bg: Color32::WHITE,
            panel: Color32::from_rgb(247, 248, 250), // #F7F8FA
            surface: Color32::from_rgb(242, 243, 245), // #F2F3F5
            hover: Color32::from_rgb(233, 234, 236), // #E9EAEC
            border: Color32::from_rgb(235, 236, 240), // #EBECF0
            accent: Color32::from_rgb(53, 116, 240), // #3574F0
            run_green: Color32::from_rgb(32, 138, 60), // #208A3C
            error_red: Color32::from_rgb(199, 43, 43), // #C72B2B
            ok_green: Color32::from_rgb(32, 138, 60), // #208A3C
        },
    }
}

/// The brand gradient stops (blue -> violet -> magenta), as on
/// JetBrains product splash strips.
const GRADIENT: [Color32; 3] = [
    Color32::from_rgb(53, 116, 240), // #3574F0
    Color32::from_rgb(151, 71, 255), // #9747FF
    Color32::from_rgb(232, 74, 163), // #E84AA3
];

/// Paint a thin horizontal brand-gradient strip filling the available
/// width.
pub fn gradient_strip(ui: &mut egui::Ui, height: f32) {
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), height),
        egui::Sense::hover(),
    );
    paint_gradient(ui.painter(), rect, &GRADIENT);
}

/// Fill `rect` with a horizontal multi-stop gradient.
fn paint_gradient(painter: &egui::Painter, rect: Rect, stops: &[Color32]) {
    if stops.len() < 2 {
        return;
    }
    let mut mesh = Mesh::default();
    let segments = stops.len() - 1;
    let width = rect.width() / segments as f32;
    for (i, pair) in stops.windows(2).enumerate() {
        let x0 = rect.left() + width * i as f32;
        let x1 = x0 + width;
        let base = mesh.vertices.len() as u32;
        for (x, color) in [(x0, pair[0]), (x1, pair[1])] {
            mesh.colored_vertex(egui::pos2(x, rect.top()), color);
            mesh.colored_vertex(egui::pos2(x, rect.bottom()), color);
        }
        mesh.add_triangle(base, base + 1, base + 2);
        mesh.add_triangle(base + 1, base + 3, base + 2);
    }
    painter.add(mesh);
}

/// Apply the full IDE style for the given theme to an egui context.
pub fn apply(ctx: &egui::Context, theme: Theme) {
    let p = palette(theme);
    let mut visuals = match theme {
        Theme::Dark => Visuals::dark(),
        Theme::Light => Visuals::light(),
    };

    // Soft rounding instead of sharp corners.
    let radius = CornerRadius::same(6);
    for widget in [
        &mut visuals.widgets.noninteractive,
        &mut visuals.widgets.inactive,
        &mut visuals.widgets.hovered,
        &mut visuals.widgets.active,
        &mut visuals.widgets.open,
    ] {
        widget.corner_radius = radius;
    }
    visuals.window_corner_radius = CornerRadius::same(10);
    visuals.menu_corner_radius = CornerRadius::same(8);

    // Int UI surfaces.
    visuals.panel_fill = p.panel;
    visuals.window_fill = p.panel;
    visuals.extreme_bg_color = p.editor_bg;
    visuals.faint_bg_color = p.surface;
    visuals.widgets.inactive.weak_bg_fill = p.surface;
    visuals.widgets.hovered.weak_bg_fill = p.hover;
    visuals.widgets.active.weak_bg_fill = p.hover;
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, p.border);
    visuals.window_stroke = Stroke::new(1.0, p.border);

    // Accent: selection, hover, links.
    visuals.selection.bg_fill = p.accent.gamma_multiply(0.35);
    visuals.selection.stroke = Stroke::new(1.0, p.accent);
    visuals.hyperlink_color = p.accent;
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, p.accent.gamma_multiply(0.5));
    visuals.widgets.active.bg_stroke = Stroke::new(1.0, p.accent);

    let mut style = (*ctx.style()).clone();
    style.visuals = visuals;

    // Air: spacing and element heights.
    style.spacing.item_spacing = egui::vec2(8.0, 6.0);
    style.spacing.button_padding = egui::vec2(12.0, 5.0);
    style.spacing.menu_margin = egui::Margin::same(8);
    style.spacing.window_margin = egui::Margin::same(12);
    style.spacing.interact_size.y = 26.0;

    // Typography a step larger than the default.
    style.text_styles = [
        (
            TextStyle::Heading,
            FontId::new(17.0, FontFamily::Proportional),
        ),
        (TextStyle::Body, FontId::new(14.0, FontFamily::Proportional)),
        (
            TextStyle::Button,
            FontId::new(14.0, FontFamily::Proportional),
        ),
        (
            TextStyle::Monospace,
            FontId::new(13.5, FontFamily::Monospace),
        ),
        (
            TextStyle::Small,
            FontId::new(11.0, FontFamily::Proportional),
        ),
    ]
    .into();

    ctx.set_style(style);
}
