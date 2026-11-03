//! Small panel glyphs are checked at final, composited pixels.

mod drawn_support;

use std::cell::RefCell;
use std::sync::mpsc;

use bevy::camera::Hdr;
use bevy::camera::NormalizedRenderTarget;
use bevy::camera::RenderTarget;
use bevy::picking::backend::HitData;
use bevy::picking::pointer::Location;
use bevy::picking::pointer::PointerId;
use bevy::post_process::bloom::Bloom;
use bevy::post_process::bloom::BloomPrefilter;
use bevy::prelude::*;
use bevy::render::render_resource::TextureFormat;
use bevy::render::view::Tonemapping;
use bevy::window::RequestRedraw;
use bevy::window::WindowRef;
use drawn_support::CaptureRegion;
use drawn_support::CaseApp;
use drawn_support::DrawnCase;
use hana_diegetic::AlignX;
use hana_diegetic::AlignY;
use hana_diegetic::Anchor;
use hana_diegetic::CascadeEntityCommandsExt as _;
use hana_diegetic::ComputedDiegeticPanel;
use hana_diegetic::DiegeticPanel;
use hana_diegetic::DiegeticPerfStats;
use hana_diegetic::DiegeticText;
use hana_diegetic::DisplayCompensationReadiness;
use hana_diegetic::El;
use hana_diegetic::GlyphShadowMode;
use hana_diegetic::ImeBuiltInFieldKind;
use hana_diegetic::ImeBuiltInFieldSpec;
use hana_diegetic::ImeCursorState;
use hana_diegetic::ImeEditableFieldSpec;
use hana_diegetic::ImeStarted;
use hana_diegetic::ImeTextChanged;
use hana_diegetic::LayoutBuilder;
use hana_diegetic::Lighting;
use hana_diegetic::Mm;
use hana_diegetic::Padding;
use hana_diegetic::PaperSize;
use hana_diegetic::Pt;
use hana_diegetic::ShadowCasting;
use hana_diegetic::Sizing;
use hana_diegetic::StableTransparency;
use hana_diegetic::TextContent;
use hana_diegetic::TextStyle;
use hana_diegetic::Unit;

const SIDE: u32 = 256;
const SIDE_F32: f32 = 256.0;
const DARK_FILL: [u8; 3] = [24, 32, 40];
const LIGHT_INK: [u8; 3] = [232, 237, 241];
const WHITE: [u8; 3] = [255, 255, 255];
const BLACK: [u8; 3] = [0, 0, 0];

thread_local! {
    static DUMP_CONTEXT: RefCell<String> = const { RefCell::new(String::new()) };
    static DUMP_REGION: RefCell<CaptureRegion> = const { RefCell::new(CaptureRegion {
        full_size: UVec2::new(SIDE, SIDE),
        origin: UVec2::ZERO,
        size: UVec2::new(SIDE, SIDE),
    }) };
}

#[derive(Clone, Copy, Debug)]
enum DrawingPath {
    Standard,
    Stable,
}

#[derive(Clone, Copy, Debug)]
struct GlyphScene {
    text:          &'static str,
    fill:          [u8; 3],
    ink:           [u8; 3],
    font_mm:       f32,
    pixels_per_em: f32,
    offset_px:     f32,
    offset_y_px:   f32,
    tilt_degrees:  f32,
    alpha_mode:    AlphaMode,
    drawing_path:  DrawingPath,
}

impl GlyphScene {
    fn camera_distance(self) -> f32 {
        self.font_mm * 0.001 * SIDE_F32
            / (2.0 * (std::f32::consts::FRAC_PI_8).tan() * self.pixels_per_em)
    }

    fn panel_bounds(self) -> (u32, u32, u32, u32) {
        let radius = 50.0 * self.pixels_per_em / self.font_mm;
        let center_x = SIDE_F32 / 2.0 + self.offset_px;
        let center_y = SIDE_F32 / 2.0 - self.offset_y_px;
        (
            first_pixel_at_or_after(center_x - radius),
            first_pixel_at_or_after(center_y - radius),
            last_pixel_at_or_before(center_x + radius),
            last_pixel_at_or_before(center_y + radius),
        )
    }

    fn text_bounds(self) -> (u32, u32, u32, u32) {
        let (left, top, panel_right, panel_bottom) = self.panel_bounds();
        let character_count =
            f32::from(u16::try_from(self.text.chars().count()).unwrap_or(u16::MAX));
        let text_width =
            pixel_count_before((character_count * self.pixels_per_em).mul_add(0.75, 4.0));
        let text_height = pixel_count_before(self.pixels_per_em.mul_add(1.25, 4.0));
        (
            left,
            top,
            (left + text_width).min(panel_right),
            (top + text_height).min(panel_bottom),
        )
    }
}

fn pixel_center(index: u32) -> f32 { f32::from(u16::try_from(index).unwrap_or(u16::MAX)) + 0.5 }

fn first_pixel_at_or_after(edge: f32) -> u32 {
    (0..SIDE)
        .find(|&index| pixel_center(index) >= edge)
        .unwrap_or(SIDE - 1)
}

fn last_pixel_at_or_before(edge: f32) -> u32 {
    (0..SIDE)
        .rev()
        .find(|&index| pixel_center(index) <= edge)
        .unwrap_or(0)
}

fn pixel_count_before(width: f32) -> u32 {
    (0..SIDE)
        .take_while(|&index| f32::from(u16::try_from(index).unwrap_or(u16::MAX)) < width)
        .last()
        .map_or(0, |index| index + 1)
}

struct DrawnPixels(Vec<u8>);

impl DrawnPixels {
    fn rgb(&self, x: u32, y: u32) -> [u8; 3] {
        let index = ((y * SIDE + x) * 4) as usize;
        [self.0[index], self.0[index + 1], self.0[index + 2]]
    }

    fn ink_bounds(&self, scene: GlyphScene) -> Result<(u32, u32, u32, u32), String> {
        let mut bounds = (SIDE, SIDE, 0, 0);
        let mut count = 0;
        let (text_left, text_top, text_right, text_bottom) = scene.text_bounds();
        for y in text_top..=text_bottom {
            for x in text_left..=text_right {
                let pixel = self.rgb(x, y);
                if closer_to(pixel, scene.ink, scene.fill) {
                    bounds.0 = bounds.0.min(x);
                    bounds.1 = bounds.1.min(y);
                    bounds.2 = bounds.2.max(x);
                    bounds.3 = bounds.3.max(y);
                    count += 1;
                }
            }
        }
        if count == 0 {
            return Err("text drew no ink pixels".to_string());
        }
        Ok(bounds)
    }
}

fn color_distance(pixel: [u8; 3], target: [u8; 3]) -> u16 {
    pixel
        .into_iter()
        .zip(target)
        .map(|(actual, expected)| u16::from(actual.abs_diff(expected)))
        .sum()
}

fn closer_to(pixel: [u8; 3], first: [u8; 3], second: [u8; 3]) -> bool {
    color_distance(pixel, first) < color_distance(pixel, second)
}

fn setup_scene(scene: GlyphScene) -> Result<(CaseApp, Handle<Image>), String> {
    setup_scene_with_panel_width(scene, 100.0)
}

fn setup_scene_with_panel_width(
    scene: GlyphScene,
    panel_width_mm: f32,
) -> Result<(CaseApp, Handle<Image>), String> {
    setup_scene_with_panel_width_and_pitch(scene, panel_width_mm, 0.0)
}

fn setup_scene_with_panel_width_and_pitch(
    scene: GlyphScene,
    panel_width_mm: f32,
    camera_pitch_rad: f32,
) -> Result<(CaseApp, Handle<Image>), String> {
    let region = if matches!(
        drawn_support::current_case_name(),
        "zero_dot_is_absent_through_nine_pixels_per_em_and_draws_at_ten"
            | "small_dark_glyph_open_spaces_have_no_isolated_black_pixel"
    ) {
        CaptureRegion::new(UVec2::splat(SIDE), UVec2::ZERO, UVec2::splat(128))
    } else {
        CaptureRegion::new(UVec2::splat(SIDE), UVec2::ZERO, UVec2::splat(SIDE))
    };
    DUMP_REGION.with(|dump_region| *dump_region.borrow_mut() = region);
    DUMP_CONTEXT.with(|context| {
        *context.borrow_mut() = format!(
            "pixels_per_em={} phase=({},{}) pitch={} scene={scene:?}",
            scene.pixels_per_em,
            scene.offset_px.fract(),
            scene.offset_y_px.fract(),
            camera_pitch_rad,
        );
    });
    let mut app = CaseApp::new();
    app.world_mut().insert_resource(Lighting::Unlit);
    app.insert_resource(ClearColor(Color::BLACK));

    let target = app.target(region);
    let bloom = Bloom {
        prefilter: BloomPrefilter {
            threshold:          1.0,
            threshold_softness: 0.1,
        },
        ..default()
    };
    app.world_mut().spawn((
        Camera3d::default(),
        RenderTarget::Window(WindowRef::Primary),
        Hdr,
        Tonemapping::KhronosPbrNeutral,
        bloom.clone(),
    ));
    let mut image_camera = Camera::default();
    region.apply_to_camera(&mut image_camera);
    let camera = app
        .world_mut()
        .spawn((
            Camera3d::default(),
            image_camera,
            RenderTarget::Image(target.clone().into()),
            Hdr,
            Tonemapping::KhronosPbrNeutral,
            bloom,
            Msaa::Off,
            Transform::from_xyz(
                0.0,
                scene.camera_distance() * camera_pitch_rad.sin(),
                scene.camera_distance() * camera_pitch_rad.cos(),
            )
            .looking_at(Vec3::ZERO, Vec3::Y),
        ))
        .id();
    if matches!(scene.drawing_path, DrawingPath::Stable) {
        app.world_mut()
            .entity_mut(camera)
            .insert(StableTransparency);
    }

    spawn_glyph_panel(
        &mut app,
        scene,
        panel_width_mm,
        100.0,
        Vec2::new(scene.offset_px, scene.offset_y_px),
        camera_pitch_rad,
    )?;

    Ok((app, target))
}

