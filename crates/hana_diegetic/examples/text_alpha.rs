//! `text_alpha` — interactive walk-through of every `AlphaMode` variant for
//! text.
//!
//! Slug renders glyph coverage as per-pixel alpha; this example shows how each
//! `AlphaMode` composites that coverage. The scene launches in the library
//! default (`AlphaMode::Blend` with MSAA on). Coplanar text such as the
//! floating "GROUND" label stays stable as the camera moves because coplanar
//! runs are ordered by a per-record depth nudge inside the text batch. To see
//! the view-angle color shift that order-independent transparency fixes, and
//! the MSAA trade it makes, run the `oit_msaa` example.
//!
//! Hotkeys:
//! - `H` — home the camera.
//! - `1..7` — select the active `AlphaMode`: 1 Blend (default) · 2 Premultiplied · 3 Coverage · 4
//!   Add · 5 Multiply · 6 Mask · 7 Opaque.

use bevy::prelude::AlphaMode;
use bevy::prelude::Assets;
use bevy::prelude::Color;
use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::Cuboid;
use bevy::prelude::DetectChanges;
use bevy::prelude::DirectionalLight;
use bevy::prelude::Entity;
use bevy::prelude::GlobalAmbientLight;
use bevy::prelude::KeyCode;
use bevy::prelude::Mesh;
use bevy::prelude::Mesh3d;
use bevy::prelude::MeshMaterial3d;
use bevy::prelude::Meshable;
use bevy::prelude::Plane3d;
use bevy::prelude::Quat;
use bevy::prelude::Query;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::Resource;
use bevy::prelude::StandardMaterial;
use bevy::prelude::Startup;
use bevy::prelude::Transform;
use bevy::prelude::Update;
use bevy::prelude::Vec3;
use bevy::prelude::With;
use bevy::prelude::default;
use bevy::prelude::error;
use fairy_dust::CameraHomeTarget;
use hana_diegetic::AlignY;
use hana_diegetic::Anchor;
use hana_diegetic::Border;
use hana_diegetic::DiegeticPanel;
use hana_diegetic::DiegeticText;
use hana_diegetic::El;
use hana_diegetic::LayoutBuilder;
use hana_diegetic::Padding;
use hana_diegetic::Pt;
use hana_diegetic::Px;
use hana_diegetic::Sidedness;
use hana_diegetic::Sizing;
use hana_diegetic::TextAlpha;
use hana_diegetic::TextStyle;
use hana_lagrange::OrbitCamPreset;

const HOME_YAW: f32 = 0.3;
const HOME_PITCH: f32 = 0.80;

const HUD_HEIGHT: Px = Px(44.0);
const HUD_PADDING: Px = Px(10.0);
const HUD_GAP: Px = Px(14.0);
const HUD_WIDTH: Px = Px(600.0);

const INFO_HEIGHT: Px = Px(720.0);
const INFO_HEADER_SIZE: Pt = Pt(14.0);
const INFO_BODY_SIZE: Pt = Pt(10.0);
const INFO_TITLE_SIZE: Pt = Pt(16.0);

const HUD_TITLE_SIZE: Pt = Pt(14.0);
const HUD_HINT_SIZE: Pt = Pt(11.0);
const HUD_FRAME_BACKGROUND: Color = Color::srgba(0.01, 0.01, 0.03, 0.95);
const HUD_BACKGROUND: Color = Color::srgba(0.02, 0.03, 0.07, 0.80);
const HUD_BORDER_ACCENT: Color = Color::srgba(0.15, 0.7, 0.9, 0.5);
const HUD_BORDER_DIM: Color = Color::srgba(0.1, 0.4, 0.6, 0.3);
const HUD_TITLE_COLOR: Color = Color::srgb(0.9, 0.95, 1.0);
const HUD_DIVIDER_COLOR: Color = Color::srgba(0.15, 0.4, 0.6, 0.25);
const HUD_INACTIVE_COLOR: Color = Color::srgba(0.55, 0.60, 0.75, 0.85);
const HUD_ACTIVE_COLOR: Color = Color::srgb(0.3, 1.0, 0.8);

#[derive(Component)]
struct HudPanel;

#[derive(Component)]
struct InfoPanel;

#[derive(Resource)]
struct ControlsState {
    alpha_mode: AlphaMode,
}

impl Default for ControlsState {
    fn default() -> Self {
        // Matches the `TextAlpha` the cascade installs.
        Self {
            alpha_mode: AlphaMode::Blend,
        }
    }
}

