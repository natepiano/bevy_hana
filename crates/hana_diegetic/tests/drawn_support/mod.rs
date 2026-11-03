//! One headless renderer for the named drawn cases in a test process.

use std::any::Any;
use std::cell::Cell;
use std::cell::RefCell;
use std::collections::HashSet;
use std::fmt::Write as _;
use std::io::Write as _;
use std::ops::Deref;
use std::ops::DerefMut;
use std::panic::AssertUnwindSafe;
use std::panic::catch_unwind;
use std::sync::mpsc;
use std::time::Duration;

use bevy::asset::Asset;
use bevy::asset::AssetId;
use bevy::camera::SubCameraView;
use bevy::ecs::resource::IsResource;
use bevy::prelude::*;
use bevy::render::RenderPlugin;
use bevy::render::render_resource::PollType;
use bevy::render::render_resource::TextureFormat;
use bevy::render::renderer::RenderDevice;
use bevy::render::view::screenshot::Screenshot;
use bevy::render::view::screenshot::ScreenshotCaptured;
use bevy::window::ExitCondition;
use bevy::winit::WinitPlugin;
use hana_diegetic::DiegeticPanel;
use hana_diegetic::DiegeticUiPlugin;
use hana_diegetic::DisplayCompensationReadiness;
use hana_diegetic::Lighting;
use hana_diegetic::StableTransparency;
use hana_diegetic::reset_drawn_glyph_cache;

thread_local! {
    static RENDERER: RefCell<DrawnRenderer> = RefCell::new(DrawnRenderer::new());
    static CURRENT_CASE: RefCell<&'static str> = const { RefCell::new("") };
    static DUMP_CAPTURE: Cell<usize> = const { Cell::new(0) };
}

/// Returns the active named case while its assertion function runs.
pub(crate) fn current_case_name() -> &'static str { CURRENT_CASE.with(|name| *name.borrow()) }

/// Writes sampled pixels in the saved fixture format when requested.
pub(crate) fn dump_pixels<const DECIMAL: bool>(
    binary: &str,
    case: &str,
    metadata: &str,
    region: CaptureRegion,
    sample: CaptureRegion,
    pixels: &[u8],
) -> Result<(), String> {
    let Some(root) = std::env::var_os("HANA_DRAWN_PIXEL_DUMP") else {
        return Ok(());
    };
    assert!(sample.origin.cmpge(region.origin).all());
    assert!(
        (sample.origin + sample.size)
            .cmple(region.origin + region.size)
            .all()
    );
    assert_eq!(pixels.len(), (region.size.x * region.size.y * 4) as usize);
    let case = if case.is_empty() {
        current_case_name()
    } else {
        case
    };
    let path = std::path::Path::new(&root)
        .join(binary)
        .join(format!("{case}.txt"));
    std::fs::create_dir_all(path.parent().ok_or("dump path has no parent")?)
        .map_err(|error| format!("create pixel dump directory: {error}"))?;
    if DECIMAL {
        let file = std::fs::File::create(path).map_err(|error| error.to_string())?;
        let mut dump = std::io::BufWriter::new(file);
        writeln!(dump, "{metadata}").map_err(|error| error.to_string())?;
        writeln!(dump, "phase=full-frame pixel centers x+0.5,y+0.5")
            .map_err(|error| error.to_string())?;
        for y in sample.origin.y..sample.origin.y + sample.size.y {
            for x in sample.origin.x..sample.origin.x + sample.size.x {
                let local = (y - region.origin.y) * region.size.x + x - region.origin.x;
                let index = (local * 4) as usize;
                writeln!(
                    dump,
                    "{x},{y}={},{},{},{}",
                    pixels[index],
                    pixels[index + 1],
                    pixels[index + 2],
                    pixels[index + 3]
                )
                .map_err(|error| error.to_string())?;
            }
        }
        return Ok(());
    }
    let capture = DUMP_CAPTURE.with(|number| {
        number.set(number.get() + 1);
        number.get()
    });
    let mut output = String::new();
    writeln!(
        output,
        "capture={capture} width={} height={} {metadata}",
        region.size.x, region.size.y,
    )
    .map_err(|error| error.to_string())?;
    for y in sample.origin.y..sample.origin.y + sample.size.y {
        write!(output, "y={} x={}..", y, sample.origin.x).map_err(|error| error.to_string())?;
        for x in sample.origin.x..sample.origin.x + sample.size.x {
            let local = (y - region.origin.y) * region.size.x + x - region.origin.x;
            let index = (local * 4) as usize;
            for channel in &pixels[index..index + 4] {
                write!(output, "{channel:02x}").map_err(|error| error.to_string())?;
            }
        }
        output.push('\n');
    }
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .and_then(|mut file| file.write_all(output.as_bytes()))
        .map_err(|error| format!("write pixel dump: {error}"))
}