fn spawn_glyph_panel(
    app: &mut App,
    scene: GlyphScene,
    panel_width_mm: f32,
    panel_height_mm: f32,
    center_px: Vec2,
    camera_pitch_rad: f32,
) -> Result<(), String> {
    let mut layout = LayoutBuilder::new(panel_width_mm, panel_height_mm);
    layout.with(
        El::new()
            .width(Sizing::grow_min(0.0))
            .height(Sizing::grow_min(0.0))
            .background(Color::srgb_u8(scene.fill[0], scene.fill[1], scene.fill[2])),
        |builder| {
            builder.text((
                scene.text,
                TextStyle::new(Mm(scene.font_mm))
                    .with_color(Color::srgb_u8(scene.ink[0], scene.ink[1], scene.ink[2]))
                    .with_shadow_mode(GlyphShadowMode::None)
                    .with_alpha_mode(scene.alpha_mode),
            ));
        },
    );
    let panel = DiegeticPanel::world()
        .size(Mm(panel_width_mm), Mm(panel_height_mm))
        .anchor(Anchor::Center)
        .with_tree(layout.build())
        .build()
        .map_err(|error| format!("invalid text panel: {error}"))?;
    let pixel_world = scene.font_mm * 0.001 / scene.pixels_per_em;
    let world_y = center_px.y * pixel_world * camera_pitch_rad.cos();
    let panel = app
        .world_mut()
        .spawn((
            panel,
            Transform::from_xyz(
                center_px.x * pixel_world,
                world_y,
                -world_y * camera_pitch_rad.tan(),
            )
            .with_rotation(Quat::from_rotation_y(scene.tilt_degrees.to_radians())),
        ))
        .id();
    app.world_mut()
        .commands()
        .entity(panel)
        .override_lighting(Lighting::Unlit);

    Ok(())
}

fn render(scene: GlyphScene) -> Result<DrawnPixels, String> {
    let (app, target) = setup_scene(scene)?;
    read_scene(app, target)
}

fn render_with_panel_width(scene: GlyphScene, panel_width_mm: f32) -> Result<DrawnPixels, String> {
    let (app, target) = setup_scene_with_panel_width(scene, panel_width_mm)?;
    read_scene(app, target)
}

fn read_scene(app: CaseApp, target: Handle<Image>) -> Result<DrawnPixels, String> {
    let region = DUMP_REGION.with(|region| *region.borrow());
    read_scene_bytes(app, target).map(|pixels| {
        if region.size == region.full_size {
            return DrawnPixels(pixels);
        }
        let mut full = vec![0_u8; (region.full_size.x * region.full_size.y * 4) as usize];
        for y in 0..region.size.y {
            let source = (y * region.size.x * 4) as usize;
            let target =
                (((y + region.origin.y) * region.full_size.x + region.origin.x) * 4) as usize;
            let row = (region.size.x * 4) as usize;
            full[target..target + row].copy_from_slice(&pixels[source..source + row]);
        }
        DrawnPixels(full)
    })
}

