//! Cold and reused panel glyphs keep their pixels, clipped bounds and visibility.

mod drawn_support;

#[path = "drawn_support/paired_capture.rs"]
mod paired_capture;

use bevy::asset::RenderAssetUsages;
use bevy::camera::Hdr;
use bevy::camera::RenderTarget;
use bevy::camera::ScalingMode;
use bevy::camera::visibility::RenderLayers;
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
use hana_diegetic::CascadeEntityCommandsExt;
use hana_diegetic::DiegeticPanel;
use hana_diegetic::DrawnTextBounds;
use hana_diegetic::El;
use hana_diegetic::FontWeight;
use hana_diegetic::GlyphShadowMode;
use hana_diegetic::LayoutBuilder;
use hana_diegetic::Lighting;
use hana_diegetic::Mm;
use hana_diegetic::Padding;
use hana_diegetic::StableTransparency;
use hana_diegetic::Text;
use hana_diegetic::TextContent;
use hana_diegetic::TextRunOf;
use hana_diegetic::TextStyle;
use hana_diegetic::TextWrap;
use paired_capture::capture_pair;

const SLOT: UVec2 = UVec2::new(192, 128);
const VIEW: UVec2 = UVec2::new(SLOT.x * 2, SLOT.y);

#[derive(Clone, Copy)]
enum PanelTextView {
    Cold,
    Reused,
}

impl PanelTextView {
    const fn layers(self) -> RenderLayers {
        RenderLayers::layer(match self {
            Self::Cold => 0,
            Self::Reused => 1,
        })
    }

    const fn column(self) -> u32 {
        match self {
            Self::Cold => 0,
            Self::Reused => SLOT.x,
        }
    }

    fn write_pixels(self, frame: &mut [u8], pixels: &[u8]) {
        assert_eq!(pixels.len(), (SLOT.x * SLOT.y * 4) as usize);
        for y in 0..SLOT.y {
            let from = (y * SLOT.x * 4) as usize;
            let to = ((y * VIEW.x + self.column()) * 4) as usize;
            frame[to..to + (SLOT.x * 4) as usize]
                .copy_from_slice(&pixels[from..from + (SLOT.x * 4) as usize]);
        }
    }
}

fn spawn_panel(
    app: &mut CaseApp,
    view: PanelTextView,
    material: Handle<StandardMaterial>,
    headline: &str,
) -> Result<Entity, String> {
    let mut tree = LayoutBuilder::with_root(
        El::column()
            .size(72.0, 48.0)
            .padding(Padding::all(4.0))
            .gap(2.0)
            .clip(),
    );
    for (text, weight) in [
        (headline, FontWeight::NORMAL),
        (headline, FontWeight::BOLD),
        ("Hgj 0123456789", FontWeight::NORMAL),
        ("   ", FontWeight::NORMAL),
    ] {
        tree.text(
            Text::new(
                text,
                TextStyle::new(Mm(12.0))
                    .with_weight(weight)
                    .with_color(Color::WHITE)
                    .with_shadow_mode(GlyphShadowMode::None),
            )
            .layout(El::new().size(100.0, 16.0))
            .wrap(TextWrap::None),
        );
    }
    let panel = DiegeticPanel::world()
        .text_material(material)
        .size(Mm(72.0), Mm(48.0))
        .anchor(Anchor::Center)
        .with_tree(tree.build())
        .build()
        .map_err(|error| error.to_string())?;
    Ok(app
        .world_mut()
        .spawn((panel, Transform::default(), view.layers()))
        .id())
}

fn slot(frame: &[u8], right: bool) -> Vec<u8> {
    let offset = if right { SLOT.x } else { 0 };
    (0..SLOT.y)
        .flat_map(|y| {
            let start = ((y * VIEW.x + offset) * 4) as usize;
            frame[start..start + (SLOT.x * 4) as usize].iter().copied()
        })
        .collect()
}

fn assert_same(label: &str, actual: &[u8], expected: &[u8]) {
    assert_eq!(actual.len(), expected.len());
    let largest = actual
        .iter()
        .zip(expected)
        .map(|(actual, expected)| actual.abs_diff(*expected))
        .max()
        .unwrap_or(0);
    assert!(
        largest <= 1,
        "{label}: maximum channel difference {largest}"
    );
}

