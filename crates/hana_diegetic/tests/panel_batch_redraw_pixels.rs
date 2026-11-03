//! Retained panel batches must redraw on their first changed frame.

mod drawn_support;

#[path = "drawn_support/paired_capture.rs"]
mod paired_capture;

use bevy::asset::RenderAssetUsages;
use bevy::camera::Hdr;
use bevy::camera::RenderTarget;
use bevy::camera::ScalingMode;
use bevy::camera::visibility::RenderLayers;
use bevy::ecs::system::RunSystemOnce;
use bevy::prelude::*;
use bevy::render::render_resource::Extent3d;
use bevy::render::render_resource::TextureDimension;
use bevy::render::render_resource::TextureFormat;
use bevy::render::view::Tonemapping;
use bevy::window::WindowRef;
use drawn_support::CaptureRegion;
use drawn_support::CaseApp;
use drawn_support::DrawnCase;
use drawn_support::dump_pixels;
use drawn_support::run_cases;
use hana_diegetic::Anchor;
use hana_diegetic::CascadeEntityCommandsExt as _;
use hana_diegetic::DiegeticPanel;
use hana_diegetic::DiegeticPanelCommands as _;
use hana_diegetic::DiegeticTextBatch;
use hana_diegetic::El;
use hana_diegetic::GlyphShadowMode;
use hana_diegetic::LayoutBuilder;
use hana_diegetic::LayoutTree;
use hana_diegetic::Lighting;
use hana_diegetic::Mm;
use hana_diegetic::Padding;
use hana_diegetic::PanelDraw;
use hana_diegetic::PanelElementId;
use hana_diegetic::PanelLine;
use hana_diegetic::PanelText;
use hana_diegetic::Sizing;
use hana_diegetic::StableTransparency;
use hana_diegetic::Text;
use hana_diegetic::TextStyle;
use paired_capture::capture_pair;

const SLOT_SIZE: UVec2 = UVec2::new(192, 128);
const VIEW_SIZE: UVec2 = UVec2::new(SLOT_SIZE.x * 2, SLOT_SIZE.y);
const VIEW_HEIGHT: f32 = 0.128;
const PANEL_SLOT_X: f32 = 0.096;
const PANEL_SIZE: Vec2 = Vec2::new(72.0, 48.0);
const LABEL_ID: &str = "redraw-label";
const OLD_FILL: Color = Color::srgb_u8(24, 48, 96);
const NEW_FILL: Color = Color::srgb_u8(96, 40, 24);
const OLD_LINE: Color = Color::srgb_u8(250, 210, 40);
const NEW_LINE: Color = Color::srgb_u8(40, 245, 215);
const IMAGE_RED: [u8; 4] = [230, 35, 55, 255];
const IMAGE_GREEN: [u8; 4] = [35, 220, 80, 255];

#[derive(Clone, Copy)]
enum TestImage {
    Red,
    Green,
}

#[derive(Clone, Copy)]
struct PanelLook {
    text:        &'static str,
    fill:        Color,
    line:        Color,
    image:       TestImage,
    translation: Vec3,
}

const INITIAL: PanelLook = PanelLook {
    text:        "OLD",
    fill:        OLD_FILL,
    line:        OLD_LINE,
    image:       TestImage::Red,
    translation: Vec3::ZERO,
};

#[derive(Clone)]
struct TestImages {
    red:   Handle<Image>,
    green: Handle<Image>,
}

impl TestImages {
    fn selected(&self, image: TestImage) -> Handle<Image> {
        match image {
            TestImage::Red => self.red.clone(),
            TestImage::Green => self.green.clone(),
        }
    }
}

#[derive(Clone, Copy)]
enum PanelBatchView {
    Retained,
    FreshReference,
}

impl PanelBatchView {
    const fn layers(self) -> RenderLayers {
        RenderLayers::layer(match self {
            Self::Retained => 0,
            Self::FreshReference => 1,
        })
    }
}

#[derive(Clone, Copy)]
enum PanelReference {
    Fresh(PanelLook),
    Empty,
}

struct PanelComparisonViews {
    retained:  Handle<Image>,
    reference: Handle<Image>,
}

impl PanelComparisonViews {
    fn capture(&self, app: &mut CaseApp) -> Result<[Vec<u8>; 2], String> {
        capture_pair(app, [self.retained.clone(), self.reference.clone()])
    }
}

struct PanelScene {
    app:    CaseApp,
    views:  PanelComparisonViews,
    panel:  Entity,
    images: TestImages,
}

