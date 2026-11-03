//! Capability: simple scene primitives for examples.

use std::borrow::Cow;
use std::f32::consts::FRAC_PI_2;
use std::f32::consts::PI;
use std::sync::Mutex;
use std::sync::PoisonError;

use bevy::ecs::system::EntityCommands;
use bevy::light::NotShadowCaster;
use bevy::prelude::Alpha;
use bevy::prelude::AlphaMode;
use bevy::prelude::App;
use bevy::prelude::Assets;
use bevy::prelude::Bundle;
use bevy::prelude::Color;
use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::Cuboid;
use bevy::prelude::Entity;
use bevy::prelude::Mesh;
use bevy::prelude::Mesh3d;
use bevy::prelude::MeshMaterial3d;
use bevy::prelude::Meshable;
use bevy::prelude::Plane3d;
use bevy::prelude::Quat;
use bevy::prelude::ResMut;
use bevy::prelude::StandardMaterial;
use bevy::prelude::Startup;
use bevy::prelude::Transform;
use bevy::prelude::Vec3;
use bevy::prelude::default;
use bevy::prelude::warn;
use hana_diegetic::AlignX;
use hana_diegetic::AlignY;
use hana_diegetic::Anchor;
use hana_diegetic::ControlMetrics;
use hana_diegetic::DiegeticPanel;
use hana_diegetic::DiegeticPanelCommands;
use hana_diegetic::DiegeticText;
use hana_diegetic::El;
use hana_diegetic::GlyphShadowMode;
use hana_diegetic::LayoutBuilder;
use hana_diegetic::LayoutTree;
use hana_diegetic::Padding;
use hana_diegetic::PanelBuildError;
use hana_diegetic::Sizing;
use hana_diegetic::TextAlign;
use hana_diegetic::TextStyle;
use hana_diegetic::Unit;
use hana_diegetic::default_panel_material;

use crate::constants::CUBE_DEFAULT_COLOR;
use crate::constants::CUBE_DEFAULT_SIZE;
use crate::constants::CUBE_FACE_LABEL_SIZE;
use crate::constants::CUBE_FACE_PANEL_ACTIVE_BODY_SIZE;
use crate::constants::CUBE_FACE_PANEL_BLUE;
use crate::constants::CUBE_FACE_PANEL_BODY_SIZE;
use crate::constants::CUBE_FACE_PANEL_SIZE_FRACTION;
use crate::constants::CUBE_FACE_PANEL_TITLE_SIZE;
use crate::constants::FACE_TEXT_Z_OFFSET;
use crate::constants::GROUND_PLANE_ALPHA;
use crate::constants::GROUND_PLANE_DEFAULT_COLOR;
use crate::constants::GROUND_PLANE_DEFAULT_SIZE;
use crate::constants::GROUND_PLANE_METALLIC;
use crate::constants::GROUND_PLANE_REFLECTANCE;
use crate::constants::GROUND_PLANE_ROUGHNESS;

/// Names a single face of an axis-aligned cube.
///
/// Used by [`PrimitiveBuilder::face_text`](crate::PrimitiveBuilder::face_text)
/// and [`cube_face_text`] to place a centered [`DiegeticText`] label on one
/// face of a cube.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Face {
    /// +Z face.
    Front,
    /// -Z face.
    Back,
    /// -X face.
    Left,
    /// +X face.
    Right,
    /// +Y face.
    Top,
    /// -Y face.
    Bottom,
}

impl Face {
    /// Local transform for a label centered on this face of a cube whose
    /// half-extent (size / 2) is `half_extent`. The text is offset slightly
    /// outward to avoid z-fighting with the cube surface.
    fn local_transform(self, half_extent: f32) -> Transform {
        let face_pos = half_extent + FACE_TEXT_Z_OFFSET;
        match self {
            Self::Front => Transform::from_xyz(0.0, 0.0, face_pos),
            Self::Back => {
                Transform::from_xyz(0.0, 0.0, -face_pos).with_rotation(Quat::from_rotation_y(PI))
            },
            Self::Right => Transform::from_xyz(face_pos, 0.0, 0.0)
                .with_rotation(Quat::from_rotation_y(FRAC_PI_2)),
            Self::Left => Transform::from_xyz(-face_pos, 0.0, 0.0)
                .with_rotation(Quat::from_rotation_y(-FRAC_PI_2)),
            Self::Top => Transform::from_xyz(0.0, face_pos, 0.0)
                .with_rotation(Quat::from_rotation_x(-FRAC_PI_2)),
            Self::Bottom => Transform::from_xyz(0.0, -face_pos, 0.0)
                .with_rotation(Quat::from_rotation_x(FRAC_PI_2)),
        }
    }
}

