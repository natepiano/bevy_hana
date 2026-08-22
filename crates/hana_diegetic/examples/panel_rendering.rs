//! Panel geometry rendering test — backgrounds, borders, and text.
//!
//! Displays several diegetic panels with different background colors,
//! border configurations, and text content to verify that panel geometry
//! (rectangles and borders) renders correctly alongside MSDF text.

use std::time::Duration;

use bevy::anti_alias::taa::TemporalAntiAliasing;
use bevy::camera::visibility::RenderLayers;
use bevy::picking::mesh_picking::MeshPickingPlugin;
use bevy::prelude::AlphaMode;
use bevy::prelude::Assets;
use bevy::prelude::ButtonInput;
use bevy::prelude::Click;
use bevy::prelude::Color;
use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::DetectChanges;
use bevy::prelude::DirectionalLight;
use bevy::prelude::Entity;
use bevy::prelude::Handle;
use bevy::prelude::KeyCode;
use bevy::prelude::Local;
use bevy::prelude::Msaa;
use bevy::prelude::Name;
use bevy::prelude::On;
use bevy::prelude::PerspectiveProjection;
use bevy::prelude::PointLight;
use bevy::prelude::Pointer;
use bevy::prelude::PointerButton;
use bevy::prelude::PostStartup;
use bevy::prelude::Projection;
use bevy::prelude::Query;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::Resource;
use bevy::prelude::Single;
use bevy::prelude::StandardMaterial;
use bevy::prelude::Startup;
use bevy::prelude::Transform;
use bevy::prelude::Update;
use bevy::prelude::Vec3;
use bevy::prelude::Vec4;
use bevy::prelude::With;
use bevy::prelude::Without;
use bevy::prelude::default;
use bevy::prelude::error;
use bevy_kana::ToU8;
use fairy_dust::CameraHomeTarget;
use fairy_dust::DEFAULT_PANEL_BACKGROUND;
use fairy_dust::LABEL_SIZE;
use fairy_dust::OrbitCamPose;
use fairy_dust::TITLE_SIZE;
use fairy_dust::TitleBar;
use hana_diegetic::Anchor;
use hana_diegetic::Border;
use hana_diegetic::CascadeEntityCommandsExt as _;
use hana_diegetic::ChildDivider;
use hana_diegetic::CornerRadius;
use hana_diegetic::DiegeticPanel;
use hana_diegetic::DiegeticPanelCommands;
use hana_diegetic::El;
use hana_diegetic::Fit;
use hana_diegetic::LayoutBuilder;
use hana_diegetic::LayoutTree;
use hana_diegetic::Mm;
use hana_diegetic::Padding;
use hana_diegetic::Pt;
use hana_diegetic::Px;
use hana_diegetic::Sizing;
use hana_diegetic::TextStyle;
use hana_diegetic::default_panel_material;
use hana_lagrange::OrbitCamPreset;
use hana_lagrange::ZoomToFit;

// ── Colors ──────────────────────────────────────────────────────────
const DARK_BG: Color = Color::srgba(0.3, 0.3, 0.35, 1.0);
const BLUE_BG: Color = Color::srgba(0.12, 0.18, 0.28, 0.95);
const GREEN_BG: Color = Color::srgba(0.08, 0.20, 0.12, 0.95);
const RED_ACCENT: Color = Color::srgb(1.0, 0.0, 0.0);
const BLUE_ACCENT: Color = Color::srgb(0.0, 0.0, 1.0);
const GREEN_ACCENT: Color = Color::srgb(0.0, 1.0, 0.0);
const DIVIDER_COLOR: Color = Color::srgba(0.4, 0.4, 0.5, 0.6);
const TEXT_COLOR: Color = Color::WHITE;
const SUBTLE_TEXT: Color = Color::srgba(0.6, 0.6, 0.65, 0.9);
const BORDER_COLOR: Color = Color::srgba(0.5, 0.5, 0.6, 0.7);

