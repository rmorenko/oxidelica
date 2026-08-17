//! IDE look and feel: rounding, spacing, typography, accent color.
//! A single source of truth for both themes.

use crate::settings::Theme;
use bevy_egui::egui::{
    self, Color32, CornerRadius, FontFamily, FontId, Stroke, TextStyle, Visuals,
};

/// Theme accent color (buttons, selection, links).
pub fn accent(theme: Theme) -> Color32 {
    match theme {
        Theme::Dark => Color32::from_rgb(122, 148, 255),
        Theme::Light => Color32::from_rgb(64, 98, 235),
    }
}

/// Apply the full IDE style for the given theme to an egui context.
pub fn apply(ctx: &egui::Context, theme: Theme) {
    let accent = accent(theme);
    let mut visuals = match theme {
        Theme::Dark => Visuals::dark(),
        Theme::Light => Visuals::light(),
    };

    // Soft rounding instead of sharp corners.
    let radius = CornerRadius::same(7);
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

    // Calm panel tones.
    match theme {
        Theme::Dark => {
            visuals.panel_fill = Color32::from_rgb(26, 28, 34);
            visuals.window_fill = Color32::from_rgb(30, 32, 39);
            visuals.extreme_bg_color = Color32::from_rgb(18, 20, 25); // editor background
            visuals.faint_bg_color = Color32::from_rgb(34, 37, 45);
        }
        Theme::Light => {
            visuals.panel_fill = Color32::from_rgb(245, 246, 250);
            visuals.window_fill = Color32::WHITE;
            visuals.extreme_bg_color = Color32::WHITE;
            visuals.faint_bg_color = Color32::from_rgb(236, 238, 244);
        }
    }

    // Accent: selection, hover, links.
    visuals.selection.bg_fill = accent.gamma_multiply(0.35);
    visuals.selection.stroke = Stroke::new(1.0, accent);
    visuals.hyperlink_color = accent;
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, accent.gamma_multiply(0.6));
    visuals.widgets.active.bg_stroke = Stroke::new(1.0, accent);

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
