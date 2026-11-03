//! Drawn-pixel coverage for Lucide icons on world and screen panels.

mod drawn_support;

use std::collections::HashSet;

use bevy::camera::ClearColorConfig;
use bevy::camera::Hdr;
use bevy::camera::RenderTarget;
use bevy::camera::ScalingMode;
use bevy::camera::visibility::RenderLayers;
use bevy::prelude::*;
use bevy::render::view::DebandDither;
use bevy::render::view::Tonemapping;
use bevy::window::PrimaryWindow;
use bevy::window::WindowRef;
use bevy::window::WindowResolution;
use drawn_support::CaptureRegion;
use drawn_support::CaseApp;
use drawn_support::DrawnCase;
use drawn_support::dump_pixels;
use drawn_support::run_cases;
use hana_diegetic::AlignX;
use hana_diegetic::AlignY;
use hana_diegetic::Anchor;
use hana_diegetic::CascadeEntityCommandsExt as _;
use hana_diegetic::ComputedDiegeticPanel;
use hana_diegetic::DiegeticIcon;
use hana_diegetic::DiegeticPanel;
use hana_diegetic::DiegeticTextMeasurer;
use hana_diegetic::El;
use hana_diegetic::FontRegistered;
use hana_diegetic::FontRegistry;
use hana_diegetic::HeadlessDiegeticUiPlugin;
use hana_diegetic::Icon;
use hana_diegetic::LayoutBuilder;
use hana_diegetic::Lighting;
use hana_diegetic::LucideIcon;
use hana_diegetic::Mm;
use hana_diegetic::PanelTextLayout;
use hana_diegetic::PanelTextRuns;
use hana_diegetic::Pt;
use hana_diegetic::Px;
use hana_diegetic::RenderCommandKind;
use hana_diegetic::ShadowCasting;
use hana_diegetic::Text;
use hana_diegetic::TextContent;
use hana_diegetic::TextInkMeasurement;
use hana_diegetic::TextStyle;
use hana_diegetic::Unit;

const VIEW: UVec2 = UVec2::splat(256);
const VIEW_SIDE: f32 = 256.0;
const WORLD_VIEW_HEIGHT: f32 = 0.256;
const ICON_SIZE: u32 = 64;
const ICON_SIZE_F32: f32 = 64.0;
const ICON_SIZE_POINTS: f32 = ICON_SIZE_F32 * Unit::Pixels.to_points();
const ICON_BOX_MIN: UVec2 = UVec2::splat((VIEW.x - ICON_SIZE) / 2);
const ICON_BOX_MAX: UVec2 = UVec2::splat(u32::midpoint(VIEW.x, ICON_SIZE));
const PANEL_LAYER: usize = 31;
const SCREEN_CAPTURE_DISTANCE: f32 = 1_000.0;
const ICON_COLOR: Color = Color::srgb_u8(36, 220, 92);
const LUCIDE_FONT_FAMILY: &str = "lucide";
const SCREEN_ICON_POSITION: Vec2 = Vec2::new(44.0, 70.0);

#[derive(Resource, Default)]
struct LucideRegistrationEvents(usize);

#[derive(Clone, Copy, Debug)]
struct PixelBounds {
    min: UVec2,
    max: UVec2,
}

impl PixelBounds {
    fn include(&mut self, pixel: UVec2) {
        self.min = self.min.min(pixel);
        self.max = self.max.max(pixel);
    }
}

#[derive(Debug)]
struct IconPixels {
    pixels: Vec<u8>,
    ink:    HashSet<UVec2>,
    bounds: PixelBounds,
}

fn pixel(pixels: &[u8], point: UVec2) -> Result<[u8; 4], String> {
    if point.cmpge(VIEW).any() {
        return Err(format!("pixel {point:?} is outside the {VIEW:?} capture"));
    }
    let offset = ((point.y * VIEW.x + point.x) * 4) as usize;
    let rgba = pixels
        .get(offset..offset + 4)
        .ok_or_else(|| format!("capture has no pixel at {point:?}"))?;
    Ok([rgba[0], rgba[1], rgba[2], rgba[3]])
}

