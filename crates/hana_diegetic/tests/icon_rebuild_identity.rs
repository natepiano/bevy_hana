//! Rebuilding a panel must neither retain nor redraw an icon's old text run.

mod drawn_support;

use bevy::camera::Hdr;
use bevy::camera::RenderTarget;
use bevy::camera::ScalingMode;
use bevy::ecs::system::RunSystemOnce;
use bevy::prelude::*;
use bevy::render::view::DebandDither;
use bevy::render::view::Tonemapping;
use bevy::window::WindowRef;
use drawn_support::CaptureRegion;
use drawn_support::CaseApp;
use drawn_support::DrawnCase;
use hana_diegetic::AlignY;
use hana_diegetic::Anchor;
use hana_diegetic::CascadeEntityCommandsExt as _;
use hana_diegetic::ComputedDiegeticPanel;
use hana_diegetic::DiegeticPanel;
use hana_diegetic::DiegeticPanelCommands as _;
use hana_diegetic::El;
use hana_diegetic::GlyphShadowMode;
use hana_diegetic::Icon;
use hana_diegetic::LayoutBuilder;
use hana_diegetic::LayoutTree;
use hana_diegetic::Lighting;
use hana_diegetic::LucideIcon;
use hana_diegetic::Mm;
use hana_diegetic::Padding;
use hana_diegetic::PanelElementId;
use hana_diegetic::PanelTextLayout;
use hana_diegetic::PanelTextRuns;
use hana_diegetic::RenderCommandKind;
use hana_diegetic::StableTransparency;
use hana_diegetic::Text;
use hana_diegetic::TextContent;
use hana_diegetic::TextStyle;
use hana_diegetic::Unit;

const SLOT_SIZE: UVec2 = UVec2::new(192, 128);
const VIEW_SIZE: UVec2 = UVec2::new(SLOT_SIZE.x * 2, SLOT_SIZE.y);
const VIEW_HEIGHT: f32 = 0.128;
const PANEL_SLOT_X: f32 = 0.096;
const PANEL_SIZE: Vec2 = Vec2::new(128.0, 64.0);
const ICON_SIZE: f32 = 32.0;
const ICON_COLOR: Color = Color::srgb_u8(36, 220, 92);

struct ReifiedIconRun {
    entity:      Entity,
    id:          PanelElementId,
    line_index:  usize,
    element_idx: usize,
}

struct IconRebuildScene {
    app:    CaseApp,
    target: Handle<Image>,
    panel:  Entity,
}

fn invisible_collision_text(builder: &mut LayoutBuilder, text: &'static str) {
    builder.text(
        Text::new(
            text,
            TextStyle::new(Mm(12.0))
                .with_color(Color::BLACK)
                .with_shadow_mode(GlyphShadowMode::None),
        )
        .layout(El::new().size(12.0, ICON_SIZE)),
    );
}

fn icon_tree(with_leading_text: bool) -> LayoutTree {
    let mut builder = LayoutBuilder::with_root(
        El::row()
            .size(PANEL_SIZE.x, PANEL_SIZE.y)
            .padding(Padding::all(8.0))
            .gap(8.0)
            .align_y(AlignY::Center),
    );
    if with_leading_text {
        invisible_collision_text(&mut builder, "leading");
    }
    builder.icon(
        Icon::new(LucideIcon::Camera)
            .size(ICON_SIZE * Unit::Millimeters.to_points())
            .color(ICON_COLOR),
    );
    invisible_collision_text(&mut builder, "first");
    invisible_collision_text(&mut builder, "second");
    builder.build()
}

fn spawn_camera(app: &mut CaseApp, target: Handle<Image>, region: CaptureRegion) {
    app.world_mut().spawn((
        Camera3d::default(),
        RenderTarget::Window(WindowRef::Primary),
        Hdr,
        Tonemapping::KhronosPbrNeutral,
        DebandDither::Disabled,
    ));
    let mut image_camera = Camera::default();
    region.apply_to_camera(&mut image_camera);
    app.world_mut().spawn((
        Camera3d::default(),
        image_camera,
        Projection::Orthographic(OrthographicProjection {
            scaling_mode: ScalingMode::FixedVertical {
                viewport_height: VIEW_HEIGHT,
            },
            ..OrthographicProjection::default_3d()
        }),
        RenderTarget::Image(target.into()),
        Hdr,
        Tonemapping::KhronosPbrNeutral,
        // Screen-space noise differs between the two comparison slots.
        DebandDither::Disabled,
        Msaa::Off,
        StableTransparency,
        Transform::from_xyz(0.0, 0.0, 0.25).looking_at(Vec3::ZERO, Vec3::Y),
    ));
}

fn spawn_panel(app: &mut CaseApp, tree: LayoutTree, slot_x: f32) -> Result<Entity, String> {
    let panel = DiegeticPanel::world()
        .size(Mm(PANEL_SIZE.x), Mm(PANEL_SIZE.y))
        .anchor(Anchor::Center)
        .with_tree(tree)
        .build()
        .map_err(|error| format!("invalid icon identity panel: {error}"))?;
    let panel = app
        .world_mut()
        .spawn((panel, Transform::from_xyz(slot_x, 0.0, 0.0)))
        .id();
    app.world_mut()
        .commands()
        .entity(panel)
        .override_lighting(Lighting::Unlit);
    Ok(panel)
}

fn setup_scene() -> Result<IconRebuildScene, String> {
    let mut app = CaseApp::new();
    let region = CaptureRegion::new(VIEW_SIZE, UVec2::ZERO, VIEW_SIZE);
    let target = app.target(region);
    spawn_camera(&mut app, target.clone(), region);
    let panel = spawn_panel(&mut app, icon_tree(false), -PANEL_SLOT_X)?;
    spawn_panel(&mut app, icon_tree(true), PANEL_SLOT_X)?;
    app.settle()?;
    Ok(IconRebuildScene { app, target, panel })
}