fn solid_image(color: [u8; 4]) -> Image {
    Image::new_fill(
        Extent3d {
            width:                 2,
            height:                2,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        &color,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::default(),
    )
}

fn add_test_images(app: &mut CaseApp) -> TestImages {
    let mut images = app.world_mut().resource_mut::<Assets<Image>>();
    TestImages {
        red:   images.add(solid_image(IMAGE_RED)),
        green: images.add(solid_image(IMAGE_GREEN)),
    }
}

fn panel_tree(look: PanelLook, images: &TestImages) -> LayoutTree {
    let line = PanelLine::new((4.0, 40.0), (68.0, 8.0))
        .width(Mm(3.0))
        .color(look.line);
    let mut builder = LayoutBuilder::with_root(
        El::row()
            .size(PANEL_SIZE.x, PANEL_SIZE.y)
            .padding(Padding::all(4.0))
            .gap(4.0)
            .background(look.fill)
            .draw(PanelDraw::lines([line])),
    );
    builder.text(
        Text::new(
            look.text,
            TextStyle::new(Mm(10.0))
                .with_color(Color::WHITE)
                .with_shadow_mode(GlyphShadowMode::None),
        )
        .id(LABEL_ID)
        .layout(
            El::new()
                .width(Sizing::fixed(36.0))
                .height(Sizing::fixed(24.0)),
        ),
    );
    builder.image(
        El::new()
            .width(Sizing::fixed(20.0))
            .height(Sizing::fixed(20.0)),
        images.selected(look.image),
        Color::WHITE,
    );
    builder.build()
}

fn spawn_camera(
    app: &mut CaseApp,
    target: Handle<Image>,
    region: CaptureRegion,
    view: PanelBatchView,
) {
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
        Msaa::Off,
        StableTransparency,
        view.layers(),
        Transform::from_xyz(0.0, 0.0, 0.25).looking_at(Vec3::ZERO, Vec3::Y),
    ));
}

fn spawn_panel(
    app: &mut CaseApp,
    look: PanelLook,
    images: &TestImages,
    slot_x: f32,
    view: PanelBatchView,
) -> Result<Entity, String> {
    let panel = DiegeticPanel::world()
        .size(Mm(PANEL_SIZE.x), Mm(PANEL_SIZE.y))
        .anchor(Anchor::Center)
        .with_tree(panel_tree(look, images))
        .build()
        .map_err(|error| format!("invalid redraw panel: {error}"))?;
    let panel = app
        .world_mut()
        .spawn((
            panel,
            Transform::from_translation(look.translation + Vec3::X * slot_x),
            view.layers(),
        ))
        .id();
    app.world_mut()
        .commands()
        .entity(panel)
        .override_lighting(Lighting::Unlit);
    Ok(panel)
}

