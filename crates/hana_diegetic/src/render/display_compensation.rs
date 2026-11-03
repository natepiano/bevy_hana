//! Inverse display transform for unlit colors drawn into an HDR world view.

use bevy::asset::RenderAssetUsages;
use bevy::camera::RenderTarget;
use bevy::core_pipeline::tonemapping::TonemappingLuts;
use bevy::image::Image;
use bevy::image::ImageSampler;
use bevy::image::ImageSamplerDescriptor;
use bevy::post_process::bloom::Bloom;
use bevy::post_process::bloom::BloomCompositeMode;
use bevy::prelude::*;
use bevy::render::render_resource::Extent3d;
use bevy::render::render_resource::TextureDimension;
use bevy::render::render_resource::TextureFormat;
use bevy::render::view::ColorGrading;
use bevy::render::view::ColorGradingUniform;
use bevy::render::view::Tonemapping;
use bevy::tasks::AsyncComputeTaskPool;
use bevy::tasks::Task;
use bevy::tasks::futures::check_ready;
use bevy::window::WindowRef;

use super::constants::DISPLAY_TABLE_FORMAT;
use super::constants::DISPLAY_TABLE_SIZE;
use super::constants::TONY_LUT_SIZE;
use crate::screen_space::ScreenSpaceCamera;

/// Stable image handles consumed by both world and screen-space materials.
#[derive(Resource)]
pub(super) struct DisplayCompensationTextures {
    pub(super) identity: Handle<Image>,
    pub(super) world:    Handle<Image>,
}

/// Why the world table currently contains its particular transform.
#[derive(Resource)]
enum DisplayCompensation {
    AwaitingWorldCamera,
    AwaitingTonemappingLut,
    Identity,
    Building(Task<DisplayTableBuild>),
    Compensated,
    Unsupported(Tonemapping),
}

/// Whether the world display table has landed for the selected camera.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Resource)]
pub enum DisplayCompensationReadiness {
    /// The camera or its display table is still pending.
    #[default]
    Pending,
    /// The selected camera uses no tonemapping, so identity is final.
    Identity,
    /// The computed world compensation table has replaced the identity image.
    Compensated,
    /// The selected tonemapping method has no compensation table.
    Unsupported(Tonemapping),
}

impl From<&DisplayCompensation> for DisplayCompensationReadiness {
    fn from(state: &DisplayCompensation) -> Self {
        match state {
            DisplayCompensation::AwaitingWorldCamera
            | DisplayCompensation::AwaitingTonemappingLut
            | DisplayCompensation::Building(_) => Self::Pending,
            DisplayCompensation::Identity => Self::Identity,
            DisplayCompensation::Compensated => Self::Compensated,
            DisplayCompensation::Unsupported(method) => Self::Unsupported(*method),
        }
    }
}

enum DisplayTableBuild {
    Ready(Box<Image>),
    UnreadableTonemappingLut,
}

#[derive(Default)]
enum SelectedDisplayConfiguration {
    #[default]
    NoWorldCamera,
    WorldCamera(WorldDisplayConfiguration),
}

struct WorldDisplayConfiguration {
    camera:  Entity,
    order:   isize,
    window:  DisplayWindow,
    method:  Tonemapping,
    grading: ColorGrading,
    bloom:   WorldBloom,
}

/// The selected world camera's bloom settings that affect authored colours.
enum WorldBloom {
    Disabled,
    Enabled {
        threshold:          f32,
        threshold_softness: f32,
        intensity:          f32,
        composite_mode:     BloomCompositeMode,
    },
}

/// The largest compensated channel that cannot contribute to bloom.
#[derive(Clone, Copy, PartialEq)]
enum BloomCeiling {
    Unrestricted,
    AtMost(f32),
}

/// Scene-independent inverse for a fill kept below Bloom's prefilter.
#[derive(Clone, Copy, PartialEq)]
struct BloomCompensation {
    ceiling:        BloomCeiling,
    retained_color: f32,
}

impl WorldBloom {
    const fn from_camera(bloom: Option<&Bloom>) -> Self {
        match bloom {
            Some(bloom) => Self::Enabled {
                threshold:          bloom.prefilter.threshold,
                threshold_softness: bloom.prefilter.threshold_softness,
                intensity:          bloom.intensity,
                composite_mode:     bloom.composite_mode,
            },
            None => Self::Disabled,
        }
    }

    fn same_effect_as(&self, other: &Self) -> bool { self.compensation() == other.compensation() }

    fn ceiling(&self) -> BloomCeiling {
        let Self::Enabled {
            threshold,
            threshold_softness,
            ..
        } = self
        else {
            return BloomCeiling::Unrestricted;
        };
        let ceiling = threshold * (1.0 - threshold_softness.clamp(0.0, 1.0));
        if !ceiling.is_finite() || ceiling <= 0.0 {
            return BloomCeiling::Unrestricted;
        }
        let rounded = to_f16_bits(ceiling);
        let floor = if f16_positive_to_f32(rounded) > ceiling {
            rounded.saturating_sub(1)
        } else {
            rounded
        };
        BloomCeiling::AtMost(f16_positive_to_f32(floor))
    }

    fn compensation(&self) -> BloomCompensation {
        let ceiling = self.ceiling();
        // Only a positive prefilter ceiling guarantees that the fill itself
        // contributes no bloom; without one the blurred colour is scene-dependent.
        let retained_color = match (self, ceiling) {
            (
                Self::Enabled {
                    intensity,
                    composite_mode: BloomCompositeMode::EnergyConserving,
                    ..
                },
                BloomCeiling::AtMost(_),
            ) if intensity.is_finite() => 1.0 - intensity.clamp(0.0, 1.0),
            _ => 1.0,
        };
        BloomCompensation {
            ceiling,
            retained_color,
        }
    }
}

impl BloomCompensation {
    /// The ceiling wins over the dimming inverse, giving bright fills the best-feasible colour.
    fn input_for(self, target_before_bloom: Vec3) -> Vec3 {
        // Bevy composites the sub-threshold bloom pyramid over the source as
        // source * (1 - intensity). Keep the input finite when intensity is 1.
        self.ceiling
            .limit(target_before_bloom / self.retained_color.max(1.0 / 65504.0))
    }
}

impl BloomCeiling {
    fn limit(self, color: Vec3) -> Vec3 {
        match self {
            Self::AtMost(ceiling) if color.max_element() > ceiling => {
                color * (ceiling / color.max_element())
            },
            Self::Unrestricted | Self::AtMost(_) => color,
        }
    }
}

fn f16_positive_to_f32(bits: u16) -> f32 {
    let exponent = (bits >> 10) & 0x1f;
    let mantissa = bits & 0x03ff;
    if exponent == 0 {
        f32::from(mantissa) * 2.0_f32.powi(-24)
    } else {
        (1.0 + f32::from(mantissa) / 1024.0) * 2.0_f32.powi(i32::from(exponent) - 15)
    }
}

