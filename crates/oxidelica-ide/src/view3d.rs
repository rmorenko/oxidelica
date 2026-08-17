//! The 3D trajectory view: a real Bevy scene rendered to an offscreen
//! texture and embedded into an egui panel.
//!
//! A model can declare what to draw by instantiating
//! `Oxidelica.Visualizers.Shape` components: the viewer reads their
//! kind, size and colour parameters and follows their x/y/z/phi
//! variables, so bodies appear with the right orientation.
//!
//! Without declared shapes, bodies fall back to being detected from
//! result columns by naming convention:
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

/// A shape the model asked the viewer to draw.
pub struct ShapeSpec {
    /// Instance path, used as the identity of the shape.
    pub prefix: String,
    /// 0 = box, 1 = sphere, 2 = cylinder.
    pub kind: u8,
    /// Extents: length along the shape axis, width, height.
    pub size: Vec3,
    /// Surface colour.
    pub color: Color,
    /// Result columns holding x, y, z and the rotation about z.
    pub columns: [usize; 4],
}

/// Find the `Visualizers.Shape` instances of a result: a component is
/// one when it has the parameters of a shape and the four columns a
/// shape drives. The model therefore declares what to draw, instead of
/// the viewer guessing from variable names.
pub fn detect_shapes(data: &SimData) -> Vec<ShapeSpec> {
    let parameter = |name: &str| {
        data.parameters
            .iter()
            .find(|(candidate, _)| candidate == name)
            .map(|(_, value)| *value)
    };
    let column = |name: &str| data.columns.iter().position(|candidate| candidate == name);

    let mut shapes = Vec::new();
    for (name, _) in &data.parameters {
        let Some(prefix) = name.strip_suffix(".kind") else {
            continue;
        };
        let (Some(kind), Some(length), Some(width), Some(height)) = (
            parameter(name),
            parameter(&format!("{prefix}.length")),
            parameter(&format!("{prefix}.width")),
            parameter(&format!("{prefix}.height")),
        ) else {
            continue;
        };
        let (Some(x), Some(y), Some(z), Some(phi)) = (
            column(&format!("{prefix}.x")),
            column(&format!("{prefix}.y")),
            column(&format!("{prefix}.z")),
            column(&format!("{prefix}.phi")),
        ) else {
            continue;
        };
        shapes.push(ShapeSpec {
            prefix: prefix.to_string(),
            kind: kind.max(0.0) as u8,
            size: Vec3::new(length as f32, width as f32, height as f32),
            color: Color::srgb(
                parameter(&format!("{prefix}.red")).unwrap_or(0.2) as f32,
                parameter(&format!("{prefix}.green")).unwrap_or(0.45) as f32,
                parameter(&format!("{prefix}.blue")).unwrap_or(0.94) as f32,
            ),
            columns: [x, y, z, phi],
        });
    }
    shapes.sort_by(|a, b| a.prefix.cmp(&b.prefix));
    shapes
}

/// Marker for a spawned shape entity, carrying its index.
#[derive(Component)]
pub struct ShapeEntity(pub usize);

/// The shape entities of the scene, excluding the camera and the
/// fallback bodies so their transforms stay disjoint.
type ShapeQuery<'w, 's> = Query<
    'w,
    's,
    (Entity, &'static ShapeEntity, &'static mut Transform),
    (Without<SceneCamera>, Without<Body3d>),
