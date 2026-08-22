mod appearance;
mod button;
mod capture;
mod constants;
mod editable;
mod focus;
mod id;
mod input;
mod interactivity;
mod picking;
mod reify;
mod relationship;
mod slider;
mod tooltip;
mod visual;

pub use appearance::Appearance;
pub use appearance::BackgroundColor;
pub use appearance::BorderColor;
pub use appearance::IntoAppearance;
pub use appearance::PathColor;
pub(crate) use appearance::ResolvedWidgetStateAppearances;
pub(crate) use appearance::StateAppearance;
pub use appearance::TextColor;
pub use appearance::Tint;
pub use appearance::WidgetDisabledAppearance;
pub use appearance::WidgetFocusedAppearance;
pub use appearance::WidgetHoveredAppearance;
pub use appearance::WidgetPressedAppearance;
pub(crate) use appearance::WidgetState;
use bevy::ecs::schedule::ApplyDeferred;
use bevy::ecs::schedule::common_conditions::resource_exists;
use bevy::picking::PickingSettings;
use bevy::picking::PickingSystems;
use bevy::picking::backend::ray::RayMap;
use bevy::picking::events::PointerState;
use bevy::picking::hover::HoverMap;
use bevy::picking::mesh_picking::MeshPickingSettings;
use bevy::picking::pointer::PointerInput;
use bevy::prelude::App;
use bevy::prelude::Assets;
use bevy::prelude::IntoScheduleConfigs;
use bevy::prelude::Mesh;
use bevy::prelude::Messages;
use bevy::prelude::Plugin;
use bevy::prelude::PostUpdate;
use bevy::prelude::PreUpdate;
use bevy::prelude::SystemCondition;
use bevy::prelude::SystemSet;
use bevy::prelude::Update;
use bevy::window::WindowFocused;
pub use button::Button;
pub use button::ButtonCancelCause;
pub use button::ButtonCanceled;
pub(crate) use button::ButtonCaptures;
pub use button::ButtonClicked;
pub(crate) use button::ButtonPress;
pub use button::ButtonPressed;
pub use button::ButtonReleased;
pub(crate) use button::finalize_panel_buttons;
use capture::WidgetCaptures;
pub use focus::ClearWidgetFocus;
pub use focus::RequestPanelFocus;
pub use focus::RequestWidgetFocus;
pub(crate) use focus::WidgetFocusAuthority;
pub use focus::WidgetFocusChangeCause;
pub use focus::WidgetFocusChanged;
pub(crate) use focus::WidgetFocusVisible;
pub use focus::WidgetFocusable;
pub use focus::WidgetFocused;
pub(crate) use focus::finalize_panel_focus;
use hana_valence::AnchorSystems;
pub(crate) use id::ComputedWidgetRecord;
pub use id::PanelWidget;
pub(crate) use id::PanelWidgetIndex;
pub use id::PanelWidgetReader;
pub(crate) use id::VisualElementCapabilities;
pub(crate) use id::WidgetKind;
pub(crate) use id::WidgetSpec;
pub(crate) use id::validate_tree;
pub(crate) use input::SemanticWidgetIntent;
pub use input::WidgetControlSummary;
pub use input::WidgetInput;
pub use input::WidgetInputBindings;
pub use input::WidgetInputBindingsBuilder;
pub use input::WidgetInputBindingsError;
pub use input::WidgetInputDisabled;
pub use input::WidgetInputMode;
pub use input::WidgetInputPlugin;
pub use interactivity::PanelWidgetWriter;
pub use interactivity::WidgetDisabled;
pub use interactivity::WidgetInteractivity;
pub use picking::FacePicking;
pub use picking::PanelPicking;
pub(crate) use reify::WidgetAnchorRect;
pub(crate) use reify::on_screen_widget_demand_added;
pub(crate) use reify::on_screen_widget_demand_removed;
pub(crate) use reify::update_screen_anchor_geometry;
pub use relationship::PanelWidgets;
pub(crate) use relationship::ScreenWidgetAnchorProxy;
pub(crate) use relationship::ScreenWidgetAnchoredHere;
pub(crate) use relationship::ScreenWidgetAnchoredTo;
pub use relationship::TooltipFor;
pub use relationship::Tooltips;
pub use relationship::WidgetOf;
pub use slider::RequestSliderAdjustment;
pub use slider::Slider;
pub use slider::SliderAdjustment;
pub use slider::SliderCancelCause;
pub use slider::SliderCanceled;
pub(crate) use slider::SliderCaptures;
pub use slider::SliderChangeRequested;
pub use slider::SliderConfigError;
pub use slider::SliderDirection;
pub(crate) use slider::SliderDrag;
pub use slider::SliderGrabbed;
use slider::SliderPlugin;
pub use slider::SliderRange;
pub use slider::SliderReleased;
pub use slider::SliderResetBehavior;
pub use slider::SliderState;
pub use slider::SliderStep;
pub(crate) use slider::finalize_panel_sliders;
pub use slider::slider_self_update;
pub(crate) use tooltip::ComputedTooltipRecord;
pub(crate) use tooltip::MaterializedTooltip;
pub use tooltip::MeshAnchorCommandsExt;
pub(crate) use tooltip::MeshAnchorTarget;
pub(crate) use tooltip::MeshAnchorWarnings;
pub use tooltip::MeshFace;
pub use tooltip::ScreenAnchorCommandsExt;
pub use tooltip::Tooltip;
pub use tooltip::TooltipCommandsExt;
pub(crate) use tooltip::TooltipControllerIndex;
pub use tooltip::TooltipDisabledPolicy;
pub use tooltip::TooltipHidden;
pub use tooltip::TooltipPlacementPolicy;
use tooltip::TooltipPlugin;
pub use tooltip::TooltipShown;
pub use tooltip::TooltipTarget;
pub use tooltip::TooltipTargetEntity;
pub use tooltip::TooltipTargetSpace;
pub(crate) use tooltip::remove_materialized_state as remove_materialized_tooltip_state;
pub(crate) use tooltip::select_window_presentation_camera;
pub(crate) use visual::ComputedVisualSlot;
pub(crate) use visual::VisualOverrideIndex;
pub(crate) use visual::VisualSlotId;
pub(crate) use visual::VisualSlotOverride;
pub(crate) use visual::WidgetVisualOverrides;
pub(crate) use visual::WidgetVisualSlots;

