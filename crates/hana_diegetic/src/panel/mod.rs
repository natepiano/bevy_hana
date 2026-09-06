//! Panel integration: components, layout computation, and gizmo rendering
//! — the Bevy-facing half of diegetic UI.

mod anchor_geometry;
mod anchoring;
mod arrangement;
mod builder;
mod compute_layout;
mod constants;
mod conversion;
mod coordinate_space;
mod diegetic_panel;
mod events;
mod field;
mod gizmos;
mod lifecycle;
mod perf;
mod precompose;
mod sizing;
mod valence_provider;

pub use anchor_geometry::PanelAnchorEdge;
pub use anchor_geometry::PanelAnchorEdgeEndpoints;
pub use anchor_geometry::PanelAnchorGeometryError;
pub use anchor_geometry::PanelAnchorGeometryParam;
pub use anchor_geometry::PanelAnchorPoint;
pub use anchor_geometry::PanelAnchorPoints;
pub use anchor_geometry::PanelPlane;
pub use anchor_geometry::PanelScreenBounds;
pub use anchor_geometry::ResolvedPanelAnchorGeometry;
pub(crate) use anchor_geometry::screen_anchor_position;
pub use anchoring::PanelAnchorOffset;
pub use anchoring::PanelAttachment;
pub(crate) use anchoring::PanelAttachmentAuthored;
pub(crate) use anchoring::ResolvedScreenPanelPosition;
pub(crate) use anchoring::WidgetOwnerLayout;
pub(crate) use anchoring::refresh_world_anchor_globals;
pub(crate) use anchoring::world_attachment_is_ready;
pub(crate) use anchoring::write_panel_anchor_offsets;
pub use arrangement::ArrangedPanel;
use bevy::ecs::schedule::ApplyDeferred;
use bevy::ecs::schedule::common_conditions::resource_exists;
use bevy::prelude::App;
use bevy::prelude::IntoScheduleConfigs;
use bevy::prelude::ObserverSystemExt;
use bevy::prelude::Plugin;
use bevy::prelude::PostUpdate;
use bevy::prelude::SystemSet;
use bevy::prelude::Update;
use bevy::transform::TransformSystems;
pub use builder::DiegeticPanelBuilder;
pub use builder::PanelBuildError;
pub use builder::PanelEntity;
pub use builder::PanelEntityReader;
pub use builder::Screen;
pub use builder::WidgetEntity;
pub use builder::World;
pub use conversion::PanelProjectionError;
pub use conversion::PanelProjectionParam;
pub use conversion::PanelScreenConversion;
pub use conversion::PanelScreenHandoff;
pub use conversion::PanelScreenProjection;
pub use conversion::PanelScreenTarget;
pub use conversion::PanelWorldConversion;
pub use conversion::PanelWorldProjection;
pub use conversion::PanelWorldTarget;
pub use conversion::SavedPanelScreenState;
pub use conversion::SavedPanelWorldState;
pub(crate) use conversion::apply_screen_conversion;
pub(crate) use conversion::apply_screen_root_sizing;
pub(crate) use conversion::apply_world_conversion;
pub(crate) use conversion::validate_screen_conversion;
pub(crate) use conversion::validate_world_conversion;
pub use coordinate_space::CoordinateSpace;
pub use coordinate_space::PanelSpace;
pub use coordinate_space::ScreenPosition;
pub use coordinate_space::SurfaceShadow;
pub(crate) use coordinate_space::constrain_fit_width;
pub use diegetic_panel::ComputedDiegeticPanel;
pub use diegetic_panel::DiegeticPanel;
pub(crate) use diegetic_panel::DiegeticPanelChangeClassification;
pub use diegetic_panel::DiegeticPanelCommands;
pub use diegetic_panel::PanelLayout;
pub(crate) use diegetic_panel::apply_precompose_helper_panel;
pub(crate) use events::LastPanelDimensions;
pub use events::PanelChangeKind;
pub use events::PanelChanged;
pub use events::PanelDimensions;
pub use events::PanelDimensionsChanged;
pub(crate) use events::trigger_panel_dimensions_changed;
pub(crate) use field::PanelFieldPresentation;
pub use field::PanelFieldRecord;
pub use gizmos::ShowTextGizmos;
use hana_valence::AnchorResolveDiagnostics;
use hana_valence::AnchorSystems;
use hana_valence::HingePlugin;
pub(crate) use lifecycle::PanelComponentOwnership;
pub(crate) use lifecycle::PanelOwned;
pub(crate) use lifecycle::PanelRenderLayersOwnership;
pub(crate) use lifecycle::remove_owned_component;
pub(crate) use lifecycle::write_owned_component;
pub(crate) use lifecycle::write_owned_render_layers;
pub use perf::BatchPerfStats;
pub use perf::BatchSummary;
use perf::DiagnosticsPlugin;
pub use perf::DiegeticPerfStats;
pub use perf::DroppedSdfSurfaces;
pub use perf::FrameWork;
pub use perf::LifetimeTotal;
pub use perf::LiveCount;
pub use perf::MaterialTablePerfStats;
pub use perf::PanelGeometryPerfStats;
pub use perf::PanelShapeBatchPerfStats;
pub use perf::PanelTextPerfStats;
pub(crate) use precompose::PanelPrecomposeCache;
pub(crate) use precompose::PrecomposeCacheEntry;
pub use precompose::PrecomposeHelper;
pub use sizing::AnyUnit;
pub use sizing::CompatibleUnits;
pub use sizing::Fit;
pub use sizing::FitMax;
pub use sizing::FitRange;
pub use sizing::Grow;
pub use sizing::GrowMax;
pub use sizing::GrowRange;
pub use sizing::Inches;
pub use sizing::Millimeters;
pub use sizing::PanelSizing;
pub use sizing::Percent;
pub use sizing::Pixels;
pub use sizing::Points;