/// Activity state for cube face panel content.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CubeFacePanelActivity {
    /// Render with the active body size.
    Active,
    /// Render with the idle body size.
    Idle,
}

/// Text content for a canonical cube face panel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CubeFacePanelContent {
    /// Panel title.
    pub title:    Cow<'static, str>,
    /// Body rows.
    pub rows:     Vec<Cow<'static, str>>,
    /// Activity state.
    pub activity: CubeFacePanelActivity,
}

impl CubeFacePanelContent {
    /// Creates idle panel content.
    #[must_use]
    pub fn idle(
        title: impl Into<Cow<'static, str>>,
        rows: impl IntoIterator<Item = impl Into<Cow<'static, str>>>,
    ) -> Self {
        Self {
            title:    title.into(),
            rows:     rows.into_iter().map(Into::into).collect(),
            activity: CubeFacePanelActivity::Idle,
        }
    }

    /// Creates active panel content.
    #[must_use]
    pub fn active(
        title: impl Into<Cow<'static, str>>,
        rows: impl IntoIterator<Item = impl Into<Cow<'static, str>>>,
    ) -> Self {
        Self {
            title:    title.into(),
            rows:     rows.into_iter().map(Into::into).collect(),
            activity: CubeFacePanelActivity::Active,
        }
    }
}

/// How a cube face spaces its content.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CubeFaceSpacing {
    /// Padding and row gap follow `body_size` through the `ControlMetrics` spacing scale.
    FollowsBodySize,
    /// Fixed padding and row gap, in meters.
    Fixed {
        /// Panel inner padding in meters.
        padding: f32,
        /// Body row gap in meters.
        row_gap: f32,
    },
}

/// Visual sizing for canonical cube face panels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CubeFacePanelStyle {
    /// Square panel edge length.
    pub size:             f32,
    /// How panel padding and body row gap are resolved.
    pub spacing:          CubeFaceSpacing,
    /// Title text size.
    pub title_size:       f32,
    /// Idle body text size.
    pub body_size:        f32,
    /// Active body text size.
    pub active_body_size: f32,
    /// Text color.
    pub color:            Color,
}

impl CubeFacePanelStyle {
    /// Canonical panel style scaled for a cube of `cube_size`.
    #[must_use]
    pub const fn for_cube(cube_size: f32) -> Self {
        Self {
            size:             cube_size * CUBE_FACE_PANEL_SIZE_FRACTION,
            spacing:          CubeFaceSpacing::FollowsBodySize,
            title_size:       CUBE_FACE_PANEL_TITLE_SIZE,
            body_size:        CUBE_FACE_PANEL_BODY_SIZE,
            active_body_size: CUBE_FACE_PANEL_ACTIVE_BODY_SIZE,
            color:            CUBE_FACE_PANEL_BLUE,
        }
    }

    /// Resolves panel inner padding in meters.
    #[must_use]
    pub fn padding(&self) -> f32 {
        match self.spacing {
            CubeFaceSpacing::FollowsBodySize => {
                ControlMetrics::from_label_size(self.body_size * 0.001)
                    .spacing
                    .panel_padding
            },
            CubeFaceSpacing::Fixed { padding, .. } => padding,
        }
    }

    /// Resolves body row gap in meters.
    #[must_use]
    pub fn row_gap(&self) -> f32 {
        match self.spacing {
            CubeFaceSpacing::FollowsBodySize => {
                ControlMetrics::from_label_size(self.body_size * 0.001)
                    .spacing
                    .row_gap
            },
            CubeFaceSpacing::Fixed { row_gap, .. } => row_gap,
        }
    }
}

