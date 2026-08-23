//! `hana_diegetic` — Diegetic UI for Bevy.
//!
//! Provides an in-world UI layout engine inspired by [Clay](https://github.com/nicbarker/clay),
//! implemented in pure Rust with no global state and full thread safety.
//!
//! # Retained-mode layout
//!
//! Clay is immediate-mode: the tree is rebuilt from scratch every frame and layout is computed
//! inline as you build it. `hana_diegetic` is retained-mode: the [`LayoutTree`] is built once
//! via [`LayoutBuilder`], stored on a component, and the
//! `LayoutEngine` only recomputes positions when the tree changes.
//! This is the natural fit for Bevy — the entire ECS is built around doing nothing unless something
//! changed (`Changed<T>`, `Res::is_changed()`, observers). An immediate-mode engine would fight the
//! framework by recomputing unconditionally every frame; retained mode lets Bevy's change detection
//! skip layout entirely on frames where the tree hasn't been touched.
//!
//! # Quick start
//!
//! ```ignore
//! use bevy::prelude::*;
//! use hana_diegetic::*;
//!
//! App::new()
//!     .add_plugins(DefaultPlugins)
//!     .add_plugins(DiegeticUiPlugin)
//!     .add_systems(Startup, setup)
//!     .run();
//! ```
//!
//! # Configuration
//!
//! Insert default resources before adding [`DiegeticUiPlugin`] to override
//! construction-time defaults or cascade defaults:
//!
//! ```ignore
//! App::new()
//!     .insert_resource(PanelDefaults {
//!         panel_font_unit: Unit::Millimeters,
//!         ..default()
//!     })
//!     .insert_resource(TextAlpha(AlphaMode::Add))
//!     // Optional: add coverage compensation beyond the automatic
//!     // screen-size adjustment when HDR makes text look too thin.
//!     .insert_resource(HdrTextCoverageBias(2.0))
//!     .add_plugins(DiegeticUiPlugin);
//! ```

mod callouts;
mod cascade;
mod constants;
#[cfg(feature = "typography_overlay")]
mod debug;
mod fluent;
mod ime;
mod layout;
mod panel;
mod render;
mod screen_space;
mod text;
mod widgets;

#[cfg(feature = "bench_support")]
#[doc(hidden)]
/// Internal exports used by feature-gated benchmarks.
pub mod bench_support {
    pub use crate::layout::LayoutEngine;
    pub use crate::layout::LayoutResult;
    pub use crate::layout::LayoutTreeChange;
    pub use crate::layout::MeasureTextFn;
    pub use crate::layout::RectangleSource;
    pub use crate::layout::RenderCommand;
    pub use crate::layout::RenderCommandKind;
}