// ── Layout ──────────────────────────────────────────────────────────
const CARD_WIDTH: f32 = 80.0; // mm — each card
const CARD_HEIGHT: f32 = 60.0; // mm
const CARD_PAD: f32 = 4.0; // mm
const CHILD_GAP: f32 = 2.0; // mm
const CARD_GAP: f32 = 6.0; // mm — gap between the three cards
const MM_TO_WORLD: f32 = 0.001;
const CARD_X_STEP: f32 = (CARD_WIDTH + CARD_GAP) * MM_TO_WORLD;
const ZOOM_MARGIN: f32 = 0.02;
const HOME_MARGIN: f32 = 0.10;
const LIGHT_AIM: Vec3 = Vec3::ZERO;
const KEY_LIGHT_POS: Vec3 = Vec3::new(0.0, 1.2, 3.0);
const ZOOM_DURATION_MS: u64 = 600;
/// Illuminance calibrated so a Lambertian surface facing the light
/// produces output ≈ albedo at the default Bevy exposure (EV100 = 9.7).
/// Formula: PI / (2^(-9.7) / 1.2) ≈ 3137.
const SCENE_ILLUMINANCE: f32 = 3137.0;

/// How much illuminance changes per frame while +/- is held.
const ILLUMINANCE_STEP: f32 = 50.0;
const LIGHT_CONTROL: &str = "+/- Light";
const RESET_CONTROL: &str = "R Reset";
const HOME_CONTROL: &str = "H Home";
const LIGHT_READOUT_LABEL: &str = "Lux";
const PRESET_PANEL_TITLE: &str = "Panel Material";
const PRESET_PANEL_PADDING: Px = Px(10.0);
const PRESET_PANEL_RADIUS: Px = Px(10.0);
const PRESET_PANEL_BORDER_WIDTH: Px = Px(1.0);
const PRESET_ROW_GAP: Px = Px(4.0);
const PRESET_KEY_GAP: Px = Px(8.0);
const PRESET_KEY_COLUMN_WIDTH: f32 = 16.0;
const PRESET_MATERIAL_COLUMN_WIDTH: f32 = 74.0;
const PRESET_LIGHTS_COLUMN_WIDTH: f32 = 56.0;
const PRESET_TITLE_COLOR: Color = Color::WHITE;
const PRESET_HEADER_COLOR: Color = Color::srgb(0.55, 0.78, 0.95);
const PRESET_ACTIVE_COLOR: Color = Color::srgb(1.0, 0.9, 0.25);
const PRESET_INACTIVE_COLOR: Color = Color::srgba(0.68, 0.72, 0.82, 0.9);
const PRESET_PANEL_BORDER_COLOR: Color = Color::srgba(0.15, 0.7, 0.9, 0.4);
const LIGHTING_PRESET_ROWS: [(&str, &str, &str); 4] = [
    ("1", "Lit", "On"),
    ("2", "Lit", "Off"),
    ("3", "Unlit", "On"),
    ("4", "Unlit", "Off"),
];

// ── Home camera position ────────────────────────────────────────────
const HOME_FOCUS: Vec3 = Vec3::new(0.0, -0.02, 0.0);
const HOME_RADIUS: f32 = 0.35;
const HOME_YAW: f32 = 0.0;
const HOME_PITCH: f32 = 0.0;

/// Current lighting/material preset and saved illuminance.
#[derive(Resource, Clone, Copy)]
struct LightingPreset {
    index:             u8,
    /// Illuminance to restore when switching back to lights-on.
    saved_illuminance: f32,
}

impl Default for LightingPreset {
    fn default() -> Self {
        Self {
            index:             0,
            saved_illuminance: SCENE_ILLUMINANCE,
        }
    }
}

/// A preset index a digit shortcut asked for, consumed by `cycle_lighting_preset`.
#[derive(Resource, Default)]
struct RequestedPreset(Option<u8>);

/// Registered panel source materials used when lighting presets swap handles.
#[derive(Resource)]
struct PanelMaterialHandles {
    /// Lit source material handle for panels that should use scene lighting.
    lit:   Handle<StandardMaterial>,
    /// Unlit source material handle for panels that should ignore scene lights.
    unlit: Handle<StandardMaterial>,
}