/// Mesh kind spawned by [`crate::SprinkleBuilder`] scene helpers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PrimitiveKind {
    /// A square ground plane in the XZ plane.
    GroundPlane,
    /// A cube centered on its transform.
    Cube,
}

/// Identity marker inserted on cubes spawned by Fairy Dust primitive builders.
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct FairyDustCube;

/// Marker inserted on the per-face [`DiegeticText`] labels of a cube.
///
/// Lets a caller retarget only the cube's face labels without also matching
/// panel-child or other text. The label's editable string is stored in a
/// `TextContent` run child, so address it through `DiegeticTextMut<M>` keyed on
/// this marker rather than a direct `TextContent` query:
///
/// ```ignore
/// fn relabel_faces(mut labels: DiegeticTextMut<CubeFaceLabel>) {
///     labels.set("Orthographic");
/// }
/// ```
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct CubeFaceLabel;

#[derive(Clone, Debug)]
pub(crate) struct FaceTextSpec {
    pub(crate) face:      Face,
    pub(crate) text:      String,
    pub(crate) text_size: f32,
    pub(crate) color:     Color,
}

/// Configuration shared by all simple scene primitives.
#[derive(Clone, Debug)]
pub(crate) struct PrimitiveConfig {
    kind:       PrimitiveKind,
    size:       f32,
    color:      Color,
    material:   Option<StandardMaterial>,
    transform:  Option<Transform>,
    face_texts: Vec<FaceTextSpec>,
}

impl PrimitiveConfig {
    pub(crate) const fn ground_plane() -> Self {
        Self {
            kind:       PrimitiveKind::GroundPlane,
            size:       GROUND_PLANE_DEFAULT_SIZE,
            color:      GROUND_PLANE_DEFAULT_COLOR,
            material:   None,
            transform:  None,
            face_texts: Vec::new(),
        }
    }

    pub(crate) const fn cube() -> Self {
        Self {
            kind:       PrimitiveKind::Cube,
            size:       CUBE_DEFAULT_SIZE,
            color:      CUBE_DEFAULT_COLOR,
            material:   None,
            transform:  None,
            face_texts: Vec::new(),
        }
    }

    pub(crate) const fn set_size(&mut self, size: f32) { self.size = size; }

    pub(crate) const fn set_color(&mut self, color: Color) { self.color = color; }

    pub(crate) fn with_material(mut self, material: StandardMaterial) -> Self {
        self.material = Some(material);
        self
    }

    pub(crate) const fn set_transform(&mut self, transform: Transform) {
        self.transform = Some(transform);
    }

    pub(crate) fn push_face_text(&mut self, spec: FaceTextSpec) { self.face_texts.push(spec); }
}

/// Boxed deferred-insert closure applied to a primitive entity at spawn time.
type PrimitiveInsert = Box<dyn FnOnce(&mut EntityCommands) + Send + Sync>;

/// Bundle that renders a single line of world-space [`DiegeticText`] centered on
/// one face of a cube. Spawn as a child of the cube entity (or independently
/// using the cube's transform as a parent).
///
/// Use this when you spawn a cube manually with `commands.spawn`. For cubes
/// built through [`PrimitiveBuilder`](crate::PrimitiveBuilder), prefer
/// [`PrimitiveBuilder::face_text`](crate::PrimitiveBuilder::face_text).
#[must_use]
pub fn cube_face_text(
    face: Face,
    text: impl Into<String>,
    cube_size: f32,
    text_size: f32,
    color: Color,
) -> impl Bundle {
    (
        CubeFaceLabel,
        DiegeticText::world(text)
            .size(text_size)
            .color(color)
            .transform(face.local_transform(cube_size * 0.5))
            .build(),
    )
}

/// Canonical single-line blue label centered on a cube face.
#[must_use]
pub fn cube_face_label(face: Face, text: impl Into<String>, cube_size: f32) -> impl Bundle {
    cube_face_text(
        face,
        text,
        cube_size,
        CUBE_FACE_LABEL_SIZE,
        CUBE_FACE_PANEL_BLUE,
    )
}

