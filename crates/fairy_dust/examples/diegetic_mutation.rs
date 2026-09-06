//! Runtime text mutation across all four diegetic flavors, side by side.
//!
//! Each flavor shows the exact call that retexts it once per second:
//!   - `DiegeticText::world` / `DiegeticText::screen` (standalone labels) mutate through
//!     [`DiegeticTextMut<M>`], keyed on a marker component — one call, `labels.set(text)`,
//!     regardless of coordinate space.
//!   - `DiegeticPanel::world` / `DiegeticPanel::screen` (panels with a named field) mutate through
//!     [`PanelText::set_text`], keyed on a [`PanelElementId`] for a named run in a panel tree.
//!
//! Both APIs write the same `TextContent` storage: standalone labels address it
//! through `DiegeticTextMut<M>`, while panel fields address it through
//! `PanelText` and `PanelElementId`.

use bevy::prelude::Assets;
use bevy::prelude::Color;
use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::DetectChanges;
use bevy::prelude::Entity;
use bevy::prelude::Handle;
use bevy::prelude::IntoScheduleConfigs;
use bevy::prelude::Query;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::Resource;
use bevy::prelude::StandardMaterial;
use bevy::prelude::Startup;
use bevy::prelude::Time;
use bevy::prelude::Transform;
use bevy::prelude::Update;
use bevy::prelude::Vec2;
use bevy::prelude::Vec3;
use bevy::prelude::With;
use bevy::prelude::error;
use fairy_dust::DEFAULT_PANEL_BACKGROUND;
use hana_diegetic::Anchor;
use hana_diegetic::CornerRadius;
use hana_diegetic::DiegeticPanel;
use hana_diegetic::DiegeticText;
use hana_diegetic::DiegeticTextMut;
use hana_diegetic::El;
use hana_diegetic::Fit;
use hana_diegetic::LayoutBuilder;
use hana_diegetic::LayoutTree;
use hana_diegetic::Padding;
use hana_diegetic::PanelElementId;
use hana_diegetic::PanelText;
use hana_diegetic::Px;
use hana_diegetic::Sizing;
use hana_diegetic::Text;
use hana_diegetic::TextStyle;
use hana_diegetic::Unit;
use hana_diegetic::default_panel_material;
use hana_lagrange::OrbitCamPreset;

// ═════════════════════════════════════════════════════════════════════════════
// RUNTIME TEXT MUTATION — DiegeticTextMut<M> for markers, PanelText for ids.
// ═════════════════════════════════════════════════════════════════════════════
//
// `setup` spawns one label and one panel in each space. Every frame
// `advance_tick` rounds the elapsed time to whole seconds and only bumps
// the `Tick` resource when that integer changes, so a retext (and its relayout)
// happens once per second, not every frame. Each mutator gates on
// `tick.is_changed()` and writes its flavor's text: the two standalone labels
// through `DiegeticTextMut<M>::set` (marker-addressed), the two panels through
// `PanelText::set_text` with a `PanelElementId` (id-addressed).

const HOME_YAW: f32 = 0.4;
const HOME_PITCH: f32 = 0.3;

const TEXT_COLOR: Color = Color::srgb(0.92, 0.92, 0.94);

/// Shared element id for the single counter run inside each panel's tree. The
/// panel's structure is fixed, so the run is named once at build time and
/// retexted in place rather than rebuilt.
const COUNTER_FIELD: &str = "counter";

// World-space placement and sizing (meters).
const WORLD_TEXT_POS: Vec3 = Vec3::new(-0.7, 0.7, 0.0);
const WORLD_PANEL_POS: Vec3 = Vec3::new(0.7, 0.7, 0.0);
const WORLD_TEXT_SIZE_M: f32 = 0.12;
const WORLD_PANEL_TEXT_M: f32 = 0.1;
const WORLD_PANEL_PADDING_M: f32 = 0.03;
const WORLD_PANEL_RADIUS_M: f32 = 0.015;