/// Marker for the three world panels under test.
#[derive(Component)]
struct RenderPanel;

/// Marker for the bottom-left lighting preset explanation panel.
#[derive(Component)]
struct PresetPanel;

/// Studio directional light captured with its base illuminance.
#[derive(Component)]
struct SceneLight {
    base_illuminance: f32,
}

/// Studio point light captured with its base intensity.
#[derive(Component)]
struct ScenePointLight {
    base_intensity: f32,
}

impl LightingPreset {
    const fn is_unlit(self) -> bool { self.index == 2 || self.index == 3 }
    const fn lights_on(self) -> bool { self.index == 0 || self.index == 2 }
}

fn main() {
    // `hana_diegetic::DiegeticUiPlugin` is registered automatically by
    // `fairy_dust::sprinkle_example`.
    fairy_dust::sprinkle_example()
        .with_brp_extras()
        .with_save_window_position()
        .add_plugins(MeshPickingPlugin)
        .with_studio_lighting()
        .aim_at(LIGHT_AIM)
        .key_light_pos(KEY_LIGHT_POS)
        .key_light_illuminance(SCENE_ILLUMINANCE)
        .with_ground_plane()
        .size(0.35)
        .transform(Transform::from_xyz(0.0, -0.04, 0.0))
        .with_orbit_cam_preset_bundle(
            |cam| {
                OrbitCamPose {
                    focus:  HOME_FOCUS,
                    yaw:    HOME_YAW,
                    pitch:  HOME_PITCH,
                    radius: HOME_RADIUS,
                }
                .apply_to(cam);
                cam.zoom.set_sensitivity(1.0);
            },
            OrbitCamPreset::blender_like(),
            (
                Projection::Perspective(PerspectiveProjection {
                    near: 0.001,
                    near_clip_plane: Vec4::new(0.0, 0.0, -1.0, -0.001),
                    ..default()
                }),
                bevy::camera::Exposure::default(),
                Msaa::Off,
                TemporalAntiAliasing::default(),
            ),
        )
        .unclamped()
        .with_stable_transparency()
        .with_camera_home()
        .yaw(HOME_YAW)
        .pitch(HOME_PITCH)
        .margin(HOME_MARGIN)
        .with_title_bar(panel_rendering_title_bar(SCENE_ILLUMINANCE))
        .with_camera_control_panel()
        .init_resource::<LightingPreset>()
        .init_resource::<RequestedPreset>()
        .insert_resource(bevy::light::GlobalAmbientLight {
            color:                      Color::BLACK,
            brightness:                 0.0,
            affects_lightmapped_meshes: false,
        })
        .add_systems(Startup, (setup, spawn_preset_panel))
        .add_systems(PostStartup, capture_scene_lights)
        // `cycle_lighting_preset` consumes the digit request; `adjust_illuminance`
        // is a held +/- (and R) brightness control that reads the physical keys
        // regardless of Shift, which the modifier-guarded shortcut binding can't
        // reproduce, so it stays a raw per-frame reader.
        .add_systems(
            Update,
            (
                cycle_lighting_preset,
                adjust_illuminance,
                refresh_preset_panel,
                refresh_title_bar_light_readout,
            ),
        )
        // 1..4 select a lighting preset through Fairy Dust's shortcut binding,
        // which fires each only when no modifier is held.
        .with_shortcut(KeyCode::Digit1, request_preset_1)
        .with_shortcut(KeyCode::Digit2, request_preset_2)
        .with_shortcut(KeyCode::Digit3, request_preset_3)
        .with_shortcut(KeyCode::Digit4, request_preset_4)
        .run();
}

fn panel_rendering_title_bar(lux: f32) -> TitleBar {
    TitleBar::new().with_title("Panel Rendering").controls([
        HOME_CONTROL.to_string(),
        LIGHT_CONTROL.to_string(),
        light_readout_control(lux),
        RESET_CONTROL.to_string(),
    ])
}

fn light_readout_control(lux: f32) -> String { format!("{LIGHT_READOUT_LABEL} {lux:.0}") }

