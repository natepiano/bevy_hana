//! @generated `bevy_example_template`
//! OpenType font feature showcase.
//!
//! Demonstrates `FontFeatures` by showing text with each OpenType feature
//! enabled (shaper default) vs explicitly disabled. Loads Noto Sans for
//! `liga` samples and uses the built-in `JetBrains` Mono for `calt`/`kern`.
//! Rendered as a [`DiegeticPanel`] hovering above a visible ground plane.

use bevy::prelude::App;
use bevy::prelude::AssetServer;
use bevy::prelude::Assets;
use bevy::prelude::Changed;
use bevy::prelude::Color;
use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::DefaultPlugins;
use bevy::prelude::DirectionalLight;
use bevy::prelude::Handle;
use bevy::prelude::Mesh;
use bevy::prelude::Mesh3d;
use bevy::prelude::MeshMaterial3d;
use bevy::prelude::Meshable;
use bevy::prelude::On;
use bevy::prelude::Plane3d;
use bevy::prelude::Query;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::Resource;
use bevy::prelude::StandardMaterial;
use bevy::prelude::Startup;
use bevy::prelude::Transform;
use bevy::prelude::Update;
use bevy::prelude::Vec3;
use bevy::prelude::Window;
use bevy::prelude::With;
use bevy::prelude::Without;
use bevy::prelude::default;
use bevy::prelude::error;
use bevy::prelude::info;
use bevy_brp_extras::BrpExtrasPlugin;
use bevy_brp_extras::PortDisplay;
use hana_clerestory::WindowManagerPlugin;
use hana_diegetic::AlignX;
use hana_diegetic::AlignY;
use hana_diegetic::Anchor;
use hana_diegetic::Border;
use hana_diegetic::DiegeticPanel;
use hana_diegetic::DiegeticUiPlugin;
use hana_diegetic::El;
use hana_diegetic::Font;
use hana_diegetic::FontFeatureFlags;
use hana_diegetic::FontFeatures;
use hana_diegetic::FontRegistered;
use hana_diegetic::LayoutBuilder;
use hana_diegetic::Padding;
use hana_diegetic::Pt;
use hana_diegetic::Sizing;
use hana_diegetic::TextStyle;
use hana_lagrange::Focus;
use hana_lagrange::LagrangePlugin;
use hana_lagrange::OrbitAngles;
use hana_lagrange::OrbitCam;
use hana_lagrange::Radius;

/// World-space panel height. Width follows the window aspect ratio.
const PANEL_WORLD_HEIGHT: f32 = 3.1;

/// Space between the panel's bottom edge and the ground plane.
const PANEL_GROUND_CLEARANCE: f32 = 0.45;

/// Base offset used to place the ground plane in front of the origin.
const PANEL_FORWARD_OFFSET: f32 = 1.35;

/// Panel sits this far back from the plane's front edge.
const PANEL_FRONT_DEPTH_FRACTION: f32 = 0.10;

/// Extra plane coverage around the panel.
const GROUND_SIDE_MARGIN: f32 = 1.4;
const GROUND_FRONT_MARGIN: f32 = 0.65;
const GROUND_BACK_MARGIN: f32 = 2.0;

/// Layout height in points (width follows window aspect ratio).
const LAYOUT_HEIGHT: f32 = 792.0;

/// Font size for feature samples (pt).
const SAMPLE_SIZE: f32 = 48.0;

/// Font size for Off/On labels (pt).
const ON_OFF_SIZE: f32 = 14.0;

/// Font size for section headers (pt).
const SECTION_SIZE: f32 = 18.0;

/// Font size for the typeface label shown at the top-right of each cell (pt).
const FONT_NAME_SIZE: f32 = 16.0;
const EB_GARAMOND_REGULAR_FONT_ASSET_PATH: &str = "fonts/EBGaramond-Regular.ttf";

/// Marker for the showcase panel.
#[derive(Component)]
struct ShowcasePanel;

#[derive(Component)]
struct GroundPlane;

/// Keeps font handles alive so Bevy doesn't unload the assets.
#[derive(Resource, Default)]
struct FontHandles(Vec<Handle<Font>>);

/// Tracks the EB Garamond font ID once registered.
#[derive(Resource, Default)]
struct SerifFontId(Option<u16>);

fn build_panel_or_log(
    panel: Result<DiegeticPanel, hana_diegetic::PanelBuildError>,
    label: &str,
) -> Option<DiegeticPanel> {
    match panel {
        Ok(panel) => Some(panel),
        Err(error) => {
            error!("failed to build {label}: {error}");
            None
        },
    }
}