#[derive(PartialEq, Eq)]
enum DisplayWindow {
    Primary,
    Entity(Entity),
}

impl WorldDisplayConfiguration {
    fn same_transform_as(&self, other: &Self) -> bool {
        self.camera == other.camera
            && self.order == other.order
            && self.window == other.window
            && self.method == other.method
            && same_grading(&self.grading, &other.grading)
            && self.bloom.same_effect_as(&other.bloom)
    }
}

fn same_grading(a: &ColorGrading, b: &ColorGrading) -> bool {
    let global_bits = |grading: &ColorGrading| {
        let global = &grading.global;
        [
            global.exposure.to_bits(),
            global.temperature.to_bits(),
            global.tint.to_bits(),
            global.hue.to_bits(),
            global.post_saturation.to_bits(),
            global.midtones_range.start.to_bits(),
            global.midtones_range.end.to_bits(),
        ]
    };
    global_bits(a) == global_bits(b)
        && a.shadows == b.shadows
        && a.midtones == b.midtones
        && a.highlights == b.highlights
}

pub(super) struct DisplayCompensationPlugin;

impl Plugin for DisplayCompensationPlugin {
    fn build(&self, app: &mut App) {
        if !app.world().contains_resource::<Assets<Image>>() {
            app.init_asset::<Image>();
        }
        let identity_image = identity_image();
        let mut images = app.world_mut().resource_mut::<Assets<Image>>();
        let identity = images.add(identity_image.clone());
        let world = images.add(identity_image);
        app.insert_resource(DisplayCompensationTextures { identity, world })
            .insert_resource(DisplayCompensation::AwaitingWorldCamera)
            .insert_resource(DisplayCompensationReadiness::Pending)
            .add_systems(
                Update,
                (
                    refresh_display_compensation,
                    publish_display_compensation_readiness,
                )
                    .chain(),
            );
    }
}

fn publish_display_compensation_readiness(
    state: Res<DisplayCompensation>,
    mut readiness: ResMut<DisplayCompensationReadiness>,
) {
    let next = DisplayCompensationReadiness::from(state.as_ref());
    if *readiness != next {
        *readiness = next;
    }
}

fn image_from_table(data: Vec<u8>) -> Image {
    let mut image = Image::new(
        Extent3d {
            width:                 u32::from(DISPLAY_TABLE_SIZE),
            height:                u32::from(DISPLAY_TABLE_SIZE),
            depth_or_array_layers: u32::from(DISPLAY_TABLE_SIZE),
        },
        TextureDimension::D3,
        data,
        DISPLAY_TABLE_FORMAT,
        RenderAssetUsages::default(),
    );
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor::linear());
    image
}

fn identity_image() -> Image { table_image(|color| color) }

fn table_image(mut transform: impl FnMut(Vec3) -> Vec3) -> Image {
    let mut data = Vec::with_capacity(usize::from(DISPLAY_TABLE_SIZE).pow(3) * 8);
    let scale = 1.0 / f32::from(DISPLAY_TABLE_SIZE - 1);
    for blue in 0..DISPLAY_TABLE_SIZE {
        for green in 0..DISPLAY_TABLE_SIZE {
            for red in 0..DISPLAY_TABLE_SIZE {
                let encoded = Vec3::new(f32::from(red), f32::from(green), f32::from(blue)) * scale;
                let srgb = Color::srgb(encoded.x, encoded.y, encoded.z).to_linear();
                let color = Vec3::new(srgb.red, srgb.green, srgb.blue);
                for channel in transform(color).extend(1.0).to_array() {
                    data.extend_from_slice(&to_f16_bits(channel).to_le_bytes());
                }
            }
        }
    }
    image_from_table(data)
}

fn to_f16_bits(value: f32) -> u16 {
    if value <= 0.0 || !value.is_finite() {
        return 0;
    }
    if value >= 65504.0 {
        return 0x7bff;
    }
    let bits = value.to_bits();
    let exponent = ((bits >> 23) & 0xff).cast_signed() - 112;
    let mantissa = bits & 0x007f_ffff;
    if exponent <= 0 {
        if exponent < -10 {
            return 0;
        }
        let shift = 14 - exponent;
        return u16::try_from(((mantissa | 0x0080_0000) + (1 << (shift - 1))) >> shift)
            .unwrap_or_default();
    }
    (u16::try_from(exponent).unwrap_or_default() << 10)
        + u16::try_from((mantissa + 0x1000) >> 13).unwrap_or_default()
}

fn refresh_display_compensation(
    cameras: Query<
        (
            Entity,
            &Camera,
            &RenderTarget,
            Option<&Tonemapping>,
            Option<&ColorGrading>,
            Option<&Bloom>,
        ),
        (With<Camera3d>, Without<ScreenSpaceCamera>),
    >,
    mut selected: Local<SelectedDisplayConfiguration>,
    mut state: ResMut<DisplayCompensation>,
    tables: Res<DisplayCompensationTextures>,
    luts: Option<Res<TonemappingLuts>>,
    mut images: ResMut<Assets<Image>>,
) {
    let primary = cameras
        .iter()
        .filter(|(_, camera, target, _, _, _)| {
            camera.is_active && matches!(target, RenderTarget::Window(_))
        })
        .min_by_key(|(entity, camera, _, _, _, _)| (camera.order, entity.index()));
    let Some((entity, camera, RenderTarget::Window(window), method, grading, bloom)) = primary
    else {
        if matches!(&*selected, SelectedDisplayConfiguration::NoWorldCamera) {
            return;
        }
        *selected = SelectedDisplayConfiguration::NoWorldCamera;
        set_identity(&mut images, &tables);
        *state = DisplayCompensation::AwaitingWorldCamera;
        return;
    };
    let configuration = WorldDisplayConfiguration {
        camera:  entity,
        order:   camera.order,
        window:  match window {
            WindowRef::Primary => DisplayWindow::Primary,
            WindowRef::Entity(window) => DisplayWindow::Entity(*window),
        },
        method:  method.copied().unwrap_or_default(),
        grading: grading.cloned().unwrap_or_default(),
        bloom:   WorldBloom::from_camera(bloom),
    };
    let unchanged = matches!(&*selected, SelectedDisplayConfiguration::WorldCamera(previous) if previous.same_transform_as(&configuration));
    let lut_became_ready = matches!(&*state, DisplayCompensation::AwaitingTonemappingLut)
        && luts.as_deref().is_some_and(|luts| {
            images
                .get(&luts.tony_mc_mapface)
                .is_some_and(|image| image.texture_descriptor.size.width == TONY_LUT_SIZE)
        });
    if unchanged && !lut_became_ready {
        if let DisplayCompensation::Building(task) = &mut *state
            && let Some(result) = check_ready(task)
        {
            match result {
                DisplayTableBuild::Ready(table) => {
                    replace_world_image(&mut images, &tables.world, *table);
                    *state = DisplayCompensation::Compensated;
                },
                DisplayTableBuild::UnreadableTonemappingLut => {
                    *state = DisplayCompensation::AwaitingTonemappingLut;
                },
            }
        }
        return;
    }
    let method = configuration.method;
    let grading = configuration.grading.clone();
    let bloom_compensation = configuration.bloom.compensation();
    *selected = SelectedDisplayConfiguration::WorldCamera(configuration);
    let lut_image = if method == Tonemapping::TonyMcMapface {
        luts.as_deref()
            .and_then(|luts| images.get(&luts.tony_mc_mapface))
            .filter(|image| image.texture_descriptor.size.width == TONY_LUT_SIZE)
            .cloned()
    } else {
        None
    };
    let next = match method {
        Tonemapping::None => DisplayCompensation::Identity,
        Tonemapping::KhronosPbrNeutral => start_khronos_table_build(grading, bloom_compensation),
        Tonemapping::TonyMcMapface => {
            if let Some(lut_image) = lut_image {
                start_tony_table_build(lut_image, grading, bloom_compensation)
            } else {
                DisplayCompensation::AwaitingTonemappingLut
            }
        },
        other => DisplayCompensation::Unsupported(other),
    };
    let already_identity = matches!(
        (&*state, &next),
        (DisplayCompensation::Identity, DisplayCompensation::Identity)
            | (
                DisplayCompensation::AwaitingTonemappingLut,
                DisplayCompensation::AwaitingTonemappingLut
            )
    ) || matches!(
        (&*state, &next),
        (DisplayCompensation::Unsupported(previous), DisplayCompensation::Unsupported(current))
            if previous == current
    );
    if !matches!(
        &next,
        DisplayCompensation::Building(_) | DisplayCompensation::AwaitingTonemappingLut
    ) && !already_identity
    {
        set_identity(&mut images, &tables);
    }
    *state = next;
}