/// 1..4 request a lighting preset through Fairy Dust's shortcut binding; this
/// system applies the request. Each fires only when no modifier is held.
fn request_preset_1(mut requested: ResMut<RequestedPreset>) { requested.0 = Some(0); }

fn request_preset_2(mut requested: ResMut<RequestedPreset>) { requested.0 = Some(1); }

fn request_preset_3(mut requested: ResMut<RequestedPreset>) { requested.0 = Some(2); }

fn request_preset_4(mut requested: ResMut<RequestedPreset>) { requested.0 = Some(3); }

/// Applies a requested lighting preset.
fn cycle_lighting_preset(
    mut requested: ResMut<RequestedPreset>,
    mut preset: ResMut<LightingPreset>,
    material_handles: Res<PanelMaterialHandles>,
    panels: Query<Entity, With<RenderPanel>>,
    mut lights: Query<(&mut DirectionalLight, &SceneLight)>,
    mut point_lights: Query<(&mut PointLight, &ScenePointLight)>,
    mut commands: Commands,
) {
    let Some(idx) = requested.0.take() else {
        return;
    };

    // Save current illuminance before switching away from lights-on.
    if preset.lights_on() {
        preset.saved_illuminance = current_key_illuminance_mut(&lights);
    }

    preset.index = idx;

    let unlit = preset.is_unlit();
    let lights_visible = preset.lights_on();
    let source_material = if unlit {
        material_handles.unlit.clone()
    } else {
        material_handles.lit.clone()
    };

    for panel in &panels {
        commands
            .entity(panel)
            .override_sdf_material(source_material.clone())
            .override_text_material(source_material.clone());
    }

    // Restore saved illuminance for lights-on, zero for lights-off.
    let key_illuminance = if lights_visible {
        preset.saved_illuminance
    } else {
        0.0
    };
    apply_key_illuminance(&mut lights, &mut point_lights, key_illuminance);
}

fn current_key_illuminance_mut(lights: &Query<(&mut DirectionalLight, &SceneLight)>) -> f32 {
    lights
        .iter()
        .map(|(light, _)| light.illuminance)
        .fold(0.0, f32::max)
}

fn apply_key_illuminance(
    lights: &mut Query<(&mut DirectionalLight, &SceneLight)>,
    point_lights: &mut Query<(&mut PointLight, &ScenePointLight)>,
    key_illuminance: f32,
) {
    let base_key = lights
        .iter()
        .map(|(_, scene_light)| scene_light.base_illuminance)
        .fold(0.0, f32::max);
    if base_key <= f32::EPSILON {
        return;
    }

    let scale = key_illuminance / base_key;
    for (mut light, scene_light) in lights {
        light.illuminance = scene_light.base_illuminance * scale;
    }
    for (mut light, scene_light) in point_lights {
        light.intensity = scene_light.base_intensity * scale;
    }
}

/// Animates the camera to the home viewing angle, framing the panel.
/// Adjusts scene illuminance with +/- keys (continuous while held).
/// [R] resets to the calibrated default.
fn adjust_illuminance(
    keyboard: Res<ButtonInput<KeyCode>>,
    mut lights: Query<(&mut DirectionalLight, &SceneLight)>,
    mut point_lights: Query<(&mut PointLight, &ScenePointLight)>,
) {
    let up = keyboard.pressed(KeyCode::Equal) || keyboard.pressed(KeyCode::NumpadAdd);
    let down = keyboard.pressed(KeyCode::Minus) || keyboard.pressed(KeyCode::NumpadSubtract);
    let reset = keyboard.just_pressed(KeyCode::KeyR);

    if !up && !down && !reset {
        return;
    }

    let current = current_key_illuminance_mut(&lights);
    let target = if reset {
        SCENE_ILLUMINANCE
    } else if up {
        current + ILLUMINANCE_STEP
    } else {
        (current - ILLUMINANCE_STEP).max(0.0)
    };
    apply_key_illuminance(&mut lights, &mut point_lights, target);
}