/// Pixels in the original view, including the original subpixel phase.
#[derive(Clone, Copy, Debug)]
pub(crate) struct CaptureRegion {
    /// Size of the uncropped view from which pixel coordinates are measured.
    pub full_size: UVec2,
    /// Top-left pixel in the uncropped view.
    pub origin:    UVec2,
    /// Physical size of the render target for this capture.
    pub size:      UVec2,
}

impl CaptureRegion {
    /// Captures a rectangle in the original view; pass zero offset and full size for no crop.
    pub(crate) const fn new(full_size: UVec2, origin: UVec2, size: UVec2) -> Self {
        assert!(size.x > 0 && size.y > 0);
        assert!(origin.x + size.x <= full_size.x && origin.y + size.y <= full_size.y);
        Self {
            full_size,
            origin,
            size,
        }
    }

    /// Keeps the original projection while rendering into the cropped target.
    pub(crate) fn apply_to_camera(self, camera: &mut Camera) {
        if self.size != self.full_size || self.origin != UVec2::ZERO {
            camera.sub_camera_view = Some(SubCameraView {
                full_size: self.full_size,
                offset:    self.origin.as_vec2(),
                size:      self.size,
            });
        }
    }
}

/// One named assertion function run against the shared renderer.
pub(crate) struct DrawnCase {
    /// Name reported when this case fails and used for its dump file.
    pub name: &'static str,
    /// Assertion function for the case.
    pub run:  fn() -> Result<(), String>,
}

struct DrawnRenderer {
    app:                App,
    original_entities:  HashSet<Entity>,
    original_images:    HashSet<AssetId<Image>>,
    original_meshes:    HashSet<AssetId<Mesh>>,
    original_materials: HashSet<AssetId<StandardMaterial>>,
}

impl DrawnRenderer {
    fn new() -> Self {
        let mut app = App::new();
        app.add_plugins(
            DefaultPlugins
                .set(WindowPlugin {
                    primary_window: None,
                    exit_condition: ExitCondition::DontExit,
                    ..default()
                })
                .set(RenderPlugin {
                    synchronous_pipeline_compilation: true,
                    ..default()
                })
                .disable::<WinitPlugin>(),
        );
        app.add_plugins(DiegeticUiPlugin);
        app.world_mut().insert_resource(Lighting::Unlit);
        app.insert_resource(ClearColor(Color::BLACK));
        app.finish();
        app.cleanup();
        for _ in 0..4 {
            app.update();
        }
        let original_entities = app
            .world()
            .iter_entities()
            .map(|entity| entity.id())
            .collect();
        let original_images = app.world().resource::<Assets<Image>>().ids().collect();
        let original_meshes = app.world().resource::<Assets<Mesh>>().ids().collect();
        let original_materials = app
            .world()
            .resource::<Assets<StandardMaterial>>()
            .ids()
            .collect();
        Self {
            app,
            original_entities,
            original_images,
            original_meshes,
            original_materials,
        }
    }

    fn reset(&self, app: &mut App) {
        for _ in 0..4 {
            app.update();
        }
        app.world_mut().flush();
        let stable_cameras: Vec<Entity> = app
            .world()
            .iter_entities()
            .map(|entity| entity.id())
            .filter(|entity| {
                !self.original_entities.contains(entity)
                    && app.world().get::<StableTransparency>(*entity).is_some()
            })
            .collect();
        for camera in stable_cameras {
            app.world_mut()
                .entity_mut(camera)
                .remove::<StableTransparency>();
        }
        app.world_mut().flush();
        let panels: Vec<Entity> = app
            .world()
            .iter_entities()
            .map(|entity| entity.id())
            .filter(|entity| {
                !self.original_entities.contains(entity)
                    && app.world().get::<DiegeticPanel>(*entity).is_some()
                    && app.world().get::<IsResource>(*entity).is_none()
            })
            .collect();
        for panel in panels {
            if app.world().get_entity(panel).is_ok() {
                // Let attachment cleanup reach live entities before panel despawn cascades.
                app.world_mut().entity_mut(panel).remove::<DiegeticPanel>();
                app.world_mut().flush();
                if app.world().get_entity(panel).is_ok() {
                    app.world_mut().despawn(panel);
                    app.world_mut().flush();
                }
            }
        }
        for _ in 0..4 {
            app.update();
        }
        let entities: Vec<Entity> = app
            .world()
            .iter_entities()
            .map(|entity| entity.id())
            .filter(|entity| {
                !self.original_entities.contains(entity)
                    && app.world().get::<ChildOf>(*entity).is_none()
                    && app.world().get::<IsResource>(*entity).is_none()
            })
            .collect();
        for entity in entities {
            app.world_mut().despawn(entity);
        }
        for _ in 0..4 {
            app.update();
        }
        remove_case_assets::<Image>(app, &self.original_images);
        remove_case_assets::<Mesh>(app, &self.original_meshes);
        remove_case_assets::<StandardMaterial>(app, &self.original_materials);
        reset_drawn_glyph_cache(app);
        app.world_mut().insert_resource(Lighting::Unlit);
        app.insert_resource(ClearColor(Color::BLACK));
        app.world_mut()
            .insert_resource(GlobalAmbientLight::default());
    }
}