/// A hue bucket rather than an exact color, because coverage and GPU dithering
/// can change every channel while retaining the authored green.
const fn is_icon_ink(rgba: [u8; 4]) -> bool {
    let [red, green, blue, _] = rgba;
    green > 24 && green.saturating_sub(red) > 12 && green.saturating_sub(blue) > 8
}

const fn is_solid_icon_ink(rgba: [u8; 4]) -> bool {
    let [red, green, blue, _] = rgba;
    green > 110 && green.saturating_sub(red) > 40 && green.saturating_sub(blue) > 28
}

fn icon_pixels(pixels: Vec<u8>) -> Result<IconPixels, String> {
    let mut ink = HashSet::new();
    let mut bounds = None;
    for y in 0..VIEW.y {
        for x in 0..VIEW.x {
            let point = UVec2::new(x, y);
            if !is_icon_ink(pixel(&pixels, point)?) {
                continue;
            }
            ink.insert(point);
            let bounds = bounds.get_or_insert(PixelBounds {
                min: point,
                max: point,
            });
            bounds.include(point);
        }
    }
    let bounds = bounds.ok_or_else(|| "capture contains no icon-colored pixels".to_string())?;
    Ok(IconPixels {
        pixels,
        ink,
        bounds,
    })
}

fn count_lucide_registration(
    trigger: On<FontRegistered>,
    mut registrations: ResMut<LucideRegistrationEvents>,
) {
    if trigger.name == LUCIDE_FONT_FAMILY {
        registrations.0 += 1;
    }
}

fn primary_window(app: &mut App) -> Entity {
    let mut resolution = WindowResolution::new(VIEW.x, VIEW.y);
    resolution.set_scale_factor_override(Some(1.0));
    app.world_mut()
        .spawn((
            Window {
                resolution,
                ..default()
            },
            PrimaryWindow,
        ))
        .id()
}