/// Local transform for a cube-mounted panel centered on `face`.
#[must_use]
pub fn cube_face_transform(face: Face, cube_size: f32) -> Transform {
    face.local_transform(cube_size * 0.5)
}

/// Transparent unlit material for cube-mounted face panels.
#[must_use]
pub fn cube_face_panel_material() -> StandardMaterial {
    StandardMaterial {
        base_color: Color::NONE,
        alpha_mode: AlphaMode::Blend,
        unlit: true,
        ..default_panel_material()
    }
}

/// Builds a cube face panel with the canonical transparent material and tree.
///
/// # Errors
///
/// Returns [`PanelBuildError`] when `style.size` is not a positive, finite value.
pub fn cube_face_panel(
    style: CubeFacePanelStyle,
    content: CubeFacePanelContent,
    materials: &mut Assets<StandardMaterial>,
) -> Result<DiegeticPanel, PanelBuildError> {
    cube_face_panel_with_tree(style.size, cube_face_panel_tree(style, content), materials)
}

/// Builds a transparent cube face panel from a caller-authored layout tree.
///
/// # Errors
///
/// Returns [`PanelBuildError`] when `size` is not a positive, finite value.
pub fn cube_face_panel_with_tree(
    size: f32,
    tree: LayoutTree,
    materials: &mut Assets<StandardMaterial>,
) -> Result<DiegeticPanel, PanelBuildError> {
    let transparent = materials.add(cube_face_panel_material());
    DiegeticPanel::world()
        .size(size, size)
        .font_unit(Unit::Millimeters)
        .anchor(Anchor::Center)
        .material(transparent.clone())
        .text_material(transparent)
        .with_tree(tree)
        .build()
}

/// Builds the layout tree for a cube face panel.
#[must_use]
pub fn cube_face_panel_tree(
    style: CubeFacePanelStyle,
    content: CubeFacePanelContent,
) -> LayoutTree {
    let spacing = ControlMetrics::from_label_size(style.body_size * 0.001).spacing;
    let mut builder = LayoutBuilder::with_root(
        El::column()
            .width(Sizing::fixed(style.size))
            .height(Sizing::fixed(style.size))
            .alignment(AlignX::Center, AlignY::Center)
            .gap(spacing.title_gap)
            .padding(Padding::all(style.padding()))
            .clip(),
    );

    let title = TextStyle::new(style.title_size)
        .with_color(style.color)
        .with_align(TextAlign::Center)
        .with_shadow_mode(GlyphShadowMode::None);
    builder.text((content.title, title));

    let body_size = match content.activity {
        CubeFacePanelActivity::Active => style.active_body_size,
        CubeFacePanelActivity::Idle => style.body_size,
    };
    let body = TextStyle::new(body_size)
        .with_color(style.color)
        .with_align(TextAlign::Center)
        .with_shadow_mode(GlyphShadowMode::None);
    builder.with(
        El::column()
            .width(Sizing::GROW)
            .height(Sizing::FIT)
            .gap(style.row_gap()),
        |builder| {
            for row in content.rows {
                builder.text((row, body.clone()));
            }
        },
    );

    builder.build()
}

/// Replaces a cube face panel's layout tree.
pub fn set_cube_face_panel_tree(commands: &mut Commands, entity: Entity, tree: LayoutTree) {
    if let Err(error) = commands.set_tree(entity, tree) {
        warn!("failed to replace cube-face panel {entity:?} tree: {error}");
    }
}

pub(crate) fn install(app: &mut App, config: PrimitiveConfig, inserts: Vec<PrimitiveInsert>) {
    let inserts_cell: Mutex<Option<Vec<PrimitiveInsert>>> = Mutex::new(Some(inserts));
    app.add_systems(
        Startup,
        move |mut commands: Commands,
              mut meshes: ResMut<Assets<Mesh>>,
              mut materials: ResMut<Assets<StandardMaterial>>| {
            let inserts = inserts_cell
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .take()
                .unwrap_or_default();
            spawn_primitive(
                &mut commands,
                &mut meshes,
                &mut materials,
                config.clone(),
                inserts,
            );
        },
    );
}