use crate::PanelSystems;
use crate::cascade;
use crate::cascade::CascadeSet;
use crate::ime::ImeSystemSet;
use crate::screen_space::ScreenSpaceSystems;

/// Named scheduling points for semantic widget work.
#[derive(SystemSet, Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum WidgetSystems {
    /// Reifies widget identities from the latest computed panel output.
    Reify,
    /// Applies widget creation and synchronization commands.
    ReifyCommandsApplied,
    /// Synchronizes the final disabled marker from the resolved cascade value.
    ResolveInteractivity,
    /// Applies commands that synchronize the disabled marker.
    InteractivityCommandsApplied,
    /// Reconciles window-scoped focus after widget and window removals.
    Focus,
    /// Routes window-scoped semantic requests to focus or widget intents.
    SemanticInput,
    /// Applies pointer, app, and semantic-traversal focus marker commands.
    FocusCommandsApplied,
    /// Applies first-insertion commands from state presentation writers.
    PresentationCommandsApplied,
}

/// Named scheduling points for tooltip controller work.
#[derive(SystemSet, Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum TooltipSystems {
    /// Creates and synchronizes associated tooltip controllers.
    ReifyControllers,
    /// Applies controller creation and relationship commands.
    ControllerCommandsApplied,
    /// Resolves current hover/focus eligibility and advances visibility timers.
    Eligibility,
    /// Applies presentation-camera and preparation commands.
    EligibilityCommandsApplied,
    /// Materializes requested tooltip controllers as hidden panels.
    Materialize,
    /// Applies tooltip panel insertion and required-component commands.
    MaterializationCommandsApplied,
    /// Acquires current target handles and queues same-space attachments.
    Attach,
    /// Applies tooltip attachment and demand commands.
    AttachmentCommandsApplied,
    /// Publishes post-propagation tooltip readiness.
    Readiness,
    /// Reveals ready tooltip panels before retained batches are built.
    Reveal,
    /// Emits post-propagation tooltip visibility events.
    VisibilityEvents,
}

/// Installs headless panel widget identity and reification.
pub(crate) struct WidgetsPlugin;

fn add_widget_observers(app: &mut App) {
    app.add_observer(focus::request_panel_focus)
        .add_observer(focus::request_widget_focus)
        .add_observer(focus::clear_widget_focus)
        .add_observer(focus::focus_from_pointer_press)
        .add_observer(button::press_from_pointer)
        .add_observer(button::click_from_pointer)
        .add_observer(button::release_from_pointer)
        .add_observer(button::cancel_from_pointer)
        .add_observer(button::cancel_from_drag_end)
        .add_observer(button::cancel_from_pointer_removal)
        .add_observer(button::cancel_from_disabled)
        .add_observer(button::cancel_from_widget_removal)
        .add_observer(button::cancel_before_widget_despawn)
        .add_observer(button::handle_semantic_intent)
        .add_observer(button::dispatch_click_callback);
}

fn add_mesh_anchor_systems(app: &mut App) {
    app.add_systems(
        PostUpdate,
        (
            reify::update_world_anchor_geometry,
            tooltip::update_mesh_anchor_geometry,
            tooltip::warn_pending_mesh_anchor_geometry,
        )
            .chain()
            .in_set(AnchorSystems::FillGeometry),
    );
}