const ALPHA_MODES: [(AlphaMode, &str); 7] = [
    (AlphaMode::Blend, "Blend"),
    (AlphaMode::Premultiplied, "Premultiplied"),
    (AlphaMode::AlphaToCoverage, "Coverage"),
    (AlphaMode::Add, "Add"),
    (AlphaMode::Multiply, "Multiply"),
    (AlphaMode::Mask(0.5), "Mask"),
    (AlphaMode::Opaque, "Opaque"),
];

fn alpha_mode_label(mode: AlphaMode) -> &'static str {
    ALPHA_MODES
        .iter()
        .find(|(m, _)| std::mem::discriminant(m) == std::mem::discriminant(&mode))
        .map_or("?", |(_, l)| *l)
}

const fn alpha_mode_description(mode: AlphaMode) -> &'static str {
    match mode {
        AlphaMode::Blend => {
            "Classic alpha compositing. Slug emits per-pixel coverage as \
             alpha, and Blend composites each fragment with the background: \
             interiors render fully opaque when the color's alpha is 1.0, \
             edges blend smoothly.\n\n\
             Routes through the transparent queue. Batched text orders \
             coplanar runs with a per-record depth nudge, so it stays \
             stable as the camera angle changes."
        },
        AlphaMode::Premultiplied => {
            "Sibling of Blend; assumes RGB channels are pre-multiplied by \
             alpha. Per Bevy's docs, behaves like Blend at alphas near 1.0 \
             and like Add at alphas near 0.0. Avoids border/outline \
             artifacts that Blend can show on some textures.\n\n\
             Worth comparing with Blend on your scene."
        },
        AlphaMode::AlphaToCoverage => {
            "Order-independent anti-aliasing via sub-pixel coverage. Slug's \
             fractional coverage becomes a sample-mask pattern; MSAA smooths \
             it into a perceived gradient. Bypasses the transparent queue \
             entirely — no depth-sort issues.\n\n\
             Requires MSAA. Without it, degrades to Mask(0.5) and looks \
             jagged."
        },
        AlphaMode::Add => {
            "Additive blending — glyph color is added to whatever is behind \
             it. Great for neon, glow, and holographic effects over dark \
             backgrounds."
        },
        AlphaMode::Multiply => {
            "Multiplicative blending — glyph color multiplies into the \
             background. Good for ink or tint effects over light \
             backgrounds; disappears on dark ones.\n\n\
             The world text and the camera panel follow this global default \
             and go dark over the dark scene — expected, not a bug. The mode \
             bar and this info panel pin Blend to stay readable."
        },
        AlphaMode::Mask(_) => {
            "Hard alpha test at the configured threshold (0.5 here). \
             Fragments above threshold render fully opaque; below threshold \
             are discarded. Bypasses the transparent queue — no sorting \
             issues, no MSAA dependency.\n\n\
             Caveat: slug's smooth coverage edge is thresholded away — edges \
             look jagged. Use only for retro / pixel-art looks."
        },
        AlphaMode::Opaque => {
            "Disables alpha blending. Batched text routes Opaque through \
             Mask(0.0) on the GPU material, so coverage discards still cut \
             the glyph outline: glyphs render as hard-edged, depth-writing \
             silhouettes.\n\n\
             Edges are aliased — prefer Blend or Coverage for readable text."
        },
    }
}