fn spawn_primitive(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    config: PrimitiveConfig,
    inserts: Vec<PrimitiveInsert>,
) {
    let mesh = match config.kind {
        PrimitiveKind::GroundPlane => {
            Mesh::from(Plane3d::default().mesh().size(config.size, config.size))
        },
        PrimitiveKind::Cube => Mesh::from(Cuboid::from_size(Vec3::splat(config.size))),
    };
    let material = config
        .material
        .unwrap_or_else(|| default_material(config.kind, config.color));
    let transform = config.transform.unwrap_or_else(|| match config.kind {
        PrimitiveKind::GroundPlane => Transform::default(),
        PrimitiveKind::Cube => Transform::from_xyz(0.0, config.size * 0.5, 0.0),
    });

    let cube_size = config.size;
    let face_texts = config.face_texts;
    let mut entity = commands.spawn((
        Mesh3d(meshes.add(mesh)),
        MeshMaterial3d(materials.add(material)),
        transform,
    ));
    match config.kind {
        PrimitiveKind::GroundPlane => {
            // The floor still receives object shadows without casting its own
            // depth-map shadow back onto itself across the studio cascades.
            entity.insert(NotShadowCaster);
        },
        PrimitiveKind::Cube => {
            entity.insert(FairyDustCube);
        },
    }
    for insert in inserts {
        insert(&mut entity);
    }
    if !face_texts.is_empty() {
        entity.with_children(|parent| {
            for spec in face_texts {
                parent.spawn(cube_face_text(
                    spec.face,
                    spec.text,
                    cube_size,
                    spec.text_size,
                    spec.color,
                ));
            }
        });
    }
}