use bevy::asset::embedded_asset;
use bevy::prelude::App;
use bevy::prelude::IntoScheduleConfigs;
use bevy::prelude::Plugin;
use bevy::prelude::Update;
pub use callouts::ArrowStyle;
pub use callouts::CalloutCap;
pub use cascade::CascadeEntityCommandsExt;
pub use cascade::CascadeSet;
pub use cascade::FontUnit;
pub use cascade::HdrTextCoverageBias;
pub use cascade::PanelDefaults;
pub use cascade::SdfMaterial;
pub use cascade::ShapeMaterial;
pub use cascade::TextAlpha;
pub use cascade::TextMaterial;
pub use cascade::resolved_anti_alias;
pub use cascade::resolved_font_unit;
pub use cascade::resolved_glyph_shadow_mode;
pub use cascade::resolved_hairline_fade;
pub use cascade::resolved_hdr_text_coverage_bias;
pub use cascade::resolved_lighting;
pub use cascade::resolved_sdf_material;
pub use cascade::resolved_shadow_casting;
pub use cascade::resolved_shape_material;
pub use cascade::resolved_sidedness;
pub use cascade::resolved_text_alpha;
pub use cascade::resolved_text_material;
#[cfg(feature = "typography_overlay")]
pub use debug::GlyphMetricVisibility;
#[cfg(feature = "typography_overlay")]
pub use debug::OverlayBoundingBox;
#[cfg(feature = "typography_overlay")]
pub use debug::TypographyOverlay;
use debug::TypographyOverlayPlugin;
pub use fluent::DiegeticText;
pub use fluent::DiegeticTextBuilder;
use fluent::DiegeticTextPlugin;
pub use ime::ImeAcceptCommit;
pub use ime::ImeAppInputContext;
pub use ime::ImeAppInputDisposition;
pub use ime::ImeAppInputDispositionHook;
pub use ime::ImeAppOwnedFieldSpec;
pub use ime::ImeApplied;
pub use ime::ImeAppliedResult;
pub use ime::ImeBufferBoundary;
pub use ime::ImeBufferRange;
pub use ime::ImeBufferSnapshot;
pub use ime::ImeBuiltInApplied;
pub use ime::ImeBuiltInFieldKind;
pub use ime::ImeBuiltInFieldSpec;
pub use ime::ImeBuiltInValue;
pub use ime::ImeCancelCause;
pub use ime::ImeCanceled;
pub use ime::ImeCommitAttemptId;
pub use ime::ImeCommitAuthority;
pub use ime::ImeCommitAuthorityToken;
pub use ime::ImeCommitCause;
pub use ime::ImeCommitRequested;
pub use ime::ImeCursorState;
pub use ime::ImeEditableFieldSpec;
pub use ime::ImeInputBlocker;
pub use ime::ImeOpenSession;
pub use ime::ImePanelField;
use ime::ImePlugin;
pub use ime::ImePreedit;
pub use ime::ImePreeditBoundary;
pub use ime::ImeRejectCommit;
pub use ime::ImeRejection;
pub use ime::ImeReplacePanelTree;
pub use ime::ImeRequestCancel;
pub use ime::ImeRequestCommit;
pub use ime::ImeSelectionSnapshot;
pub use ime::ImeSessionAnchor;
pub use ime::ImeSessionId;
pub use ime::ImeStarted;
pub use ime::ImeSystemSet;
pub use ime::ImeTarget;
pub use ime::ImeTextChanged;
pub use ime::ImeValidationRejected;
pub use ime::ImeValueRevision;
pub use ime::PanelElementId;
pub use layout::AcceptsElement;
pub use layout::AlignX;
pub use layout::AlignY;
pub use layout::Anchor;
pub use layout::Border;
pub use layout::BoundingBox;
pub use layout::ChildDivider;
pub use layout::ChildLayoutState;
pub use layout::Column;
pub use layout::CornerRadius;
pub use layout::Dimension;
pub use layout::DimensionMatch;
pub use layout::Direction;
pub use layout::DrawOverflow;
pub use layout::DrawZIndex;
pub use layout::EditableField;
pub use layout::EditorStateColors;
pub use layout::El;
pub use layout::ElementRole;
pub use layout::FontFeatureFlags;
pub use layout::FontFeatures;
pub use layout::FontSlant;
pub use layout::FontWeight;
pub use layout::GlyphRenderMode;
pub use layout::GlyphShadowMode;
pub use layout::HasUnit;
pub use layout::In;
pub use layout::InvalidPanelScalar;
pub use layout::InvalidSize;
pub use layout::LayoutBuilder;
pub use layout::LayoutContentBuilder;
pub use layout::LayoutOnly;
pub use layout::LayoutTree;
pub use layout::LayoutTreeChange;
pub use layout::Lighting;
pub use layout::LineStyle;
/// Function signature for custom text measurement. Takes a text string and
/// a [`TextMeasure`] describing the font configuration, returns
/// [`TextDimensions`]. See [`DiegeticTextMeasurer`] and the `side_by_side`
/// example for usage.
pub use layout::MeasureTextFn;
pub use layout::Mm;
pub use layout::Overlay;
pub use layout::Padding;
pub use layout::PanelCircle;
pub use layout::PanelCoord;
pub use layout::PanelDraw;
pub use layout::PanelLine;
pub use layout::PanelPoint;
pub use layout::PanelShape;
pub use layout::PanelShapePrimitiveGeometry;
pub use layout::PanelShapePrimitiveKey;
pub use layout::PanelShapePrimitiveKind;
pub use layout::PanelShapeSourceKey;
pub use layout::PanelSize;
pub use layout::PaperSize;
pub use layout::Pressable;
pub use layout::PressedEditorStateColors;
pub use layout::PressedPart;
pub use layout::Pt;
pub use layout::Px;
pub use layout::ResolvedPanelShape;
pub use layout::ResolvedPanelShapePrimitive;
pub use layout::Row;
pub use layout::ShadowCasting;
pub use layout::Sidedness;
pub use layout::Sizing;
pub use layout::Text;
pub use layout::TextAlign;
/// Measured width and height of a text string, returned by [`MeasureTextFn`].
pub use layout::TextDimensions;
/// Font configuration passed to [`MeasureTextFn`]: font ID, size, weight,
/// slant, line height, letter/word spacing. See the `side_by_side` example
/// for a real-world custom measurer that bridges clay-layout to our
/// parley-backed measurement via this type.
pub use layout::TextMeasure;
pub use layout::TextSizing;
pub use layout::TextStyle;
pub use layout::TextWrap;
pub use layout::Unit;
pub use layout::Widget;
pub use layout::WidgetBuilder;
pub use layout::WidgetChild;
pub use layout::WidgetDeclaration;
pub use layout::WidgetElement;
pub use layout::WidgetOwner;
pub use layout::WidgetPart;
pub use layout::WidgetRootSlot;
pub use panel::AnyUnit;
pub use panel::ArrangedPanel;
pub use panel::BatchPerfStats;
pub use panel::BatchSummary;
pub use panel::CompatibleUnits;
pub use panel::ComputedDiegeticPanel;
pub use panel::CoordinateSpace;
pub use panel::DiegeticPanel;
pub use panel::DiegeticPanelBuilder;
pub use panel::DiegeticPanelCommands;
pub use panel::DiegeticPanelGizmoGroup;
pub use panel::DiegeticPerfStats;
pub use panel::DroppedSdfSurfaces;
pub use panel::Fit;
pub use panel::FitMax;
pub use panel::FitRange;
pub use panel::FrameWork;
pub use panel::Grow;
pub use panel::GrowMax;
pub use panel::GrowRange;
pub use panel::HeadlessLayoutPlugin;
pub use panel::Inches;
pub use panel::LifetimeTotal;
pub use panel::LiveCount;
pub use panel::MaterialTablePerfStats;
pub use panel::Millimeters;
pub use panel::PanelAnchorEdge;
pub use panel::PanelAnchorEdgeEndpoints;
pub use panel::PanelAnchorGeometryError;
pub use panel::PanelAnchorGeometryParam;
pub use panel::PanelAnchorOffset;
pub use panel::PanelAnchorPoint;
pub use panel::PanelAnchorPoints;
pub use panel::PanelAttachment;
pub use panel::PanelBuildError;
pub use panel::PanelChangeKind;
pub use panel::PanelChanged;
pub use panel::PanelDimensions;
pub use panel::PanelDimensionsChanged;
pub use panel::PanelEntity;
pub use panel::PanelEntityReader;
pub use panel::PanelFieldRecord;
pub use panel::PanelGeometryPerfStats;
pub use panel::PanelLayout;
pub use panel::PanelPlane;
use panel::PanelPlugin;
pub use panel::PanelProjectionError;
pub use panel::PanelProjectionParam;
pub use panel::PanelScreenBounds;
pub use panel::PanelScreenConversion;
pub use panel::PanelScreenHandoff;
pub use panel::PanelScreenProjection;
pub use panel::PanelScreenTarget;
pub use panel::PanelShapeBatchPerfStats;
pub use panel::PanelSizing;
pub use panel::PanelSpace;
pub use panel::PanelSystems;
pub use panel::PanelTextPerfStats;
pub use panel::PanelWorldConversion;
pub use panel::PanelWorldProjection;
pub use panel::PanelWorldTarget;
pub use panel::Percent;
pub use panel::Pixels;
pub use panel::Points;
pub use panel::PrecomposeHelper;
pub use panel::ResolvedPanelAnchorGeometry;
pub use panel::SavedPanelScreenState;
pub use panel::SavedPanelWorldState;
pub use panel::Screen;
pub use panel::ScreenPosition;
pub use panel::ShowTextGizmos;
pub use panel::SurfaceShadow;
pub use panel::WidgetEntity;
pub use panel::World;
#[doc(hidden)]
pub use render::AnalyticLine;
#[doc(hidden)]
pub use render::AnalyticLineProbe;
#[doc(hidden)]
pub use render::AnalyticLineProbePlugin;
pub use render::AntiAlias;
pub use render::DiegeticTextBatch;
pub use render::DiegeticTextMut;
pub use render::HairlineFade;
pub use render::HairlineWidth;
pub use render::PanelText;
pub use render::PanelTextLayout;
pub use render::PanelTextReader;
pub use render::PanelTextRuns;
use render::RenderPlugin;
pub use render::StableTransparency;
pub use render::TextContent;
pub use render::TextEdit;
pub use render::TextRunOf;
pub use render::WorldTextReady;
pub use render::default_panel_material;
pub use screen_space::ScreenAnchorTarget;
pub use screen_space::ScreenSpaceCamera;
pub use screen_space::ScreenSpaceLight;
use screen_space::ScreenSpacePlugin;
pub use text::DiegeticTextMeasurer;
pub use text::Font;
pub use text::FontId;
pub use text::FontLoadFailed;
pub use text::FontMetrics;
pub use text::FontRegistered;
pub use text::FontRegistry;
pub use text::FontSource;
#[cfg(feature = "typography_overlay")]
pub use text::GlyphBounds;
#[cfg(feature = "typography_overlay")]
pub use text::GlyphTypographyMetrics;
pub use text::IntegralAdvanceSizeError;
use text::TextPlugin;
pub use widgets::Appearance;
pub use widgets::BackgroundColor;
pub use widgets::BorderColor;
pub use widgets::Button;
pub use widgets::ButtonCancelCause;
pub use widgets::ButtonCanceled;
pub use widgets::ButtonClicked;
pub use widgets::ButtonPressed;
pub use widgets::ButtonReleased;
pub use widgets::ClearWidgetFocus;
pub use widgets::FacePicking;
pub use widgets::IntoAppearance;
pub use widgets::MeshAnchorCommandsExt;
pub use widgets::MeshFace;
pub use widgets::PanelPicking;
pub use widgets::PanelWidget;
pub use widgets::PanelWidgetReader;
pub use widgets::PanelWidgetWriter;
pub use widgets::PanelWidgets;
pub use widgets::PathColor;
pub use widgets::RequestPanelFocus;
pub use widgets::RequestSliderAdjustment;
pub use widgets::RequestWidgetFocus;
pub use widgets::ScreenAnchorCommandsExt;
pub use widgets::Slider;
pub use widgets::SliderAdjustment;
pub use widgets::SliderCancelCause;
pub use widgets::SliderCanceled;
pub use widgets::SliderChangeRequested;
pub use widgets::SliderConfigError;
pub use widgets::SliderDirection;
pub use widgets::SliderGrabbed;
pub use widgets::SliderRange;
pub use widgets::SliderReleased;
pub use widgets::SliderResetBehavior;
pub use widgets::SliderState;
pub use widgets::SliderStep;
pub use widgets::TextColor;
pub use widgets::Tint;
pub use widgets::Tooltip;
pub use widgets::TooltipCommandsExt;
pub use widgets::TooltipDisabledPolicy;
pub use widgets::TooltipFor;
pub use widgets::TooltipHidden;
pub use widgets::TooltipPlacementPolicy;
pub use widgets::TooltipShown;
pub use widgets::TooltipTarget;
pub use widgets::TooltipTargetEntity;
pub use widgets::TooltipTargetSpace;
pub use widgets::Tooltips;
pub use widgets::WidgetControlSummary;
pub use widgets::WidgetDisabled;
pub use widgets::WidgetDisabledAppearance;
pub use widgets::WidgetFocusChangeCause;
pub use widgets::WidgetFocusChanged;
pub use widgets::WidgetFocusable;
pub use widgets::WidgetFocused;
pub use widgets::WidgetFocusedAppearance;
pub use widgets::WidgetHoveredAppearance;
pub use widgets::WidgetInput;
pub use widgets::WidgetInputBindings;
pub use widgets::WidgetInputBindingsBuilder;
pub use widgets::WidgetInputBindingsError;
pub use widgets::WidgetInputDisabled;
pub use widgets::WidgetInputMode;
pub use widgets::WidgetInputPlugin;
pub use widgets::WidgetInteractivity;
pub use widgets::WidgetOf;
pub use widgets::WidgetPressedAppearance;
use widgets::WidgetsPlugin;
pub use widgets::slider_self_update;