fn start_khronos_table_build(
    grading: ColorGrading,
    bloom: BloomCompensation,
) -> DisplayCompensation {
    DisplayCompensation::Building(AsyncComputeTaskPool::get().spawn(async move {
        let transform = ForwardTransform::new(ToneCurve::KhronosPbrNeutral, grading);
        DisplayTableBuild::Ready(Box::new(table_image(|target| {
            bloom.input_for(transform.inverse(target))
        })))
    }))
}

fn start_tony_table_build(
    lut_image: Image,
    grading: ColorGrading,
    bloom: BloomCompensation,
) -> DisplayCompensation {
    DisplayCompensation::Building(AsyncComputeTaskPool::get().spawn(async move {
        let Some(lut) = TonyLut::from_image(&lut_image) else {
            return DisplayTableBuild::UnreadableTonemappingLut;
        };
        let transform = ForwardTransform::new(ToneCurve::TonyMcMapface(lut), grading);
        DisplayTableBuild::Ready(Box::new(table_image(|target| {
            bloom.input_for(transform.inverse(target))
        })))
    }))
}

fn set_identity(images: &mut Assets<Image>, tables: &DisplayCompensationTextures) {
    if let Some(identity) = images.get(&tables.identity).cloned() {
        replace_world_image(images, &tables.world, identity);
    }
}

fn replace_world_image(images: &mut Assets<Image>, handle: &Handle<Image>, replacement: Image) {
    if let Some(mut image) = images.get_mut(handle) {
        *image = replacement;
    }
}

struct ForwardTransform {
    curve:   ToneCurve,
    grading: ColorGrading,
    uniform: ColorGradingUniform,
}

enum ToneCurve {
    KhronosPbrNeutral,
    TonyMcMapface(TonyLut),
}

impl ForwardTransform {
    fn new(curve: ToneCurve, grading: ColorGrading) -> Self {
        let uniform = grading.clone().into();
        Self {
            curve,
            grading,
            uniform,
        }
    }

    fn apply(&self, input: Vec3) -> Vec3 {
        let mut color = input.max(Vec3::ZERO);
        if self.grading.global.hue != 0.0 {
            color = rotate_hue(color, self.grading.global.hue);
        }
        if self.grading.global.temperature != 0.0 || self.grading.global.tint != 0.0 {
            color = (self.uniform.balance * color).max(Vec3::ZERO);
        }
        if self
            .grading
            .all_sections()
            .any(|section| *section != default())
        {
            let level = (color.x + color.y + color.z) / 3.0;
            let [low, high] = self.uniform.midtone_range.to_array();
            let weights = if level < low - 0.1 {
                Vec3::X
            } else if level < low + 0.1 {
                let mid = (level - low).mul_add(5.0, 0.5).clamp(0.0, 1.0);
                Vec3::new(0.0, mid, 1.0 - mid)
            } else if level < high - 0.1 {
                Vec3::Y
            } else if level < high + 0.1 {
                let highlights = (level - high).mul_add(5.0, 0.5).clamp(0.0, 1.0);
                Vec3::new(0.0, 1.0 - highlights, highlights)
            } else {
                Vec3::Z
            };
            let saturation = weights.dot(self.uniform.saturation);
            let contrast = weights.dot(self.uniform.contrast);
            let gamma = weights.dot(self.uniform.gamma);
            let gain = weights.dot(self.uniform.gain);
            let lift = weights.dot(self.uniform.lift);
            let luma = color.dot(Vec3::new(0.2126, 0.7152, 0.0722));
            color = Vec3::splat(luma) + saturation * (color - Vec3::splat(luma));
            color = Vec3::splat(0.5) + (color - Vec3::splat(0.5)) * contrast;
            color = (color * gain + Vec3::splat(lift))
                .max(Vec3::ZERO)
                .powf(1.0 / gamma);
        }
        color = (color * self.uniform.exposure.exp2()).max(Vec3::ZERO);
        color = match &self.curve {
            ToneCurve::KhronosPbrNeutral => pbr_neutral(color),
            ToneCurve::TonyMcMapface(lut) => sample_tony(lut, color),
        };
        let luma = color.dot(Vec3::new(0.2126, 0.7152, 0.0722));
        Vec3::splat(luma) + self.uniform.post_saturation * (color - Vec3::splat(luma))
    }

