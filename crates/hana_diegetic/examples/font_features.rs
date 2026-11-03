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
use bevy::prelude::PerspectiveProjection;
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
use hana_diegetic::CornerRadius;
use hana_diegetic::DiegeticPanel;
use hana_diegetic::DiegeticTextMeasurer;
use hana_diegetic::DiegeticUiPlugin;
use hana_diegetic::El;
use hana_diegetic::Font;
use hana_diegetic::FontFeatureFlags;
use hana_diegetic::FontFeatures;
use hana_diegetic::FontId;
use hana_diegetic::FontRegistered;
use hana_diegetic::FontRegistry;
use hana_diegetic::LayoutBuilder;
use hana_diegetic::Lighting;
use hana_diegetic::Padding;
use hana_diegetic::Pt;
use hana_diegetic::Sizing;
use hana_diegetic::TextInkMeasurement;
use hana_diegetic::TextStyle;
use hana_lagrange::Focus;
use hana_lagrange::LagrangePlugin;
use hana_lagrange::OrbitAngles;
use hana_lagrange::OrbitCam;
use hana_lagrange::OrbitCamHomePose;
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

/// Reference layout height; content determines the actual panel height.
const REFERENCE_LAYOUT_HEIGHT: f32 = 800.0;
const HOME_WINDOW_HEIGHT: f32 = 720.0;
const HOME_FOCUS_Z: f32 = 2.400_296_7;
const HOME_RADIUS: f32 = 4.385_594_4;
const BORDER_LOGICAL_PIXELS: f32 = 1.5;
const OUTER_RADIUS_LOGICAL_PIXELS: f32 = 12.0;
const OUTER_MARGIN: f32 = 12.0;

/// Font size for feature samples (pt).
const SAMPLE_SIZE: f32 = 48.0;

/// Font size for Off/On labels (pt).
const ON_OFF_SIZE: f32 = 14.0;

/// Font size for section headers (pt).
const SECTION_SIZE: f32 = 18.0;

/// Font size for the typeface label shown at the top-right of each cell (pt).
const FONT_NAME_SIZE: f32 = 16.0;
const CARD_MARGIN: f32 = 12.0;
const SAMPLE_ROW_PITCH: f32 = 80.0;
const SAMPLE_ROW_BOTTOM_INSET: f32 = 4.0;
const EB_GARAMOND_REGULAR_FONT_ASSET_PATH: &str = "fonts/EBGaramond-Regular.ttf";
const EB_GARAMOND_FAMILY: &str = "EB Garamond";

/// Marker for the showcase panel.
#[derive(Component)]
struct ShowcasePanel;

#[derive(Component)]
struct GroundPlane;

/// Holds the loaded font handles so Bevy does not unload the assets.
#[derive(Resource, Default)]
struct FontHandles(Vec<Handle<Font>>);

/// Whether EB Garamond is ready for the feature grid.
#[derive(Resource, Clone, Copy, Default)]
enum SerifFont {
    #[default]
    Pending,
    Registered(FontId),
}

struct ShowcaseSize {
    layout_width:  f32,
    layout_height: f32,
    world_width:   f32,
    world_height:  f32,
}