>;

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
    /// Identity of the shape set currently spawned, so the scene is
    /// rebuilt only when the model changes.
    shape_signature: String,
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
        shape_signature: String::new(),
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
    mut shape_entities: ShapeQuery,
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

    // A model that declares shapes has them drawn with position and
    // orientation; anything else falls back to a sphere per coordinate
    // pair.
    let shapes = detect_shapes(data);
    if !shapes.is_empty() {
        for (_, _, mut visibility) in &mut bodies {
            *visibility = Visibility::Hidden;
        }
        sync_shapes(
            &ide,
            data,
            &shapes,
            &mut scene,
            &mut commands,
            &mut meshes,
            &mut materials,
            &mut shape_entities,
            &mut cam_transform,
            &mut gizmos,
        );
        return;
    }

    let groups = detect_bodies(&data.columns);
    if groups.is_empty() {
        for (_, _, mut visibility) in &mut bodies {
            *visibility = Visibility::Hidden;
        }
        return;
    }

    // Robust scene extent: the 95th percentile of per-sample coordinate
    // magnitudes, so a single escaping body does not dwarf the rest.
    let stride = (data.rows.len() / 2000).max(1);
    let mut samples: Vec<f64> = data
        .rows
        .iter()
        .step_by(stride)
        .map(|row| {
            groups
                .iter()
                .map(|g| {
                    row[g.x]
                        .abs()
                        .max(row[g.y].abs())
                        .max(g.z.map(|z| row[z].abs()).unwrap_or(0.0))
                })
                .fold(0.0f64, f64::max)
        })
        .collect();
    samples.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let extent = samples
        .get(
            samples
                .len()
                .saturating_sub(1)
                .min(samples.len() * 95 / 100),
        )
        .copied()
        .unwrap_or(1.0)
        .max(1e-6) as f32;
    // Bodies are point masses, so the sphere size is decorative — except
    // when the model defines contact physics (kc/dc parameters by
    // convention): then the rendered radius is dc/2, and spheres touch
    // exactly when the contact force engages.
    let tuner_param = |name: &str| {
        ide.tuner
            .params
            .iter()
            .find(|entry| entry.name == name)
            .map(|entry| entry.value)
    };
    let radius = match (tuner_param("kc"), tuner_param("dc")) {
        (Some(kc), Some(dc)) if kc > 0.0 && dc > 0.0 => {
            ((dc / 2.0) as f32).clamp(extent * 0.005, extent * 0.2)
        }
        _ => extent * 0.035,
    };

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

/// Draw the declared shapes: spawn one entity per shape when the set
/// changes, then follow the animation clock with position and rotation.
#[allow(clippy::too_many_arguments)]
fn sync_shapes(
    ide: &Ide,
    data: &SimData,
    shapes: &[ShapeSpec],
    scene: &mut Scene3d,
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    entities: &mut ShapeQuery,
    camera: &mut Transform,
    gizmos: &mut Gizmos,
) {
    // Rebuild only when the model's shape set changes: sizes and colours
    // are baked into the meshes and materials.
    let signature: String = shapes
        .iter()
        .map(|shape| {
            format!(
                "{}:{}:{:.4}:{:.4}:{:.4};",
                shape.prefix, shape.kind, shape.size.x, shape.size.y, shape.size.z
            )
        })
        .collect();
    if signature != scene.shape_signature {
        for (entity, _, _) in entities.iter() {
            commands.entity(entity).despawn();
        }
        for (index, shape) in shapes.iter().enumerate() {
            let mesh = match shape.kind {
                1 => meshes.add(Sphere::new(shape.size.x.max(1e-4) * 0.5)),
                2 => meshes.add(Cylinder::new(
                    shape.size.y.max(1e-4) * 0.5,
                    shape.size.x.max(1e-4),
                )),
                _ => meshes.add(Cuboid::new(
                    shape.size.x.max(1e-4),
                    shape.size.y.max(1e-4),
                    shape.size.z.max(1e-4),
                )),
            };
            commands.spawn((
                Mesh3d(mesh),
                MeshMaterial3d(materials.add(StandardMaterial {
                    base_color: shape.color,
                    perceptual_roughness: 0.4,
                    ..default()
                })),
                Transform::default(),
                ShapeEntity(index),
            ));
        }
        scene.shape_signature = signature;
    }

    // Scene extent from every shape position, so the camera frames the
    // whole mechanism.
    let mut extent = 0.0f64;
    for row in &data.rows {
        for shape in shapes {
            for axis in 0..3 {
                extent = extent.max(row[shape.columns[axis]].abs());
            }
        }
    }
    let extent = (extent.max(1e-6) as f32)
        + shapes
            .iter()
            .map(|shape| shape.size.max_element())
            .fold(0.0f32, f32::max);

    let distance = scene.dist * extent;
    let (yaw, pitch) = (scene.yaw, scene.pitch);
    *camera = Transform::from_translation(Vec3::new(
        distance * pitch.cos() * yaw.sin(),
        distance * pitch.sin(),
        distance * pitch.cos() * yaw.cos(),
    ))
    .looking_at(Vec3::ZERO, Vec3::Y);

    let sample = data
        .rows
        .partition_point(|row| row[0] < ide.anim.time)
        .min(data.rows.len() - 1);
    let row = &data.rows[sample];

    for (_, marker, mut transform) in entities.iter_mut() {
        let Some(shape) = shapes.get(marker.0) else {
            continue;
        };
        let position = Vec3::new(
            row[shape.columns[0]] as f32,
            row[shape.columns[1]] as f32,
            row[shape.columns[2]] as f32,
        );
        let phi = row[shape.columns[3]] as f32;
        // A cylinder's axis is Y in Bevy, so it is turned onto the local
        // X axis first; boxes already extend along X.
        let extra = if shape.kind == 2 {
            Quat::from_rotation_z(-std::f32::consts::FRAC_PI_2)
        } else {
            Quat::IDENTITY
        };
        *transform = Transform {
            translation: position,
            rotation: Quat::from_rotation_z(phi) * extra,
            scale: Vec3::ONE,
        };
    }

    // Trails behind the round shapes, and a floor grid for scale.
    if ide.view == ViewMode::ThreeD {
        let step = (sample / 2000).max(1);
        for shape in shapes.iter().filter(|shape| shape.kind == 1) {
            gizmos.linestrip(
                data.rows[..=sample].iter().step_by(step).map(|row| {
                    Vec3::new(
                        row[shape.columns[0]] as f32,
                        row[shape.columns[1]] as f32,
                        row[shape.columns[2]] as f32,
                    )
                }),
                shape.color.with_alpha(0.55),
            );
        }
        gizmos.grid(
            Isometry3d::from_rotation(Quat::from_rotation_x(std::f32::consts::FRAC_PI_2)),
            UVec2::splat(10),
            Vec2::splat(extent * 0.3),
            Color::srgba(0.5, 0.5, 0.55, 0.15),
        );
    }
}