    fn inverse(&self, target: Vec3) -> Vec3 {
        if (target.x - target.y).abs() < 1e-6
            && (target.y - target.z).abs() < 1e-6
            && self.grading.global.temperature == 0.0
            && self.grading.global.tint == 0.0
        {
            let mut low = 0.0;
            let mut high = 64.0;
            for _ in 0..20 {
                let midpoint = f32::midpoint(low, high);
                if self.apply(Vec3::splat(midpoint)).x < target.x {
                    low = midpoint;
                } else {
                    high = midpoint;
                }
            }
            return Vec3::splat(f32::midpoint(low, high));
        }
        let mut input = target / self.uniform.exposure.exp2();
        for _ in 0..14 {
            let base = self.apply(input);
            let error = target - base;
            if error.abs().max_element() < 0.0002 {
                break;
            }
            let step = 0.002 * (1.0 + input.max_element());
            let jacobian = Mat3::from_cols(
                (self.apply(input + Vec3::X * step) - base) / step,
                (self.apply(input + Vec3::Y * step) - base) / step,
                (self.apply(input + Vec3::Z * step) - base) / step,
            );
            let determinant = jacobian.determinant();
            if !determinant.is_finite() || determinant.abs() < 1e-12 {
                break;
            }
            let delta = jacobian.inverse() * error;
            if !delta.is_finite() {
                break;
            }
            let bounded = delta.clamp(Vec3::splat(-4.0), Vec3::splat(4.0));
            let mut fraction = 1.0;
            let mut improved = false;
            for _ in 0..8 {
                let candidate = (input + bounded * fraction).clamp(Vec3::ZERO, Vec3::splat(64.0));
                let candidate_error = target - self.apply(candidate);
                if candidate_error.length_squared() < error.length_squared() {
                    input = candidate;
                    improved = true;
                    break;
                }
                fraction *= 0.5;
            }
            if !improved {
                break;
            }
        }
        let luma_weights = Vec3::new(0.2126, 0.7152, 0.0722);
        if (self.apply(input) - target).abs().max_element() > 0.002 {
            for _ in 0..4 {
                let remaining = target.dot(luma_weights) - self.apply(input).dot(luma_weights);
                if remaining.abs() < 0.0002 {
                    break;
                }
                let step = 0.002 * (1.0 + input.max_element());
                let gradient = Vec3::new(
                    (self.apply(input + Vec3::X * step) - self.apply(input)).dot(luma_weights)
                        / step,
                    (self.apply(input + Vec3::Y * step) - self.apply(input)).dot(luma_weights)
                        / step,
                    (self.apply(input + Vec3::Z * step) - self.apply(input)).dot(luma_weights)
                        / step,
                );
                let available = if remaining > 0.0 {
                    gradient
                } else {
                    Vec3::select(input.cmpgt(Vec3::ZERO), gradient, Vec3::ZERO)
                };
                if available.length_squared() < 1e-12 {
                    break;
                }
                input = (input + available * (remaining / available.length_squared()))
                    .clamp(Vec3::ZERO, Vec3::splat(64.0));
            }
        }
        input
    }
}

fn pbr_neutral(input: Vec3) -> Vec3 {
    let minimum = input.min_element();
    let offset = if minimum < 0.08 {
        (6.25 * minimum).mul_add(-minimum, minimum)
    } else {
        0.04
    };
    let color = input - Vec3::splat(offset);
    let maximum = color.max_element();
    if maximum < 0.76 {
        return color;
    }
    let d = 0.24;
    let new_maximum = 1.0 - d * d / (maximum + d - 0.76);
    let compressed = color * (new_maximum / maximum);
    let blend = 1.0 - 1.0 / 0.15_f32.mul_add(maximum - new_maximum, 1.0);
    compressed.lerp(Vec3::splat(new_maximum), blend)
}

fn rotate_hue(color: Vec3, offset: f32) -> Vec3 {
    let maximum = color.max_element();
    let minimum = color.min_element();
    let chroma = maximum - minimum;
    let swizzle = if color.x >= color.y && color.x >= color.z {
        Vec3::new(color.y, color.z, 0.0)
    } else if color.y >= color.z {
        Vec3::new(color.z, color.x, 2.0)
    } else {
        Vec3::new(color.x, color.y, 4.0)
    };
    let hue = std::f32::consts::FRAC_PI_3 * (((swizzle.x - swizzle.y) / chroma + swizzle.z) % 6.0);
    let saturation = if maximum > 0.0 { chroma / maximum } else { 0.0 };
    let hue = (hue + offset) % std::f32::consts::TAU;
    let channel = |n: f32| {
        let k = (n + hue / std::f32::consts::FRAC_PI_3) % 6.0;
        (maximum * saturation).mul_add(-k.min((4.0 - k).min(1.0)).max(0.0), maximum)
    };
    Vec3::new(channel(5.0), channel(3.0), channel(1.0))
}

struct TonyLut {
    texels: Vec<Vec3>,
}

impl TonyLut {
    fn from_image(image: &Image) -> Option<Self> {
        let mut texels = Vec::with_capacity((TONY_LUT_SIZE as usize).pow(3));
        for z in 0..TONY_LUT_SIZE {
            for y in 0..TONY_LUT_SIZE {
                for x in 0..TONY_LUT_SIZE {
                    let color = if image.texture_descriptor.format == TextureFormat::Rgb9e5Ufloat {
                        let bytes: [u8; 4] = image
                            .pixel_bytes(UVec3::new(x, y, z))
                            .ok()?
                            .try_into()
                            .ok()?;
                        let packed = u32::from_le_bytes(bytes);
                        let scale = 2.0_f32.powi(((packed >> 27) & 0x1f).cast_signed() - 24);
                        Vec3::new(
                            f32::from(u16::try_from(packed & 0x1ff).ok()?),
                            f32::from(u16::try_from((packed >> 9) & 0x1ff).ok()?),
                            f32::from(u16::try_from((packed >> 18) & 0x1ff).ok()?),
                        ) * scale
                    } else {
                        let color = image.get_color_at_3d(x, y, z).ok()?.to_linear();
                        Vec3::new(color.red, color.green, color.blue)
                    };
                    texels.push(color);
                }
            }
        }
        Some(Self { texels })
    }

    fn texel(&self, x: u32, y: u32, z: u32) -> Vec3 {
        self.texels[(z * TONY_LUT_SIZE * TONY_LUT_SIZE + y * TONY_LUT_SIZE + x) as usize]
    }
}

