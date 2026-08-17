//! The 3D trajectory view: a real Bevy scene rendered to an offscreen
//! texture and embedded into an egui panel.
//!
//! Bodies are detected from result columns by naming convention:
//! `x`/`y`[/`z`] with a shared suffix form one body (`x1`,`y1` ->
//! body "1"; plain `x`,`y` -> a single body). Each body is a sphere;
//! trails and the pendulum rod are drawn with gizmos. The camera
//! orbits with mouse drag and zooms with the scroll wheel.

use crate::settings::Theme;
use crate::{Ide, SimData, ViewMode};
use bevy::prelude::*;
use bevy::render::camera::RenderTarget;
use bevy::render::render_resource::{
    Extent3d, TextureDescriptor, TextureDimension, TextureFormat, TextureUsages,
};
use bevy_egui::egui;

/// Side length of the offscreen render target in pixels.
const TARGET_SIZE: u32 = 1200;

/// Distinct body colors (JetBrains-flavored).
const BODY_COLORS: [Color; 5] = [
    Color::srgb(0.21, 0.45, 0.94), // blue
    Color::srgb(0.94, 0.65, 0.20), // orange
    Color::srgb(0.34, 0.61, 0.36), // green
    Color::srgb(0.91, 0.29, 0.64), // magenta
    Color::srgb(0.26, 0.67, 0.72), // cyan
];

/// State of the embedded 3D scene.
#[derive(Resource)]
pub struct Scene3d {
    /// Offscreen render target the camera draws into.
    pub target: Handle<Image>,
    /// The egui texture id of the render target (registered lazily).
    pub texture: Option<egui::TextureId>,
    /// Orbit camera yaw in radians.
    pub yaw: f32,
    /// Orbit camera pitch in radians.
    pub pitch: f32,
    /// Camera distance as a multiple of the scene extent.
    pub dist: f32,
    /// Shared unit-sphere mesh for bodies (created lazily).
    sphere: Option<Handle<Mesh>>,
    /// Number of body entities currently spawned.
    spawned: usize,
}

/// Marker for body entities; the payload is the body index.
#[derive(Component)]
pub struct Body3d(pub usize);

/// Marker for the offscreen scene camera.
#[derive(Component)]
pub struct SceneCamera;

/// A body detected from result columns.
pub struct BodyCols {
    /// Column index of the x coordinate.
    pub x: usize,
    /// Column index of the y coordinate.
    pub y: usize,
    /// Column index of the z coordinate, when present.
    pub z: Option<usize>,
}

/// Detect coordinate column groups: `x<sfx>`/`y<sfx>`[/`z<sfx>`].
pub fn detect_bodies(columns: &[String]) -> Vec<BodyCols> {
    let find = |name: &str| columns.iter().position(|c| c == name);
    let mut bodies = Vec::new();
    for (index, column) in columns.iter().enumerate() {
        if let Some(suffix) = column.strip_prefix('x') {
            if let Some(y) = find(&format!("y{suffix}")) {
                bodies.push(BodyCols {
                    x: index,
                    y,
                    z: find(&format!("z{suffix}")),
                });
            }
        }
    }
    bodies
}