fn default_material(kind: PrimitiveKind, color: Color) -> StandardMaterial {
    match kind {
        PrimitiveKind::GroundPlane => StandardMaterial {
            base_color: color.with_alpha(GROUND_PLANE_ALPHA),
            alpha_mode: AlphaMode::Blend,
            double_sided: true,
            cull_mode: None,
            metallic: GROUND_PLANE_METALLIC,
            reflectance: GROUND_PLANE_REFLECTANCE,
            perceptual_roughness: GROUND_PLANE_ROUGHNESS,
            ..default()
        },
        PrimitiveKind::Cube => StandardMaterial {
            base_color: color,
            ..default()
        },
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::panic,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use bevy::light::NotShadowCaster;
    use bevy::prelude::Assets;
    use bevy::prelude::Mesh;
    use bevy::prelude::MinimalPlugins;
    use bevy::prelude::StandardMaterial;
    use hana_diegetic::ComputedDiegeticPanel;
    use hana_diegetic::ControlMetrics;
    use hana_diegetic::DiegeticPanel;
    use hana_diegetic::DiegeticTextMeasurer;
    use hana_diegetic::HeadlessLayoutPlugin;

    use super::CubeFacePanelContent;
    use super::CubeFacePanelStyle;
    use super::CubeFaceSpacing;
    use super::PrimitiveConfig;
    use super::cube_face_panel_tree;
    use super::install;
    use crate::constants::CUBE_FACE_PANEL_BODY_SIZE;

    #[test]
    fn ground_does_not_cast_its_own_shadow() {
        let mut app = crate::test_app();
        app.add_plugins(MinimalPlugins)
            .insert_resource(Assets::<Mesh>::default())
            .insert_resource(Assets::<StandardMaterial>::default());
        install(&mut app, PrimitiveConfig::ground_plane(), Vec::new());
        install(&mut app, PrimitiveConfig::cube(), Vec::new());
        app.update();

        let world = app.world_mut();
        let ground_count = world.query::<&NotShadowCaster>().iter(world).count();
        assert_eq!(ground_count, 1);
    }

    #[test]
    fn cube_face_insets_and_row_gap_follow_body_text_scale() {
        let style = CubeFacePanelStyle::for_cube(1.0);
        let spacing = ControlMetrics::from_label_size(CUBE_FACE_PANEL_BODY_SIZE * 0.001).spacing;
        assert!((style.padding() - spacing.panel_padding).abs() < 1e-6);
        assert!((style.row_gap() - spacing.row_gap).abs() < 1e-6);
        assert!(style.row_gap() < style.padding());
        assert!(spacing.title_gap < style.padding());
    }

    #[test]
    fn cube_face_spacing_accessors_follow_overridden_body_size() {
        let style = CubeFacePanelStyle {
            body_size: 66.0,
            ..CubeFacePanelStyle::for_cube(1.0)
        };
        let spacing = ControlMetrics::from_label_size(style.body_size * 0.001).spacing;
        assert!((style.padding() - spacing.panel_padding).abs() < 1e-6);
        assert!((style.row_gap() - spacing.row_gap).abs() < 1e-6);
    }

    #[test]
    fn cube_face_tree_scales_padding_and_gaps_with_overridden_body_size() {
        let mut style = CubeFacePanelStyle::for_cube(1.0);
        style.body_size = 66.0;
        let spacing = ControlMetrics::from_label_size(style.body_size * 0.001).spacing;
        let tree = cube_face_panel_tree(
            style,
            CubeFacePanelContent::active("Title", ["First", "Second"]),
        );
        let [title, body] = tree.children_of(0) else {
            panic!("face should contain title and body");
        };
        let (title, body) = (*title, *body);
        let [first, second] = tree.children_of(body) else {
            panic!("body should contain two rows");
        };
        let (first, second) = (*first, *second);
        let mut app = crate::test_app();
        app.add_plugins(MinimalPlugins)
            .insert_resource(DiegeticTextMeasurer::default())
            .add_plugins(HeadlessLayoutPlugin);
        let panel = app
            .world_mut()
            .spawn(
                DiegeticPanel::world()
                    .size(style.size, style.size)
                    .with_tree(tree)
                    .build()
                    .expect("face panel should build"),
            )
            .id();
        app.update();
        let solved = app
            .world()
            .get::<ComputedDiegeticPanel>(panel)
            .and_then(|computed| computed.layout().solved())
            .expect("face panel should solve");
        let root = solved.computed[0].bounds;
        let title = solved.computed[title].bounds;
        let body = solved.computed[body].bounds;
        let first = solved.computed[first].bounds;
        let second = solved.computed[second].bounds;
        let points_per_meter = root.width / style.size;
        let close = |actual: f32, expected: f32| {
            assert!((actual - expected).abs() < 0.5, "{actual} != {expected}");
        };
        close(body.x - root.x, spacing.panel_padding * points_per_meter);
        close(
            body.y - title.y - title.height,
            spacing.title_gap * points_per_meter,
        );
        close(
            second.y - first.y - first.height,
            spacing.row_gap * points_per_meter,
        );
    }

    #[test]
    fn cube_face_tree_honors_fixed_spacing_equal_to_body_size_defaults() {
        let style = CubeFacePanelStyle {
            body_size: 66.0,
            spacing: CubeFaceSpacing::Fixed {
                padding: 0.066,
                row_gap: 0.022,
            },
            ..CubeFacePanelStyle::for_cube(1.0)
        };
        let tree = cube_face_panel_tree(
            style,
            CubeFacePanelContent::idle("Title", ["First", "Second"]),
        );
        let [_, body] = tree.children_of(0) else {
            panic!("face should contain title and body");
        };
        let body = *body;
        let [first, second] = tree.children_of(body) else {
            panic!("body should contain two rows");
        };
        let (first, second) = (*first, *second);
        let mut app = crate::test_app();
        app.add_plugins(MinimalPlugins)
            .insert_resource(DiegeticTextMeasurer::default())
            .add_plugins(HeadlessLayoutPlugin);
        let panel = app
            .world_mut()
            .spawn(
                DiegeticPanel::world()
                    .size(style.size, style.size)
                    .with_tree(tree)
                    .build()
                    .expect("face panel should build"),
            )
            .id();
        app.update();
        let solved = app
            .world()
            .get::<ComputedDiegeticPanel>(panel)
            .and_then(|computed| computed.layout().solved())
            .expect("face panel should solve");
        let root = solved.computed[0].bounds;
        let body = solved.computed[body].bounds;
        let first = solved.computed[first].bounds;
        let second = solved.computed[second].bounds;
        let points_per_meter = root.width / style.size;
        let close = |actual: f32, expected: f32| {
            assert!((actual - expected).abs() < 0.5, "{actual} != {expected}");
        };
        close(body.x - root.x, 0.066 * points_per_meter);
        close(second.y - first.y - first.height, 0.022 * points_per_meter);
    }
}

#[cfg(test)]
mod floor_pixel_tests {
    use std::sync::mpsc;

    use bevy::camera::Hdr;
    use bevy::camera::RenderTarget;
    use bevy::prelude::*;
    use bevy::render::RenderPlugin;
    use bevy::render::render_resource::PollType;
    use bevy::render::render_resource::TextureFormat;
    use bevy::render::renderer::RenderDevice;
    use bevy::render::view::Tonemapping;
    use bevy::render::view::screenshot::Screenshot;
    use bevy::render::view::screenshot::ScreenshotCaptured;
    use bevy::window::ExitCondition;
    use bevy::winit::WinitPlugin;

    use super::*;
    use crate::lighting;

    const SIZE: u32 = 256;

    #[test]
    fn studio_floor_draws_without_local_blue_streak() -> Result<(), String> {
        let mut app = crate::test_app();
        app.add_plugins(
            DefaultPlugins
                .set(WindowPlugin {
                    primary_window: None,
                    exit_condition: ExitCondition::DontExit,
                    ..default()
                })
                .set(RenderPlugin {
                    synchronous_pipeline_compilation: true,
                    ..default()
                })
                .disable::<WinitPlugin>(),
        );
        lighting::install(&mut app, lighting::StudioLightingConfig::default());
        let mut ground = PrimitiveConfig::ground_plane();
        ground.set_size(100.0);
        install(&mut app, ground, Vec::new());
        let target =
            app.world_mut()
                .resource_mut::<Assets<Image>>()
                .add(Image::new_target_texture(
                    SIZE,
                    SIZE,
                    TextureFormat::Rgba8UnormSrgb,
                    None,
                ));
        app.world_mut().spawn((
            Camera3d::default(),
            RenderTarget::Image(target.clone().into()),
            Hdr,
            Tonemapping::KhronosPbrNeutral,
            Msaa::Off,
            Transform::from_xyz(0.0, 10.0, 0.0).looking_at(Vec3::ZERO, Vec3::Z),
        ));
        app.finish();
        app.cleanup();
        for _ in 0..12 {
            app.update();
        }
        let (sender, receiver) = mpsc::channel();
        app.world_mut().spawn(Screenshot::image(target)).observe(
            move |event: On<ScreenshotCaptured>| {
                let result = event
                    .image
                    .data
                    .as_ref()
                    .ok_or("screenshot has no pixels")
                    .map(|pixels| {
                        let blue =
                            (32..224).map(|x| pixels[((SIZE / 2 * SIZE + x) * 4 + 2) as usize]);
                        let minimum = blue.clone().min().unwrap_or_default();
                        let maximum = blue.max().unwrap_or_default();
                        let center = pixels[((SIZE / 2 * SIZE + SIZE / 2) * 4 + 2) as usize];
                        (minimum, maximum, center)
                    });
                let _ = sender.send(result);
            },
        );
        // Bevy finishes the readback on its async compute pool, which shader
        // compilation can hold for many frames, so this waits on the capture itself.
        loop {
            app.update();
            app.world()
                .resource::<RenderDevice>()
                .wgpu_device()
                .poll(PollType::Wait {
                    submission_index: None,
                    timeout:          None,
                })
                .map_err(|error| format!("GPU screenshot readback failed: {error}"))?;
            if let Ok(sample) = receiver.try_recv() {
                let (minimum, maximum, center) = sample.map_err(str::to_string)?;
                assert!(center > 20, "floor pixel was not drawn: {center}");
                assert!(
                    maximum.abs_diff(minimum) <= 5,
                    "blue floor streak: {minimum}..{maximum}"
                );
                return Ok(());
            }
        }
    }
}