fn a4_article_column(
    builder: &mut LayoutBuilder,
    heading: &'static str,
    paragraphs: &[&'static str],
) {
    builder.with(El::column().width(Sizing::GROW).gap(Mm(4.0)), |builder| {
        builder.with(El::new().width(Sizing::GROW), |builder| {
            builder.text((heading, TextStyle::new(Pt(18.0)).with_color(Color::BLACK)));
        });
        for paragraph in paragraphs {
            builder.with(El::new().width(Sizing::GROW), |builder| {
                builder.text((
                    *paragraph,
                    TextStyle::new(Pt(12.0)).with_color(Color::BLACK),
                ));
            });
        }
    });
}

const fn a4_body_region(scale_factor: u32) -> CaptureRegion {
    // Small-text fitting reads the target's physical viewport, so a sub-view
    // changes its sampled glyph edges even when the projection is cropped.
    let size = UVec2::new(900 * scale_factor, 800 * scale_factor);
    CaptureRegion::new(size, UVec2::ZERO, size)
}

fn setup_units_a4_scene(scale_factor: u32) -> (CaseApp, Handle<Image>, CaptureRegion) {
    DUMP_REGION.with(|dump_region| *dump_region.borrow_mut() = a4_body_region(scale_factor));
    DUMP_CONTEXT.with(|context| {
        *context.borrow_mut() =
            format!("pixels_per_em=point_sizes_vary scale_factor={scale_factor} phase=(0,0)");
    });
    let mut app = CaseApp::new();
    app.world_mut().insert_resource(Lighting::Unlit);
    app.insert_resource(ClearColor(Color::BLACK));
    let region = a4_body_region(scale_factor);
    let target = app.target(region);
    let bloom = Bloom {
        prefilter: BloomPrefilter {
            threshold:          1.0,
            threshold_softness: 0.1,
        },
        ..default()
    };
    app.world_mut().spawn((
        Camera3d::default(),
        RenderTarget::Window(WindowRef::Primary),
        Hdr,
        Tonemapping::KhronosPbrNeutral,
        bloom.clone(),
    ));
    let mut camera = Camera::default();
    region.apply_to_camera(&mut camera);
    app.world_mut().spawn((
        Camera3d::default(),
        camera,
        RenderTarget::Image(target.clone().into()),
        Hdr,
        Tonemapping::KhronosPbrNeutral,
        bloom,
        Msaa::Off,
        Transform::from_xyz(0.0, 0.07, 0.7).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    (app, target, region)
}

#[expect(
    clippy::too_many_lines,
    reason = "the fixture mirrors the complete A4 article layout"
)]
fn render_units_a4_article(scale_factor: u32) -> Result<(Vec<u8>, CaptureRegion), String> {
    let (mut app, target, region) = setup_units_a4_scene(scale_factor);
    let mut layout = LayoutBuilder::new(Mm(210.0), Mm(297.0));
    layout.with(
        El::column()
            .size(Mm(210.0), Mm(297.0))
            .padding(Padding::all(Mm(15.0)))
            .gap(Mm(4.0))
            .background(Color::WHITE),
        |builder| {
            builder.with(
                El::row()
                    .width(Sizing::GROW)
                    .gap(Mm(6.0))
                    .align_y(AlignY::Bottom),
                |builder| {
                    for (label, size) in [
                        ("72pt", 72.0),
                        ("36pt", 36.0),
                        ("24pt", 24.0),
                        ("18pt", 18.0),
                        ("12pt", 12.0),
                        ("9pt", 9.0),
                    ] {
                        builder.with(El::new().width(Sizing::GROW), |builder| {
                            builder.text((
                                label,
                                TextStyle::new(Pt(size)).with_color(Color::BLACK),
                            ));
                        });
                    }
                },
            );
            builder.with(
                El::new()
                    .width(Sizing::GROW)
                    .height(Sizing::fixed(Mm(0.3)))
                    .background(Color::srgba(0.0, 0.0, 0.1, 1.0)),
                |_| {},
            );
            builder.with(
                El::row()
                    .width(Sizing::GROW)
                    .height(Sizing::GROW)
                    .gap(Mm(8.0)),
                |builder| {
                    a4_article_column(
                        builder,
                        "Real Units, Real Sizes",
                        &[
                            "This page is 210 by 297 millimeters — an A4 sheet at true physical scale, in a game world where one unit equals one meter. The business card beside it is 3.5 by 2 inches. Neither requires manual conversion. Each panel declares its own layout unit, and the system handles the rest.",
                            "Font sizes are in typographic points. Padding and spacing follow the panel’s layout unit. Mix freely: Mm(6.0) for a margin, Pt(18.0) for a heading, In(0.5) for a gutter. The convenience types carry their unit through the layout engine — no math required.",
                            "By default, a panel’s dimensions map directly to meters. This 210mm-wide A4 page occupies 0.21 meters in the scene — physically correct at 1:1 world scale.",
                        ],
                    );
                    builder.with(
                        El::new()
                            .width(Sizing::fixed(Mm(0.3)))
                            .height(Sizing::GROW),
                        |_| {},
                    );
                    a4_article_column(
                        builder,
                        "Any Scale You Need",
                        &[
                            "Set world_height on any panel and it scales uniformly to fit. A museum placard at 0.3 meters. A highway billboard at 12 meters. A planetary-scale announcement at 50,000 meters — zoom out far enough and there it is. Text stays sharp at any distance by rendering glyphs using SDFs (signed distance fields), not rasterized bitmaps. — The GPU evaluates the distance function per fragment - edges remain crisp regardless of scale.",
                            "PanelDefaults sets panel construction defaults: layout in meters, fonts in points. Override per-panel with layout_unit and font_unit, or per-element with types like Mm(10.0) and Pt(24.0) inline. The system converts at layout time so the engine always works in a consistent coordinate space internally.",
                            "Custom(0.01) defines a centimeter. Custom(9.461e15) defines a light-year. The unit system does not care about scale — it only needs to know the ratio to meters.",
                        ],
                    );
                },
            );
            builder.with(
                El::new()
                    .width(Sizing::GROW)
                    .height(Sizing::fixed(Mm(0.3)))
                    .background(Color::srgba(0.0, 0.0, 0.1, 1.0)),
                |_| {},
            );
            builder.with(El::new().width(Sizing::GROW).align_x(AlignX::Center), |builder| {
                builder.with(El::new(), |builder| {
                    builder.text((
                        "PaperSize::A4  |  layout: Millimeters  |  font: Points",
                        TextStyle::new(Pt(14.0)).with_color(Color::srgba(0.0, 0.0, 0.1, 1.0)),
                    ));
                });
            });
        },
    );
    let panel = DiegeticPanel::world()
        .paper(PaperSize::A4)
        .anchor(Anchor::Center)
        .shadow_casting(ShadowCasting::On)
        .with_tree(layout.build())
        .build()
        .map_err(|error| format!("invalid A4 panel: {error}"))?;
    let panel = app
        .world_mut()
        .spawn((panel, Transform::from_xyz(-0.029, 0.0, 0.0)))
        .id();
    app.world_mut()
        .commands()
        .entity(panel)
        .override_font_unit(Unit::Points)
        .override_lighting(Lighting::Unlit);
    read_scene_bytes(app, target).map(|pixels| (pixels, region))
}

fn units_a4_body_has_no_more_lone_near_black_pixels_than_pre_phase() -> Result<(), String> {
    for scale_factor in [1_usize, 2] {
        let (pixels, region) = render_units_a4_article(
            u32::try_from(scale_factor).map_err(|error| format!("scale factor: {error}"))?,
        )?;
        let pixel = |x: usize, y: usize| {
            let mut channels = [0_u32; 3];
            for sub_y in 0..scale_factor {
                for sub_x in 0..scale_factor {
                    let local_y = y * scale_factor + sub_y - region.origin.y as usize;
                    let local_x = x * scale_factor + sub_x - region.origin.x as usize;
                    let index = (local_y * region.size.x as usize + local_x) * 4;
                    for channel in 0..3 {
                        channels[channel] += u32::from(pixels[index + channel]);
                    }
                }
            }
            let samples = u32::try_from(scale_factor * scale_factor).unwrap_or(1);
            channels.map(|channel| u8::try_from(channel / samples).unwrap_or(u8::MAX))
        };
        let mut lone = Vec::new();
        for y in 305..624 {
            for x in 265..555 {
                let center = pixel(x, y);
                if center.iter().all(|channel| *channel < 30)
                    && [x - 1, x + 1]
                        .into_iter()
                        .all(|neighbor| pixel(neighbor, y).iter().all(|channel| *channel >= 128))
                {
                    lone.push((x, y));
                }
            }
        }
        eprintln!(
            "A4 body scale {scale_factor}: {} lone near-black pixels",
            lone.len()
        );
        // The renderer before small-text fitting draws 16 at scale 1 and 0 at scale 2.
        let old_lone_pixel_count = if scale_factor == 1 { 16 } else { 0 };
        assert!(
            lone.len() <= old_lone_pixel_count,
            "A4 article at scale {scale_factor} has {} isolated near-black pixels (old: {old_lone_pixel_count}): {:?}",
            lone.len(),
            lone.iter().take(20).collect::<Vec<_>>()
        );
    }
    Ok(())
}

fn read_scene_bytes(mut app: CaseApp, target: Handle<Image>) -> Result<Vec<u8>, String> {
    app.settle()?;
    capture_scene_bytes(&mut app, target)
}

fn capture_scene_bytes(app: &mut CaseApp, target: Handle<Image>) -> Result<Vec<u8>, String> {
    let size = app
        .world()
        .resource::<Assets<Image>>()
        .get(&target)
        .ok_or("capture target is missing")?
        .texture_descriptor
        .size;
    let pixels = app.capture(target)?;
    let region = DUMP_REGION.with(|region| *region.borrow());
    debug_assert_eq!(region.size.x, size.width);
    debug_assert_eq!(region.size.y, size.height);
    DUMP_CONTEXT.with(|context| {
        drawn_support::dump_pixels::<false>(
            "small_glyph_pixels",
            "",
            &context.borrow(),
            region,
            region,
            &pixels,
        )
    })?;
    Ok(pixels)
}

fn world_text_scene(alpha: AlphaMode, text: &'static str) -> (CaseApp, Handle<Image>) {
    DUMP_REGION.with(|region| {
        *region.borrow_mut() =
            CaptureRegion::new(UVec2::splat(SIDE), UVec2::ZERO, UVec2::splat(SIDE));
    });
    DUMP_CONTEXT.with(|context| {
        *context.borrow_mut() = format!(
            "pixels_per_em=world_text phase=(camera_projection) alpha={alpha:?} text={text:?}"
        );
    });
    let mut app = CaseApp::new();
    app.insert_resource(ClearColor(Color::BLACK));

    let target = app.target(CaptureRegion::new(
        UVec2::splat(SIDE),
        UVec2::ZERO,
        UVec2::splat(SIDE),
    ));
    let bloom = Bloom {
        prefilter: BloomPrefilter {
            threshold:          1.0,
            threshold_softness: 0.1,
        },
        ..default()
    };
    app.world_mut().spawn((
        Camera3d::default(),
        RenderTarget::Window(WindowRef::Primary),
        Hdr,
        Tonemapping::KhronosPbrNeutral,
        bloom.clone(),
    ));
    app.world_mut().spawn((
        Camera3d::default(),
        RenderTarget::Image(target.clone().into()),
        Hdr,
        Tonemapping::KhronosPbrNeutral,
        bloom,
        Msaa::Sample4,
        Transform::from_xyz(0.6, 0.55, 3.2).looking_at(Vec3::ZERO, Vec3::Y),
    ));

    let cube = app
        .world_mut()
        .resource_mut::<Assets<Mesh>>()
        .add(Cuboid::default());
    let face = app
        .world_mut()
        .resource_mut::<Assets<StandardMaterial>>()
        .add(StandardMaterial {
            base_color: Color::srgb_u8(196, 176, 157),
            ..default()
        });
    app.world_mut()
        .spawn((Mesh3d(cube), MeshMaterial3d(face)))
        .with_children(|parent| {
            parent.spawn(
                DiegeticText::world(text)
                    .size(0.22)
                    .color(Color::srgb_u8(226, 96, 55))
                    .lighting(Lighting::Lit)
                    .alpha_mode(alpha)
                    .transform(Transform::from_xyz(0.0, 0.0, 0.501))
                    .build(),
            );
        });
    app.world_mut().insert_resource(GlobalAmbientLight {
        color:                      Color::WHITE,
        brightness:                 400.0,
        affects_lightmapped_meshes: true,
    });
    app.world_mut().spawn((
        DirectionalLight::default(),
        Transform::from_xyz(4.0, 8.0, 4.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    (app, target)
}

fn render_world_text(alpha: AlphaMode, text: &'static str) -> Result<DrawnPixels, String> {
    let (app, target) = world_text_scene(alpha, text);
    read_scene(app, target)
}

fn eight_grid_x_offset(first_eight: usize, column: u16, pixels_per_em: f32) -> f32 {
    let first_hole_phase = (0.3_f32.mul_add(
        pixels_per_em,
        f32::from(u16::try_from(first_eight).unwrap_or(u16::MAX)).mul_add(3.6, 27.0 - 10.5),
    ))
    .fract();
    let target_phase = [0.90_f32, 0.95, 0.99, 0.00, 0.05, 0.25, 0.50, 0.75][usize::from(column)];
    (target_phase - first_hole_phase).rem_euclid(1.0)
}

fn render_counter_phase_grid(
    scene: GlyphScene,
    camera_pitch_rad: f32,
) -> Result<DrawnPixels, String> {
    let (mut app, target) = setup_scene_with_panel_width_and_pitch(
        GlyphScene { text: "", ..scene },
        0.1,
        camera_pitch_rad,
    )?;
    let first_eight = scene
        .text
        .chars()
        .position(|character| character == '8')
        .unwrap_or(0);
    for row in 0_u16..8 {
        for column in 0_u16..8 {
            let offset_x = eight_grid_x_offset(first_eight, column, scene.pixels_per_em);
            let offset_y = f32::from(row) / 8.0;
            let center_x = f32::from(column).mul_add(28.0, 27.0) + offset_x;
            let center_y = f32::from(row).mul_add(28.0, 27.0) - offset_y;
            spawn_glyph_panel(
                &mut app,
                scene,
                35.0,
                20.0,
                Vec2::new(center_x - SIDE_F32 / 2.0, SIDE_F32 / 2.0 - center_y),
                camera_pitch_rad,
            )?;
        }
    }
    read_scene(app, target)
}

fn linear_channel(channel: u8) -> f32 {
    let srgb = f32::from(channel) / 255.0;
    if srgb <= 0.04045 {
        srgb / 12.92
    } else {
        ((srgb + 0.055) / 1.055).powf(2.4)
    }
}

const DARKNESS_SIDE: u32 = 512;
const DARKNESS_SIDE_F32: f32 = 512.0;

fn darkness_first_pixel_at_or_after(edge: f32) -> u32 {
    (0..DARKNESS_SIDE)
        .find(|&index| pixel_center(index) >= edge)
        .unwrap_or(DARKNESS_SIDE - 1)
}

fn darkness_last_pixel_at_or_before(edge: f32) -> u32 {
    (0..DARKNESS_SIDE)
        .rev()
        .find(|&index| pixel_center(index) <= edge)
        .unwrap_or(0)
}

fn settled_label_ink(
    pixels: &[u8],
    region: CaptureRegion,
    pixels_per_em: f32,
    label_centers: &[(f32, f32)],
) -> f32 {
    let rgb = |x: u32, y: u32| {
        let index = (((y - region.origin.y) * region.size.x + x - region.origin.x) * 4) as usize;
        [pixels[index], pixels[index + 1], pixels[index + 2]]
    };
    let scale = pixels_per_em / 6.0;
    let mut total_ink = 0.0;
    for &(x, y) in label_centers {
        let center_x = x.mul_add(scale, DARKNESS_SIDE_F32 / 2.0);
        let center_y = (-y).mul_add(scale, DARKNESS_SIDE_F32 / 2.0);
        let left = darkness_first_pixel_at_or_after((-13.0_f32).mul_add(scale, center_x));
        let right = darkness_last_pixel_at_or_before(13.0_f32.mul_add(scale, center_x));
        let top = darkness_first_pixel_at_or_after((-10.0_f32).mul_add(scale, center_y));
        let bottom = darkness_last_pixel_at_or_before(10.0_f32.mul_add(scale, center_y));
        let fill_y = darkness_first_pixel_at_or_after(pixels_per_em.mul_add(0.7, center_y));
        let mut fill_samples = [-2.0, -1.0, 0.0, 1.0, 2.0]
            .map(|offset| rgb(darkness_first_pixel_at_or_after(center_x + offset), fill_y));
        fill_samples.sort_unstable();
        let fill = fill_samples[2];
        let fill_light = fill.map(linear_channel);
        for y in top..=bottom {
            for x in left..=right {
                let color = rgb(x, y).map(linear_channel);
                total_ink += (0..3)
                    .map(|channel| (color[channel] - fill_light[channel]).max(0.0))
                    .sum::<f32>()
                    / 3.0;
            }
        }
    }
    let label_count = f32::from(u16::try_from(label_centers.len()).unwrap_or(u16::MAX));
    total_ink / (label_count * pixels_per_em * pixels_per_em)
}

#[expect(
    clippy::too_many_lines,
    reason = "the settled render sweep covers both camera pitches and every requested size"
)]
fn settled_small_text_ink_has_no_new_neighboring_size_jumps() -> Result<(), String> {
    let scene = GlyphScene {
        text:          "",
        fill:          DARK_FILL,
        ink:           LIGHT_INK,
        font_mm:       10.0,
        pixels_per_em: 6.0,
        offset_px:     0.0,
        offset_y_px:   0.0,
        tilt_degrees:  0.0,
        alpha_mode:    AlphaMode::Blend,
        drawing_path:  DrawingPath::Standard,
    };
    let mut failures = Vec::new();
    for pitch in [0.0, 0.18] {
        let (mut app, target) = setup_scene_with_panel_width_and_pitch(scene, 0.1, pitch)?;
        // The fit pass reads the target viewport, so this sweep needs its
        // original size to keep the measured pixel phases unchanged.
        let region = CaptureRegion::new(
            UVec2::splat(DARKNESS_SIDE),
            UVec2::ZERO,
            UVec2::splat(DARKNESS_SIDE),
        );
        DUMP_REGION.with(|dump_region| *dump_region.borrow_mut() = region);
        *app.world_mut()
            .resource_mut::<Assets<Image>>()
            .get_mut(&target)
            .ok_or_else(|| "settled text target disappeared".to_string())? =
            Image::new_target_texture(
                region.size.x,
                region.size.y,
                TextureFormat::Rgba8UnormSrgb,
                None,
            );
        let camera_entity = {
            let world = app.world_mut();
            let mut cameras = world.query_filtered::<(Entity, &Transform), With<Camera3d>>();
            cameras
                .iter(world)
                .find(|(_, transform)| transform.translation.z > 0.0)
                .map(|(entity, _)| entity)
                .ok_or_else(|| "no image camera for settled text".to_string())?
        };
        let mut camera = app
            .world_mut()
            .get_mut::<Camera>(camera_entity)
            .ok_or_else(|| "settled text image camera disappeared".to_string())?;
        region.apply_to_camera(&mut camera);
        let mut centers = Vec::new();
        let pixel_world = scene.font_mm * 0.001 / scene.pixels_per_em;
        for row in 0_u16..10 {
            for column in 0_u16..10 {
                let index = row * 10 + column;
                let x_jitter = f32::from((index * 37) % 101).mul_add(4.0 / 101.0, -2.0);
                let x = (f32::from(column) - 4.5).mul_add(27.5, x_jitter);
                let row_phase = f32::from(index % 4) * 0.25;
                let y = (f32::from(row) - 4.5).mul_add(27.5, row_phase);
                let text = format!("{index:02} 807");
                spawn_glyph_panel(&mut app, scene, 40.0, 20.0, Vec2::new(x, y), pitch)?;
                let label_y = (y - 2.0) * pixel_world * pitch.cos();
                app.world_mut().spawn(
                    DiegeticText::world(text)
                        .size(scene.font_mm * 0.001)
                        .color(Color::srgb_u8(scene.ink[0], scene.ink[1], scene.ink[2]))
                        .lighting(Lighting::Unlit)
                        .alpha_mode(scene.alpha_mode)
                        .transform(Transform::from_xyz(
                            (x - 11.0) * pixel_world,
                            label_y,
                            (-label_y).mul_add(pitch.tan(), 0.001),
                        ))
                        .build(),
                );
                centers.push((x, y));
            }
        }
        while *app.world().resource::<DisplayCompensationReadiness>()
            != DisplayCompensationReadiness::Compensated
        {
            app.update();
        }
        let mut previous: Option<(f32, f32)> = None;
        let mut worst_change = (0.0_f32, 0.0_f32, 0.0_f32);
        for step in 0_u16..=84 {
            let pixels_per_em = f32::from(step).mul_add(0.05, 4.9);
            let distance = GlyphScene {
                pixels_per_em,
                ..scene
            }
            .camera_distance()
                * 2.0;
            {
                let mut camera = app
                    .world_mut()
                    .get_mut::<Transform>(camera_entity)
                    .ok_or_else(|| "settled text camera disappeared".to_string())?;
                camera.translation = Vec3::new(0.0, distance * pitch.sin(), distance * pitch.cos());
            }
            for _ in 0..5 {
                app.update();
            }
            let pixels = capture_scene_bytes(&mut app, target.clone())?;
            let ink = settled_label_ink(&pixels, region, pixels_per_em, &centers);
            if let Some((previous_size, previous_ink)) = previous {
                let change = (ink / previous_ink - 1.0).abs();
                if change > worst_change.0 {
                    worst_change = (change, previous_size, pixels_per_em);
                }
                // The pre-fitting renderer's worst step plus 0.1 px phase noise.
                let old_step_plus_noise = if pitch == 0.0 {
                    0.028_738 + 0.014_717 // 2.8738% old step; 1.4717% repeat/shift spread.
                } else {
                    0.027_459 + 0.019_854 // 2.7459% old step; 1.9854% repeat/shift spread.
                };
                if change > old_step_plus_noise {
                    failures.push(format!("pitch {pitch:.2}, {previous_size:.2}->{pixels_per_em:.2}: {previous_ink:.5}->{ink:.5}, {:.2}%", change * 100.0));
                }
            }
            previous = Some((pixels_per_em, ink));
        }
        eprintln!(
            "settled text pitch {pitch:.2}: worst neighboring step {:.2}% at {:.2}->{:.2} px/em",
            worst_change.0 * 100.0,
            worst_change.1,
            worst_change.2,
        );
    }
    assert!(
        failures.is_empty(),
        "settled small text ink discontinuities: {}",
        failures.join("; ")
    );
    Ok(())
}

fn text_ink_centroid(pixels: &DrawnPixels, scene: GlyphScene) -> Result<Vec2, String> {
    let page = pixels.rgb(108, 145);
    let ink_direction = [
        f32::from(scene.ink[0]) - f32::from(page[0]),
        f32::from(scene.ink[1]) - f32::from(page[1]),
        f32::from(scene.ink[2]) - f32::from(page[2]),
    ];
    let ink_length_sq = ink_direction
        .iter()
        .map(|component| component * component)
        .sum::<f32>();
    let mut mass = 0.0_f32;
    let mut weighted = Vec2::ZERO;
    for y in 105..=150 {
        for x in 105..=150 {
            let pixel = pixels.rgb(x, y);
            let ink = (0..3)
                .map(|channel| {
                    (f32::from(pixel[channel]) - f32::from(page[channel])) * ink_direction[channel]
                })
                .sum::<f32>()
                / ink_length_sq;
            if ink < 0.02 {
                continue;
            }
            mass += ink;
            weighted += Vec2::new(pixel_center(x), pixel_center(y)) * ink;
        }
    }
    if mass <= 0.0 {
        return Err("moving text drew no ink".to_string());
    }
    Ok(weighted / mass)
}

fn moving_text_sample(
    app: &mut CaseApp,
    target: Handle<Image>,
    camera_entity: Entity,
    scene: GlyphScene,
) -> Result<(Vec2, Vec2), String> {
    let pixels = DrawnPixels(capture_scene_bytes(app, target)?);
    let centroid = text_ink_centroid(&pixels, scene)?;
    let world = app.world_mut();
    let mut labels = world.query::<(&GlobalTransform, &TextContent)>();
    let label_origin = labels
        .iter(world)
        .find(|(_, content)| content.text() == "8888")
        .ok_or_else(|| "moving text label disappeared".to_string())?
        .0
        .translation();
    let camera = world
        .get::<Camera>(camera_entity)
        .ok_or_else(|| "moving text camera disappeared".to_string())?;
    let transform = world
        .get::<GlobalTransform>(camera_entity)
        .ok_or_else(|| "moving camera transform disappeared".to_string())?;
    let anchor = camera
        .world_to_viewport(transform, label_origin)
        .map_err(|error| format!("moving text anchor projection: {error}"))?;
    Ok((centroid, anchor))
}

#[expect(
    clippy::too_many_lines,
    reason = "the rendered motion and idle sequence is one test"
)]
fn small_text_glides_then_settles_without_idle_uploads_or_redraws() -> Result<(), String> {
    let scene = GlyphScene {
        text:          "8888",
        fill:          DARK_FILL,
        ink:           LIGHT_INK,
        font_mm:       10.0,
        pixels_per_em: 5.5,
        offset_px:     0.0,
        offset_y_px:   0.0,
        tilt_degrees:  0.0,
        alpha_mode:    AlphaMode::Blend,
        drawing_path:  DrawingPath::Standard,
    };
    let (mut app, target) = setup_scene(GlyphScene { text: "", ..scene })?;
    app.world_mut().spawn(
        DiegeticText::world(scene.text)
            .size(scene.font_mm * 0.001)
            .color(Color::srgb_u8(scene.ink[0], scene.ink[1], scene.ink[2]))
            .lighting(Lighting::Unlit)
            .alpha_mode(scene.alpha_mode)
            .transform(Transform::from_xyz(-0.007, 0.0, 0.001))
            .build(),
    );
    while *app.world().resource::<DisplayCompensationReadiness>()
        != DisplayCompensationReadiness::Compensated
    {
        app.update();
    }
    for _ in 0..4 {
        app.update();
    }
    let camera_entity = {
        let world = app.world_mut();
        let mut cameras = world.query_filtered::<(Entity, &Transform), With<Camera3d>>();
        cameras
            .iter(world)
            .find(|(_, transform)| transform.translation.z > 0.0)
            .map(|(entity, _)| entity)
            .ok_or_else(|| "no image camera for moving text".to_string())?
    };
    let mut previous = moving_text_sample(&mut app, target.clone(), camera_entity, scene)?;
    let mut largest_motion_error = 0.0_f32;
    let mut worst_motion_step = String::new();
    for step in 1_u16..=160 {
        let pixels_per_em = f32::from(step).mul_add(0.025, 5.5);
        let distance = GlyphScene {
            pixels_per_em,
            ..scene
        }
        .camera_distance();
        let Some(mut camera) = app.world_mut().get_mut::<Transform>(camera_entity) else {
            return Err("moving text camera transform disappeared".to_string());
        };
        camera.translation.z = distance;
        let sample = moving_text_sample(
            &mut app,
            target.clone(),
            camera_entity,
            GlyphScene {
                pixels_per_em,
                ..scene
            },
        )?;
        let error = ((sample.0 - previous.0) - (sample.1 - previous.1))
            .abs()
            .max_element();
        if error > largest_motion_error {
            largest_motion_error = error;
            worst_motion_step = format!("zoom {pixels_per_em:.2}: from {previous:?} to {sample:?}");
        }
        previous = sample;
    }
    let pixels_per_em = 9.5;
    let world_per_pixel = scene.font_mm * 0.001 / pixels_per_em;
    for step in 1_u16..=30 {
        let pan_px = f32::from(step) * 0.1;
        let Some(mut camera) = app.world_mut().get_mut::<Transform>(camera_entity) else {
            return Err("moving text camera transform disappeared".to_string());
        };
        camera.translation.x = pan_px * world_per_pixel;
        let sample = moving_text_sample(
            &mut app,
            target.clone(),
            camera_entity,
            GlyphScene {
                pixels_per_em,
                offset_px: -pan_px,
                ..scene
            },
        )?;
        let error = ((sample.0 - previous.0) - (sample.1 - previous.1))
            .abs()
            .max_element();
        if error > largest_motion_error {
            largest_motion_error = error;
            worst_motion_step = format!("pan {pan_px:.2}: from {previous:?} to {sample:?}");
        }
        previous = sample;
    }
    // Pre-fitting renderer: 0.321442 px worst jump; 0.065094 px spread at 0, ±0.1 px, and a repeat.
    // One centroid run is one sample here: its centre follows ink shifting between glyphs.
    let old_jump_plus_noise = 0.321_442 + 0.065_094;
    assert!(
        largest_motion_error <= old_jump_plus_noise,
        "ink centroid jumped {largest_motion_error:.3} px away from its projected anchor: {worst_motion_step}"
    );
    let mut settled = Vec::new();
    for _ in 0..5 {
        settled.push(
            moving_text_sample(
                &mut app,
                target.clone(),
                camera_entity,
                GlyphScene {
                    pixels_per_em,
                    offset_px: -3.0,
                    ..scene
                },
            )?
            .0,
        );
    }
    let final_position = settled[3];
    assert!((settled[4] - final_position).length() < 0.02);
    for pair in settled[..4].windows(2) {
        for axis in 0..2 {
            let remaining = final_position[axis] - pair[0][axis];
            let step = pair[1][axis] - pair[0][axis];
            assert!(
                step * remaining >= -0.01 && step.abs() <= remaining.abs() + 0.05,
                "settle overshot or reversed: {settled:?}"
            );
        }
    }
    // Bevy swaps message buffers only on a frame after a fixed-timestep tick,
    // so the settle samples' own redraws can stay buffered through these
    // frames. Count only the redraws each idle frame writes.
    let mut idle_redraws = app
        .world()
        .resource::<Messages<RequestRedraw>>()
        .get_cursor_current();
    for _ in 0..3 {
        app.update();
        let uploads = app
            .world()
            .resource::<DiegeticPerfStats>()
            .batch
            .instance_uploads
            .get();
        assert_eq!(uploads, 0, "settled text uploaded its path records");
        let redraws = app.world().resource::<Messages<RequestRedraw>>();
        assert_eq!(
            idle_redraws.read(redraws).count(),
            0,
            "settled text requested another redraw"
        );
    }
    Ok(())
}