/// Visible row text and its clipped quad bounds, relative to the text entity.
#[derive(Debug, PartialEq)]
struct DrawnInkRow {
    text:    String,
    minimum: Vec3,
    maximum: Vec3,
}

fn drawn_rows(app: &mut CaseApp, panel: Entity) -> Vec<DrawnInkRow> {
    let world = app.world_mut();
    let mut runs = world.query::<(&TextRunOf, &TextContent, &DrawnTextBounds)>();
    let mut result: Vec<_> = runs
        .iter(world)
        .filter(|(owner, _, _)| owner.panel() == panel)
        .map(|(_, text, bounds)| {
            let aabb = bounds.local_aabb();
            DrawnInkRow {
                text:    text.text().to_owned(),
                minimum: aabb.min().into(),
                maximum: aabb.max().into(),
            }
        })
        .collect();
    result.sort_by(|left, right| {
        left.text
            .cmp(&right.text)
            .then(left.minimum.y.total_cmp(&right.minimum.y))
    });
    result
}

// Translated copies can straddle the analytic shader's 2% discard cutoff:
// llvmpipe gives one opaque edge pixel coverage below it at x=-0.096 and
// above it at x=0.096. Compare the cache at identical world and pixel
// coordinates instead. Layers isolate the panels; both views render together.
struct PanelTextViews {
    cold:   Handle<Image>,
    reused: Handle<Image>,
}

impl PanelTextViews {
    fn settled_reference(&self, app: &mut CaseApp) -> Result<Vec<u8>, String> {
        let cold = app.capture(self.cold.clone())?;
        let empty = app.capture(self.reused.clone())?;
        let mut frame = vec![0; (VIEW.x * VIEW.y * 4) as usize];
        PanelTextView::Cold.write_pixels(&mut frame, &cold);
        PanelTextView::Reused.write_pixels(&mut frame, &empty);
        Ok(frame)
    }

    fn capture(&self, app: &mut CaseApp) -> Result<Vec<u8>, String> {
        let [cold, reused] = capture_pair(app, [self.cold.clone(), self.reused.clone()])?;
        let mut frame = vec![0; (VIEW.x * VIEW.y * 4) as usize];
        PanelTextView::Cold.write_pixels(&mut frame, &cold);
        PanelTextView::Reused.write_pixels(&mut frame, &reused);
        Ok(frame)
    }
}

fn panel_text_targets<const OIT: bool>(app: &mut CaseApp) -> PanelTextViews {
    let region = CaptureRegion::new(SLOT, UVec2::ZERO, SLOT);
    app.world_mut().spawn((
        Camera3d::default(),
        RenderTarget::Window(WindowRef::Primary),
        Hdr,
        Tonemapping::KhronosPbrNeutral,
    ));
    let cold = spawn_text_camera::<OIT>(app, region, PanelTextView::Cold);
    let reused = spawn_text_camera::<OIT>(app, region, PanelTextView::Reused);
    PanelTextViews { cold, reused }
}

fn spawn_text_camera<const OIT: bool>(
    app: &mut CaseApp,
    region: CaptureRegion,
    view: PanelTextView,
) -> Handle<Image> {
    let target = app.target(region);
    let mut capture_camera = Camera::default();
    region.apply_to_camera(&mut capture_camera);
    let camera = app
        .world_mut()
        .spawn((
            capture_camera,
            Camera3d::default(),
            Projection::Orthographic(OrthographicProjection {
                scaling_mode: ScalingMode::FixedVertical {
                    viewport_height: 0.128,
                },
                ..OrthographicProjection::default_3d()
            }),
            RenderTarget::Image(target.clone().into()),
            Hdr,
            Tonemapping::KhronosPbrNeutral,
            Msaa::Off,
            view.layers(),
            Transform::from_xyz(0.0, 0.0, 0.25).looking_at(Vec3::ZERO, Vec3::Y),
        ))
        .id();
    if OIT {
        app.world_mut()
            .entity_mut(camera)
            .insert(StableTransparency);
    }
    target
}