// Screen-space placement and sizing (pixels).
const SCREEN_TEXT_POS: Vec2 = Vec2::new(40.0, 40.0);
const SCREEN_TEXT_SIZE_PX: f32 = 28.0;
const SCREEN_PANEL_TEXT_PX: f32 = 28.0;
const SCREEN_PANEL_PADDING_PX: Px = Px(12.0);
const SCREEN_PANEL_RADIUS_PX: Px = Px(10.0);

/// Marker on the world-space standalone label, addressed by `DiegeticTextMut`.
#[derive(Component)]
struct WorldLabel;

/// Marker on the screen-space standalone label, addressed by `DiegeticTextMut`.
#[derive(Component)]
struct ScreenLabel;

/// Marker on the world-space panel, located by `PanelText` then addressed by id.
#[derive(Component)]
struct WorldPanel;

/// Marker on the screen-space panel, located by `PanelText` then addressed by id.
#[derive(Component)]
struct ScreenPanel;

/// Whole-second counter. Only changes when the integer second advances, so each
/// mutator's `is_changed` gate retexts once per second.
#[derive(Resource, Default)]
struct Tick(u64);

fn main() {
    // `hana_diegetic::DiegeticUiPlugin` is registered automatically by
    // `fairy_dust::sprinkle_example`. `advance_tick` runs first in the chain so
    // the four mutators read the second-counter change in the same frame.
    fairy_dust::sprinkle_example()
        .with_studio_lighting()
        .with_ground_plane()
        .with_orbit_cam_preset(|_| {}, OrbitCamPreset::blender_like())
        .with_camera_home()
        .yaw(HOME_YAW)
        .pitch(HOME_PITCH)
        .insert_resource(Tick::default())
        .add_systems(Startup, setup)
        .add_systems(
            Update,
            (
                advance_tick,
                mutate_world_text,
                mutate_screen_text,
                mutate_world_panel,
                mutate_screen_panel,
            )
                .chain(),
        )
        .run();
}

fn setup(mut commands: Commands, mut materials: ResMut<Assets<StandardMaterial>>) {
    // Standalone labels: a marker plus the one-element `DiegeticText` bundle.
    commands.spawn((
        WorldLabel,
        DiegeticText::world(world_text_label(0))
            .size(WORLD_TEXT_SIZE_M)
            .color(TEXT_COLOR)
            .transform(Transform::from_translation(WORLD_TEXT_POS))
            .build(),
    ));
    commands.spawn((
        ScreenLabel,
        DiegeticText::screen(screen_text_label(0))
            .size(SCREEN_TEXT_SIZE_PX)
            .color(TEXT_COLOR)
            .screen_position(SCREEN_TEXT_POS.x, SCREEN_TEXT_POS.y)
            .build(),
    ));

    // Panels: a marker plus a built `DiegeticPanel` whose tree names one run.
    let panel_material = materials.add(panel_surface());
    match world_panel(panel_material.clone()) {
        Ok(panel) => {
            commands.spawn((
                WorldPanel,
                panel,
                Transform::from_translation(WORLD_PANEL_POS),
            ));
        },
        Err(error) => error!("diegetic_mutation: world panel build failed: {error}"),
    }
    match screen_panel(panel_material) {
        Ok(panel) => {
            commands.spawn((ScreenPanel, panel, Transform::default()));
        },
        Err(error) => error!("diegetic_mutation: screen panel build failed: {error}"),
    }
}

/// Bumps the counter only when the elapsed whole-second changes.
fn advance_tick(time: Res<Time>, mut tick: ResMut<Tick>) {
    let secs = time.elapsed().as_secs();
    if secs != tick.0 {
        tick.0 = secs;
    }
}

/// World-space standalone label → `DiegeticTextMut<WorldLabel>::set`.
fn mutate_world_text(tick: Res<Tick>, mut labels: DiegeticTextMut<WorldLabel>) {
    if !tick.is_changed() {
        return;
    }
    labels.set(world_text_label(tick.0));
}