#[expect(
    clippy::too_many_lines,
    reason = "the outline-column measure covers the full pose grid"
)]
fn eights_in_short_runs_keep_both_counters_lighter_than_ink_at_every_phase_and_tilt()
-> Result<(), String> {
    let mut failures = Vec::new();
    let mut checked = 0_u32;
    for text in ["283", "8888", "806"] {
        for tilt_degrees in [0.0, 3.0] {
            for camera_pitch_rad in [0.0, 0.18] {
                let scene = GlyphScene {
                    text,
                    fill: DARK_FILL,
                    ink: LIGHT_INK,
                    font_mm: 10.0,
                    pixels_per_em: 6.0,
                    offset_px: 0.0,
                    offset_y_px: 0.0,
                    tilt_degrees,
                    alpha_mode: AlphaMode::Blend,
                    drawing_path: DrawingPath::Standard,
                };
                let pixels = render_counter_phase_grid(scene, camera_pitch_rad)?;
                let first_eight = text
                    .chars()
                    .position(|character| character == '8')
                    .unwrap_or(0);
                for row in 0_u16..8 {
                    for column in 0_u16..8 {
                        let offset_x =
                            eight_grid_x_offset(first_eight, column, scene.pixels_per_em);
                        let offset_y = f32::from(row) / 8.0;
                        let center_x = f32::from(column).mul_add(28.0, 27.0) + offset_x;
                        let panel_left = first_pixel_at_or_after(center_x - 10.5);
                        let fill = scene.fill;
                        let pixel_ref = &pixels;
                        let mut bounds = (SIDE, SIDE, 0, 0);
                        for y in (u32::from(row) * 28 + 14)..=(u32::from(row) * 28 + 40) {
                            for x in (u32::from(column) * 28 + 14)..=(u32::from(column) * 28 + 40) {
                                if closer_to(pixels.rgb(x, y), scene.ink, fill) {
                                    bounds.0 = bounds.0.min(x);
                                    bounds.1 = bounds.1.min(y);
                                    bounds.2 = bounds.2.max(x);
                                    bounds.3 = bounds.3.max(y);
                                }
                            }
                        }
                        let (_, first_dark_row, _, bottom) = bounds;
                        if first_dark_row >= bottom {
                            failures.push(format!("{text} tilt={tilt_degrees} pitch={camera_pitch_rad} phase=({offset_x},{offset_y}): no ink"));
                            continue;
                        }
                        for (index, character) in text.chars().enumerate() {
                            if character != '8' {
                                continue;
                            }
                            let glyph_index = u32::try_from(index)
                                .map_err(|error| format!("glyph index exceeds u32: {error}"))?;
                            let cell_left = panel_left + glyph_index * 18 / 5;
                            let cell_right = panel_left + (glyph_index + 1) * 18 / 5;
                            let glyph_origin_x =
                                f32::from(u16::try_from(index).unwrap_or(u16::MAX))
                                    .mul_add(3.6, center_x - 10.5);
                            let hole_phase = 0.3_f32
                                .mul_add(scene.pixels_per_em, glyph_origin_x)
                                .rem_euclid(1.0);
                            let ink_range = f32::from(color_distance(scene.ink, fill));
                            let bands: [(f32, f32); 5] = [
                                (0.660, 0.730),
                                (0.422, 0.660),
                                (0.342, 0.422),
                                (0.071, 0.342),
                                (0.000, 0.071),
                            ];
                            let mut clearest_holes =
                                (f32::INFINITY, (0_u32, 0_u32), (0_u32, 0_u32));
                            let mut both_open = false;
                            for top in [
                                first_dark_row.saturating_sub(1),
                                first_dark_row,
                                first_dark_row + 1,
                            ] {
                                let baseline =
                                    0.70_f32.mul_add(scene.pixels_per_em, pixel_center(top));
                                for x in cell_left..=cell_right {
                                    let glyph_x =
                                        (pixel_center(x) - glyph_origin_x) / scene.pixels_per_em;
                                    if !(0.176..=0.424).contains(&glyph_x) {
                                        continue;
                                    }
                                    let mut profile = [None::<(f32, u32, u32)>; 5];
                                    for (band, (lower_em, upper_em)) in
                                        bands.into_iter().enumerate()
                                    {
                                        let upper_row =
                                            upper_em.mul_add(-scene.pixels_per_em, baseline);
                                        let lower_row =
                                            lower_em.mul_add(-scene.pixels_per_em, baseline);
                                        for y in top..=bottom + 1 {
                                            if pixel_center(y) - 0.5 < lower_row
                                                && pixel_center(y) + 0.5 > upper_row
                                            {
                                                let ink = f32::from(color_distance(
                                                    pixel_ref.rgb(x, y),
                                                    fill,
                                                ));
                                                profile[band] = Some(match profile[band] {
                                                    Some(value)
                                                        if band % 2 == 0 && value.0 > ink =>
                                                    {
                                                        value
                                                    },
                                                    Some(value)
                                                        if band % 2 != 0 && value.0 < ink =>
                                                    {
                                                        value
                                                    },
                                                    _ => (ink, x, y),
                                                });
                                            }
                                        }
                                    }
                                    if let [
                                        Some(top_bar),
                                        Some(upper_hole),
                                        Some(crossbar),
                                        Some(lower_hole),
                                        Some(bottom_bar),
                                    ] = profile
                                    {
                                        if ![
                                            (top_bar.1, top_bar.2),
                                            (crossbar.1, crossbar.2),
                                            (bottom_bar.1, bottom_bar.2),
                                        ]
                                        .into_iter()
                                        .all(|(x, y)| closer_to(pixels.rgb(x, y), scene.ink, fill))
                                        {
                                            continue;
                                        }
                                        let darker_hole = upper_hole.0.max(lower_hole.0);
                                        if darker_hole < clearest_holes.0 {
                                            clearest_holes = (
                                                darker_hole,
                                                (upper_hole.1, upper_hole.2),
                                                (lower_hole.1, lower_hole.2),
                                            );
                                        }
                                        both_open |= closer_to(
                                            pixels.rgb(upper_hole.1, upper_hole.2),
                                            fill,
                                            scene.ink,
                                        ) && closer_to(
                                            pixels.rgb(lower_hole.1, lower_hole.2),
                                            fill,
                                            scene.ink,
                                        );
                                    }
                                }
                            }
                            if !both_open {
                                let upper = clearest_holes.1;
                                let lower = clearest_holes.2;
                                failures.push(format!(
                                    "{text} glyph {index} at 6 px/em tilt={tilt_degrees} pitch={camera_pitch_rad} x phase={hole_phase:.2} y phase={offset_y:.3}: upper ({},{}) {:?}, lower ({},{}) {:?}, darker-hole distance {:.0}/{:.0}",
                                    upper.0, upper.1, pixels.rgb(upper.0, upper.1),
                                    lower.0, lower.1, pixels.rgb(lower.0, lower.1),
                                    clearest_holes.0, ink_range * 0.5,
                                ));
                            }
                            checked += 1;
                        }
                    }
                }
            }
        }
    }
    eprintln!(
        "8 counters: {} of {checked} poses failed the absolute hole-pixel floor",
        failures.len()
    );
    assert!(
        failures.is_empty(),
        "8 counter pixels must be closer to fill than ink: {} failures; {}",
        failures.len(),
        failures
            .iter()
            .take(20)
            .cloned()
            .collect::<Vec<_>>()
            .join("; "),
    );
    Ok(())
}