fn on_panel_clicked(mut click: On<Pointer<Click>>, mut commands: Commands) {
    if click.button != PointerButton::Primary {
        return;
    }
    click.propagate(false);
    let camera = click.hit.camera;
    commands.trigger(
        ZoomToFit::new(camera, click.entity)
            .margin(ZOOM_MARGIN)
            .duration(Duration::from_millis(ZOOM_DURATION_MS)),
    );
}

fn setup(mut commands: Commands, mut materials: ResMut<Assets<StandardMaterial>>) {
    let lit_panel_material = materials.add(default_panel_material());
    let unlit_panel_material = materials.add(StandardMaterial {
        unlit: true,
        ..default_panel_material()
    });
    commands.insert_resource(PanelMaterialHandles {
        lit:   lit_panel_material,
        unlit: unlit_panel_material,
    });

    let opaque_surface_material = materials.add(StandardMaterial {
        alpha_mode: AlphaMode::Opaque,
        ..default_panel_material()
    });
    spawn_panel_card(
        &mut commands,
        "Backgrounds panel",
        -CARD_X_STEP,
        build_backgrounds_panel(opaque_surface_material),
    );
    spawn_panel_card(&mut commands, "Borders panel", 0.0, build_borders_panel());
    spawn_panel_card(
        &mut commands,
        "Combined panel",
        CARD_X_STEP,
        build_combined_panel(),
    );
}

fn capture_scene_lights(
    mut commands: Commands,
    directional_lights: Query<
        (Entity, &DirectionalLight, Option<&RenderLayers>),
        Without<SceneLight>,
    >,
    point_lights: Query<(Entity, &PointLight), Without<ScenePointLight>>,
) {
    for (entity, light, layers) in &directional_lights {
        if layers.is_some() {
            continue;
        }
        commands.entity(entity).insert(SceneLight {
            base_illuminance: light.illuminance,
        });
    }
    for (entity, light) in &point_lights {
        commands.entity(entity).insert(ScenePointLight {
            base_intensity: light.intensity,
        });
    }
}

fn spawn_panel_card(commands: &mut Commands, name: &'static str, x: f32, tree: LayoutTree) {
    let panel = DiegeticPanel::world()
        .size(Mm(CARD_WIDTH), Mm(CARD_HEIGHT))
        .anchor(Anchor::Center)
        .with_tree(tree)
        .build();
    let Ok(panel) = panel else {
        error!("failed to build panel dimensions");
        return;
    };

    commands
        .spawn((
            Name::new(name),
            RenderPanel,
            CameraHomeTarget,
            panel,
            Transform::from_xyz(x, 0.0, 0.0),
        ))
        .observe(on_panel_clicked);
}

fn spawn_preset_panel(
    mut commands: Commands,
    mut materials: ResMut<Assets<StandardMaterial>>,
    preset: Res<LightingPreset>,
) {
    let unlit = materials.add(StandardMaterial {
        unlit: true,
        ..default_panel_material()
    });
    let panel = DiegeticPanel::screen()
        .size(Fit, Fit)
        .anchor(Anchor::BottomLeft)
        .material(unlit.clone())
        .text_material(unlit)
        .with_tree(build_preset_panel_tree(*preset))
        .build();

    match panel {
        Ok(panel) => {
            commands.spawn((PresetPanel, panel, Transform::default()));
        },
        Err(error) => {
            error!("panel_rendering: failed to build preset panel: {error}");
        },
    }
}

fn refresh_preset_panel(
    preset: Res<LightingPreset>,
    panel: Single<Entity, With<PresetPanel>>,
    mut commands: Commands,
) {
    if !preset.is_changed() {
        return;
    }
    if let Err(error) = commands.set_tree(*panel, build_preset_panel_tree(*preset)) {
        error!("failed to replace rendering preset panel tree: {error}");
    }
}

fn refresh_title_bar_light_readout(
    lights: Query<&DirectionalLight, With<SceneLight>>,
    mut title_bar: Single<&mut TitleBar>,
    mut previous_lux: Local<u32>,
) {
    let lux = current_light_intensity(&lights);
    let rounded = lux.round().to_bits();
    if *previous_lux == rounded {
        return;
    }
    *previous_lux = rounded;
    **title_bar = panel_rendering_title_bar(lux);
}