use crate::cascade;
use crate::cascade::CascadeSet;
use crate::cascade::FontUnit;
use crate::cascade::HdrTextCoverageBias;
use crate::cascade::PanelDefaults;
use crate::cascade::SdfMaterial;
use crate::cascade::ShapeMaterial;
use crate::cascade::TextAlpha;
use crate::cascade::TextMaterial;
use crate::layout::GlyphShadowMode;
use crate::layout::Lighting;
use crate::layout::ShadowCasting;
use crate::layout::ShapedTextCache;
use crate::layout::Sidedness;
use crate::render::AntiAlias;
use crate::render::HairlineFade;
use crate::widgets::WidgetDisabledAppearance;
use crate::widgets::WidgetFocusAuthority;
use crate::widgets::WidgetFocusedAppearance;
use crate::widgets::WidgetHoveredAppearance;
use crate::widgets::WidgetInteractivity;
use crate::widgets::WidgetPressedAppearance;

/// System sets for ordering panel work and its cross-module dependencies.
///
/// Other plugins (e.g., `ScreenSpacePlugin`)
/// use `.before(PanelSystems::ComputeLayout)` / `.after(PanelSystems::ComputeLayout)`
/// rather than referencing concrete system symbols.
#[derive(SystemSet, Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum PanelSystems {
    /// Applies deferred tree replacement commands before layout responds.
    ApplyTreeChanges,
    /// Applies queued coordinate-space conversions before layout responds.
    ApplyConversions,
    /// Runs `compute_panel_layouts`.
    ComputeLayout,
    /// Runs `resolve_world_panel_fit`
    /// — shrinks world panels with `Fit` axes to their content bounds.
    ResolveWorldFit,
    /// Resolves panel-to-panel attachments before screen-space positioning.
    ResolvePanelAttachments,
    /// Positions screen-space panels after `Fit` dimensions have resolved.
    PositionScreenSpace,
    /// Runs gizmo reconciliation
    /// (`render_debug_gizmos`).
    RenderGizmos,
    /// `PostUpdate` ordering point for animation systems that write
    /// resolver-read inputs — `hana_valence::AnchorPose`, and relation
    /// insert/remove at state boundaries — before `hana_valence::resolve_anchors`.
    /// Writes here land this frame; writes after the resolver land next frame.
    AnimateAnchorPose,
}

macro_rules! add_cascade_ownership_observers {
    ($app:expr, $($attribute:ty),+ $(,)?) => {
        $(
            $app.add_observer(lifecycle::record_resolved_ownership::<$attribute>);
            $app.add_observer(lifecycle::restore_preserved_resolved::<$attribute>);
        )+
    };
}

fn register_cascade_ownership_observers(app: &mut App) {
    add_cascade_ownership_observers!(
        app,
        FontUnit,
        AntiAlias,
        HairlineFade,
        HdrTextCoverageBias,
        ShadowCasting,
        WidgetInteractivity,
        WidgetHoveredAppearance,
        WidgetPressedAppearance,
        WidgetFocusedAppearance,
        WidgetDisabledAppearance,
        TextAlpha,
        Lighting,
        Sidedness,
        GlyphShadowMode,
        SdfMaterial,
        TextMaterial,
        ShapeMaterial,
    );
}

/// Headless layout runner — schedules `compute_panel_layouts` on `Update`
/// and initializes the resources it consumes (diagnostics, perf stats,
/// shaped-text cache).
///
/// External consumers (benchmarks, non-UI apps) register this plugin
/// instead of [`DiegeticUiPlugin`](crate::DiegeticUiPlugin) when they
/// only need [`DiegeticPanel`] → [`ComputedDiegeticPanel`] computation.
/// Client tests that also need widget and IME behavior use
/// [`HeadlessDiegeticUiPlugin`](crate::HeadlessDiegeticUiPlugin) instead.
/// The plugin initializes [`PanelDefaults`] itself (idempotent); callers
/// insert their own [`DiegeticTextMeasurer`](crate::DiegeticTextMeasurer)
/// and optionally override construction defaults before adding this plugin.
pub struct HeadlessLayoutPlugin;