fn configure_widget_system_sets(app: &mut App) {
    app.configure_sets(
        Update,
        (
            WidgetSystems::Reify
                .after(PanelSystems::ComputeLayout)
                .after(PanelSystems::ResolveWorldFit)
                .after(ScreenSpaceSystems::ResolveDimensions),
            WidgetSystems::ReifyCommandsApplied.after(WidgetSystems::Reify),
            TooltipSystems::ReifyControllers.after(WidgetSystems::ReifyCommandsApplied),
            TooltipSystems::ControllerCommandsApplied
                .after(TooltipSystems::ReifyControllers)
                .before(PanelSystems::ResolvePanelAttachments),
            WidgetSystems::ResolveInteractivity
                .after(WidgetSystems::ReifyCommandsApplied)
                .before(WidgetSystems::InteractivityCommandsApplied),
            WidgetSystems::InteractivityCommandsApplied
                .after(WidgetSystems::ResolveInteractivity)
                .before(WidgetSystems::Focus),
            WidgetSystems::Focus
                .after(ImeSystemSet::PublishInputBlockers)
                .before(WidgetSystems::SemanticInput),
            WidgetSystems::SemanticInput
                .after(WidgetSystems::Focus)
                .before(PanelSystems::ResolvePanelAttachments),
            WidgetSystems::FocusCommandsApplied
                .after(WidgetSystems::SemanticInput)
                .before(ImeSystemSet::Input),
            WidgetSystems::PresentationCommandsApplied
                .after(WidgetSystems::FocusCommandsApplied)
                .after(WidgetSystems::ReifyCommandsApplied)
                .after(CascadeSet::Propagate),
        ),
    );
}

impl Plugin for WidgetsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<MeshPickingSettings>()
            .init_resource::<WidgetFocusAuthority>()
            .init_resource::<WidgetCaptures>()
            .init_resource::<ButtonCaptures>()
            .init_resource::<VisualOverrideIndex>()
            .init_resource::<MeshAnchorWarnings>()
            .add_message::<WindowFocused>()
            .add_message::<WidgetInput>()
            .add_plugins((
                cascade::cascade_plugin::<WidgetInteractivity>(),
                cascade::cascade_plugin::<WidgetHoveredAppearance>(),
                cascade::cascade_plugin::<WidgetPressedAppearance>(),
                cascade::cascade_plugin::<WidgetFocusedAppearance>(),
                cascade::cascade_plugin::<WidgetDisabledAppearance>(),
                SliderPlugin,
                TooltipPlugin,
            ));
        configure_widget_system_sets(app);
        app.add_systems(
            PreUpdate,
            picking::update_hits
                .run_if(resource_exists::<RayMap>.and_then(resource_exists::<Assets<Mesh>>))
                .in_set(PickingSystems::Backend),
        )
        .add_systems(
            PreUpdate,
            capture::reconcile_pointer_input
                .run_if(
                    resource_exists::<Messages<PointerInput>>
                        .and_then(resource_exists::<PointerState>)
                        .and_then(resource_exists::<HoverMap>)
                        .and_then(resource_exists::<PickingSettings>),
                )
                .in_set(PickingSystems::Last),
        )
        .add_systems(
            Update,
            (
                reify::reify_widgets.in_set(WidgetSystems::Reify),
                ApplyDeferred.in_set(WidgetSystems::ReifyCommandsApplied),
                reify::reify_tooltip_controllers.in_set(TooltipSystems::ReifyControllers),
                ApplyDeferred.in_set(TooltipSystems::ControllerCommandsApplied),
                interactivity::resolve_interactivity.in_set(WidgetSystems::ResolveInteractivity),
                ApplyDeferred.in_set(WidgetSystems::InteractivityCommandsApplied),
                focus::cleanup_removed_focus_participants.in_set(WidgetSystems::Focus),
                input::route_semantic_input.in_set(WidgetSystems::SemanticInput),
                ApplyDeferred.in_set(WidgetSystems::FocusCommandsApplied),
                button::present_button_state
                    .after(WidgetSystems::FocusCommandsApplied)
                    .after(WidgetSystems::ReifyCommandsApplied)
                    .after(CascadeSet::Propagate)
                    .before(WidgetSystems::PresentationCommandsApplied),
                editable::present_editable_state
                    .after(WidgetSystems::FocusCommandsApplied)
                    .after(WidgetSystems::ReifyCommandsApplied)
                    .after(CascadeSet::Propagate)
                    .before(WidgetSystems::PresentationCommandsApplied),
                slider::present_slider_state
                    .after(WidgetSystems::FocusCommandsApplied)
                    .after(WidgetSystems::ReifyCommandsApplied)
                    .after(CascadeSet::Propagate)
                    .before(WidgetSystems::PresentationCommandsApplied),
                ApplyDeferred.in_set(WidgetSystems::PresentationCommandsApplied),
                visual::dispatch_visual_overrides.after(WidgetSystems::PresentationCommandsApplied),
            ),
        );
        add_widget_observers(app);
        add_mesh_anchor_systems(app);
    }
}