impl ShowcaseSize {
    fn for_window(
        window: &Window,
        serif_font: SerifFont,
        fonts: &FontRegistry,
        text_measurer: &DiegeticTextMeasurer,
    ) -> Self {
        let layout_width = REFERENCE_LAYOUT_HEIGHT * window.width() / window.height();
        let mut builder = LayoutBuilder::with_root(
            El::new()
                .width(Sizing::fixed(layout_width))
                .height(Sizing::FIT),
        );
        build_panel_content(&mut builder, serif_font, fonts, text_measurer);
        let layout_height = builder.build().measured_height_at_width(
            layout_width,
            1.0,
            text_measurer.measure_fn.clone(),
        );
        let world_height = PANEL_WORLD_HEIGHT * layout_height / REFERENCE_LAYOUT_HEIGHT;
        let world_width = PANEL_WORLD_HEIGHT * layout_width / REFERENCE_LAYOUT_HEIGHT;
        Self {
            layout_width,
            layout_height,
            world_width,
            world_height,
        }
    }
}

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
        .insert_resource(Lighting::Unlit)
        .init_resource::<FontHandles>()
        .init_resource::<SerifFont>()
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
    serif_font: Res<SerifFont>,
    fonts: Res<FontRegistry>,
    text_measurer: Res<DiegeticTextMeasurer>,
) {
    font_handles
        .0
        .push(asset_server.load(EB_GARAMOND_REGULAR_FONT_ASSET_PATH));

    let Ok(window) = windows.single() else {
        return;
    };
    let size = ShowcaseSize::for_window(window, *serif_font, &fonts, &text_measurer);
    let (ground_width, ground_depth) = ground_dimensions(size.world_width);
    let ground_z = ground_center_z();

    commands.spawn((
        GroundPlane,
        Mesh3d(meshes.add(Plane3d::default().mesh().size(ground_width, ground_depth))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgb(0.08, 0.08, 0.08),
            reflectance: 0.0,
            perceptual_roughness: 1.0,
            double_sided: true,
            cull_mode: None,
            ..default()
        })),
        Transform::from_xyz(0.0, 0.0, ground_z),
    ));

    let Some(showcase_panel) = build_panel_or_log(
        DiegeticPanel::world()
            .size(Pt(size.layout_width), Pt(size.layout_height))
            .world_height(size.world_height)
            .anchor(Anchor::TopLeft)
            .layout(|b| {
                build_panel_content(b, *serif_font, &fonts, &text_measurer);
            })
            .build(),
        "panel dimensions",
    ) else {
        return;
    };

    commands.spawn((
        ShowcasePanel,
        showcase_panel,
        panel_transform(size.world_width, size.world_height),
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

    let focus = home_focus(size.world_height);
    let angles = OrbitAngles {
        yaw:   0.0,
        pitch: 0.0,
    };
    let radius = Radius(HOME_RADIUS);
    commands.spawn((
        OrbitCam::from_pose(focus, angles, radius),
        OrbitCamHomePose {
            orbit: angles,
            pan:   focus,
            zoom:  radius,
        },
    ));
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

fn home_points_per_logical_pixel() -> f32 {
    let distance = HOME_FOCUS_Z + HOME_RADIUS - panel_z();
    let half_fov = PerspectiveProjection::default().fov * 0.5;
    let visible_world_height = 2.0 * distance * half_fov.tan();
    REFERENCE_LAYOUT_HEIGHT * visible_world_height / (PANEL_WORLD_HEIGHT * HOME_WINDOW_HEIGHT)
}

fn outer_corner_radius() -> f32 { OUTER_RADIUS_LOGICAL_PIXELS * home_points_per_logical_pixel() }

fn card_corner_radius() -> f32 {
    BORDER_LOGICAL_PIXELS
        .mul_add(
            -home_points_per_logical_pixel(),
            outer_corner_radius() - OUTER_MARGIN,
        )
        .max(0.0)
}

fn panel_transform(world_width: f32, world_height: f32) -> Transform {
    Transform::from_xyz(
        -world_width * 0.5,
        world_height + PANEL_GROUND_CLEARANCE,
        panel_z(),
    )
}

const fn home_focus(world_height: f32) -> Focus {
    Focus(Vec3::new(
        0.0,
        world_height.mul_add(0.5, PANEL_GROUND_CLEARANCE),
        HOME_FOCUS_Z,
    ))
}

fn resize_panel(
    windows: Query<&Window, Changed<Window>>,
    mut panels: Query<
        (&mut DiegeticPanel, &mut Transform),
        (With<ShowcasePanel>, Without<GroundPlane>),
    >,
    mut ground: Query<(&mut Mesh3d, &mut Transform), (With<GroundPlane>, Without<ShowcasePanel>)>,
    mut meshes: ResMut<Assets<Mesh>>,
    serif_font: Res<SerifFont>,
    fonts: Res<FontRegistry>,
    text_measurer: Res<DiegeticTextMeasurer>,
    mut home_poses: Query<&mut OrbitCamHomePose>,
) {
    let Ok(window) = windows.single() else {
        return;
    };
    let size = ShowcaseSize::for_window(window, *serif_font, &fonts, &text_measurer);
    let (ground_width, ground_depth) = ground_dimensions(size.world_width);
    let ground_z = ground_center_z();

    for (mut panel, mut transform) in &mut panels {
        let width_matches = (panel.width() - size.layout_width).abs() <= f32::EPSILON;
        let height_matches = (panel.height() - size.layout_height).abs() <= f32::EPSILON;
        if width_matches && height_matches {
            continue;
        }
        let Some(new) = build_panel_or_log(
            DiegeticPanel::world()
                .size(Pt(size.layout_width), Pt(size.layout_height))
                .world_height(size.world_height)
                .anchor(Anchor::TopLeft)
                .layout(|b| {
                    build_panel_content(b, *serif_font, &fonts, &text_measurer);
                })
                .build(),
            "panel dimensions",
        ) else {
            return;
        };
        *panel = new;
        *transform = panel_transform(size.world_width, size.world_height);
    }

    for (mut mesh3d, mut transform) in &mut ground {
        mesh3d.0 = meshes.add(Plane3d::default().mesh().size(ground_width, ground_depth));
        transform.translation.z = ground_z;
    }
    for mut home in &mut home_poses {
        home.pan = home_focus(size.world_height);
    }
}

fn on_font_registered(
    trigger: On<FontRegistered>,
    mut serif_font: ResMut<SerifFont>,
    mut panels: Query<(&mut DiegeticPanel, &mut Transform), With<ShowcasePanel>>,
    mut cameras: Query<(&mut OrbitCam, &mut OrbitCamHomePose)>,
    windows: Query<&Window>,
    fonts: Res<FontRegistry>,
    text_measurer: Res<DiegeticTextMeasurer>,
) {
    info!(
        "FontRegistered: {} (id: {}, {})",
        trigger.name, trigger.id.0, trigger.source
    );
    if trigger.name != EB_GARAMOND_FAMILY {
        return;
    }
    *serif_font = SerifFont::Registered(trigger.id);

    let Ok(window) = windows.single() else {
        return;
    };
    let size = ShowcaseSize::for_window(window, *serif_font, &fonts, &text_measurer);
    for (mut panel, mut transform) in &mut panels {
        let Some(new) = build_panel_or_log(
            DiegeticPanel::world()
                .size(Pt(size.layout_width), Pt(size.layout_height))
                .world_height(size.world_height)
                .anchor(Anchor::TopLeft)
                .layout(|b| build_panel_content(b, *serif_font, &fonts, &text_measurer))
                .build(),
            "panel after font registration",
        ) else {
            return;
        };
        *panel = new;
        *transform = panel_transform(size.world_width, size.world_height);
    }
    for (mut camera, mut home) in &mut cameras {
        let focus = home_focus(size.world_height);
        camera.pan.snap_to(focus);
        home.pan = focus;
    }
}

// ── Panel layout ────────────────────────────────────────────────────────────

/// Populates the panel layout. Called from the builder's `.layout()` closure.
/// All spatial values are in points.
fn build_panel_content(
    b: &mut LayoutBuilder,
    serif_font: SerifFont,
    fonts: &FontRegistry,
    text_measurer: &DiegeticTextMeasurer,
) {
    let bg = Color::srgb_u8(46, 52, 64);
    let border_color = Color::srgba(129.0 / 255.0, 161.0 / 255.0, 193.0 / 255.0, 0.85);
    let column_border_color = Color::srgba(0.75, 0.8, 0.9, 0.3);
    let section_color = Color::srgb(0.55, 0.75, 1.0);
    let label_color = Color::srgba(1.0, 1.0, 1.0, 0.65);
    let on_color = Color::WHITE;
    let off_color = Color::srgb(0.7, 0.7, 0.7);
    let title_top_padding = match fonts.measure_ink(
        "Font features",
        &TextStyle::new(SECTION_SIZE + 4.0).as_measure(),
    ) {
        Ok(TextInkMeasurement::Visible(ink)) => OUTER_MARGIN + ink.left - ink.top,
        Ok(TextInkMeasurement::NoVisibleInk) | Err(_) => OUTER_MARGIN,
    };

    b.with(
        El::column()
            .width(Sizing::GROW)
            .height(Sizing::FIT)
            .padding(Padding::new(
                OUTER_MARGIN,
                OUTER_MARGIN,
                title_top_padding,
                OUTER_MARGIN,
            ))
            .gap(8.0)
            .background(bg)
            .corner_radius(CornerRadius::all(outer_corner_radius()))
            // Convert the shared slate outline's logical-pixel weight to panel points.
            .border(Border::all(
                BORDER_LOGICAL_PIXELS * home_points_per_logical_pixel(),
                border_color,
            )),
        |b| {
            b.text((
                "Font features",
                TextStyle::new(SECTION_SIZE + 4.0).with_color(section_color),
            ));

            let (serif_name, serif_font_id) = match serif_font {
                SerifFont::Pending => ("(loading...)", FontId::MONOSPACE),
                SerifFont::Registered(id) => (EB_GARAMOND_FAMILY, id),
            };

            build_feature_grid(
                b,
                serif_name,
                serif_font_id,
                section_color,
                label_color,
                on_color,
                off_color,
                column_border_color,
                fonts,
                text_measurer,
            );
        },
    );
}

/// Builds the 2x2 feature grid (LIGA|CALT over DLIG|KERN).
fn build_feature_grid(
    b: &mut LayoutBuilder,
    serif_name: &str,
    serif_font: FontId,
    section_color: Color,
    label_color: Color,
    on_color: Color,
    off_color: Color,
    column_border_color: Color,
    fonts: &FontRegistry,
    text_measurer: &DiegeticTextMeasurer,
) {
    b.with(
        El::column()
            .width(Sizing::GROW)
            .height(Sizing::FIT)
            .gap(12.0),
        |b| {
            b.with(
                El::row().width(Sizing::GROW).height(Sizing::FIT).gap(12.0),
                |b| {
                    build_feature_column(
                        b,
                        "Standard ligatures",
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
                        fonts,
                        text_measurer,
                    );
                    build_feature_column(
                        b,
                        "Code ligatures",
                        "JetBrains Mono",
                        FontId::MONOSPACE,
                        FontFeatureFlags::CALT,
                        true,
                        &["::", "->", "=>", "!="],
                        section_color,
                        label_color,
                        on_color,
                        off_color,
                        column_border_color,
                        fonts,
                        text_measurer,
                    );
                },
            );
            b.with(
                El::row().width(Sizing::GROW).height(Sizing::FIT).gap(12.0),
                |b| {
                    build_feature_column(
                        b,
                        "Discretionary ligatures",
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
                        fonts,
                        text_measurer,
                    );
                    build_feature_column(
                        b,
                        "Kerning",
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
                        fonts,
                        text_measurer,
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
    font_id: FontId,
    feature: FontFeatureFlags,
    default_on: bool,
    samples: &[&str],
    section_color: Color,
    label_color: Color,
    on_color: Color,
    off_color: Color,
    column_border_color: Color,
    fonts: &FontRegistry,
    text_measurer: &DiegeticTextMeasurer,
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
        .with_font(font_id.0)
        .with_color(on_color)
        .with_font_features(on_features);

    let off_config = TextStyle::new(SAMPLE_SIZE)
        .with_font(font_id.0)
        .with_color(off_color)
        .with_font_features(off_features);

    let label_config = TextStyle::new(ON_OFF_SIZE).with_color(label_color);
    let font_name_config = TextStyle::new(FONT_NAME_SIZE)
        .with_font(font_id.0)
        .with_color(label_color);
    let (title_top_padding, title_left_ink) =
        match fonts.measure_ink(title, &TextStyle::new(SECTION_SIZE).as_measure()) {
            Ok(TextInkMeasurement::Visible(ink)) => (CARD_MARGIN + ink.left - ink.top, ink.left),
            Ok(TextInkMeasurement::NoVisibleInk) | Err(_) => (CARD_MARGIN, 0.0),
        };
    let sample_measure = on_config.as_measure();
    let last_sample = samples.last().copied().unwrap_or("");
    let last_line_height = (text_measurer.measure_fn)(last_sample, &sample_measure).line_height;
    let bottom_padding = match fonts.measure_ink(last_sample, &sample_measure) {
        Ok(TextInkMeasurement::Visible(ink)) => {
            (CARD_MARGIN + title_left_ink - (last_line_height - ink.bottom)).max(0.0)
        },
        Ok(TextInkMeasurement::NoVisibleInk) | Err(_) => CARD_MARGIN,
    };

    b.with(
        El::column()
            .width(Sizing::GROW)
            .height(Sizing::FIT)
            .padding(Padding::new(
                CARD_MARGIN,
                CARD_MARGIN,
                title_top_padding,
                bottom_padding,
            ))
            .gap(4.0)
            .corner_radius(CornerRadius::all(card_corner_radius()))
            .border(Border::all(0.75, column_border_color)),
        |b| {
            b.with(
                El::row()
                    .width(Sizing::GROW)
                    .height(Sizing::FIT)
                    .gap(8.0)
                    .align_y(AlignY::Bottom),
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
            .height(Sizing::FIT)
            .gap(0.0),
        |b| {
            b.with(
                El::row()
                    .width(Sizing::GROW)
                    .height(Sizing::FIT)
                    .padding(Padding::new(6.0, 4.0, 6.0, 4.0))
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

            for (index, &sample) in samples.iter().enumerate() {
                let last_row = index + 1 == samples.len();
                let row_height = if last_row {
                    SAMPLE_ROW_PITCH - SAMPLE_ROW_BOTTOM_INSET
                } else {
                    SAMPLE_ROW_PITCH
                };
                let bottom_inset = if last_row {
                    0.0
                } else {
                    SAMPLE_ROW_BOTTOM_INSET
                };
                b.with(
                    El::row()
                        .width(Sizing::GROW)
                        .height(Sizing::fixed(row_height))
                        .padding(Padding::new(0.0, 4.0, 0.0, bottom_inset))
                        .gap(10.0)
                        .align_y(AlignY::Bottom),
                    |b| {
                        b.with(
                            El::new()
                                .width(Sizing::GROW)
                                .height(Sizing::FIT)
                                .alignment(AlignX::Center, AlignY::Top),
                            |b| {
                                b.text((sample, off_config.clone()));
                            },
                        );
                        b.with(
                            El::new()
                                .width(Sizing::GROW)
                                .height(Sizing::FIT)
                                .alignment(AlignX::Center, AlignY::Top),
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