fn remove_case_assets<A: Asset>(app: &mut App, original: &HashSet<AssetId<A>>) {
    let mut assets = app.world_mut().resource_mut::<Assets<A>>();
    let new_ids: Vec<_> = assets.ids().filter(|id| !original.contains(id)).collect();
    for id in new_ids {
        assets.remove(id);
    }
}

/// Owns one case's scene and returns the initialized app on drop.
pub(crate) struct CaseApp {
    app: App,
}

impl CaseApp {
    /// Takes the initialized app for one capture scene.
    pub(crate) fn new() -> Self {
        let app = RENDERER.with(|renderer| {
            let mut renderer = renderer.borrow_mut();
            std::mem::replace(&mut renderer.app, App::new())
        });
        Self { app }
    }

    /// Adds a render target with the physical size of the capture region.
    pub(crate) fn target(&mut self, region: CaptureRegion) -> Handle<Image> {
        self.app
            .world_mut()
            .resource_mut::<Assets<Image>>()
            .add(Image::new_target_texture(
                region.size.x,
                region.size.y,
                TextureFormat::Rgba8UnormSrgb,
                None,
            ))
    }

    /// Waits for a final display transform and four settled frames.
    pub(crate) fn settle(&mut self) -> Result<(), String> {
        eprintln!("waiting for display compensation to settle");
        loop {
            if matches!(
                *self.app.world().resource::<DisplayCompensationReadiness>(),
                DisplayCompensationReadiness::Compensated | DisplayCompensationReadiness::Identity
            ) {
                for _ in 0..4 {
                    self.app.update();
                }
                return Ok(());
            }
            self.app.update();
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    /// Reads the pixels rendered into an image target.
    pub(crate) fn capture(&mut self, target: Handle<Image>) -> Result<Vec<u8>, String> {
        capture_scene(&mut self.app, target)
    }
}

impl Deref for CaseApp {
    type Target = App;
    fn deref(&self) -> &App { &self.app }
}

impl DerefMut for CaseApp {
    fn deref_mut(&mut self) -> &mut App { &mut self.app }
}

impl Drop for CaseApp {
    fn drop(&mut self) {
        RENDERER.with(|renderer| {
            let mut renderer = renderer.borrow_mut();
            renderer.reset(&mut self.app);
            std::mem::swap(&mut renderer.app, &mut self.app);
        });
    }
}

/// Captures an image target in an app borrowed through another fixture API.
fn capture_scene(app: &mut App, target: Handle<Image>) -> Result<Vec<u8>, String> {
    let (sender, receiver) = mpsc::channel();
    app.world_mut().spawn(Screenshot::image(target)).observe(
        move |event: On<ScreenshotCaptured>| {
            let _ = sender.send(event.image.data.clone());
        },
    );
    // Bevy finishes the readback on its async compute pool, which shader
    // compilation can hold for many frames, so this waits on the capture itself.
    loop {
        app.update();
        app.world()
            .resource::<RenderDevice>()
            .wgpu_device()
            .poll(PollType::Wait {
                submission_index: None,
                timeout:          None,
            })
            .map_err(|error| format!("GPU screenshot readback failed: {error}"))?;
        if let Ok(data) = receiver.try_recv() {
            return data.ok_or_else(|| "screenshot has no pixel data".to_string());
        }
    }
}

/// Runs every case, including those after an assertion failure.
pub(crate) fn run_cases(cases: &[DrawnCase]) -> Result<(), String> {
    let mut failures = Vec::new();
    let reverse = std::env::var_os("HANA_DRAWN_REVERSE_CASES").is_some();
    let ordered: Box<dyn Iterator<Item = &DrawnCase>> = if reverse {
        Box::new(cases.iter().rev())
    } else {
        Box::new(cases.iter())
    };
    for case in ordered {
        CURRENT_CASE.with(|name| *name.borrow_mut() = case.name);
        DUMP_CAPTURE.with(|number| number.set(0));
        let result = catch_unwind(AssertUnwindSafe(|| (case.run)()));
        match result {
            Ok(Ok(())) => {},
            Ok(Err(message)) => failures.push(format!("{}: {message}", case.name)),
            Err(payload) => failures.push(format!("{}: {}", case.name, panic_message(payload))),
        }
    }
    CURRENT_CASE.with(|name| *name.borrow_mut() = "");
    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures.join("\n"))
    }
}

fn panic_message(payload: Box<dyn Any + Send>) -> String {
    if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else if let Some(message) = payload.downcast_ref::<&str>() {
        (*message).to_string()
    } else {
        "case panicked without a message".to_string()
    }
}