fn current_light_intensity(lights: &Query<&DirectionalLight, With<SceneLight>>) -> f32 {
    lights
        .iter()
        .map(|light| light.illuminance)
        .fold(0.0, f32::max)
}

// ── Panel builders ──────────────────────────────────────────────────

fn build_preset_panel_tree(preset: LightingPreset) -> LayoutTree {
    let mut builder = LayoutBuilder::with_root(El::new().width(Sizing::FIT).height(Sizing::FIT));
    build_preset_panel_layout(&mut builder, preset);
    builder.build()
}

fn build_preset_panel_layout(builder: &mut LayoutBuilder, preset: LightingPreset) {
    let title = TextStyle::new(TITLE_SIZE).with_color(PRESET_TITLE_COLOR);
    let header = TextStyle::new(LABEL_SIZE).with_color(PRESET_HEADER_COLOR);
    let key_active = TextStyle::new(LABEL_SIZE).with_color(PRESET_ACTIVE_COLOR);
    let key_inactive = TextStyle::new(LABEL_SIZE).with_color(PRESET_INACTIVE_COLOR);
    let body_active = TextStyle::new(LABEL_SIZE).with_color(PRESET_ACTIVE_COLOR);
    let body_inactive = TextStyle::new(LABEL_SIZE).with_color(PRESET_INACTIVE_COLOR);

    builder.with(
        El::column()
            .width(Sizing::FIT)
            .height(Sizing::FIT)
            .gap(PRESET_ROW_GAP)
            .padding(Padding::all(PRESET_PANEL_PADDING))
            .corner_radius(CornerRadius::all(PRESET_PANEL_RADIUS))
            .background(DEFAULT_PANEL_BACKGROUND)
            .border(Border::all(
                PRESET_PANEL_BORDER_WIDTH,
                PRESET_PANEL_BORDER_COLOR,
            )),
        |builder| {
            builder.text((PRESET_PANEL_TITLE, title));
            panel_divider(builder);
            build_preset_row(builder, "", "Material", "Lights", &header, &header, &header);
            panel_divider(builder);
            for (index, (key, material, lights)) in LIGHTING_PRESET_ROWS.into_iter().enumerate() {
                let active = preset.index == index.to_u8();
                let key_style = if active { &key_active } else { &key_inactive };
                let body_style = if active { &body_active } else { &body_inactive };
                build_preset_row(
                    builder, key, material, lights, key_style, body_style, body_style,
                );
            }
        },
    );
}

fn build_preset_row(
    builder: &mut LayoutBuilder,
    key: &str,
    material: &str,
    lights: &str,
    key_style: &TextStyle,
    material_style: &TextStyle,
    lights_style: &TextStyle,
) {
    builder.with(
        El::row()
            .width(Sizing::FIT)
            .height(Sizing::FIT)
            .gap(PRESET_KEY_GAP),
        |builder| {
            builder.with(
                El::new()
                    .width(Sizing::fixed(PRESET_KEY_COLUMN_WIDTH))
                    .height(Sizing::FIT),
                |builder| {
                    builder.text((key, key_style.clone()));
                },
            );
            builder.with(
                El::new()
                    .width(Sizing::fixed(PRESET_MATERIAL_COLUMN_WIDTH))
                    .height(Sizing::FIT),
                |builder| {
                    builder.text((material, material_style.clone()));
                },
            );
            builder.with(
                El::new()
                    .width(Sizing::fixed(PRESET_LIGHTS_COLUMN_WIDTH))
                    .height(Sizing::FIT),
                |builder| {
                    builder.text((lights, lights_style.clone()));
                },
            );
        },
    );
}

fn panel_divider(builder: &mut LayoutBuilder) {
    builder.with(
        El::new()
            .width(Sizing::GROW)
            .height(Sizing::fixed(Px(1.0)))
            .background(PRESET_PANEL_BORDER_COLOR),
        |_| {},
    );
}