fn rebuild_panel(scene: &mut IconRebuildScene) -> Result<(), String> {
    let panel = scene.panel;
    let tree = icon_tree(true);
    scene
        .app
        .world_mut()
        .run_system_once(move |mut commands: Commands| commands.set_tree(panel, tree.clone()))
        .map_err(|error| format!("could not run icon panel rebuild: {error}"))?
        .map_err(|error| format!("invalid rebuilt icon panel: {error}"))?;
    for _ in 0..4 {
        scene.app.update();
    }
    Ok(())
}

fn solved_icon_element(app: &App, panel: Entity) -> Result<usize, String> {
    let computed = app
        .world()
        .get::<ComputedDiegeticPanel>(panel)
        .ok_or_else(|| "rebuilt icon panel has no computed layout".to_owned())?;
    let solved = computed
        .layout()
        .solved()
        .ok_or_else(|| "rebuilt icon panel layout is not solved".to_owned())?;
    let elements = solved
        .commands
        .iter()
        .filter_map(|command| {
            matches!(&command.kind, RenderCommandKind::Icon(_)).then_some(command.element_idx)
        })
        .collect::<Vec<_>>();
    let [element_idx] = elements.as_slice() else {
        return Err(format!(
            "rebuilt panel expected one solved icon command, found {}",
            elements.len()
        ));
    };
    Ok(*element_idx)
}

fn is_private_use_glyph(text: &str) -> bool {
    text.chars().count() == 1
        && text.chars().all(|glyph| {
            ('\u{E000}'..='\u{F8FF}').contains(&glyph)
                || ('\u{F0000}'..='\u{FFFFD}').contains(&glyph)
                || ('\u{100000}'..='\u{10FFFD}').contains(&glyph)
        })
}

fn reified_icon_runs(app: &App, panel: Entity) -> Result<Vec<ReifiedIconRun>, String> {
    let runs = app
        .world()
        .get::<PanelTextRuns>(panel)
        .ok_or_else(|| "rebuilt icon panel has no reified text-run index".to_owned())?;
    let mut icons = Vec::new();
    for entity in runs.iter() {
        let content = app
            .world()
            .get::<TextContent>(entity)
            .ok_or_else(|| format!("panel run {entity} has no text content"))?;
        if !is_private_use_glyph(content.text()) {
            continue;
        }
        let layout = app
            .world()
            .get::<PanelTextLayout>(entity)
            .ok_or_else(|| format!("icon run {entity} has no panel text layout"))?;
        icons.push(ReifiedIconRun {
            entity,
            id: layout.id.clone(),
            line_index: layout.line_index,
            element_idx: layout.element_idx,
        });
    }
    Ok(icons)
}

fn slot_pixels(frame: &[u8], right: bool) -> Vec<u8> {
    let x_offset = if right { SLOT_SIZE.x } else { 0 };
    let source_row_width = (VIEW_SIZE.x * 4) as usize;
    let slot_row_width = (SLOT_SIZE.x * 4) as usize;
    let mut pixels = Vec::with_capacity((SLOT_SIZE.x * SLOT_SIZE.y * 4) as usize);
    for y in 0..SLOT_SIZE.y {
        let start = (y as usize * source_row_width) + (x_offset * 4) as usize;
        pixels.extend_from_slice(&frame[start..start + slot_row_width]);
    }
    pixels
}

fn visible_channel_mismatches(actual: &[u8], expected: &[u8]) -> usize {
    actual
        .iter()
        .zip(expected)
        .filter(|&(&actual, &expected)| actual.abs_diff(expected) > 1)
        .count()
}

fn rebuilt_panel_draws_each_icon_once() -> Result<(), String> {
    let mut scene = setup_scene()?;
    rebuild_panel(&mut scene)?;

    let solved_element = solved_icon_element(&scene.app, scene.panel)?;
    let icon_runs = reified_icon_runs(&scene.app, scene.panel)?;
    let frame = scene.app.capture(scene.target)?;
    let region = CaptureRegion::new(VIEW_SIZE, UVec2::ZERO, VIEW_SIZE);
    drawn_support::dump_pixels::<false>(
        "icon_rebuild_identity",
        "",
        "rebuilt panel left, fresh final-state panel right",
        region,
        region,
        &frame,
    )?;
    let pixel_mismatches =
        visible_channel_mismatches(&slot_pixels(&frame, false), &slot_pixels(&frame, true));

    let one_current_run = matches!(
        icon_runs.as_slice(),
        [run] if run.line_index == 0 && run.element_idx == solved_element
    );
    if !one_current_run || pixel_mismatches != 0 {
        let run_details = icon_runs
            .iter()
            .map(|run| {
                format!(
                    "entity {} id {:?} line {} element {}",
                    run.entity, run.id, run.line_index, run.element_idx
                )
            })
            .collect::<Vec<_>>();
        return Err(format!(
            "rebuilt panel must retain one current icon run and draw like a fresh panel; solved element {solved_element}, reified runs {run_details:?}, visible channel mismatches {pixel_mismatches}"
        ));
    }
    Ok(())
}

#[test]
fn rebuilt_panels_draw_each_icon_once() -> Result<(), String> {
    drawn_support::run_cases(&[DrawnCase {
        name: "rebuilt_panel_draws_each_icon_once",
        run:  rebuilt_panel_draws_each_icon_once,
    }])
}