fn main() {
    App::new()
        .add_plugins((
            DefaultPlugins,
            DiegeticUiPlugin,
            LagrangePlugin,
            BrpExtrasPlugin::default().port_in_title(PortDisplay::NonDefault),
            WindowManagerPlugin,
        ))
        .init_resource::<FontHandles>()
        .init_resource::<SerifFontId>()
        .add_observer(on_font_registered)
        .add_systems(Startup, setup)
        .add_systems(Update, resize_panel)
        .run();
}

fn setup(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut font_handles: ResMut<FontHandles>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    windows: Query<&Window>,
    noto_id: Res<SerifFontId>,
) {
    font_handles
        .0
        .push(asset_server.load(EB_GARAMOND_REGULAR_FONT_ASSET_PATH));

    let Ok(window) = windows.single() else {
        return;
    };
    let (layout_width, layout_height) = layout_dimensions(window);
    let world_height = PANEL_WORLD_HEIGHT;
    let world_width = world_height * (layout_width / layout_height);
    let (ground_width, ground_depth) = ground_dimensions(world_width);
    let ground_z = ground_center_z();

    commands.spawn((
        GroundPlane,
        Mesh3d(meshes.add(Plane3d::default().mesh().size(ground_width, ground_depth))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgb(0.08, 0.08, 0.08),
            double_sided: true,
            cull_mode: None,
            ..default()
        })),
        Transform::from_xyz(0.0, 0.0, ground_z),
    ));

    let Some(showcase_panel) = build_panel_or_log(
        DiegeticPanel::world()
            .size(Pt(layout_width), Pt(layout_height))
            .world_height(PANEL_WORLD_HEIGHT)
            .anchor(Anchor::TopLeft)
            .layout(|b| {
                build_panel_content(b, noto_id.0);
            })
            .build(),
        "panel dimensions",
    ) else {
        return;
    };

    commands.spawn((
        ShowcasePanel,
        showcase_panel,
        panel_transform(world_width, world_height),
    ));

    commands.spawn((
        DirectionalLight {
            shadow_maps_enabled: true,
            ..default()
        },
        Transform::from_xyz(1.5, 7.5, 6.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    commands.spawn((
        DirectionalLight {
            shadow_maps_enabled: false,
            ..default()
        },
        Transform::from_xyz(-1.5, 7.5, -6.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));

    commands.spawn((OrbitCam::from_pose(
        Focus(Vec3::new(0.067_647_74, 1.913_066_5, 2.400_296_7)),
        OrbitAngles {
            yaw:   -0.004_848_164,
            pitch: 0.026_128_9,
        },
        Radius(4.385_594_4),
    ),));
}

/// Returns layout dimensions in points, matching the window aspect ratio.
fn layout_dimensions(window: &Window) -> (f32, f32) {
    let aspect = window.width() / window.height();
    (LAYOUT_HEIGHT * aspect, LAYOUT_HEIGHT)
}

fn ground_dimensions(world_width: f32) -> (f32, f32) {
    (
        GROUND_SIDE_MARGIN.mul_add(2.0, world_width),
        GROUND_FRONT_MARGIN + GROUND_BACK_MARGIN + 2.0,
    )
}

fn ground_center_z() -> f32 {
    (GROUND_FRONT_MARGIN - GROUND_BACK_MARGIN).mul_add(0.5, PANEL_FORWARD_OFFSET)
}

fn panel_z() -> f32 {
    let (_, ground_depth) = ground_dimensions(0.0);
    ground_depth.mul_add(0.5 - PANEL_FRONT_DEPTH_FRACTION, ground_center_z())
}

fn panel_transform(world_width: f32, world_height: f32) -> Transform {
    Transform::from_xyz(
        -world_width * 0.5,
        world_height + PANEL_GROUND_CLEARANCE,
        panel_z(),
    )
}

fn resize_panel(
    windows: Query<&Window, Changed<Window>>,
    mut panels: Query<
        (&mut DiegeticPanel, &mut Transform),
        (With<ShowcasePanel>, Without<GroundPlane>),
    >,
    mut ground: Query<(&mut Mesh3d, &mut Transform), (With<GroundPlane>, Without<ShowcasePanel>)>,
    mut meshes: ResMut<Assets<Mesh>>,
    noto_id: Res<SerifFontId>,
) {
    let Ok(window) = windows.single() else {
        return;
    };
    let (layout_width, layout_height) = layout_dimensions(window);
    let world_height = PANEL_WORLD_HEIGHT;
    let world_width = world_height * (layout_width / layout_height);
    let (ground_width, ground_depth) = ground_dimensions(world_width);
    let ground_z = ground_center_z();

    for (mut panel, mut transform) in &mut panels {
        let width_matches = (panel.width() - layout_width).abs() <= f32::EPSILON;
        let height_matches = (panel.height() - layout_height).abs() <= f32::EPSILON;
        if width_matches && height_matches {
            continue;
        }
        let Some(new) = build_panel_or_log(
            DiegeticPanel::world()
                .size(Pt(layout_width), Pt(layout_height))
                .world_height(PANEL_WORLD_HEIGHT)
                .anchor(Anchor::TopLeft)
                .layout(|b| {
                    build_panel_content(b, noto_id.0);
                })
                .build(),
            "panel dimensions",
        ) else {
            return;
        };
        *panel = new;
        *transform = panel_transform(world_width, world_height);
    }

    for (mut mesh3d, mut transform) in &mut ground {
        mesh3d.0 = meshes.add(Plane3d::default().mesh().size(ground_width, ground_depth));
        transform.translation.z = ground_z;
    }
}

fn on_font_registered(
    trigger: On<FontRegistered>,
    mut noto_id: ResMut<SerifFontId>,
    mut panels: Query<&mut DiegeticPanel, With<ShowcasePanel>>,
    windows: Query<&Window>,
) {
    info!(
        "FontRegistered: {} (id: {}, {})",
        trigger.name, trigger.id.0, trigger.source
    );
    noto_id.0 = Some(trigger.id.0);

    let Ok(window) = windows.single() else {
        return;
    };
    let (layout_width, layout_height) = layout_dimensions(window);
    for mut panel in &mut panels {
        let Some(new) = build_panel_or_log(
            DiegeticPanel::world()
                .size(Pt(layout_width), Pt(layout_height))
                .world_height(PANEL_WORLD_HEIGHT)
                .anchor(Anchor::TopLeft)
                .layout(|b| {
                    build_panel_content(b, noto_id.0);
                })
                .build(),
            "panel dimensions",
        ) else {
            return;
        };
        *panel = new;
    }
}

// ── Panel layout ────────────────────────────────────────────────────────────

/// Populates the panel layout. Called from the builder's `.layout()` closure.
/// All spatial values are in points.
fn build_panel_content(b: &mut LayoutBuilder, serif_font_id: Option<u16>) {
    let bg = Color::srgb_u8(40, 40, 45);
    let border_color = Color::srgb_u8(70, 75, 85);
    let column_border_color = Color::srgba(0.75, 0.8, 0.9, 0.3);
    let section_color = Color::srgb(0.55, 0.75, 1.0);
    let label_color = Color::srgba(1.0, 1.0, 1.0, 0.45);
    let on_color = Color::WHITE;
    let off_color = Color::srgb(0.7, 0.7, 0.7);

    b.with(
        El::column()
            .width(Sizing::GROW)
            .height(Sizing::GROW)
            .padding(Padding::all(12.0))
            .gap(8.0)
            .background(bg)
            .border(Border::all(1.0, border_color)),
        |b| {
            b.text((
                "Font Features",
                TextStyle::new(SECTION_SIZE + 4.0).with_color(section_color),
            ));

            let serif_font = serif_font_id.unwrap_or(0);
            let serif_name = if serif_font_id.is_some() {
                "EB Garamond"
            } else {
                "(loading...)"
            };

            build_feature_grid(
                b,
                serif_name,
                serif_font,
                section_color,
                label_color,
                on_color,
                off_color,
                column_border_color,
            );
        },
    );
}

/// Builds the 2x2 feature grid (LIGA|CALT over DLIG|KERN).
fn build_feature_grid(
    b: &mut LayoutBuilder,
    serif_name: &str,
    serif_font: u16,
    section_color: Color,
    label_color: Color,
    on_color: Color,
    off_color: Color,
    column_border_color: Color,
) {
    b.with(
        El::column()
            .width(Sizing::GROW)
            .height(Sizing::GROW)
            .gap(12.0),
        |b| {
            b.with(
                El::row().width(Sizing::GROW).height(Sizing::GROW).gap(12.0),
                |b| {
                    build_feature_column(
                        b,
                        "LIGA — Standard Ligatures",
                        serif_name,
                        serif_font,
                        FontFeatureFlags::LIGA,
                        true,
                        &["fi", "fl", "ffi", "ffl"],
                        section_color,
                        label_color,
                        on_color,
                        off_color,
                        column_border_color,
                    );
                    build_feature_column(
                        b,
                        "CALT — Contextual Alternates",
                        "JetBrains Mono",
                        0,
                        FontFeatureFlags::CALT,
                        true,
                        &["::", "->", "=>", "!="],
                        section_color,
                        label_color,
                        on_color,
                        off_color,
                        column_border_color,
                    );
                },
            );
            b.with(
                El::row().width(Sizing::GROW).height(Sizing::GROW).gap(12.0),
                |b| {
                    build_feature_column(
                        b,
                        "DLIG — Discretionary",
                        serif_name,
                        serif_font,
                        FontFeatureFlags::DLIG,
                        false,
                        &["Th", "st", "ct"],
                        section_color,
                        label_color,
                        on_color,
                        off_color,
                        column_border_color,
                    );
                    build_feature_column(
                        b,
                        "KERN — Kerning",
                        serif_name,
                        serif_font,
                        FontFeatureFlags::KERN,
                        true,
                        &["AVAV", "Type", "Wolf"],
                        section_color,
                        label_color,
                        on_color,
                        off_color,
                        column_border_color,
                    );
                },
            );
        },
    );
}

/// Builds a single feature column: header, font name, then pairs of
/// on/off samples for each test string.
fn build_feature_column(
    b: &mut LayoutBuilder,
    title: &str,
    font_name: &str,
    font_id: u16,
    feature: FontFeatureFlags,
    default_on: bool,
    samples: &[&str],
    section_color: Color,
    label_color: Color,
    on_color: Color,
    off_color: Color,
    column_border_color: Color,
) {
    // "on" explicitly enables the feature, "off" explicitly disables it.
    // For features on by default (liga, calt, kern), "on" = default, "off" = disabled.
    // For features off by default (dlig), "on" = enabled, "off" = default.
    let on_features = if default_on {
        FontFeatures::new()
    } else {
        FontFeatures::new().with(feature)
    };
    let off_features = if default_on {
        FontFeatures::new().without(feature)
    } else {
        FontFeatures::new()
    };

    let on_config = TextStyle::new(SAMPLE_SIZE)
        .with_font(font_id)
        .with_color(on_color)
        .with_font_features(on_features);

    let off_config = TextStyle::new(SAMPLE_SIZE)
        .with_font(font_id)
        .with_color(off_color)
        .with_font_features(off_features);

    let label_config = TextStyle::new(ON_OFF_SIZE).with_color(label_color);
    let font_name_config = TextStyle::new(FONT_NAME_SIZE)
        .with_font(font_id)
        .with_color(label_color);

    b.with(
        El::column()
            .width(Sizing::GROW)
            .height(Sizing::GROW)
            .padding(Padding::all(8.0))
            .gap(4.0)
            .border(Border::all(0.75, column_border_color)),
        |b| {
            b.with(
                El::row()
                    .width(Sizing::GROW)
                    .height(Sizing::FIT)
                    .gap(8.0)
                    .align_y(AlignY::Top),
                |b| {
                    b.text((
                        title,
                        TextStyle::new(SECTION_SIZE).with_color(section_color),
                    ));
                    b.with(
                        El::new()
                            .width(Sizing::GROW)
                            .height(Sizing::FIT)
                            .align_x(AlignX::Right),
                        |b| {
                            b.text((font_name, font_name_config.clone()));
                        },
                    );
                },
            );

            build_sample_rows(b, samples, &label_config, &on_config, &off_config);
        },
    );
}

/// Builds the Off/On header row and per-sample comparison rows.
fn build_sample_rows(
    b: &mut LayoutBuilder,
    samples: &[&str],
    label_config: &TextStyle,
    on_config: &TextStyle,
    off_config: &TextStyle,
) {
    b.with(
        El::column()
            .width(Sizing::GROW)
            .height(Sizing::GROW)
            .gap(0.0),
        |b| {
            b.with(
                El::row()
                    .width(Sizing::GROW)
                    .height(Sizing::percent(0.11))
                    .padding(Padding::new(6.0, 4.0, 0.0, 4.0))
                    .gap(10.0)
                    .align_y(AlignY::Bottom),
                |b| {
                    b.with(
                        El::new()
                            .width(Sizing::GROW)
                            .height(Sizing::FIT)
                            .alignment(AlignX::Center, AlignY::Top),
                        |b| {
                            b.text(("Off", label_config.clone()));
                        },
                    );
                    b.with(
                        El::new()
                            .width(Sizing::GROW)
                            .height(Sizing::FIT)
                            .alignment(AlignX::Center, AlignY::Top),
                        |b| {
                            b.text(("On", label_config.clone()));
                        },
                    );
                },
            );

            for &sample in samples {
                b.with(
                    El::row()
                        .width(Sizing::GROW)
                        .height(Sizing::GROW)
                        .padding(Padding::new(0.0, 4.0, 0.0, 4.0))
                        .gap(10.0),
                    |b| {
                        b.with(
                            El::new()
                                .width(Sizing::GROW)
                                .height(Sizing::GROW)
                                .alignment(AlignX::Center, AlignY::Center),
                            |b| {
                                b.text((sample, off_config.clone()));
                            },
                        );
                        b.with(
                            El::new()
                                .width(Sizing::GROW)
                                .height(Sizing::GROW)
                                .alignment(AlignX::Center, AlignY::Center),
                            |b| {
                                b.text((sample, on_config.clone()));
                            },
                        );
                    },
                );
            }
        },
    );
}