fn setup_panel_scene(look: PanelLook, expected: PanelReference) -> Result<PanelScene, String> {
    let mut app = CaseApp::new();
    let region = CaptureRegion::new(VIEW_SIZE, UVec2::ZERO, VIEW_SIZE);
    app.world_mut().spawn((
        Camera3d::default(),
        RenderTarget::Window(WindowRef::Primary),
        Hdr,
        Tonemapping::KhronosPbrNeutral,
    ));
    let retained = app.target(region);
    let reference = app.target(region);
    spawn_camera(&mut app, retained.clone(), region, PanelBatchView::Retained);
    spawn_camera(
        &mut app,
        reference.clone(),
        region,
        PanelBatchView::FreshReference,
    );
    let images = add_test_images(&mut app);
    let panel = spawn_panel(
        &mut app,
        look,
        &images,
        -PANEL_SLOT_X,
        PanelBatchView::Retained,
    )?;
    let sibling = match expected {
        PanelReference::Fresh(expected) => {
            spawn_panel(
                &mut app,
                expected,
                &images,
                -PANEL_SLOT_X,
                PanelBatchView::FreshReference,
            )?;
            expected
        },
        PanelReference::Empty => INITIAL,
    };
    // The retained view keeps both panels in one text batch. Its unchanged sibling
    // exercises retained membership during a redraw or removal; only the
    // reference scene has a separate batch key. Corresponding panels and
    // cameras use identical coordinates, avoiding shader edge cutoff differences.
    for view in [PanelBatchView::Retained, PanelBatchView::FreshReference] {
        spawn_panel(&mut app, sibling, &images, PANEL_SLOT_X, view)?;
    }
    app.settle()?;
    let mut text_batches = app
        .world_mut()
        .query_filtered::<&RenderLayers, With<DiegeticTextBatch>>();
    assert_eq!(
        text_batches
            .iter(app.world())
            .filter(|layers| **layers == PanelBatchView::Retained.layers())
            .count(),
        1,
        "the retained panel and its unchanged sibling must share one text batch",
    );
    Ok(PanelScene {
        app,
        views: PanelComparisonViews {
            retained,
            reference,
        },
        panel,
        images,
    })
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

fn pixel_difference(left: &[u8], right: &[u8]) -> (usize, u8) {
    left.iter()
        .zip(right)
        .fold((0, 0), |(count, largest), (&left, &right)| {
            let difference = left.abs_diff(right);
            (
                count + usize::from(difference != 0),
                largest.max(difference),
            )
        })
}

fn assert_sibling_unchanged(before: &[u8], actual: &[u8]) -> Result<(), String> {
    let difference = pixel_difference(&slot_pixels(before, true), &slot_pixels(actual, true));
    if difference.0 != 0 {
        return Err(format!(
            "redrawing one panel changed {} unchanged sibling channels (largest difference {})",
            difference.0, difference.1,
        ));
    }
    Ok(())
}

fn assert_redrawn(
    label: &str,
    before: &[u8],
    actual: &[u8],
    expected: &[u8],
) -> Result<(), String> {
    let region = CaptureRegion::new(SLOT_SIZE, UVec2::ZERO, SLOT_SIZE);
    dump_pixels::<false>(
        "panel_batch_redraw_pixels",
        "",
        label,
        region,
        region,
        actual,
    )?;
    let before_to_expected = pixel_difference(before, expected);
    if before_to_expected.0 == 0 {
        return Err(format!("{label}: fixture change produced identical pixels"));
    }
    let actual_to_expected = pixel_difference(actual, expected);
    let visible_mismatches = actual
        .iter()
        .zip(expected)
        .filter(|&(&actual, &expected)| actual.abs_diff(expected) > 1)
        .count();
    let actual_to_before = pixel_difference(actual, before);
    eprintln!(
        "{label}: actual-vs-fresh {actual_to_expected:?}; before-vs-fresh {before_to_expected:?}; actual-vs-before {actual_to_before:?}"
    );
    if visible_mismatches != 0 {
        return Err(format!(
            "{label}: first changed frame differs from its paired fresh draw by more than one level in {visible_mismatches} channels; actual-vs-fresh {actual_to_expected:?}; before-vs-fresh {before_to_expected:?}; actual-vs-before {actual_to_before:?}"
        ));
    }
    Ok(())
}

fn replace_tree(scene: &mut PanelScene, look: PanelLook) -> Result<(), String> {
    let tree = panel_tree(look, &scene.images);
    let panel = scene.panel;
    let result = scene
        .app
        .world_mut()
        .run_system_once(move |mut commands: Commands| commands.set_tree(panel, tree.clone()));
    result
        .map_err(|error| format!("could not run panel tree replacement: {error}"))?
        .map_err(|error| format!("invalid replacement tree: {error}"))
}

fn text_edit_redraws_in_the_same_frame() -> Result<(), String> {
    let final_look = PanelLook {
        text: "NEW",
        ..INITIAL
    };
    let mut scene = setup_panel_scene(INITIAL, PanelReference::Fresh(final_look))?;
    let [before_frame, reference_frame] = scene.views.capture(&mut scene.app)?;
    let result = scene.app.world_mut().run_system_once({
        let panel = scene.panel;
        move |mut text: PanelText| {
            text.set_text(panel, &PanelElementId::named(LABEL_ID), final_look.text)
        }
    });
    let changed = result.map_err(|error| format!("could not run text edit: {error}"))?;
    if !changed {
        return Err("text edit could not resolve its authored run".to_string());
    }
    let [actual_frame, _] = scene.views.capture(&mut scene.app)?;
    assert_sibling_unchanged(&before_frame, &actual_frame)?;
    assert_redrawn(
        "text edit",
        &slot_pixels(&before_frame, false),
        &slot_pixels(&actual_frame, false),
        &slot_pixels(&reference_frame, false),
    )
}

fn fill_colour_redraws_in_the_same_frame() -> Result<(), String> {
    let final_look = PanelLook {
        fill: NEW_FILL,
        ..INITIAL
    };
    let mut scene = setup_panel_scene(INITIAL, PanelReference::Fresh(final_look))?;
    let [before_frame, reference_frame] = scene.views.capture(&mut scene.app)?;
    replace_tree(&mut scene, final_look)?;
    let [actual_frame, _] = scene.views.capture(&mut scene.app)?;
    assert_sibling_unchanged(&before_frame, &actual_frame)?;
    assert_redrawn(
        "fill colour change",
        &slot_pixels(&before_frame, false),
        &slot_pixels(&actual_frame, false),
        &slot_pixels(&reference_frame, false),
    )
}

fn line_change_redraws_in_the_same_frame() -> Result<(), String> {
    let final_look = PanelLook {
        line: NEW_LINE,
        ..INITIAL
    };
    let mut scene = setup_panel_scene(INITIAL, PanelReference::Fresh(final_look))?;
    let [before_frame, reference_frame] = scene.views.capture(&mut scene.app)?;
    replace_tree(&mut scene, final_look)?;
    let [actual_frame, _] = scene.views.capture(&mut scene.app)?;
    assert_sibling_unchanged(&before_frame, &actual_frame)?;
    assert_redrawn(
        "line change",
        &slot_pixels(&before_frame, false),
        &slot_pixels(&actual_frame, false),
        &slot_pixels(&reference_frame, false),
    )
}

fn image_swap_redraws_in_the_same_frame() -> Result<(), String> {
    let final_look = PanelLook {
        image: TestImage::Green,
        ..INITIAL
    };
    let mut scene = setup_panel_scene(INITIAL, PanelReference::Fresh(final_look))?;
    let [before_frame, reference_frame] = scene.views.capture(&mut scene.app)?;
    replace_tree(&mut scene, final_look)?;
    let [actual_frame, _] = scene.views.capture(&mut scene.app)?;
    assert_sibling_unchanged(&before_frame, &actual_frame)?;
    assert_redrawn(
        "image swap",
        &slot_pixels(&before_frame, false),
        &slot_pixels(&actual_frame, false),
        &slot_pixels(&reference_frame, false),
    )
}

fn moved_panel_redraws_in_the_same_frame() -> Result<(), String> {
    let final_look = PanelLook {
        translation: Vec3::new(0.022, -0.014, 0.0),
        ..INITIAL
    };
    let mut scene = setup_panel_scene(INITIAL, PanelReference::Fresh(final_look))?;
    let [before_frame, reference_frame] = scene.views.capture(&mut scene.app)?;
    let Some(mut transform) = scene.app.world_mut().get_mut::<Transform>(scene.panel) else {
        return Err("panel lost its transform".to_string());
    };
    transform.translation = final_look.translation - Vec3::X * PANEL_SLOT_X;
    let [actual_frame, _] = scene.views.capture(&mut scene.app)?;
    assert_sibling_unchanged(&before_frame, &actual_frame)?;
    assert_redrawn(
        "panel move",
        &slot_pixels(&before_frame, false),
        &slot_pixels(&actual_frame, false),
        &slot_pixels(&reference_frame, false),
    )
}

fn removed_panel_redraws_in_the_same_frame() -> Result<(), String> {
    let mut scene = setup_panel_scene(INITIAL, PanelReference::Empty)?;
    let [before_frame, reference_frame] = scene.views.capture(&mut scene.app)?;
    scene
        .app
        .world_mut()
        .entity_mut(scene.panel)
        .remove::<DiegeticPanel>();
    let [actual_frame, _] = scene.views.capture(&mut scene.app)?;
    assert_sibling_unchanged(&before_frame, &actual_frame)?;
    assert_redrawn(
        "panel removal",
        &slot_pixels(&before_frame, false),
        &slot_pixels(&actual_frame, false),
        &slot_pixels(&reference_frame, false),
    )
}

fn quiet_frame_keeps_every_painted_pixel() -> Result<(), String> {
    let mut scene = setup_panel_scene(INITIAL, PanelReference::Empty)?;
    let painted = slot_pixels(&scene.app.capture(scene.views.retained.clone())?, false);
    let quiet = slot_pixels(&scene.app.capture(scene.views.retained)?, false);
    let difference = pixel_difference(&painted, &quiet);
    if difference.0 != 0 {
        return Err(format!(
            "quiet frame changed {} painted channels (largest difference {})",
            difference.0, difference.1,
        ));
    }
    Ok(())
}

#[test]
fn drawn_panel_batch_redraw_cases_share_a_renderer() -> Result<(), String> {
    run_cases(&[
        DrawnCase {
            name: "text_edit_redraws_in_the_same_frame",
            run:  text_edit_redraws_in_the_same_frame,
        },
        DrawnCase {
            name: "fill_colour_redraws_in_the_same_frame",
            run:  fill_colour_redraws_in_the_same_frame,
        },
        DrawnCase {
            name: "line_change_redraws_in_the_same_frame",
            run:  line_change_redraws_in_the_same_frame,
        },
        DrawnCase {
            name: "image_swap_redraws_in_the_same_frame",
            run:  image_swap_redraws_in_the_same_frame,
        },
        DrawnCase {
            name: "moved_panel_redraws_in_the_same_frame",
            run:  moved_panel_redraws_in_the_same_frame,
        },
        DrawnCase {
            name: "removed_panel_redraws_in_the_same_frame",
            run:  removed_panel_redraws_in_the_same_frame,
        },
        DrawnCase {
            name: "quiet_frame_keeps_every_painted_pixel",
            run:  quiet_frame_keeps_every_painted_pixel,
        },
    ])
}