/// Whether the 3D tab has anything to show for this result.
pub fn has_bodies(data: &SimData) -> bool {
    !detect_shapes(data).is_empty() || !detect_bodies(&data.columns).is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Simulate the double pendulum example the way the IDE does.
    fn double_pendulum() -> SimData {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let library = std::fs::read_to_string(root.join("lib/Oxidelica.mo")).unwrap();
        let source = std::fs::read_to_string(root.join("examples/double_pendulum.mo")).unwrap();
        let model = oxidelica_parser::parse_model_with_libraries(&[library], &source).unwrap();
        let mut compiled = oxidelica_sim::compile(&model).unwrap();
        compiled.stop_time = 2.0;
        let result = compiled.simulate().unwrap();
        SimData {
            visible: vec![true; result.columns.len().saturating_sub(1)],
            columns: result.columns,
            rows: result.rows,
            parameters: result.parameters,
        }
    }

    #[test]
    fn declared_shapes_are_found_with_their_sizes_and_colours() {
        let data = double_pendulum();
        let shapes = detect_shapes(&data);
        let names: Vec<&str> = shapes.iter().map(|s| s.prefix.as_str()).collect();
        assert_eq!(names, vec!["elbow", "link1", "link2", "tip"]);

        let link1 = shapes.iter().find(|s| s.prefix == "link1").unwrap();
        assert_eq!(link1.kind, 0, "a rod is drawn as a box");
        assert!((link1.size.x - 0.6).abs() < 1e-9, "length from the model");
        let tip = shapes.iter().find(|s| s.prefix == "tip").unwrap();
        assert_eq!(tip.kind, 1, "a mass is drawn as a sphere");

        // The columns really point at the shape's own variables.
        let row = &data.rows[data.rows.len() / 2];
        let x1 = data.columns.iter().position(|c| c == "x1").unwrap();
        assert!(
            (row[link1.columns[0]] - 0.5 * row[x1]).abs() < 1e-9,
            "the rod sits at the midpoint of its link"
        );
    }

    #[test]
    fn shapes_take_precedence_over_the_naming_convention() {
        let data = double_pendulum();
        // The model also has x1/y1 and x2/y2, which the fallback would
        // pick up; declared shapes win.
        assert!(!detect_bodies(&data.columns).is_empty());
        assert!(has_bodies(&data));
        assert!(!detect_shapes(&data).is_empty());
    }
}