fn zero_dot_is_absent_through_nine_pixels_per_em_and_draws_at_ten() -> Result<(), String> {
    let mut failures = Vec::new();
    for pixels_per_em in [6.0_f32, 8.0, 8.5, 9.0, 10.0, 12.0] {
        let mut dot_draws_at_any_phase = false;
        for offset in [0.0_f32, 0.25, 0.5, 0.75] {
            let scene = GlyphScene {
                text: "0",
                fill: DARK_FILL,
                ink: LIGHT_INK,
                font_mm: 10.0,
                pixels_per_em,
                offset_px: offset,
                offset_y_px: offset,
                tilt_degrees: 0.0,
                alpha_mode: AlphaMode::Blend,
                drawing_path: DrawingPath::Standard,
            };
            let pixels = render(scene)?;
            let (left, top, right, bottom) = pixels.ink_bounds(scene)?;
            let center = (left.midpoint(right), top.midpoint(bottom));
            let color = pixels.rgb(center.0, center.1);
            let dot_draws = closer_to(color, scene.ink, scene.fill);
            eprintln!(
                "0 at {pixels_per_em} px/em phase {offset} center ({},{}) {:?}, dot draws={dot_draws}",
                center.0, center.1, color,
            );
            dot_draws_at_any_phase |= dot_draws;
            if pixels_per_em < 10.0 && dot_draws {
                failures.push(format!(
                    "0 at {pixels_per_em} px/em phase {offset} center ({},{}) {:?} drew a dot",
                    center.0, center.1, color,
                ));
            }
        }
        if pixels_per_em >= 10.0 && !dot_draws_at_any_phase {
            failures.push(format!(
                "0 at {pixels_per_em} px/em drew no dot in four phases"
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("; "));
    Ok(())
}

fn counter_to_wall_ratio(
    pixels: &DrawnPixels,
    fill: [u8; 3],
    origin_x: f32,
    baseline_y: f32,
    pixels_per_em: f32,
    box_em: (f32, f32, f32, f32),
) -> (f32, (u32, u32)) {
    let (left, bottom, right, top) = box_em;
    let mut best = (f32::INFINITY, (0, 0));
    let x0 = first_pixel_at_or_after(pixels_per_em.mul_add(-0.05, origin_x));
    let x1 = last_pixel_at_or_before(pixels_per_em.mul_add(0.65, origin_x));
    let y0 = first_pixel_at_or_after(pixels_per_em.mul_add(-top, baseline_y));
    let y1 = last_pixel_at_or_before(pixels_per_em.mul_add(-bottom, baseline_y));
    for y in y0..=y1 {
        let mut left_wall = 0.0_f32;
        let mut right_wall = 0.0_f32;
        for x in x0..=x1 {
            let glyph_x = (pixel_center(x) - origin_x) / pixels_per_em;
            let ink = f32::from(color_distance(pixels.rgb(x, y), fill));
            if glyph_x < left {
                left_wall = left_wall.max(ink);
            } else if glyph_x > right {
                right_wall = right_wall.max(ink);
            }
        }
        let wall = left_wall.midpoint(right_wall);
        if wall < 1.0 {
            continue;
        }
        for x in x0..=x1 {
            let glyph_x = (pixel_center(x) - origin_x) / pixels_per_em;
            if (left..=right).contains(&glyph_x) {
                let ink = f32::from(color_distance(pixels.rgb(x, y), fill));
                let ratio = ink / wall;
                if ratio < best.0 {
                    best = (ratio, (x, y));
                }
            }
        }
    }
    best
}

fn small_zero_and_eight_counters_stay_below_sixty_percent_of_their_walls() -> Result<(), String> {
    let mut failures = Vec::new();
    for text in ["804", "00"] {
        for pitch in [0.0_f32, 0.18] {
            let scene = GlyphScene {
                text,
                fill: DARK_FILL,
                ink: LIGHT_INK,
                font_mm: 10.0,
                pixels_per_em: 6.0,
                offset_px: 0.0,
                offset_y_px: 0.0,
                tilt_degrees: 0.0,
                alpha_mode: AlphaMode::Blend,
                drawing_path: DrawingPath::Standard,
            };
            let pixels = render_counter_phase_grid(scene, pitch)?;
            for row in 0_u16..8 {
                for column in 0_u16..8 {
                    let phase_x = eight_grid_x_offset(0, column, scene.pixels_per_em);
                    let center_x = f32::from(column).mul_add(28.0, 27.0) + phase_x;
                    let panel_left = center_x - 10.5;
                    let mut first_ink_row = SIDE;
                    for y in (u32::from(row) * 28 + 14)..=(u32::from(row) * 28 + 40) {
                        for x in (u32::from(column) * 28 + 14)..=(u32::from(column) * 28 + 40) {
                            if closer_to(pixels.rgb(x, y), scene.ink, scene.fill) {
                                first_ink_row = first_ink_row.min(y);
                            }
                        }
                    }
                    if first_ink_row == SIDE {
                        failures.push(format!(
                            "{text} pitch {pitch} row {row} col {column}: no ink"
                        ));
                        continue;
                    }
                    let baseline =
                        0.70_f32.mul_add(scene.pixels_per_em, pixel_center(first_ink_row));
                    for (index, character) in text.chars().enumerate() {
                        let boxes: &[(f32, f32, f32, f32)] = match character {
                            '8' => &[(0.176, 0.422, 0.424, 0.660), (0.156, 0.071, 0.444, 0.342)],
                            '0' => &[(0.167, 0.067, 0.433, 0.663)],
                            _ => continue,
                        };
                        let glyph_origin = f32::from(u16::try_from(index).unwrap_or(u16::MAX))
                            .mul_add(0.6 * scene.pixels_per_em, panel_left);
                        for (counter, &box_em) in boxes.iter().enumerate() {
                            let (ratio, at) = counter_to_wall_ratio(
                                &pixels,
                                scene.fill,
                                glyph_origin,
                                baseline,
                                scene.pixels_per_em,
                                box_em,
                            );
                            if ratio > 0.6 {
                                failures.push(format!("{text} glyph {index} counter {counter} pitch {pitch} phase ({phase_x:.2},{:.2}) pixel {at:?} ratio {ratio:.3}", f32::from(row) / 8.0));
                            }
                        }
                    }
                }
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{}",
        failures
            .iter()
            .take(20)
            .cloned()
            .collect::<Vec<_>>()
            .join("; ")
    );
    Ok(())
}

fn small_text_weight_and_position_change_smoothly_at_fit_boundaries() -> Result<(), String> {
    for sizes in [[4.999, 5.0, 5.001], [8.999, 9.0, 9.001]] {
        for offset_px in [0.0, 0.25, 0.5, 0.75] {
            let mut samples = Vec::new();
            for pixels_per_em in sizes {
                let scene = GlyphScene {
                    text: "8888",
                    fill: DARK_FILL,
                    ink: LIGHT_INK,
                    font_mm: 10.0,
                    pixels_per_em,
                    offset_px,
                    offset_y_px: offset_px,
                    tilt_degrees: 0.0,
                    alpha_mode: AlphaMode::Blend,
                    drawing_path: DrawingPath::Standard,
                };
                let pixels = render(scene)?;
                let blank = render(GlyphScene { text: "", ..scene })?;
                let page_top =
                    SIDE_F32 / 2.0 - scene.offset_y_px - 50.0 * scene.pixels_per_em / scene.font_mm;
                let mut mass = 0.0_f32;
                let mut weighted_y = 0.0_f32;
                for y in 0..SIDE {
                    for x in 0..SIDE {
                        let ink = f32::from(color_distance(pixels.rgb(x, y), blank.rgb(x, y)));
                        mass += ink;
                        weighted_y = ink.mul_add(pixel_center(y) - page_top, weighted_y);
                    }
                }
                if mass <= 0.0 {
                    return Err(format!("text at {pixels_per_em} px/em drew no ink"));
                }
                samples.push((mass, weighted_y / mass));
            }
            for pair in samples.windows(2) {
                let weight_change = (pair[1].0 - pair[0].0).abs() / pair[0].0;
                let position_change = (pair[1].1 - pair[0].1).abs();
                assert!(
                    weight_change < 0.02 && position_change < 0.08,
                    "text changed abruptly across {sizes:?} at phase {offset_px}: {samples:?}, weight={weight_change:.3}, position={position_change:.3}"
                );
            }
        }
    }
    Ok(())
}

fn clipped_small_text_run_draws_no_ink_past_its_panel_edge() -> Result<(), String> {
    let panel_width_mm = 12.0;
    for drawing_path in [DrawingPath::Standard, DrawingPath::Stable] {
        let scene = GlyphScene {
            text: "8888",
            fill: DARK_FILL,
            ink: LIGHT_INK,
            font_mm: 10.0,
            pixels_per_em: 6.0,
            offset_px: 0.25,
            offset_y_px: 0.25,
            tilt_degrees: 0.0,
            alpha_mode: AlphaMode::Blend,
            drawing_path,
        };
        let blank = render_with_panel_width(GlyphScene { text: "", ..scene }, panel_width_mm)?;
        let text = render_with_panel_width(scene, panel_width_mm)?;
        let panel_right = SIDE_F32 / 2.0
            + scene.offset_px
            + panel_width_mm * scene.pixels_per_em / scene.font_mm / 2.0;
        let inside_x = first_pixel_at_or_after(panel_right - 4.0);
        let outside_x = first_pixel_at_or_after(panel_right);
        let visible = (0..SIDE)
            .flat_map(|y| (inside_x..outside_x).map(move |x| (x, y)))
            .filter(|&(x, y)| color_distance(text.rgb(x, y), blank.rgb(x, y)) > 8)
            .count();
        assert!(
            visible > 0,
            "{drawing_path:?} clipped run drew no visible ink"
        );
        let escaped = (0..SIDE)
            .flat_map(|y| (outside_x..SIDE).map(move |x| (x, y)))
            .filter(|&(x, y)| color_distance(text.rgb(x, y), blank.rgb(x, y)) > 2)
            .take(8)
            .collect::<Vec<_>>();
        assert!(
            escaped.is_empty(),
            "{drawing_path:?} clipped run drew beyond x={panel_right:.2}: {escaped:?}"
        );
    }
    Ok(())
}

#[expect(
    clippy::too_many_lines,
    reason = "the real editor requires an open, cursor, and selection"
)]
fn render_real_inline_editor_parts(offset_px: f32) -> Result<(DrawnPixels, DrawnPixels), String> {
    let scene = GlyphScene {
        text: "",
        fill: DARK_FILL,
        ink: LIGHT_INK,
        font_mm: 10.0,
        pixels_per_em: 6.0,
        offset_px,
        offset_y_px: offset_px,
        tilt_degrees: 0.0,
        alpha_mode: AlphaMode::Blend,
        drawing_path: DrawingPath::Standard,
    };
    let (mut app, target) = setup_scene(scene)?;
    let window = app.world_mut().spawn(Window::default()).id();
    let field_spec =
        ImeEditableFieldSpec::BuiltIn(ImeBuiltInFieldSpec::new(ImeBuiltInFieldKind::Text));
    let mut layout = LayoutBuilder::new(100.0, 100.0);
    layout.with(
        El::new()
            .editable_field(
                "field",
                field_spec,
                TextStyle::new(Mm(10.0)).with_color(Color::srgb_u8(232, 237, 241)),
            )
            .width(Sizing::fixed(Mm(20.0)))
            .height(Sizing::fixed(Mm(12.0))),
        |builder| {
            builder.text(("8", TextStyle::new(Mm(10.0))));
        },
    );
    let panel = DiegeticPanel::world()
        .size(Mm(100.0), Mm(100.0))
        .anchor(Anchor::Center)
        .with_tree(layout.build())
        .build()
        .map_err(|error| format!("invalid selection panel: {error}"))?;
    let pixel_world = scene.font_mm * 0.001 / scene.pixels_per_em;
    let panel = app
        .world_mut()
        .spawn((
            panel,
            Transform::from_xyz(offset_px * pixel_world, offset_px * pixel_world, 0.0001),
        ))
        .id();
    app.world_mut()
        .commands()
        .entity(panel)
        .override_lighting(Lighting::Unlit);
    let (started_sender, started_receiver) = mpsc::channel();
    app.add_observer(move |event: On<ImeStarted>| {
        let _ = started_sender.send(event.session_id);
    });
    let (changed_sender, changed_receiver) = mpsc::channel();
    app.add_observer(move |event: On<ImeTextChanged>| {
        let _ = changed_sender.send(event.snapshot.clone());
    });
    while *app.world().resource::<DisplayCompensationReadiness>()
        != DisplayCompensationReadiness::Compensated
    {
        app.update();
    }
    for _ in 0..4 {
        app.update();
    }
    let position = {
        let world = app.world();
        let computed = world
            .get::<ComputedDiegeticPanel>(panel)
            .ok_or_else(|| "real editor panel has no computed layout".to_string())?;
        let field = computed
            .field_records()
            .first()
            .ok_or_else(|| "real editor has no field record".to_string())?;
        let panel_data = world
            .get::<DiegeticPanel>(panel)
            .ok_or_else(|| "real editor panel disappeared".to_string())?;
        let transform = world
            .get::<GlobalTransform>(panel)
            .ok_or_else(|| "real editor panel has no transform".to_string())?;
        let local = Vec2::new(
            field.bounds.width.mul_add(0.9, field.bounds.x),
            field.bounds.height.mul_add(0.5, field.bounds.y),
        );
        let (anchor_x, anchor_y) = panel_data.anchor_offsets();
        transform.transform_point(Vec3::new(
            local.x.mul_add(panel_data.points_to_world(), -anchor_x),
            (-local.y).mul_add(panel_data.points_to_world(), anchor_y),
            0.0,
        ))
    };
    let camera_entity = {
        let world = app.world_mut();
        let mut cameras = world.query_filtered::<(Entity, &Transform), With<Camera3d>>();
        cameras
            .iter(world)
            .find(|(_, transform)| transform.translation.z > 0.0)
            .map(|(entity, _)| entity)
            .ok_or_else(|| "real editor has no image camera".to_string())?
    };
    let window_ref = WindowRef::Entity(window)
        .normalize(None)
        .ok_or_else(|| "real editor window did not normalize".to_string())?;
    app.world_mut().trigger(PointerClick {
        entity:   panel,
        pointer:  Pointer::new(
            PointerId::Mouse,
            Location {
                target:   NormalizedRenderTarget::Window(window_ref),
                position: Vec2::ZERO,
            },
        ),
        button:   PointerButton::Primary,
        hit:      HitData::new(camera_entity, 0.0, Some(position), None),
        duration: std::time::Duration::ZERO,
        count:    2,
    });
    app.world_mut().flush();
    let _started = started_receiver
        .try_recv()
        .map_err(|error| format!("real editor did not start from panel click: {error}"))?;
    let first_change = changed_receiver
        .try_recv()
        .map_err(|error| format!("real editor produced no first snapshot: {error}"))?;
    assert!(matches!(first_change.cursor, ImeCursorState::Selection(_)));
    for _ in 0..4 {
        app.update();
    }
    app.world_mut().trigger(PointerClick {
        entity:   panel,
        pointer:  Pointer::new(
            PointerId::Mouse,
            Location {
                target:   NormalizedRenderTarget::Window(window_ref),
                position: Vec2::ZERO,
            },
        ),
        button:   PointerButton::Primary,
        hit:      HitData::new(camera_entity, 0.0, Some(position), None),
        duration: std::time::Duration::ZERO,
        count:    1,
    });
    app.world_mut().flush();
    let placed = changed_receiver
        .try_recv()
        .map_err(|error| format!("real editor did not place cursor: {error}"))?;
    assert!(matches!(placed.cursor, ImeCursorState::Insertion(_)));
    for _ in 0..4 {
        app.update();
    }
    let caret = DrawnPixels(capture_scene_bytes(&mut app, target.clone())?);
    app.world_mut().trigger(PointerClick {
        entity:   panel,
        pointer:  Pointer::new(
            PointerId::Mouse,
            Location {
                target:   NormalizedRenderTarget::Window(window_ref),
                position: Vec2::ZERO,
            },
        ),
        button:   PointerButton::Primary,
        hit:      HitData::new(camera_entity, 0.0, Some(position), None),
        duration: std::time::Duration::ZERO,
        count:    3,
    });
    app.world_mut().flush();
    let selected = changed_receiver
        .try_recv()
        .map_err(|error| format!("real editor did not select text: {error}"))?;
    assert!(matches!(selected.cursor, ImeCursorState::Selection(_)));
    for _ in 0..4 {
        app.update();
    }
    let selection = DrawnPixels(capture_scene_bytes(&mut app, target)?);
    assert_ne!(
        caret.0, selection.0,
        "real editor cursor and selection drew identical pixels"
    );
    Ok((caret, selection))
}

fn real_inline_editor_caret_and_selection_align_with_small_glyph() -> Result<(), String> {
    for offset_px in [0.125, 0.375, 0.625, 0.875] {
        let (caret_pixels, selection_pixels) = render_real_inline_editor_parts(offset_px)?;
        let mut selection = Vec::new();
        let mut ink = Vec::new();
        for y in 90..145 {
            for x in 90..145 {
                let [red, green, blue] = selection_pixels.rgb(x, y);
                if blue > red.saturating_add(50) && blue > green.saturating_add(25) {
                    selection.push((x, y));
                }
                if red > 90 && green > red && blue < red.saturating_add(75) {
                    ink.push((x, y));
                }
            }
        }
        if ink.is_empty() || selection.is_empty() {
            return Err(format!(
                "real editor parts missing at phase {offset_px}: ink={} selection={}",
                ink.len(),
                selection.len(),
            ));
        }
        let selection_right = selection.iter().map(|&(x, _)| x).max().unwrap_or(0);
        let selection_top = selection.iter().map(|&(_, y)| y).min().unwrap_or(SIDE);
        let selection_bottom = selection.iter().map(|&(_, y)| y).max().unwrap_or(0);
        let caret = (selection_top..=selection_bottom)
            .flat_map(|y| {
                (selection_right.saturating_sub(1)..=(selection_right + 3)).map(move |x| (x, y))
            })
            .filter(|&(x, y)| {
                let [red, green, _] = caret_pixels.rgb(x, y);
                green > red.saturating_add(10)
                    && color_distance(caret_pixels.rgb(x, y), DARK_FILL) > 75
            })
            .collect::<Vec<_>>();
        assert!(
            !caret.is_empty(),
            "real caret is absent at phase {offset_px}"
        );
        let ink_bottom = ink.iter().map(|&(_, y)| y).max().unwrap_or(0);
        let ink_top = ink.iter().map(|&(_, y)| y).min().unwrap_or(SIDE);
        let caret_top = caret.iter().map(|&(_, y)| y).min().unwrap_or(SIDE);
        let caret_bottom = caret.iter().map(|&(_, y)| y).max().unwrap_or(0);
        let ink_right = ink.iter().map(|&(x, _)| x).max().unwrap_or(0);
        let caret_left = caret.iter().map(|&(x, _)| x).min().unwrap_or(SIDE);
        assert!(
            ink_top >= selection_top.saturating_sub(1)
                && ink_bottom <= selection_bottom + 1
                && caret_top <= ink_bottom
                && caret_bottom >= ink_top
                && caret_left >= ink_right.saturating_sub(1)
                && caret_left <= ink_right + 3,
            "real editor phase {offset_px}: ink y={ink_top}..{ink_bottom} right={ink_right}, selection y={selection_top}..{selection_bottom}, caret y={caret_top}..{caret_bottom} left={caret_left}"
        );
    }
    Ok(())
}

fn small_dark_glyph_open_spaces_have_no_isolated_black_pixel() -> Result<(), String> {
    let mut failures = Vec::new();
    for path in [DrawingPath::Standard, DrawingPath::Stable] {
        for whole_pixels_per_em in 6_u16..=9 {
            let pixels_per_em = f32::from(whole_pixels_per_em);
            for text in ["m", "0", "4"] {
                for offset_px in [0.125, 0.75] {
                    let scene = GlyphScene {
                        text,
                        fill: WHITE,
                        ink: BLACK,
                        font_mm: 12.0 * 25.4 / 72.0,
                        pixels_per_em,
                        offset_px,
                        offset_y_px: offset_px,
                        tilt_degrees: 0.0,
                        alpha_mode: AlphaMode::Blend,
                        drawing_path: path,
                    };
                    let pixels = render(scene)?;
                    let (left, top, right, bottom) = scene.text_bounds();
                    let mut lone = Vec::new();
                    let mut visible = 0;
                    for y in top + 2..bottom - 1 {
                        for x in left + 1..right - 1 {
                            if color_distance(pixels.rgb(x, y), scene.fill) > 24 {
                                visible += 1;
                            }
                        }
                    }
                    if visible == 0 {
                        return Err(format!(
                            "glyph={text} path={path:?} px/em={pixels_per_em} offset={offset_px:.2} drew no visible text pixels"
                        ));
                    }
                    for y in top + 2..bottom - 1 {
                        for x in left + 1..right - 1 {
                            if pixels.rgb(x, y).iter().all(|channel| *channel < 30)
                                && [x - 1, x + 1].into_iter().all(|neighbor_x| {
                                    pixels
                                        .rgb(neighbor_x, y)
                                        .iter()
                                        .all(|channel| *channel >= 128)
                                })
                                && (left + 1..x).any(|stem_x| {
                                    closer_to(pixels.rgb(stem_x, y), scene.ink, scene.fill)
                                })
                                && (x + 1..right).any(|stem_x| {
                                    closer_to(pixels.rgb(stem_x, y), scene.ink, scene.fill)
                                })
                            {
                                lone.push((x, y, pixels.rgb(x, y)));
                            }
                        }
                    }
                    eprintln!(
                        "glyph={text} path={path:?} px/em={pixels_per_em} offset={offset_px:.2} bounds=({left},{top})-({right},{bottom}) lone={lone:?}"
                    );
                    if !lone.is_empty() {
                        failures.push(format!("glyph={text} path={path:?} px/em={pixels_per_em} offset={offset_px:.2} lone={lone:?}"));
                    }
                }
            }
        }
    }
    assert!(
        failures.is_empty(),
        "isolated black samples: {}",
        failures.join("; ")
    );
    Ok(())
}

fn premultiplied_text_edges_stay_between_ink_and_fill() -> Result<(), String> {
    let scene = GlyphScene {
        text:          "HELLO",
        fill:          [196, 176, 157],
        ink:           [226, 96, 55],
        font_mm:       20.0,
        pixels_per_em: 36.0,
        offset_px:     0.0,
        offset_y_px:   0.0,
        tilt_degrees:  0.0,
        alpha_mode:    AlphaMode::Premultiplied,
        drawing_path:  DrawingPath::Standard,
    };
    let pixels = render(scene)?;
    let (left, top, right, bottom) = pixels.ink_bounds(scene)?;
    let drawn_fill = pixels.rgb(SIDE / 2, SIDE / 2);
    let pixel_ref = &pixels;
    let drawn_ink = (top..=bottom)
        .flat_map(|y| (left..=right).map(move |x| pixel_ref.rgb(x, y)))
        .min_by_key(|pixel| color_distance(*pixel, scene.ink))
        .ok_or_else(|| "premultiplied text has no ink pixels".to_string())?;
    let mut fringe = Vec::new();
    let mut graded_edges = 0;
    for y in top..=bottom {
        for x in left..=right {
            let pixel = pixels.rgb(x, y);
            if pixel
                .into_iter()
                .zip(drawn_fill)
                .zip(drawn_ink)
                .any(|((actual, fill), ink)| actual > fill.max(ink).saturating_add(2))
            {
                fringe.push((x, y, pixel));
            }
            if color_distance(pixel, drawn_fill) > 3
                && color_distance(pixel, drawn_ink) > 3
                && pixel
                    .into_iter()
                    .zip(drawn_fill)
                    .zip(drawn_ink)
                    .all(|((actual, fill), ink)| actual >= fill.min(ink) && actual <= fill.max(ink))
            {
                graded_edges += 1;
            }
        }
    }
    eprintln!(
        "premultiplied bounds=({left},{top})-({right},{bottom}) drawn_fill={drawn_fill:?} drawn_ink={drawn_ink:?} graded_edges={graded_edges} fringe={fringe:?}"
    );
    assert!(
        fringe.is_empty(),
        "premultiplied edge has {} light fringe pixels: {fringe:?}",
        fringe.len()
    );
    assert!(
        graded_edges > 0,
        "premultiplied text has no edge between drawn ink {drawn_ink:?} and fill {drawn_fill:?}"
    );
    Ok(())
}

fn premultiplied_world_text_has_no_light_fringe_on_lit_cube() -> Result<(), String> {
    let blank = render_world_text(AlphaMode::Blend, "")?;
    let blended = render_world_text(AlphaMode::Blend, "HELLO")?;
    let premultiplied = render_world_text(AlphaMode::Premultiplied, "HELLO")?;
    let mut drawn = 0;
    for y in 48..208 {
        for x in 48..208 {
            if color_distance(blended.rgb(x, y), blank.rgb(x, y)) > 8 {
                drawn += 1;
            }
        }
    }
    assert!(drawn > 20, "lit cube drew too few HELLO pixels: {drawn}");

    let mut fringe = Vec::new();
    for y in 48..208 {
        for x in 48..208 {
            let fill = blank.rgb(x, y);
            if color_distance(blended.rgb(x, y), fill) <= 8 {
                continue;
            }
            let pixel = premultiplied.rgb(x, y);
            let ink = blended.rgb(x, y);
            if (0..3)
                .all(|channel| pixel[channel] > fill[channel].max(ink[channel]).saturating_add(2))
            {
                fringe.push((x, y, pixel, ink, fill));
            }
        }
    }
    eprintln!("lit cube drawn={drawn} fringe={fringe:?}");
    assert!(fringe.is_empty(), "lit cube has light fringe: {fringe:?}");
    Ok(())
}

fn oit_panel_text_changes_its_own_fill() -> Result<(), String> {
    for alpha in [
        AlphaMode::Add,
        AlphaMode::Multiply,
        AlphaMode::Premultiplied,
    ] {
        let scene = GlyphScene {
            text:          "HELLO",
            fill:          [80, 100, 74],
            ink:           if alpha == AlphaMode::Multiply {
                [30, 40, 25]
            } else {
                [232, 237, 241]
            },
            font_mm:       20.0,
            pixels_per_em: 36.0,
            offset_px:     0.0,
            offset_y_px:   0.0,
            tilt_degrees:  0.0,
            alpha_mode:    alpha,
            drawing_path:  DrawingPath::Stable,
        };
        let pixels = render(scene)?;
        let (left, top, right, bottom) = pixels.ink_bounds(scene)?;
        let strongest = (top..=bottom)
            .flat_map(|y| (left..=right).map(move |x| (x, y)))
            .map(|(x, y)| (x, y, pixels.rgb(x, y)))
            .max_by_key(|(_, _, pixel)| color_distance(*pixel, scene.fill))
            .ok_or_else(|| format!("OIT {alpha:?} text has no ink bounds"))?;
        eprintln!("OIT alpha={alpha:?} strongest={strongest:?}");
        assert!(
            strongest
                .2
                .into_iter()
                .zip(scene.fill)
                .any(|(actual, fill)| actual.abs_diff(fill) > 1),
            "OIT {alpha:?} text stayed at fill colour"
        );
    }
    Ok(())
}

#[test]
fn drawn_small_glyph_cases_share_a_renderer() -> Result<(), String> {
    drawn_support::run_cases(&[
        DrawnCase {
            name: "units_a4_body_has_no_more_lone_near_black_pixels_than_pre_phase",
            run:  units_a4_body_has_no_more_lone_near_black_pixels_than_pre_phase,
        },
        DrawnCase {
            name: "settled_small_text_ink_has_no_new_neighboring_size_jumps",
            run:  settled_small_text_ink_has_no_new_neighboring_size_jumps,
        },
        DrawnCase {
            name: "small_text_glides_then_settles_without_idle_uploads_or_redraws",
            run:  small_text_glides_then_settles_without_idle_uploads_or_redraws,
        },
        DrawnCase {
            name: "eights_in_short_runs_keep_both_counters_lighter_than_ink_at_every_phase_and_tilt",
            run:  eights_in_short_runs_keep_both_counters_lighter_than_ink_at_every_phase_and_tilt,
        },
        DrawnCase {
            name: "zero_dot_is_absent_through_nine_pixels_per_em_and_draws_at_ten",
            run:  zero_dot_is_absent_through_nine_pixels_per_em_and_draws_at_ten,
        },
        DrawnCase {
            name: "small_zero_and_eight_counters_stay_below_sixty_percent_of_their_walls",
            run:  small_zero_and_eight_counters_stay_below_sixty_percent_of_their_walls,
        },
        DrawnCase {
            name: "small_text_weight_and_position_change_smoothly_at_fit_boundaries",
            run:  small_text_weight_and_position_change_smoothly_at_fit_boundaries,
        },
        DrawnCase {
            name: "clipped_small_text_run_draws_no_ink_past_its_panel_edge",
            run:  clipped_small_text_run_draws_no_ink_past_its_panel_edge,
        },
        DrawnCase {
            name: "real_inline_editor_caret_and_selection_align_with_small_glyph",
            run:  real_inline_editor_caret_and_selection_align_with_small_glyph,
        },
        DrawnCase {
            name: "small_dark_glyph_open_spaces_have_no_isolated_black_pixel",
            run:  small_dark_glyph_open_spaces_have_no_isolated_black_pixel,
        },
        DrawnCase {
            name: "premultiplied_text_edges_stay_between_ink_and_fill",
            run:  premultiplied_text_edges_stay_between_ink_and_fill,
        },
        DrawnCase {
            name: "premultiplied_world_text_has_no_light_fringe_on_lit_cube",
            run:  premultiplied_world_text_has_no_light_fringe_on_lit_cube,
        },
        DrawnCase {
            name: "oit_panel_text_changes_its_own_fill",
            run:  oit_panel_text_changes_its_own_fill,
        },
    ])
}