fn main() {
    // `hana_diegetic::DiegeticUiPlugin` is registered automatically by
    // `fairy_dust::sprinkle_example`. The orbit camera keeps the default
    // `Msaa::Sample4`, so the `Coverage` (`AlphaToCoverage`) mode works.
    fairy_dust::sprinkle_example()
        .with_brp_extras()
        .with_save_window_position()
        .with_orbit_cam_preset(|_| {}, OrbitCamPreset::blender_like())
        .with_camera_home()
        .yaw(HOME_YAW)
        .pitch(HOME_PITCH)
        .with_camera_control_panel()
        .init_resource::<ControlsState>()
        .add_systems(Startup, setup)
        .add_systems(Update, apply_state_and_rebuild_hud)
        // 1..7 select the active AlphaMode through Fairy Dust's shortcut binding,
        // which fires each only when no modifier is held.
        .with_shortcut(KeyCode::Digit1, select_alpha_blend)
        .with_shortcut(KeyCode::Digit2, select_alpha_premultiplied)
        .with_shortcut(KeyCode::Digit3, select_alpha_coverage)
        .with_shortcut(KeyCode::Digit4, select_alpha_add)
        .with_shortcut(KeyCode::Digit5, select_alpha_multiply)
        .with_shortcut(KeyCode::Digit6, select_alpha_mask)
        .with_shortcut(KeyCode::Digit7, select_alpha_opaque)
        .run();
}

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    // Ground: translucent plane.
    commands.spawn((
        CameraHomeTarget,
        Mesh3d(meshes.add(Plane3d::default().mesh().size(3.5, 3.5))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgba(0.10, 0.10, 0.12, 0.55),
            alpha_mode: AlphaMode::Blend,
            double_sided: true,
            cull_mode: None,
            ..default()
        })),
    ));

    // Cube with WorldText on its front face.
    commands
        .spawn((
            CameraHomeTarget,
            Mesh3d(meshes.add(Cuboid::default())),
            MeshMaterial3d(materials.add(StandardMaterial {
                base_color: Color::srgb(0.8, 0.7, 0.6),
                ..default()
            })),
            Transform::from_xyz(0.0, 0.51, 0.0),
        ))
        .with_children(|parent| {
            parent.spawn(
                DiegeticText::world("HELLO")
                    .size(0.22)
                    .color(Color::srgb(0.9, 0.3, 0.1))
                    .sidedness(Sidedness::FrontOnly)
                    .transform(Transform::from_xyz(0.0, 0.0, 0.501))
                    .build(),
            );
        });

    // WorldText floating on the ground (coplanar reproducer).
    commands.spawn((
        CameraHomeTarget,
        DiegeticText::world("GROUND")
            .size(0.45)
            .color(Color::srgb(1.0, 0.85, 0.1))
            .transform(
                Transform::from_xyz(0.0, 0.001, 1.125)
                    .with_rotation(Quat::from_rotation_x(-core::f32::consts::FRAC_PI_2)),
            )
            .build(),
    ));

    // Lighting.
    commands.insert_resource(GlobalAmbientLight {
        color:                      Color::WHITE,
        brightness:                 400.0,
        affects_lightmapped_meshes: true,
    });
    commands.spawn((
        DirectionalLight {
            shadow_maps_enabled: true,
            ..default()
        },
        Transform::from_xyz(4.0, 8.0, 4.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));

    // HUD and info panels are spawned by `apply_state_and_rebuild_hud` on
    // the first frame (via `Res::is_changed()` on initial resource insert).
}

// Digit 1..7 → ALPHA_MODES[0..7], each bound through Fairy Dust's shortcut
// binding. Setting `alpha_mode` flips `ControlsState`, which
// `apply_state_and_rebuild_hud` picks up via change detection.
fn select_alpha_blend(mut state: ResMut<ControlsState>) { state.alpha_mode = ALPHA_MODES[0].0; }

fn select_alpha_premultiplied(mut state: ResMut<ControlsState>) {
    state.alpha_mode = ALPHA_MODES[1].0;
}

fn select_alpha_coverage(mut state: ResMut<ControlsState>) { state.alpha_mode = ALPHA_MODES[2].0; }

fn select_alpha_add(mut state: ResMut<ControlsState>) { state.alpha_mode = ALPHA_MODES[3].0; }

fn select_alpha_multiply(mut state: ResMut<ControlsState>) { state.alpha_mode = ALPHA_MODES[4].0; }

fn select_alpha_mask(mut state: ResMut<ControlsState>) { state.alpha_mode = ALPHA_MODES[5].0; }

fn select_alpha_opaque(mut state: ResMut<ControlsState>) { state.alpha_mode = ALPHA_MODES[6].0; }

fn apply_state_and_rebuild_hud(
    state: Res<ControlsState>,
    mut commands: Commands,
    mut alpha_default: ResMut<TextAlpha>,
    hud_panels: Query<Entity, With<HudPanel>>,
    info_panels: Query<Entity, With<InfoPanel>>,
) {
    if !state.is_changed() {
        return;
    }

    alpha_default.0 = state.alpha_mode;

    for e in &hud_panels {
        commands.entity(e).despawn();
    }
    for e in &info_panels {
        commands.entity(e).despawn();
    }
    spawn_hud_panel(&mut commands);
    spawn_info_panel(&mut commands, &state);
}

fn spawn_hud_panel(commands: &mut Commands) {
    let hud_panel = DiegeticPanel::screen()
        .size(HUD_WIDTH, HUD_HEIGHT)
        .anchor(Anchor::TopLeft)
        .text_alpha_mode(AlphaMode::Blend)
        .layout(build_controls)
        .build();
    let Ok(hud_panel) = hud_panel else {
        error!("failed to build HUD");
        return;
    };

    commands.spawn((HudPanel, hud_panel, Transform::default()));
}