impl Plugin for HeadlessLayoutPlugin {
    fn build(&self, app: &mut App) {
        // Panels hinge onto panels, so this crate owns the hinge driver
        // rather than leaving every application to remember it. `HingePlugin`
        // needs neither assets nor scenes, so it composes headlessly.
        app.add_plugins(HingePlugin)
            .add_plugins(DiagnosticsPlugin)
            .add_plugins(cascade::cascade_plugin::<FontUnit>())
            // `HeadlessLayoutPlugin` registers the attribute cascades because
            // `DiegeticPanel` participates even when `RenderPlugin` is absent.
            .add_plugins(cascade::cascade_plugin::<AntiAlias>())
            .add_plugins(cascade::cascade_plugin::<HairlineFade>())
            .add_plugins(cascade::cascade_plugin::<HdrTextCoverageBias>())
            .add_plugins(cascade::cascade_plugin::<ShadowCasting>())
            .add_observer(diegetic_panel::seed_panel_overrides)
            .add_observer(diegetic_panel::sync_panel_picking_on_insert);

        register_cascade_ownership_observers(app);

        app.add_observer(
            lifecycle::finalize_panel_widgets_before_despawn
                .run_if(resource_exists::<WidgetFocusAuthority>),
        );
        app.add_observer(lifecycle::finalize_orphaned_panel_owned)
            .add_observer(lifecycle::teardown_panel_role)
            .init_resource::<DiegeticPerfStats>()
            .init_resource::<ShapedTextCache>()
            .init_resource::<PanelDefaults>()
            .init_resource::<AnchorResolveDiagnostics>()
            .add_observer(coordinate_space::sync_panel_space_on_insert)
            .add_observer(anchoring::on_panel_attachment_inserted)
            .add_observer(anchoring::on_panel_attachment_removed)
            .configure_sets(
                Update,
                (
                    PanelSystems::ApplyTreeChanges.before(PanelSystems::ApplyConversions),
                    PanelSystems::ApplyConversions.before(CascadeSet::Propagate),
                    CascadeSet::Propagate.before(PanelSystems::ComputeLayout),
                    PanelSystems::ResolveWorldFit.after(PanelSystems::ComputeLayout),
                ),
            )
            .add_systems(
                Update,
                (
                    ApplyDeferred.in_set(PanelSystems::ApplyTreeChanges),
                    ApplyDeferred.in_set(PanelSystems::ApplyConversions),
                    compute_layout::compute_panel_layouts.in_set(PanelSystems::ComputeLayout),
                    compute_layout::resolve_world_panel_fit.in_set(PanelSystems::ResolveWorldFit),
                ),
            )
            .configure_sets(
                PostUpdate,
                (
                    (
                        AnchorSystems::FillGeometry,
                        AnchorSystems::AnimatePose,
                        AnchorSystems::Resolve,
                    )
                        .chain()
                        .before(TransformSystems::Propagate),
                    PanelSystems::AnimateAnchorPose
                        .in_set(AnchorSystems::AnimatePose)
                        .before(AnchorSystems::HingeToPose),
                ),
            )
            .add_systems(
                PostUpdate,
                (
                    anchoring::restore_inactive_world_panel_poses.before(AnchorSystems::Resolve),
                    valence_provider::write_panel_anchor_geometry
                        .in_set(AnchorSystems::FillGeometry),
                    anchoring::write_panel_anchor_offsets.before(AnchorSystems::Resolve),
                    anchoring::refresh_world_anchor_globals
                        .after(AnchorSystems::AnimatePose)
                        .before(anchoring::write_panel_anchor_offsets)
                        .before(AnchorSystems::Resolve)
                        .run_if(anchoring::world_anchor_transform_inputs_changed),
                    hana_valence::resolve_anchors.in_set(AnchorSystems::Resolve),
                ),
            );
    }
}

/// Panel gizmo debug rendering installed after [`HeadlessLayoutPlugin`].
/// Registered by [`DiegeticUiPlugin`](crate::DiegeticUiPlugin).
pub(crate) struct PanelPlugin;

impl Plugin for PanelPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ShowTextGizmos>()
            .configure_sets(
                Update,
                PanelSystems::RenderGizmos.after(PanelSystems::ResolveWorldFit),
            )
            .add_systems(
                Update,
                gizmos::render_debug_gizmos.in_set(PanelSystems::RenderGizmos),
            );
    }
}