fn build_backgrounds_panel(opaque_surface_material: Handle<StandardMaterial>) -> LayoutTree {
    let title_style = TextStyle::new(Pt(10.0)).with_color(TEXT_COLOR);
    let body_style = TextStyle::new(Pt(7.0)).with_color(SUBTLE_TEXT);

    let mut builder = LayoutBuilder::new(CARD_WIDTH, CARD_HEIGHT);
    build_card_backgrounds(
        &mut builder,
        &title_style,
        &body_style,
        opaque_surface_material,
    );
    builder.build()
}

fn build_borders_panel() -> LayoutTree {
    let title_style = TextStyle::new(Pt(10.0)).with_color(TEXT_COLOR);
    let body_style = TextStyle::new(Pt(7.0)).with_color(SUBTLE_TEXT);

    let mut builder = LayoutBuilder::new(CARD_WIDTH, CARD_HEIGHT);
    build_card_borders(&mut builder, &title_style, &body_style);
    builder.build()
}

fn build_combined_panel() -> LayoutTree {
    let title_style = TextStyle::new(Pt(10.0)).with_color(TEXT_COLOR);
    let body_style = TextStyle::new(Pt(7.0)).with_color(SUBTLE_TEXT);

    let mut builder = LayoutBuilder::new(CARD_WIDTH, CARD_HEIGHT);
    build_card_combined(&mut builder, &title_style, &body_style);
    builder.build()
}

fn build_card_backgrounds(
    b: &mut LayoutBuilder,
    title_style: &TextStyle,
    body_style: &TextStyle,
    opaque_surface_material: Handle<StandardMaterial>,
) {
    b.with(
        El::column()
            .padding(Padding::all(CARD_PAD))
            .gap(CHILD_GAP)
            .background(DARK_BG)
            .corner_radius(CornerRadius::all(Mm(3.0)))
            .width(Sizing::grow_min(0.0))
            .height(Sizing::grow_min(0.0)),
        |b| {
            b.text(("Backgrounds", title_style.clone()));
            b.text(("Nested elements with fills", body_style.clone()));

            b.with(
                El::row()
                    .gap(CHILD_GAP)
                    .width(Sizing::grow_min(0.0))
                    .height(Sizing::grow_min(0.0)),
                |b| {
                    b.with(
                        El::new()
                            .background(RED_ACCENT)
                            .corner_radius(CornerRadius::all(Mm(1.5)))
                            .padding(Padding::all(3.0))
                            .width(Sizing::grow_min(0.0))
                            .height(Sizing::grow_min(0.0)),
                        |b| {
                            b.text(("Red", body_style.clone()));
                        },
                    );
                    b.with(
                        El::new()
                            .background(BLUE_ACCENT)
                            .corner_radius(CornerRadius::all(Mm(1.5)))
                            .padding(Padding::all(3.0))
                            .width(Sizing::grow_min(0.0))
                            .height(Sizing::grow_min(0.0)),
                        |b| {
                            b.text(("Blue", body_style.clone()));
                        },
                    );
                    b.with(
                        El::new()
                            .background(GREEN_ACCENT)
                            .corner_radius(CornerRadius::all(Mm(1.5)))
                            .padding(Padding::all(3.0))
                            .width(Sizing::grow_min(0.0))
                            .height(Sizing::grow_min(0.0)),
                        |b| {
                            b.text(("Green", body_style.clone()));
                        },
                    );
                },
            );

            b.with(
                El::new()
                    .background(BLUE_BG)
                    .material(opaque_surface_material)
                    .padding(Padding::all(3.0))
                    .width(Sizing::grow_min(0.0))
                    .height(Sizing::grow_min(0.0)),
                |b| {
                    b.text(("Nested background", body_style.clone()));
                },
            );
        },
    );
}