fn spawn_info_panel(commands: &mut Commands, state: &ControlsState) {
    let mode = state.alpha_mode;
    let info_panel = DiegeticPanel::screen()
        .size(hana_diegetic::Percent(0.22), INFO_HEIGHT)
        .anchor(Anchor::TopRight)
        .text_alpha_mode(AlphaMode::Blend)
        .layout(move |b| build_info_panel(b, mode))
        .build();
    let Ok(info_panel) = info_panel else {
        error!("failed to build info panel");
        return;
    };

    commands.spawn((InfoPanel, info_panel, Transform::default()));
}

fn hud_text_style(active: bool) -> TextStyle {
    TextStyle::new(HUD_HINT_SIZE).with_color(if active {
        HUD_ACTIVE_COLOR
    } else {
        HUD_INACTIVE_COLOR
    })
}

fn build_controls(b: &mut LayoutBuilder) {
    let title = TextStyle::new(HUD_TITLE_SIZE).with_color(HUD_TITLE_COLOR);

    b.with(
        El::new()
            .width(Sizing::GROW)
            .height(Sizing::GROW)
            .padding(Padding::all(Px(2.0)))
            .background(HUD_FRAME_BACKGROUND)
            .border(Border::all(Px(2.0), HUD_BORDER_ACCENT)),
        |b| {
            b.with(
                El::row()
                    .width(Sizing::GROW)
                    .height(Sizing::GROW)
                    .padding(Padding::new(Px(8.0), HUD_PADDING, Px(8.0), HUD_PADDING))
                    .gap(HUD_GAP)
                    .align_y(AlignY::Center)
                    .clip()
                    .background(HUD_BACKGROUND)
                    .border(Border::all(Px(1.0), HUD_BORDER_DIM)),
                |b| {
                    b.text(("CONTROLS", title));
                    hud_divider(b);
                    b.text(("H home", hud_text_style(false)));
                    hud_divider(b);
                    b.text(("1-7 alpha mode", hud_text_style(false)));
                },
            );
        },
    );
}

fn hud_divider(b: &mut LayoutBuilder) {
    b.with(
        El::new()
            .width(Sizing::fixed(Px(1.0)))
            .height(Sizing::fixed(Px(20.0)))
            .background(HUD_DIVIDER_COLOR),
        |_| {},
    );
}

fn build_info_panel(b: &mut LayoutBuilder, active: AlphaMode) {
    let title = TextStyle::new(INFO_TITLE_SIZE).with_color(HUD_TITLE_COLOR);
    let active_header = TextStyle::new(INFO_HEADER_SIZE).with_color(HUD_ACTIVE_COLOR);
    let body = TextStyle::new(INFO_BODY_SIZE).with_color(HUD_INACTIVE_COLOR);

    b.with(
        El::new()
            .width(Sizing::GROW)
            .height(Sizing::GROW)
            .padding(Padding::all(Px(2.0)))
            .background(HUD_FRAME_BACKGROUND)
            .border(Border::all(Px(2.0), HUD_BORDER_ACCENT)),
        |b| {
            b.with(
                El::column()
                    .width(Sizing::GROW)
                    .height(Sizing::GROW)
                    .padding(Padding::all(Px(10.0)))
                    .gap(Px(8.0))
                    .background(HUD_BACKGROUND)
                    .border(Border::all(Px(1.0), HUD_BORDER_DIM)),
                |b| {
                    // Title.
                    b.with(El::new().width(Sizing::GROW), |b| {
                        b.text(("ALPHA MODES", title));
                    });
                    // Vertical list of modes 1..7; active is highlighted.
                    b.with(El::column().width(Sizing::GROW).gap(Px(2.0)), |b| {
                        for (idx, (mode, label)) in ALPHA_MODES.iter().enumerate() {
                            let is_active =
                                std::mem::discriminant(mode) == std::mem::discriminant(&active);
                            let chip_style = hud_text_style(is_active);
                            b.with(El::new().width(Sizing::GROW), |b| {
                                b.text((format!("{} {}", idx + 1, label), chip_style));
                            });
                        }
                    });
                    // Divider.
                    b.with(
                        El::new()
                            .width(Sizing::GROW)
                            .height(Sizing::fixed(Px(1.0)))
                            .background(HUD_DIVIDER_COLOR),
                        |_| {},
                    );
                    // Active mode name.
                    b.with(El::new().width(Sizing::GROW), |b| {
                        b.text((alpha_mode_label(active), active_header));
                    });
                    // Description paragraph.
                    b.with(El::new().width(Sizing::GROW), |b| {
                        b.text((alpha_mode_description(active), body));
                    });
                },
            );
        },
    );
}