fn cold_and_reused<const OIT: bool>() -> Result<(), String> {
    let mut app = CaseApp::new();
    let region = CaptureRegion::new(VIEW, UVec2::ZERO, VIEW);
    let targets = panel_text_targets::<OIT>(&mut app);
    let material = app
        .world_mut()
        .resource_mut::<Assets<StandardMaterial>>()
        .add(StandardMaterial::default());
    let cold_panel = spawn_panel(
        &mut app,
        PanelTextView::Cold,
        material.clone(),
        "0 Θ @ 8 Hgj",
    )?;
    app.settle()?;
    let cold_frame = targets.settled_reference(&mut app)?;
    let cold = slot(&cold_frame, false);
    let empty = slot(&cold_frame, true);
    assert!(cold.iter().zip(&empty).any(|(a, b)| a.abs_diff(*b) > 1));
    let cold_bounds = drawn_rows(&mut app, cold_panel);
    assert_eq!(
        cold_bounds.len(),
        3,
        "three clipped ink rows; spaces have no bounds"
    );
    dump_pixels::<false>(
        "panel_glyph_cache_pixels",
        "",
        &format!("cold panel bounds={cold_bounds:?}"),
        region,
        region,
        &cold_frame,
    )?;
    let warm_panel = spawn_panel(
        &mut app,
        PanelTextView::Reused,
        material.clone(),
        "0 Θ @ 8 Hgj",
    )?;
    let reused_frame = targets.capture(&mut app)?;
    let reused = slot(&reused_frame, true);
    assert_same("new panel reuses cold glyphs", &reused, &cold);
    assert_same(
        "atlas reuse preserves the first panel",
        &slot(&reused_frame, false),
        &cold,
    );
    assert_eq!(drawn_rows(&mut app, warm_panel), cold_bounds);
    dump_pixels::<false>(
        "panel_glyph_cache_pixels",
        "",
        "cold and reused",
        region,
        region,
        &reused_frame,
    )?;

    let scene = RetainedTextPixelScene {
        targets,
        region,
        material,
        cold_panel,
        warm_panel,
        cold,
        cold_bounds,
        empty,
    };
    let revealed = scene.hide_and_reveal(&mut app)?;
    let resource = scene.edit_material(&mut app, &revealed)?;
    scene.grow_atlas_and_retire_panel(&mut app, &resource)
}

struct RetainedTextPixelScene {
    targets:     PanelTextViews,
    region:      CaptureRegion,
    material:    Handle<StandardMaterial>,
    cold_panel:  Entity,
    warm_panel:  Entity,
    cold:        Vec<u8>,
    cold_bounds: Vec<DrawnInkRow>,
    empty:       Vec<u8>,
}

impl RetainedTextPixelScene {
    fn capture(&self, app: &mut CaseApp, label: &str) -> Result<Vec<u8>, String> {
        let frame = self.targets.capture(app)?;
        dump_pixels::<false>(
            "panel_glyph_cache_pixels",
            "",
            label,
            self.region,
            self.region,
            &frame,
        )?;
        Ok(frame)
    }

    fn hide_and_reveal(&self, app: &mut CaseApp) -> Result<Vec<u8>, String> {
        app.world_mut()
            .entity_mut(self.warm_panel)
            .insert(Visibility::Hidden);
        let hidden = self.capture(app, "hidden panel")?;
        assert_same(
            "hidden panel emits no glyph pixels",
            &slot(&hidden, true),
            &self.empty,
        );
        assert_same(
            "hiding one panel preserves the other",
            &slot(&hidden, false),
            &self.cold,
        );
        app.world_mut()
            .entity_mut(self.warm_panel)
            .insert(Visibility::Inherited);
        let revealed = self.capture(app, "revealed panel")?;
        assert_same(
            "revealed panel keeps its glyph pixels",
            &slot(&revealed, true),
            &self.cold,
        );
        assert_eq!(drawn_rows(app, self.warm_panel), self.cold_bounds);
        Ok(revealed)
    }