/// Screen-space standalone label → `DiegeticTextMut<ScreenLabel>::set`. Same one
/// call as the world label; only the constructor at spawn differs.
fn mutate_screen_text(tick: Res<Tick>, mut labels: DiegeticTextMut<ScreenLabel>) {
    if !tick.is_changed() {
        return;
    }
    labels.set(screen_text_label(tick.0));
}

/// World-space panel → `PanelText::set_text` on the named counter run.
fn mutate_world_panel(
    tick: Res<Tick>,
    panels: Query<Entity, With<WorldPanel>>,
    mut panel_text: PanelText,
) {
    if !tick.is_changed() {
        return;
    }
    let text = world_panel_label(tick.0);
    for panel in &panels {
        panel_text.set_text(panel, &PanelElementId::named(COUNTER_FIELD), text.as_str());
    }
}

/// Screen-space panel → `PanelText::set_text` on the named counter run. Same
/// id-addressed call as the world panel.
fn mutate_screen_panel(
    tick: Res<Tick>,
    panels: Query<Entity, With<ScreenPanel>>,
    mut panel_text: PanelText,
) {
    if !tick.is_changed() {
        return;
    }
    let text = screen_panel_label(tick.0);
    for panel in &panels {
        panel_text.set_text(panel, &PanelElementId::named(COUNTER_FIELD), text.as_str());
    }
}

fn world_text_label(n: u64) -> String { format!("world text {n}") }
fn screen_text_label(n: u64) -> String { format!("screen text {n}") }
fn world_panel_label(n: u64) -> String { format!("world panel {n}") }
fn screen_panel_label(n: u64) -> String { format!("screen panel {n}") }

/// World panel: a `Fit` surface in meters with one named, centered counter run.
fn world_panel(
    material: Handle<StandardMaterial>,
) -> Result<DiegeticPanel, hana_diegetic::PanelBuildError> {
    DiegeticPanel::world()
        .size(Fit, Fit)
        .font_unit(Unit::Meters)
        .anchor(Anchor::Center)
        .material(material.clone())
        .text_material(material)
        .with_tree(world_panel_tree(0))
        .build()
}

/// Screen panel: a `Fit` overlay surface, bottom-right, with one named run.
fn screen_panel(
    material: Handle<StandardMaterial>,
) -> Result<DiegeticPanel, hana_diegetic::PanelBuildError> {
    DiegeticPanel::screen()
        .size(Fit, Fit)
        .anchor(Anchor::BottomRight)
        .material(material.clone())
        .text_material(material)
        .with_tree(screen_panel_tree(0))
        .build()
}

fn world_panel_tree(n: u64) -> LayoutTree {
    let mut builder = LayoutBuilder::with_root(
        El::new()
            .width(Sizing::FIT)
            .height(Sizing::FIT)
            .padding(Padding::all(WORLD_PANEL_PADDING_M))
            .corner_radius(CornerRadius::all(WORLD_PANEL_RADIUS_M))
            .background(DEFAULT_PANEL_BACKGROUND),
    );
    builder.text(
        Text::new(
            world_panel_label(n),
            TextStyle::new(WORLD_PANEL_TEXT_M).with_color(TEXT_COLOR),
        )
        .id(PanelElementId::named(COUNTER_FIELD)),
    );
    builder.build()
}

fn screen_panel_tree(n: u64) -> LayoutTree {
    let mut builder = LayoutBuilder::with_root(
        El::new()
            .width(Sizing::FIT)
            .height(Sizing::FIT)
            .padding(Padding::all(SCREEN_PANEL_PADDING_PX))
            .corner_radius(CornerRadius::all(SCREEN_PANEL_RADIUS_PX))
            .background(DEFAULT_PANEL_BACKGROUND),
    );
    builder.text(
        Text::new(
            screen_panel_label(n),
            TextStyle::new(SCREEN_PANEL_TEXT_PX).with_color(TEXT_COLOR),
        )
        .id(PanelElementId::named(COUNTER_FIELD)),
    );
    builder.build()
}

/// Unlit transparent surface; the visible panel color comes from the tree's
/// root-element background.
fn panel_surface() -> StandardMaterial {
    StandardMaterial {
        unlit: true,
        ..default_panel_material()
    }
}