fn spawn_display_camera(app: &mut App, window: Entity) {
    app.world_mut().spawn((
        Camera3d::default(),
        Camera {
            order: 0,
            ..default()
        },
        RenderTarget::Window(WindowRef::Entity(window)),
        Hdr,
        Tonemapping::KhronosPbrNeutral,
        DebandDither::Disabled,
        Transform::from_xyz(0.0, 0.0, 1.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
}

fn spawn_world_capture_camera(app: &mut App, target: Handle<Image>, region: CaptureRegion) {
    let mut camera = Camera {
        order: 1,
        clear_color: ClearColorConfig::Custom(Color::BLACK),
        ..default()
    };
    region.apply_to_camera(&mut camera);
    app.world_mut().spawn((
        Camera3d::default(),
        camera,
        Projection::Orthographic(OrthographicProjection {
            scaling_mode: ScalingMode::FixedVertical {
                viewport_height: WORLD_VIEW_HEIGHT,
            },
            ..OrthographicProjection::default_3d()
        }),
        RenderTarget::Image(target.into()),
        Hdr,
        Msaa::Off,
        Tonemapping::None,
        DebandDither::Disabled,
        Transform::from_xyz(0.0, 0.0, 1.0).looking_at(Vec3::ZERO, Vec3::Y),
        RenderLayers::layer(PANEL_LAYER),
    ));
}

fn centered_icon_tree(icon: LucideIcon) -> hana_diegetic::LayoutTree {
    let mut layout = LayoutBuilder::with_root(
        El::overlay()
            .size(Px(VIEW_SIDE), Px(VIEW_SIDE))
            .alignment(AlignX::Center, AlignY::Center),
    );
    layout.icon(Icon::new(icon).size(ICON_SIZE_POINTS).color(ICON_COLOR));
    layout.build()
}

fn spawn_world_icon(app: &mut App, icon: LucideIcon, rotation: Quat) -> Result<Entity, String> {
    let panel = DiegeticPanel::world()
        .size(Px(VIEW_SIDE), Px(VIEW_SIDE))
        .world_width(WORLD_VIEW_HEIGHT)
        .font_unit(Unit::Pixels)
        .anchor(Anchor::Center)
        .shadow_casting(ShadowCasting::Off)
        .with_tree(centered_icon_tree(icon))
        .build()
        .map_err(|error| format!("invalid icon panel: {error}"))?;
    let panel = app
        .world_mut()
        .spawn((
            panel,
            Transform::from_rotation(rotation),
            RenderLayers::layer(PANEL_LAYER),
        ))
        .id();
    app.world_mut()
        .commands()
        .entity(panel)
        .override_lighting(Lighting::Unlit);
    Ok(panel)
}

fn capture_world_icon(icon: LucideIcon, rotation: Quat) -> Result<IconPixels, String> {
    icon_pixels(capture_world_icon_pixels(icon, rotation)?)
}

fn capture_world_icon_pixels(icon: LucideIcon, rotation: Quat) -> Result<Vec<u8>, String> {
    let mut app = CaseApp::new();
    let window = primary_window(&mut app);
    spawn_display_camera(&mut app, window);
    let region = CaptureRegion::new(VIEW, UVec2::ZERO, VIEW);
    let target = app.target(region);
    spawn_world_capture_camera(&mut app, target.clone(), region);
    spawn_world_icon(&mut app, icon, rotation)?;
    app.settle()?;
    let pixels = app.capture(target)?;
    dump_pixels::<false>(
        "icon_drawing",
        "",
        &format!("icon={} rotation={rotation:?}", icon.name()),
        region,
        region,
        &pixels,
    )?;
    Ok(pixels)
}

// Keep the original outline and layout assertions alongside the pixel capture.
// The shared fixture drains screenshot readback and cleans the scene before
// returning its renderer; this test must not drop a newly started GPU app.
fn assert_zap_layout_ink_centred(app: &CaseApp, panel: Entity, side: f32) -> Result<(), String> {
    let computed = app
        .world()
        .get::<ComputedDiegeticPanel>(panel)
        .ok_or("computed panel")?;
    let solved = computed.layout().solved().ok_or("solved panel")?;
    let slot = solved
        .commands
        .iter()
        .find_map(|command| {
            matches!(command.kind, RenderCommandKind::Icon(_)).then_some(command.bounds)
        })
        .ok_or("icon layout box")?;
    let runs = app
        .world()
        .get::<PanelTextRuns>(panel)
        .ok_or("icon text runs")?;
    let run = runs.iter().next().ok_or("reified icon")?;
    let layout = app
        .world()
        .get::<PanelTextLayout>(run)
        .ok_or("icon draw bounds")?;
    let text = app.world().get::<TextContent>(run).ok_or("icon glyph")?;
    let style = app.world().get::<TextStyle>(run).ok_or("icon style")?;
    let TextInkMeasurement::Visible(ink) = app
        .world()
        .resource::<FontRegistry>()
        .measure_ink(text.text(), &style.as_measure())
        .map_err(|error| error.to_string())?
    else {
        return Err("Zap must have visible glyph ink".to_owned());
    };
    let ink_centre = Vec2::new(
        layout.bounds.x + f32::midpoint(ink.left, ink.right),
        layout.bounds.y + f32::midpoint(ink.top, ink.bottom),
    );
    let box_centre = Vec2::new(
        f32::mul_add(slot.width, 0.5, slot.x),
        f32::mul_add(slot.height, 0.5, slot.y),
    );
    // Convert point distances to logical pixels. Compare opposite padding,
    // as the rendered trigger regression does, on both axes.
    let padding_difference = (ink_centre - box_centre).abs() * 2.0 / Unit::Pixels.to_points();
    assert!(
        padding_difference.cmple(Vec2::ONE).all(),
        "{side}pt Zap ink is off-centre: opposite padding differs by {padding_difference:?}; ink {ink:?}, draw {:?}, slot {slot:?}",
        layout.bounds
    );
    assert_eq!(
        Vec2::new(slot.width, slot.height),
        Vec2::splat(side),
        "ink placement must keep the icon's layout size"
    );
    Ok(())
}

#[test]
fn zap_ink_is_centred_in_its_layout_box_within_one_pixel() -> Result<(), String> {
    for side in [64.0, 128.0] {
        let mut app = CaseApp::new();
        let window = primary_window(&mut app);
        spawn_display_camera(&mut app, window);
        let region = CaptureRegion::new(VIEW, UVec2::ZERO, VIEW);
        let target = app.target(region);
        spawn_world_capture_camera(&mut app, target.clone(), region);
        let mut builder = LayoutBuilder::with_root(
            El::overlay()
                .size(Pt(VIEW_SIDE), Pt(VIEW_SIDE))
                .alignment(AlignX::Center, AlignY::Center),
        );
        builder.icon(Icon::new(LucideIcon::Zap).size(side).color(ICON_COLOR));
        let panel = DiegeticPanel::world()
            .size(Pt(VIEW_SIDE), Pt(VIEW_SIDE))
            .world_width(WORLD_VIEW_HEIGHT)
            .font_unit(Unit::Points)
            .anchor(Anchor::Center)
            .shadow_casting(ShadowCasting::Off)
            .with_tree(builder.build())
            .build()
            .map_err(|error| error.to_string())?;
        let panel = app
            .world_mut()
            .spawn((
                panel,
                Transform::default(),
                RenderLayers::layer(PANEL_LAYER),
            ))
            .id();
        app.settle()?;
        let drawn = icon_pixels(app.capture(target)?)?;
        assert_zap_layout_ink_centred(&app, panel, side)?;
        let padding_difference =
            (drawn.bounds.min.as_vec2() - (VIEW - UVec2::ONE - drawn.bounds.max).as_vec2()).abs();
        assert!(
            padding_difference.cmple(Vec2::ONE).all(),
            "{side}pt Zap pixels are off-centre: opposite padding differs by {padding_difference:?}; ink {:?}",
            drawn.bounds
        );
    }
    Ok(())
}

fn world_icon_stays_inside_its_box_and_uses_its_color() -> Result<(), String> {
    let drawn = capture_world_icon(LucideIcon::CircleCheck, Quat::IDENTITY)?;
    if drawn.bounds.min.cmplt(ICON_BOX_MIN).any() || drawn.bounds.max.cmpge(ICON_BOX_MAX).any() {
        return Err(format!(
            "icon ink {:?} escapes its {ICON_BOX_MIN:?}..{ICON_BOX_MAX:?} size box",
            drawn.bounds
        ));
    }

    for y in 0..VIEW.y {
        for x in 0..VIEW.x {
            let point = UVec2::new(x, y);
            let inside = point.cmpge(ICON_BOX_MIN).all() && point.cmplt(ICON_BOX_MAX).all();
            let [red, green, blue, _] = pixel(&drawn.pixels, point)?;
            if !inside && red.max(green).max(blue) > 8 {
                return Err(format!(
                    "pixel {point:?} outside the icon's size box was drawn as {red},{green},{blue}"
                ));
            }
        }
    }

    let strongest = drawn
        .ink
        .iter()
        .map(|&point| pixel(&drawn.pixels, point))
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .max_by_key(|rgba| rgba[1])
        .ok_or_else(|| "icon has no requested-color pixels".to_string())?;
    if !is_solid_icon_ink(strongest) {
        return Err(format!(
            "strongest icon pixel {strongest:?} is outside the requested green color bucket"
        ));
    }
    Ok(())
}

fn world_icon_draws_on_a_panel_slanted_seventy_degrees() -> Result<(), String> {
    let drawn = capture_world_icon(
        LucideIcon::CircleCheck,
        Quat::from_rotation_y(70.0_f32.to_radians()),
    )?;
    if drawn.ink.len() < 20 {
        return Err(format!(
            "70-degree icon drew only {} coverage pixels at {:?}",
            drawn.ink.len(),
            drawn.bounds
        ));
    }
    Ok(())
}

fn world_icon_draws_only_on_the_front() -> Result<(), String> {
    let front = capture_world_icon(LucideIcon::ArrowRight, Quat::IDENTITY)?;
    let rear = capture_world_icon_pixels(
        LucideIcon::ArrowRight,
        Quat::from_rotation_y(std::f32::consts::PI),
    )?;
    let front_solid = front
        .ink
        .iter()
        .filter(|&&point| pixel(&front.pixels, point).is_ok_and(is_solid_icon_ink))
        .count();
    assert!(
        front_solid > 20,
        "positive control: the front icon must be whole"
    );
    let rear_ink = rear
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|&&rgba| is_icon_ink(rgba))
        .count();
    assert_eq!(rear_ink, 0, "icon ink must never draw mirrored from behind");
    Ok(())
}

fn spawn_screen_capture_camera(
    app: &mut App,
    target: Handle<Image>,
    layers: RenderLayers,
    region: CaptureRegion,
) {
    let mut camera = Camera {
        order: 110,
        clear_color: ClearColorConfig::Custom(Color::BLACK),
        ..default()
    };
    region.apply_to_camera(&mut camera);
    app.world_mut().spawn((
        Camera3d::default(),
        camera,
        RenderTarget::Image(target.into()),
        Projection::Orthographic(OrthographicProjection {
            scaling_mode: ScalingMode::FixedVertical {
                viewport_height: VIEW_SIDE,
            },
            far: 2.0 * SCREEN_CAPTURE_DISTANCE,
            ..OrthographicProjection::default_3d()
        }),
        Msaa::Off,
        Tonemapping::None,
        DebandDither::Disabled,
        Transform::from_xyz(0.0, 0.0, SCREEN_CAPTURE_DISTANCE).looking_at(Vec3::ZERO, Vec3::Y),
        layers,
    ));
}

fn screen_icon_draws_at_its_position() -> Result<(), String> {
    let mut app = CaseApp::new();
    let window = primary_window(&mut app);
    spawn_display_camera(&mut app, window);
    let icon = {
        let mut commands = app.world_mut().commands();
        DiegeticIcon::screen(LucideIcon::CircleCheck)
            .size(ICON_SIZE_POINTS)
            .color(ICON_COLOR)
            .screen_position(SCREEN_ICON_POSITION)
            .spawn(&mut commands)
    };
    app.world_mut().flush();
    for _ in 0..4 {
        app.update();
    }
    let layers = app
        .world()
        .get::<RenderLayers>(icon)
        .cloned()
        .ok_or_else(|| "screen icon has no overlay render layers".to_string())?;
    let region = CaptureRegion::new(VIEW, UVec2::ZERO, VIEW);
    let target = app.target(region);
    spawn_screen_capture_camera(&mut app, target.clone(), layers, region);
    app.settle()?;
    let pixels = app.capture(target)?;
    dump_pixels::<false>("icon_drawing", "", "screen icon", region, region, &pixels)?;
    let drawn = icon_pixels(pixels)?;
    let expected_min = SCREEN_ICON_POSITION.as_uvec2();
    let expected_max = expected_min + UVec2::splat(ICON_SIZE);
    if drawn.bounds.min.cmplt(expected_min).any() || drawn.bounds.max.cmpge(expected_max).any() {
        return Err(format!(
            "screen icon ink {:?} lies outside positioned box {expected_min:?}..{expected_max:?}",
            drawn.bounds
        ));
    }
    Ok(())
}

fn icon_has_its_box_before_the_font_registers() -> Result<(), String> {
    let panel = DiegeticPanel::world()
        .size(Px(VIEW_SIDE), Px(VIEW_SIDE))
        .font_unit(Unit::Pixels)
        .with_tree(centered_icon_tree(LucideIcon::CircleCheck))
        .build()
        .map_err(|error| format!("invalid pre-registration icon panel: {error}"))?;
    let mut app = App::new();
    app.init_resource::<LucideRegistrationEvents>()
        .add_observer(count_lucide_registration)
        .add_plugins(MinimalPlugins)
        .insert_resource(DiegeticTextMeasurer::default())
        .add_plugins(HeadlessDiegeticUiPlugin);
    let panel = app.world_mut().spawn(panel).id();
    app.update();
    let registrations = app.world().resource::<LucideRegistrationEvents>().0;
    if registrations != 0 {
        return Err(format!(
            "pre-registration layout observed {registrations} Lucide font registrations"
        ));
    }

    let computed = app
        .world()
        .get::<ComputedDiegeticPanel>(panel)
        .ok_or_else(|| "pre-registration icon panel has no computed layout".to_string())?;
    let result = computed
        .layout()
        .solved()
        .ok_or_else(|| "pre-registration icon panel did not solve its layout".to_string())?;
    let icon = result
        .computed
        .get(1)
        .ok_or_else(|| "pre-registration layout has no icon leaf".to_string())?
        .bounds;
    let expected = ICON_SIZE_F32 * Unit::Pixels.to_points();
    let error = (Vec2::new(icon.width, icon.height) - Vec2::splat(expected))
        .abs()
        .max_element();
    if error > 0.001 {
        return Err(format!(
            "icon box before font registration is {} x {}, expected {expected} x {expected} \
             layout points",
            icon.width, icon.height
        ));
    }
    Ok(())
}

#[test]
fn icon_height_follows_the_text_size_in_points() -> Result<(), String> {
    const TOLERANCE: f32 = 0.001;
    const LABEL: &str = "Icon label";

    for size in [17.0, 29.0] {
        let mut builder = LayoutBuilder::with_root(El::column().size(Mm(100.0), Mm(100.0)));
        builder.icon(Icon::new(LucideIcon::Eye).size(size));
        builder.text(Text::new(LABEL, TextStyle::new(Pt(size))));
        let panel = DiegeticPanel::world()
            .size(Mm(100.0), Mm(100.0))
            .font_unit(Unit::Points)
            .with_tree(builder.build())
            .build()
            .map_err(|error| format!("invalid {size}pt icon/text panel: {error}"))?;
        let mut app = App::new();
        // Keep line height equal to the em size, independent of font metrics.
        app.add_plugins(MinimalPlugins)
            .insert_resource(DiegeticTextMeasurer::default())
            .add_plugins(HeadlessDiegeticUiPlugin);
        let panel = app.world_mut().spawn(panel).id();
        app.update();
        let computed = app
            .world()
            .get::<ComputedDiegeticPanel>(panel)
            .ok_or_else(|| format!("{size}pt icon/text panel has no computed layout"))?;
        let layout = computed
            .layout()
            .solved()
            .ok_or_else(|| format!("{size}pt icon/text panel has no solved layout"))?;
        let icon_height = layout
            .commands
            .iter()
            .find_map(|command| {
                matches!(&command.kind, RenderCommandKind::Icon(_)).then_some(command.bounds.height)
            })
            .ok_or_else(|| format!("{size}pt panel has no solved icon"))?;
        let line_height = layout
            .commands
            .iter()
            .find_map(|command| match &command.kind {
                RenderCommandKind::Text { text, .. } if text == LABEL => {
                    Some(command.bounds.height)
                },
                _ => None,
            })
            .ok_or_else(|| format!("{size}pt panel has no solved text line"))?;
        assert!(
            (line_height - size).abs() <= TOLERANCE,
            "{size}pt text solved to a {line_height}pt line height"
        );
        assert!(
            (icon_height - line_height).abs() <= TOLERANCE,
            "same {size}pt size solved to a {icon_height}pt icon and a {line_height}pt text line"
        );
    }
    Ok(())
}

#[test]
fn icons_draw_through_slug_on_panels_and_screen() -> Result<(), String> {
    run_cases(&[
        DrawnCase {
            name: "icon_has_its_box_before_the_font_registers",
            run:  icon_has_its_box_before_the_font_registers,
        },
        DrawnCase {
            name: "world_icon_stays_inside_its_box_and_uses_its_color",
            run:  world_icon_stays_inside_its_box_and_uses_its_color,
        },
        DrawnCase {
            name: "world_icon_draws_on_a_panel_slanted_seventy_degrees",
            run:  world_icon_draws_on_a_panel_slanted_seventy_degrees,
        },
        DrawnCase {
            name: "world_icon_draws_only_on_the_front",
            run:  world_icon_draws_only_on_the_front,
        },
        DrawnCase {
            name: "screen_icon_draws_at_its_position",
            run:  screen_icon_draws_at_its_position,
        },
    ])
}