fn sample_tony(lut: &TonyLut, stimulus: Vec3) -> Vec3 {
    let scale = f32::from(u16::try_from(TONY_LUT_SIZE - 1).unwrap_or_default());
    let coordinates = stimulus / (stimulus + Vec3::ONE) * scale;
    let low = coordinates
        .floor()
        .as_uvec3()
        .min(UVec3::splat(TONY_LUT_SIZE - 1));
    let high = (low + UVec3::ONE).min(UVec3::splat(TONY_LUT_SIZE - 1));
    let fraction = coordinates - low.as_vec3();
    let texel = |x, y, z| lut.texel(x, y, z);
    let x0 = texel(low.x, low.y, low.z).lerp(texel(high.x, low.y, low.z), fraction.x);
    let x1 = texel(low.x, high.y, low.z).lerp(texel(high.x, high.y, low.z), fraction.x);
    let x2 = texel(low.x, low.y, high.z).lerp(texel(high.x, low.y, high.z), fraction.x);
    let x3 = texel(low.x, high.y, high.z).lerp(texel(high.x, high.y, high.z), fraction.x);
    x0.lerp(x1, fraction.y)
        .lerp(x2.lerp(x3, fraction.y), fraction.z)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;
    use std::time::Instant;

    use bevy::asset::AssetPlugin;
    use bevy::core_pipeline::tonemapping::TonemappingPlugin;
    use bevy::image::ImagePlugin;
    use bevy::post_process::bloom::BloomPrefilter;
    use bevy::window::WindowRef;

    use super::*;

    fn authored(color: Color) -> Vec3 {
        let color = color.to_linear();
        Vec3::new(color.red, color.green, color.blue)
    }

    fn displayed(linear: Vec3) -> Vec3 {
        let color = Color::linear_rgb(linear.x, linear.y, linear.z).to_srgba();
        Vec3::new(color.red, color.green, color.blue)
    }

    fn sample_table(table: &Image, target: Vec3) -> Vec3 {
        let coordinates =
            displayed(target.clamp(Vec3::ZERO, Vec3::ONE)) * f32::from(DISPLAY_TABLE_SIZE - 1);
        let low = coordinates.floor().as_uvec3();
        let high = (low + UVec3::ONE).min(UVec3::splat(u32::from(DISPLAY_TABLE_SIZE - 1)));
        let fraction = coordinates - low.as_vec3();
        let texel = |x, y, z| {
            let sampled = table.get_color_at_3d(x, y, z);
            assert!(sampled.is_ok(), "table texel unavailable: {sampled:?}");
            let color = sampled.unwrap_or(Color::BLACK).to_linear();
            Vec3::new(color.red, color.green, color.blue)
        };
        let x0 = texel(low.x, low.y, low.z).lerp(texel(high.x, low.y, low.z), fraction.x);
        let x1 = texel(low.x, high.y, low.z).lerp(texel(high.x, high.y, low.z), fraction.x);
        let x2 = texel(low.x, low.y, high.z).lerp(texel(high.x, low.y, high.z), fraction.x);
        let x3 = texel(low.x, high.y, high.z).lerp(texel(high.x, high.y, high.z), fraction.x);
        x0.lerp(x1, fraction.y)
            .lerp(x2.lerp(x3, fraction.y), fraction.z)
    }

    fn actual_tony_lut() -> TonyLut {
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            AssetPlugin::default(),
            ImagePlugin::default(),
        ));
        app.init_asset::<Shader>();
        app.add_plugins(TonemappingPlugin);
        let handle = app
            .world()
            .resource::<TonemappingLuts>()
            .tony_mc_mapface
            .clone();
        let images = app.world().resource::<Assets<Image>>();
        let image = images.get(&handle);
        assert!(image.is_some(), "Tony LUT image is missing");
        let lut = image.and_then(TonyLut::from_image);
        assert!(
            lut.is_some(),
            "Tony LUT unreadable: {:?}",
            image.map(|image| (
                image.texture_descriptor.size,
                image.texture_descriptor.format,
                image.data.as_ref().map(Vec::len),
                image.get_color_at_3d(0, 0, 0),
            ))
        );
        lut.unwrap_or(TonyLut { texels: Vec::new() })
    }

    fn grading() -> ColorGrading {
        let mut grading = ColorGrading::default();
        grading.global.exposure = 0.25;
        grading.global.post_saturation = 0.96;
        grading.midtones.contrast = 1.03;
        grading
    }

    fn finish_table_build(app: &mut App) -> Duration {
        let started = Instant::now();
        for _ in 0..2_000 {
            app.update();
            if matches!(
                app.world().resource::<DisplayCompensation>(),
                DisplayCompensation::Compensated
            ) {
                return started.elapsed();
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(
            matches!(
                app.world().resource::<DisplayCompensation>(),
                DisplayCompensation::Compensated
            ),
            "display table build did not finish"
        );
        started.elapsed()
    }

    #[test]
    fn primary_window_camera_follows_order_activation_and_target_changes() {
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            AssetPlugin::default(),
            ImagePlugin::default(),
            DisplayCompensationPlugin,
        ));
        app.update();
        assert!(matches!(
            app.world().resource::<DisplayCompensation>(),
            DisplayCompensation::AwaitingWorldCamera
        ));

        let primary = app
            .world_mut()
            .spawn((
                Camera3d::default(),
                Camera {
                    order: 1,
                    ..default()
                },
                RenderTarget::Window(WindowRef::Primary),
                Tonemapping::None,
            ))
            .id();
        let second = app
            .world_mut()
            .spawn((
                Camera3d::default(),
                Camera {
                    order: 2,
                    ..default()
                },
                RenderTarget::Window(WindowRef::Primary),
                Tonemapping::Reinhard,
            ))
            .id();
        app.update();
        assert!(matches!(
            app.world().resource::<DisplayCompensation>(),
            DisplayCompensation::Identity
        ));

        {
            let mut entity = app.world_mut().entity_mut(second);
            let camera = entity.get_mut::<Camera>();
            assert!(camera.is_some(), "second camera is missing");
            if let Some(mut camera) = camera {
                camera.order = 0;
            }
        }
        app.update();
        assert!(matches!(
            app.world().resource::<DisplayCompensation>(),
            DisplayCompensation::Unsupported(Tonemapping::Reinhard)
        ));

        {
            let mut entity = app.world_mut().entity_mut(second);
            let camera = entity.get_mut::<Camera>();
            assert!(camera.is_some(), "second camera is missing");
            if let Some(mut camera) = camera {
                camera.is_active = false;
            }
        }
        app.update();
        assert!(matches!(
            app.world().resource::<DisplayCompensation>(),
            DisplayCompensation::Identity
        ));

        app.world_mut()
            .entity_mut(primary)
            .insert(RenderTarget::None { size: UVec2::ONE });
        app.update();
        assert!(matches!(
            app.world().resource::<DisplayCompensation>(),
            DisplayCompensation::AwaitingWorldCamera
        ));
    }

    #[test]
    fn unchanged_display_configuration_keeps_table_until_grading_changes() {
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            AssetPlugin::default(),
            ImagePlugin::default(),
            DisplayCompensationPlugin,
        ));
        let camera = app
            .world_mut()
            .spawn((
                Camera3d::default(),
                Camera::default(),
                RenderTarget::Window(WindowRef::Primary),
                Tonemapping::KhronosPbrNeutral,
            ))
            .id();
        let handle = app
            .world()
            .resource::<DisplayCompensationTextures>()
            .world
            .clone();
        let table_address = |app: &App| {
            app.world()
                .resource::<Assets<Image>>()
                .get(&handle)
                .and_then(|image| image.data.as_ref())
                .map(Vec::as_ptr)
        };
        let direct_started = Instant::now();
        let transform =
            ForwardTransform::new(ToneCurve::KhronosPbrNeutral, ColorGrading::default());
        let _direct_table = table_image(|target| transform.inverse(target));
        let direct_time = direct_started.elapsed();
        let frame_started = Instant::now();
        app.update();
        let frame_time = frame_started.elapsed();
        assert!(matches!(
            app.world().resource::<DisplayCompensation>(),
            DisplayCompensation::Building(_)
        ));
        let build_time = finish_table_build(&mut app);
        println!(
            "one table rebuild before (main thread): {direct_time:?}; after (main thread frame): {frame_time:?}; async completion: {build_time:?}"
        );
        let first = table_address(&app);
        assert!(first.is_some());

        app.world_mut().entity_mut(camera).insert(Camera::default());
        app.update();
        assert_eq!(table_address(&app), first);

        let mut changed_grading = ColorGrading::default();
        changed_grading.global.exposure = 0.25;
        app.world_mut().entity_mut(camera).insert(changed_grading);
        app.update();
        assert!(matches!(
            app.world().resource::<DisplayCompensation>(),
            DisplayCompensation::Building(_)
        ));
        assert_eq!(table_address(&app), first);
        finish_table_build(&mut app);
        assert_ne!(table_address(&app), first);
    }

    #[test]
    fn world_bloom_ceiling_bounds_white_and_every_table_texel() {
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            AssetPlugin::default(),
            ImagePlugin::default(),
            DisplayCompensationPlugin,
        ));
        app.world_mut().spawn((
            Camera3d::default(),
            Camera::default(),
            RenderTarget::Window(WindowRef::Primary),
            Tonemapping::KhronosPbrNeutral,
            Bloom {
                prefilter: BloomPrefilter {
                    threshold:          1.0,
                    threshold_softness: 0.1,
                },
                ..default()
            },
        ));
        finish_table_build(&mut app);
        let handle = &app.world().resource::<DisplayCompensationTextures>().world;
        let images = app.world().resource::<Assets<Image>>();
        let table = images.get(handle);
        assert!(table.is_some(), "world table is missing");
        if let Some(table) = table {
            let white = sample_table(table, authored(Color::WHITE));
            assert!(white.max_element() <= 0.9, "white: {white:?}");
            assert!(white.max_element() > 0.89, "white: {white:?}");
            let data = table.data.as_ref();
            assert!(data.is_some(), "world table has no texels");
            if let Some(data) = data {
                for texel in data.as_chunks::<8>().0 {
                    for channel in texel[..6].as_chunks::<2>().0 {
                        let value =
                            f16_positive_to_f32(u16::from_le_bytes([channel[0], channel[1]]));
                        assert!(value <= 0.9, "table channel exceeds bloom ceiling: {value}");
                    }
                }
            }
            let displayed_white = displayed(
                ForwardTransform::new(ToneCurve::KhronosPbrNeutral, ColorGrading::default())
                    .apply(white),
            );
            assert!(displayed_white.min_element() >= 234.0 / 255.0);
        }
    }

    #[test]
    fn energy_conserving_bloom_inverse_restores_subthreshold_fill() {
        let bloom = Bloom {
            prefilter: BloomPrefilter {
                threshold:          1.0,
                threshold_softness: 0.1,
            },
            ..default()
        };
        let compensation = WorldBloom::from_camera(Some(&bloom)).compensation();
        let transform =
            ForwardTransform::new(ToneCurve::KhronosPbrNeutral, ColorGrading::default());
        let target = authored(Color::srgb_u8(80, 100, 74));
        let pre_bloom = compensation.input_for(transform.inverse(target));
        let drawn = transform.apply(pre_bloom * (1.0 - bloom.intensity));
        assert!(
            (displayed(drawn) - displayed(target)).abs().max_element() <= 1.0 / 255.0,
            "subthreshold fill did not survive Bloom's source dimming: {drawn:?}"
        );
        assert!(pre_bloom.max_element() < 0.9);

        let additive = Bloom {
            composite_mode: BloomCompositeMode::Additive,
            ..bloom
        };
        let additive_compensation = WorldBloom::from_camera(Some(&additive)).compensation();
        assert_eq!(
            additive_compensation.input_for(transform.inverse(target)),
            transform.inverse(target)
        );
    }

    #[test]
    fn bloom_intensity_changes_effect_only_with_energy_conserving_compositing() {
        let bloom = Bloom {
            prefilter: BloomPrefilter {
                threshold:          1.0,
                threshold_softness: 0.1,
            },
            ..default()
        };
        let stronger = Bloom {
            intensity: 0.2,
            ..bloom.clone()
        };
        let additive = Bloom {
            composite_mode: BloomCompositeMode::Additive,
            ..bloom.clone()
        };
        let stronger_additive = Bloom {
            intensity: 0.2,
            ..additive.clone()
        };
        assert!(
            WorldBloom::from_camera(Some(&additive))
                .same_effect_as(&WorldBloom::from_camera(Some(&stronger_additive)))
        );
        assert!(
            !WorldBloom::from_camera(Some(&bloom))
                .same_effect_as(&WorldBloom::from_camera(Some(&stronger)))
        );
    }

    #[test]
    fn bright_fill_input_stops_at_bloom_ceiling() -> Result<(), &'static str> {
        let bloom = Bloom {
            prefilter: BloomPrefilter {
                threshold:          1.0,
                threshold_softness: 0.1,
            },
            ..default()
        };
        let compensation = WorldBloom::from_camera(Some(&bloom)).compensation();
        let transform =
            ForwardTransform::new(ToneCurve::KhronosPbrNeutral, ColorGrading::default());
        let inverse = transform.inverse(authored(Color::srgb_u8(230, 230, 230)));
        let BloomCeiling::AtMost(ceiling) = compensation.ceiling else {
            return Err("positive Bloom threshold should bound the input");
        };
        assert!(inverse.max_element() / compensation.retained_color > ceiling);
        let input = compensation.input_for(inverse);
        assert_eq!(input, Vec3::splat(ceiling));
        assert!(input.max_element() <= 0.9);
        Ok(())
    }

    #[test]
    fn world_camera_without_bloom_keeps_white_round_trip() {
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            AssetPlugin::default(),
            ImagePlugin::default(),
            DisplayCompensationPlugin,
        ));
        app.world_mut().spawn((
            Camera3d::default(),
            Camera::default(),
            RenderTarget::Window(WindowRef::Primary),
            Tonemapping::KhronosPbrNeutral,
        ));
        finish_table_build(&mut app);
        let handle = &app.world().resource::<DisplayCompensationTextures>().world;
        let images = app.world().resource::<Assets<Image>>();
        let table = images.get(handle);
        assert!(table.is_some(), "world table is missing");
        if let Some(table) = table {
            let white = authored(Color::WHITE);
            let drawn =
                ForwardTransform::new(ToneCurve::KhronosPbrNeutral, ColorGrading::default())
                    .apply(sample_table(table, white));
            assert!((displayed(drawn) - displayed(white)).abs().max_element() <= 2.0 / 255.0);
        }
    }

    #[test]
    fn zero_bloom_threshold_keeps_white_round_trip() {
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            AssetPlugin::default(),
            ImagePlugin::default(),
            DisplayCompensationPlugin,
        ));
        app.world_mut().spawn((
            Camera3d::default(),
            Camera::default(),
            RenderTarget::Window(WindowRef::Primary),
            Tonemapping::KhronosPbrNeutral,
            Bloom::default(),
        ));
        finish_table_build(&mut app);
        let handle = &app.world().resource::<DisplayCompensationTextures>().world;
        let images = app.world().resource::<Assets<Image>>();
        let table = images.get(handle);
        assert!(table.is_some(), "world table is missing");
        if let Some(table) = table {
            let white = authored(Color::WHITE);
            let drawn =
                ForwardTransform::new(ToneCurve::KhronosPbrNeutral, ColorGrading::default())
                    .apply(sample_table(table, white));
            assert!((displayed(drawn) - displayed(white)).abs().max_element() <= 2.0 / 255.0);
        }
    }

    #[test]
    fn bloom_prefilter_changes_rebuild_world_table_once() {
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            AssetPlugin::default(),
            ImagePlugin::default(),
            DisplayCompensationPlugin,
        ));
        let camera = app
            .world_mut()
            .spawn((
                Camera3d::default(),
                Camera::default(),
                RenderTarget::Window(WindowRef::Primary),
                Tonemapping::KhronosPbrNeutral,
                Bloom {
                    prefilter: BloomPrefilter {
                        threshold:          1.0,
                        threshold_softness: 0.1,
                    },
                    ..default()
                },
            ))
            .id();
        finish_table_build(&mut app);
        let handle = app
            .world()
            .resource::<DisplayCompensationTextures>()
            .world
            .clone();
        let table_data = |app: &App| {
            app.world()
                .resource::<Assets<Image>>()
                .get(&handle)
                .and_then(|table| table.data.clone())
        };
        let original = table_data(&app);
        app.update();
        assert!(matches!(
            app.world().resource::<DisplayCompensation>(),
            DisplayCompensation::Compensated
        ));
        assert_eq!(table_data(&app), original);

        let bloom = app.world_mut().get_mut::<Bloom>(camera);
        assert!(bloom.is_some(), "world camera lost Bloom");
        if let Some(mut bloom) = bloom {
            bloom.prefilter.threshold_softness = 0.2;
        }
        app.update();
        assert!(matches!(
            app.world().resource::<DisplayCompensation>(),
            DisplayCompensation::Building(_)
        ));
        assert_eq!(table_data(&app), original);
        finish_table_build(&mut app);
        let softer = table_data(&app);
        assert_ne!(softer, original);

        app.update();
        assert!(matches!(
            app.world().resource::<DisplayCompensation>(),
            DisplayCompensation::Compensated
        ));
        assert_eq!(table_data(&app), softer);

        app.world_mut().entity_mut(camera).remove::<Bloom>();
        app.update();
        assert!(matches!(
            app.world().resource::<DisplayCompensation>(),
            DisplayCompensation::Building(_)
        ));
        finish_table_build(&mut app);
        let without_bloom = table_data(&app);
        assert_ne!(without_bloom, softer);

        app.world_mut().entity_mut(camera).insert(Bloom {
            prefilter: BloomPrefilter {
                threshold:          1.0,
                threshold_softness: 0.2,
            },
            ..default()
        });
        app.update();
        assert!(matches!(
            app.world().resource::<DisplayCompensation>(),
            DisplayCompensation::Building(_)
        ));
        finish_table_build(&mut app);
        assert_eq!(table_data(&app), softer);
    }

    #[test]
    fn none_tonemapping_ignores_nondefault_grading() {
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            AssetPlugin::default(),
            ImagePlugin::default(),
            DisplayCompensationPlugin,
        ));
        let mut grading = ColorGrading::default();
        grading.global.exposure = 1.0;
        grading.global.hue = -1.0;
        grading.midtones.contrast = 1.1;
        app.world_mut().spawn((
            Camera3d::default(),
            Camera::default(),
            RenderTarget::Window(WindowRef::Primary),
            Tonemapping::None,
            grading,
            Bloom {
                prefilter: BloomPrefilter {
                    threshold:          1.0,
                    threshold_softness: 0.1,
                },
                ..default()
            },
        ));
        app.update();
        assert!(matches!(
            app.world().resource::<DisplayCompensation>(),
            DisplayCompensation::Identity
        ));
        let handle = &app.world().resource::<DisplayCompensationTextures>().world;
        let table = app.world().resource::<Assets<Image>>().get(handle);
        assert!(table.is_some(), "world table is missing");
        if let Some(table) = table {
            for authored_color in [
                Color::WHITE,
                Color::srgb(0.92, 0.96, 1.0),
                crate::widgets::ControlPalette::system_dark().accent,
            ] {
                let target = authored(authored_color);
                let drawn = sample_table(table, target);
                assert!(
                    (displayed(drawn) - displayed(target)).abs().max_element() <= 2.0 / 255.0,
                    "target {target:?}, drawn {drawn:?}"
                );
            }
        }
    }

    #[test]
    fn newer_display_configuration_supersedes_running_table_build() {
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            AssetPlugin::default(),
            ImagePlugin::default(),
            DisplayCompensationPlugin,
        ));
        let camera = app
            .world_mut()
            .spawn((
                Camera3d::default(),
                Camera::default(),
                RenderTarget::Window(WindowRef::Primary),
                Tonemapping::KhronosPbrNeutral,
            ))
            .id();
        let handle = app
            .world()
            .resource::<DisplayCompensationTextures>()
            .world
            .clone();
        let initial = app
            .world()
            .resource::<Assets<Image>>()
            .get(&handle)
            .and_then(|image| image.data.clone());
        app.update();
        assert!(matches!(
            app.world().resource::<DisplayCompensation>(),
            DisplayCompensation::Building(_)
        ));
        let current = app
            .world()
            .resource::<Assets<Image>>()
            .get(&handle)
            .and_then(|image| image.data.clone());
        assert_eq!(current, initial);

        let mut grading = ColorGrading::default();
        grading.global.exposure = 1.0;
        app.world_mut().entity_mut(camera).insert(grading.clone());
        app.update();
        assert!(matches!(
            app.world().resource::<DisplayCompensation>(),
            DisplayCompensation::Building(_)
        ));
        finish_table_build(&mut app);
        let table = app.world().resource::<Assets<Image>>().get(&handle);
        assert!(table.is_some());
        if let Some(table) = table {
            let target = authored(Color::srgb(0.5, 0.5, 0.5));
            let expected =
                ForwardTransform::new(ToneCurve::KhronosPbrNeutral, grading).inverse(target);
            assert!((sample_table(table, target) - expected).abs().max_element() < 0.01);
        }
    }

    #[test]
    fn negative_hue_rotation_matches_bevy_shader_remainder() {
        let color = Vec3::new(1.0, 0.0, 1.0);
        let offset = -std::f32::consts::PI;
        let chroma = 1.0;
        let hue = std::f32::consts::FRAC_PI_3 * (((0.0 - 1.0) / chroma) % 6.0);
        let shifted = (hue + offset) % std::f32::consts::TAU;
        let reference = Vec3::new(5.0, 3.0, 1.0).map(|n| {
            let k = (n + shifted / std::f32::consts::FRAC_PI_3) % 6.0;
            1.0 - k.min((4.0 - k).min(1.0)).max(0.0)
        });
        assert!((rotate_hue(color, offset) - reference).abs().max_element() < 1e-6);
        assert!(shifted < 0.0);
    }

    #[test]
    fn sampled_table_round_trips_dark_colors() {
        for method in [Tonemapping::KhronosPbrNeutral, Tonemapping::TonyMcMapface] {
            let curve = if method == Tonemapping::KhronosPbrNeutral {
                ToneCurve::KhronosPbrNeutral
            } else {
                ToneCurve::TonyMcMapface(actual_tony_lut())
            };
            let transform = ForwardTransform::new(curve, ColorGrading::default());
            let table = table_image(|target| transform.inverse(target));
            for channels in [
                [10, 10, 10],
                [20, 20, 20],
                [30, 30, 30],
                [40, 40, 40],
                [59, 53, 50],
            ] {
                let target = authored(Color::srgb_u8(channels[0], channels[1], channels[2]));
                let drawn = transform.apply(sample_table(&table, target));
                assert!(
                    (displayed(drawn) - displayed(target)).abs().max_element() <= 2.0 / 255.0,
                    "{method:?} {channels:?}: {:?}",
                    displayed(drawn)
                );
            }
        }
    }

    #[test]
    fn supported_methods_round_trip_text_and_bound_system_blue() {
        for method in [
            Tonemapping::None,
            Tonemapping::KhronosPbrNeutral,
            Tonemapping::TonyMcMapface,
        ] {
            if method == Tonemapping::None {
                let table = identity_image();
                for authored_color in [
                    Color::WHITE,
                    Color::srgb(0.92, 0.96, 1.0),
                    crate::widgets::ControlPalette::system_dark().accent,
                ] {
                    let target = authored(authored_color);
                    assert!(
                        (sample_table(&table, target) - target).abs().max_element() <= 2.0 / 255.0
                    );
                }
                continue;
            }
            let curve = if method == Tonemapping::KhronosPbrNeutral {
                ToneCurve::KhronosPbrNeutral
            } else {
                ToneCurve::TonyMcMapface(actual_tony_lut())
            };
            let transform = ForwardTransform::new(curve, grading());
            let table = table_image(|target| transform.inverse(target));
            for authored_color in [Color::WHITE, Color::srgb(0.92, 0.96, 1.0)] {
                let target = authored(authored_color);
                let drawn = transform.apply(sample_table(&table, target));
                assert!(
                    (displayed(drawn) - displayed(target)).abs().max_element() <= 2.0 / 255.0,
                    "{method:?}: target {target:?}, drawn {drawn:?}"
                );
            }
            let blue = authored(crate::widgets::ControlPalette::system_dark().accent);
            let compensated = transform.apply(sample_table(&table, blue));
            let error = (displayed(compensated) - displayed(blue)).abs();
            println!(
                "{method:?} system blue target {:?} drawn {:?} error {error:?}",
                displayed(blue),
                displayed(compensated)
            );
            let bound = if method == Tonemapping::TonyMcMapface {
                Vec3::new(0.19, 0.03, 0.16)
            } else {
                Vec3::new(0.18, 0.05, 0.09)
            };
            assert!(
                error.cmple(bound).all(),
                "{method:?}: system blue error {error:?} exceeds {bound}"
            );
            let white = authored(Color::WHITE);
            let drawn_white = transform.apply(sample_table(&table, white));
            let luminance = |color: Vec3| color.dot(Vec3::new(0.2126, 0.7152, 0.0722));
            let contrast =
                |ink: Vec3, ground: Vec3| (luminance(ink) + 0.05) / (luminance(ground) + 0.05);
            let authored_contrast = contrast(white, blue);
            let drawn_contrast = contrast(drawn_white, compensated);
            assert!(
                (drawn_contrast - authored_contrast).abs() < 0.1,
                "{method:?}: authored contrast {authored_contrast}, drawn {drawn_contrast}"
            );
        }
    }

    #[test]
    fn tony_lut_cannot_reach_system_blue_chroma() {
        let lut = actual_tony_lut();
        let target = authored(crate::widgets::ControlPalette::system_dark().accent);
        let greatest_blue_red_gap = lut
            .texels
            .iter()
            .map(|color| color.z - color.x)
            .fold(f32::NEG_INFINITY, f32::max);
        let required_gap = (target.z - target.x) / grading().global.post_saturation;
        assert!(
            greatest_blue_red_gap < required_gap,
            "LUT blue-red gap {greatest_blue_red_gap} reaches target gap {required_gap}"
        );
    }

    #[test]
    fn status_label_keeps_readable_contrast_after_fragment_compositing() {
        let background = authored(Color::srgb(0.22, 0.20, 0.19));
        let label = authored(Color::srgb(0.68, 0.75, 0.88));
        let alpha = (0.92_f32 * 255.0).round() / 255.0;
        let expected_ink = label * alpha + background * (1.0 - alpha);
        let luminance = |color: Vec3| color.dot(Vec3::new(0.2126, 0.7152, 0.0722));
        let contrast =
            |ink: Vec3, ground: Vec3| (luminance(ink) + 0.05) / (luminance(ground) + 0.05);
        let expected_contrast = contrast(expected_ink, background);
        for method in [Tonemapping::KhronosPbrNeutral, Tonemapping::TonyMcMapface] {
            let curve = if method == Tonemapping::KhronosPbrNeutral {
                ToneCurve::KhronosPbrNeutral
            } else {
                ToneCurve::TonyMcMapface(actual_tony_lut())
            };
            let transform = ForwardTransform::new(curve, ColorGrading::default());
            let table = table_image(|target| transform.inverse(target));
            let compensated_ink = sample_table(&table, label) * alpha
                + sample_table(&table, background) * (1.0 - alpha);
            let drawn_background = transform.apply(sample_table(&table, background));
            let drawn_ink = transform.apply(compensated_ink);
            let drawn_contrast = contrast(drawn_ink, drawn_background);
            assert!(
                (drawn_contrast - expected_contrast).abs() < 0.25,
                "{method:?}: expected {expected_contrast}, drawn {drawn_contrast}"
            );
            assert!(drawn_contrast >= 4.5, "{method:?}: {drawn_contrast}");
        }
    }
}