/// Startup: create the render target, the orbit camera and the light.
pub fn setup(mut commands: Commands, mut images: ResMut<Assets<Image>>) {
    let size = Extent3d {
        width: TARGET_SIZE,
        height: TARGET_SIZE,
        depth_or_array_layers: 1,
    };
    let mut image = Image {
        texture_descriptor: TextureDescriptor {
            label: Some("oxidelica-3d-target"),
            size,
            dimension: TextureDimension::D2,
            format: TextureFormat::Bgra8UnormSrgb,
            mip_level_count: 1,
            sample_count: 1,
            usage: TextureUsages::TEXTURE_BINDING
                | TextureUsages::COPY_DST
                | TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        },
        ..default()
    };
    image.resize(size);
    let target = images.add(image);

    commands.spawn((
        Camera3d::default(),
        Camera {
            target: RenderTarget::Image(target.clone().into()),
            ..default()
        },
        Transform::from_xyz(2.0, 1.5, 4.0).looking_at(Vec3::ZERO, Vec3::Y),
        SceneCamera,
    ));
    commands.spawn((
        DirectionalLight {
            illuminance: 12_000.0,
            shadows_enabled: false,
            ..default()
        },
        Transform::from_xyz(3.0, 6.0, 4.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    commands.insert_resource(AmbientLight {
        color: Color::WHITE,
        brightness: 250.0,
        ..default()
    });

    commands.insert_resource(Scene3d {
        target,
        texture: None,
        yaw: 0.6,
        pitch: 0.4,
        dist: 2.6,
        sphere: None,
        spawned: 0,
    });
}

/// The egui side of the 3D tab: the rendered image plus orbit/zoom input.
pub fn tab_ui(ui: &mut egui::Ui, scene: &mut Scene3d, has_bodies: bool, no_bodies_text: &str) {
    if !has_bodies {
        ui.centered_and_justified(|ui| {
            ui.label(egui::RichText::new(no_bodies_text).weak());
        });
        return;
    }
    let Some(texture) = scene.texture else { return };
    let side = ui.available_size().min_elem().max(64.0);
    let image = egui::Image::new(egui::load::SizedTexture::new(
        texture,
        egui::vec2(side, side),
    ))
    .sense(egui::Sense::drag());
    let response = ui
        .with_layout(
            egui::Layout::centered_and_justified(egui::Direction::TopDown),
            |ui| ui.add(image),
        )
        .inner;

    let drag = response.drag_delta();
    scene.yaw -= drag.x * 0.01;
    scene.pitch = (scene.pitch + drag.y * 0.01).clamp(-1.4, 1.4);
    if response.hovered() {
        let scroll = ui.input(|i| i.smooth_scroll_delta.y);
        scene.dist = (scene.dist * (1.0 - scroll * 0.002)).clamp(1.2, 12.0);
    }
}

/// Per-frame scene sync: body transforms, trails, rod, grid, camera.
#[allow(clippy::too_many_arguments)]
pub fn sync_scene(
    ide: Res<Ide>,
    mut scene: ResMut<Scene3d>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut bodies: Query<(&Body3d, &mut Transform, &mut Visibility), Without<SceneCamera>>,
    mut camera: Query<(&mut Transform, &mut Camera), With<SceneCamera>>,
    mut gizmos: Gizmos,
) {
    let Ok((mut cam_transform, mut cam)) = camera.single_mut() else {
        return;
    };
    cam.clear_color = ClearColorConfig::Custom(match ide.settings.theme {
        Theme::Dark => Color::srgb(0.10, 0.11, 0.12),
        Theme::Light => Color::srgb(0.96, 0.96, 0.98),
    });

    let Some(data) = &ide.result else {
        for (_, _, mut visibility) in &mut bodies {
            *visibility = Visibility::Hidden;
        }
        return;
    };
    let groups = detect_bodies(&data.columns);
    if groups.is_empty() {
        for (_, _, mut visibility) in &mut bodies {
            *visibility = Visibility::Hidden;
        }
        return;
    }

    // Scene extent from all body coordinates (for camera and sizes).
    let mut extent = 0.0f64;
    for row in &data.rows {
        for g in &groups {
            extent = extent.max(row[g.x].abs()).max(row[g.y].abs());
            if let Some(z) = g.z {
                extent = extent.max(row[z].abs());
            }
        }
    }
    let extent = extent.max(1e-6) as f32;
    let radius = extent * 0.05;

    // Camera orbit.
    let dist = scene.dist * extent;
    let (yaw, pitch) = (scene.yaw, scene.pitch);
    let eye = Vec3::new(
        dist * pitch.cos() * yaw.sin(),
        dist * pitch.sin(),
        dist * pitch.cos() * yaw.cos(),
    );
    *cam_transform = Transform::from_translation(eye).looking_at(Vec3::ZERO, Vec3::Y);

    // Ensure one sphere entity per body.
    let sphere = scene
        .sphere
        .get_or_insert_with(|| meshes.add(Sphere::new(1.0)))
        .clone();
    while scene.spawned < groups.len() {
        let index = scene.spawned;
        let color = BODY_COLORS[index % BODY_COLORS.len()];
        commands.spawn((
            Mesh3d(sphere.clone()),
            MeshMaterial3d(materials.add(StandardMaterial {
                base_color: color,
                perceptual_roughness: 0.35,
                ..default()
            })),
            Transform::default(),
            Body3d(index),
        ));
        scene.spawned += 1;
    }

    // Current sample index from the shared animation clock.
    let idx = data
        .rows
        .partition_point(|row| row[0] < ide.anim.time)
        .min(data.rows.len() - 1);

    let position = |row: &Vec<f64>, g: &BodyCols| {
        Vec3::new(
            row[g.x] as f32,
            row[g.y] as f32,
            g.z.map(|z| row[z] as f32).unwrap_or(0.0),
        )
    };

    for (body, mut transform, mut visibility) in &mut bodies {
        match groups.get(body.0) {
            Some(g) => {
                *visibility = Visibility::Visible;
                transform.translation = position(&data.rows[idx], g);
                transform.scale = Vec3::splat(radius);
            }
            None => *visibility = Visibility::Hidden,
        }
    }

    // Gizmos: trails, the pendulum rod and a reference grid.
    if ide.view == ViewMode::ThreeD {
        let step = (idx / 2000).max(1);
        for (index, g) in groups.iter().enumerate() {
            let color = BODY_COLORS[index % BODY_COLORS.len()].with_alpha(0.6);
            gizmos.linestrip(
                data.rows[..=idx]
                    .iter()
                    .step_by(step)
                    .map(|r| position(r, g)),
                color,
            );
        }
        if ide.anim.rod && groups.len() == 1 {
            gizmos.line(
                Vec3::ZERO,
                position(&data.rows[idx], &groups[0]),
                Color::srgba(0.8, 0.8, 0.85, 0.9),
            );
        }
        gizmos.grid(
            Isometry3d::from_rotation(Quat::from_rotation_x(std::f32::consts::FRAC_PI_2)),
            UVec2::splat(10),
            Vec2::splat(extent * 0.4),
            Color::srgba(0.5, 0.5, 0.55, 0.15),
        );
    }
}

/// Whether the 3D tab has anything to show for this result.
pub fn has_bodies(data: &SimData) -> bool {
    !detect_bodies(&data.columns).is_empty()
}