    fn edit_material(&self, app: &mut CaseApp, revealed: &[u8]) -> Result<Vec<u8>, String> {
        app.world_mut()
            .resource_mut::<Assets<StandardMaterial>>()
            .add(StandardMaterial::default());
        let unrelated = self.capture(app, "unrelated material")?;
        assert_same(
            "unrelated native material creation preserves text",
            &unrelated,
            revealed,
        );
        let lit = self.light_material_users(app)?;

        app.world_mut()
            .resource_mut::<Assets<StandardMaterial>>()
            .get_mut(&self.material)
            .ok_or("text material disappeared")?
            .emissive = LinearRgba::rgb(0.7, 0.2, 0.0);
        let scalar = self.capture(app, "scalar material edit")?;
        assert!(
            scalar.iter().zip(&lit).any(|(a, b)| a.abs_diff(*b) > 1),
            "scalar material edit changes drawn text"
        );
        assert_same(
            "same-frame material edits reach both users",
            &slot(&scalar, false),
            &slot(&scalar, true),
        );
        assert_eq!(drawn_rows(app, self.cold_panel), self.cold_bounds);

        let white = app
            .world_mut()
            .resource_mut::<Assets<Image>>()
            .add(Image::new_fill(
                Extent3d {
                    width:                 1,
                    height:                1,
                    depth_or_array_layers: 1,
                },
                TextureDimension::D2,
                &[255, 255, 255, 255],
                TextureFormat::Rgba8UnormSrgb,
                RenderAssetUsages::default(),
            ));
        app.world_mut()
            .resource_mut::<Assets<StandardMaterial>>()
            .get_mut(&self.material)
            .ok_or("text material disappeared")?
            .base_color_texture = Some(white);
        let resource = self.capture(app, "resource material edit")?;
        assert_same("texture rekey retains text pixels", &resource, &scalar);
        Ok(resource)
    }

    fn light_material_users(&self, app: &mut CaseApp) -> Result<Vec<u8>, String> {
        for panel in [self.cold_panel, self.warm_panel] {
            app.world_mut()
                .commands()
                .entity(panel)
                .override_lighting(Lighting::Lit);
        }
        app.world_mut()
            .resource_mut::<Assets<StandardMaterial>>()
            .get_mut(&self.material)
            .ok_or("text material disappeared")?
            .emissive = LinearRgba::WHITE;
        self.capture(app, "lit material users")
    }

    fn grow_atlas_and_retire_panel(
        &self,
        app: &mut CaseApp,
        resource: &[u8],
    ) -> Result<(), String> {
        app.world_mut().despawn(self.warm_panel);
        let grown_panel = spawn_panel(
            app,
            PanelTextView::Reused,
            self.material.clone(),
            "WQ % & fi",
        )?;
        app.world_mut()
            .commands()
            .entity(grown_panel)
            .override_lighting(Lighting::Lit);
        let growth = self.capture(app, "atlas growth")?;
        assert!(
            slot(&growth, true)
                .iter()
                .zip(&self.empty)
                .any(|(a, b)| a.abs_diff(*b) > 1),
            "new atlas glyphs must draw"
        );
        assert_same(
            "atlas growth preserves already drawn text",
            &slot(&growth, false),
            &slot(resource, false),
        );
        assert_eq!(drawn_rows(app, self.cold_panel), self.cold_bounds);
        app.world_mut().despawn(grown_panel);
        let retired = self.capture(app, "retired panel")?;
        assert_same(
            "despawn retires glyph records",
            &slot(&retired, true),
            &self.empty,
        );
        assert_same(
            "despawn preserves the other batch",
            &slot(&retired, false),
            &slot(resource, false),
        );
        Ok(())
    }
}

#[test]
fn drawn_panel_glyph_cache_preserves_pixels_bounds_and_visibility() -> Result<(), String> {
    run_cases(&[
        DrawnCase {
            name: "cold_and_reused_oit_on",
            run:  cold_and_reused::<true>,
        },
        DrawnCase {
            name: "cold_and_reused_oit_off",
            run:  cold_and_reused::<false>,
        },
    ])
}