/// Bevy plugin that adds diegetic layout, widgets, and IME behavior without
/// renderer initialization.
///
/// Use this plugin in client tests that need the same panel and widget
/// behavior as [`DiegeticUiPlugin`] without shaders, render assets, gizmos, or
/// a render sub-app. Insert a [`DiegeticTextMeasurer`] before adding this
/// plugin; deterministic tests may use `DiegeticTextMeasurer::default()`.
/// [`WidgetInputPlugin`] is not required for widget behavior. Add it when
/// `hana_diegetic` should translate Bevy input bindings into widget focus,
/// activation, and cancellation requests. Without it, an application or test
/// can send [`WidgetInput`] requests directly.
///
/// # Example
///
/// ```ignore
/// App::new()
///     .add_plugins(MinimalPlugins)
///     .insert_resource(DiegeticTextMeasurer::default())
///     .add_plugins(HeadlessDiegeticUiPlugin);
/// ```
pub struct HeadlessDiegeticUiPlugin;

impl Plugin for HeadlessDiegeticUiPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((HeadlessLayoutPlugin, WidgetsPlugin, ImePlugin));
    }
}

/// Bevy plugin that adds diegetic UI panel support.
///
/// Composes [`HeadlessDiegeticUiPlugin`] with rendering, text, gizmo, and
/// screen-space overlay support. Insert [`PanelDefaults`] before adding this
/// plugin; it takes effect through the child plugins at build time. A cascade
/// root resource such as [`TextAlpha`] can go on either side: inserted first it
/// is left alone, inserted afterwards it replaces the plugin's default and
/// propagates to every entity that inherits.
///
/// # Quick start
///
/// ```ignore
/// App::new().add_plugins(DiegeticUiPlugin)
/// ```
pub struct DiegeticUiPlugin;

impl Plugin for DiegeticUiPlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::load_internal_asset!(
            app,
            crate::constants::MATERIAL_TABLE_SHADER_HANDLE,
            "render/material_table.wgsl",
            bevy::shader::Shader::from_wgsl
        );
        bevy::asset::load_internal_asset!(
            app,
            crate::constants::SDF_MATERIAL_TABLE_SHADER_HANDLE,
            "shaders/sdf_material_table.wgsl",
            bevy::shader::Shader::from_wgsl
        );
        embedded_asset!(app, "shaders/sdf_panel.wgsl");
        embedded_asset!(app, "shaders/image_panel.wgsl");

        app.init_resource::<PanelDefaults>();
        app.configure_sets(
            Update,
            hana_kana::CascadeSet::Propagate.in_set(CascadeSet::Propagate),
        );
        app.add_plugins((
            TextPlugin,
            HeadlessDiegeticUiPlugin,
            PanelPlugin,
            ScreenSpacePlugin,
            RenderPlugin,
            DiegeticTextPlugin,
            #[cfg(feature = "typography_overlay")]
            TypographyOverlayPlugin,
        ));
    }
}