fn build_card_borders(b: &mut LayoutBuilder, title_style: &TextStyle, body_style: &TextStyle) {
    b.with(
        El::column()
            .padding(Padding::all(CARD_PAD))
            .gap(CHILD_GAP)
            .border(Border::all(Mm(0.5), BORDER_COLOR))
            .width(Sizing::grow_min(0.0))
            .height(Sizing::grow_min(0.0)),
        |b| {
            b.text(("Borders", title_style.clone()));

            b.with(
                El::new()
                    .border(Border::all(Mm(0.3), BLUE_ACCENT))
                    .padding(Padding::all(2.0))
                    .width(Sizing::grow_min(0.0))
                    .height(Sizing::fit_min(0.0)),
                |b| {
                    b.text(("Thin blue border", body_style.clone()));
                },
            );

            b.with(
                El::new()
                    .border(Border::all(Mm(1.0), RED_ACCENT))
                    .padding(Padding::all(2.0))
                    .width(Sizing::grow_min(0.0))
                    .height(Sizing::fit_min(0.0)),
                |b| {
                    b.text(("Thick red border", body_style.clone()));
                },
            );

            b.with(
                El::column()
                    .gap(CHILD_GAP)
                    .border(Border::all(Mm(0.3), BORDER_COLOR))
                    .child_divider(ChildDivider::new(Mm(0.3), BORDER_COLOR))
                    .padding(Padding::all(2.0))
                    .width(Sizing::grow_min(0.0))
                    .height(Sizing::grow_min(0.0)),
                |b| {
                    b.text(("Row A", body_style.clone()));
                    b.text(("Row B", body_style.clone()));
                    b.text(("Row C", body_style.clone()));
                },
            );
        },
    );
}

fn build_card_combined(b: &mut LayoutBuilder, title_style: &TextStyle, body_style: &TextStyle) {
    b.with(
        El::column()
            .padding(Padding::all(CARD_PAD))
            .gap(CHILD_GAP)
            .background(DARK_BG)
            .border(Border::all(Mm(0.5), BLUE_ACCENT))
            .corner_radius(CornerRadius::all(Mm(3.0)))
            .width(Sizing::grow_min(0.0))
            .height(Sizing::grow_min(0.0)),
        |b| {
            b.text(("Combined", title_style.clone()));

            b.with(
                El::new()
                    .background(GREEN_BG)
                    .border(Border::all(Mm(0.3), GREEN_ACCENT))
                    .padding(Padding::all(3.0))
                    .width(Sizing::grow_min(0.0))
                    .height(Sizing::fit_min(0.0)),
                |b| {
                    b.text(("Card with bg + border", body_style.clone()));
                },
            );

            b.with(
                El::row()
                    .gap(CHILD_GAP)
                    .width(Sizing::grow_min(0.0))
                    .height(Sizing::grow_min(0.0)),
                |b| {
                    // Overflow visible — second line spills past the box.
                    b.with(
                        El::column()
                            .gap(1.0)
                            .background(BLUE_BG)
                            .border(Border::all(Mm(0.3), BLUE_ACCENT))
                            .padding(Padding::all(2.0))
                            .width(Sizing::grow_min(0.0))
                            .height(Sizing::fixed(8.0)),
                        |b| {
                            b.text(("No clip", body_style.clone()));
                            b.text(("Spills out", body_style.clone()));
                        },
                    );
                    // Overflow clipped — second line hidden at the boundary.
                    b.with(
                        El::column()
                            .gap(1.0)
                            .clip()
                            .background(GREEN_BG)
                            .border(Border::all(Mm(0.3), GREEN_ACCENT))
                            .padding(Padding::all(2.0))
                            .width(Sizing::grow_min(0.0))
                            .height(Sizing::fixed(8.0)),
                        |b| {
                            b.text(("Clipped", body_style.clone()));
                            b.text(("Hidden", body_style.clone()));
                        },
                    );
                },
            );

            b.with(
                El::column()
                    .gap(1.5)
                    .background(BLUE_BG)
                    .border(Border::all(Mm(0.3), DIVIDER_COLOR))
                    .child_divider(ChildDivider::new(Mm(0.2), DIVIDER_COLOR))
                    .padding(Padding::all(2.0))
                    .width(Sizing::grow_min(0.0))
                    .height(Sizing::grow_min(0.0)),
                |b| {
                    b.text(("Item 1", body_style.clone()));
                    b.text(("Item 2", body_style.clone()));
                    b.text(("Item 3", body_style.clone()));
                },
            );
        },
    );
}
