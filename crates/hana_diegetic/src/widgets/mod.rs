mod anchor;
mod constants;
mod controls;
mod identity;
mod interaction;
mod presentation;
mod reify;
mod tooltip;
pub(crate) use anchor::ScreenWidgetAnchorProxy;
pub(crate) use anchor::ScreenWidgetAnchoredHere;
pub(crate) use anchor::ScreenWidgetAnchoredTo;
pub(crate) use anchor::TooltipBoundary;
pub(crate) use anchor::WidgetAnchorRect;
pub(crate) use anchor::on_screen_widget_demand_added;
pub(crate) use anchor::on_screen_widget_demand_removed;
pub(crate) use anchor::update_screen_anchor_geometry;
use bevy::ecs::schedule::ApplyDeferred;
use bevy::ecs::schedule::common_conditions::resource_exists;
use bevy::input::InputSystems;
use bevy::picking::PickingSettings;
use bevy::picking::PickingSystems;
use bevy::picking::backend::ray::RayMap;
use bevy::picking::events::PointerState;
use bevy::picking::hover::HoverMap;
use bevy::picking::mesh_picking::MeshPickingSettings;
use bevy::picking::pointer::PointerInput;
use bevy::prelude::App;
use bevy::prelude::Assets;
use bevy::prelude::Component;
use bevy::prelude::IntoScheduleConfigs;
use bevy::prelude::Mesh;
use bevy::prelude::Messages;
use bevy::prelude::Plugin;
use bevy::prelude::PostUpdate;
use bevy::prelude::PreUpdate;
use bevy::prelude::Reflect;
use bevy::prelude::ReflectComponent;
use bevy::prelude::SystemCondition;
use bevy::prelude::SystemSet;
use bevy::prelude::Update;
use bevy::window::WindowFocused;
pub use controls::Button;
pub use controls::ButtonActivationSource;
pub use controls::ButtonCancelCause;
pub use controls::ButtonCanceled;
pub(crate) use controls::ButtonCaptures;
pub use controls::ButtonClicked;
pub use controls::ButtonLatch;
pub use controls::ButtonMetrics;
pub use controls::ButtonMode;
pub(crate) use controls::ButtonPress;
pub use controls::ButtonPressSource;
pub use controls::ButtonPressed;
pub use controls::ButtonReleased;
pub use controls::CapsuleThumb;
pub use controls::ControlLength;
pub use controls::ControlMetrics;
pub use controls::ControlPalette;
pub use controls::FieldMetrics;
pub use controls::MenuMetrics;
pub use controls::PopUpMetrics;
pub use controls::RequestSliderAdjustment;
pub use controls::RowMetrics;
pub use controls::Slider;
pub use controls::SliderAdjustment;
pub use controls::SliderAuthoringLock;
pub use controls::SliderCancelCause;
pub use controls::SliderCanceled;
pub(crate) use controls::SliderCaptures;
pub use controls::SliderChangeRequested;
pub use controls::SliderConfigError;
pub use controls::SliderDirection;
pub(crate) use controls::SliderDrag;
pub use controls::SliderGrabbed;
pub use controls::SliderMetrics;
use controls::SliderPlugin;
pub use controls::SliderProposalSource;
pub use controls::SliderProposalValue;
pub use controls::SliderPushedValue;
pub use controls::SliderRange;
pub use controls::SliderReachableBand;
pub use controls::SliderReleased;
pub use controls::SliderState;
pub use controls::SliderStep;
pub use controls::SliderStepping;
pub use controls::SpacingMetrics;
pub use controls::Switch;
pub use controls::SwitchMetrics;
use controls::SwitchPlugin;
pub use controls::SwitchSide;
pub use controls::SwitchState;
pub use controls::SwitchToggleRequested;
pub(crate) use controls::finalize_panel_buttons;
pub(crate) use controls::finalize_panel_sliders;
pub use controls::release_button;
pub use controls::slider_self_update;
use hana_valence::AnchorSystems;
pub(crate) use identity::ComputedWidgetRecord;
pub use identity::PanelWidget;
pub(crate) use identity::PanelWidgetIndex;
pub use identity::PanelWidgetReader;
pub use identity::PanelWidgets;
pub(crate) use identity::VisualElementCapabilities;
pub(crate) use identity::WidgetHitArea;
pub use identity::WidgetKind;
pub use identity::WidgetOf;
pub(crate) use identity::WidgetSpec;
pub(crate) use identity::validate_tree;
pub use interaction::ClearWidgetFocus;
pub use interaction::FacePicking;
pub use interaction::HoveredScrollTargets;
pub use interaction::PanelPicking;
pub use interaction::PanelScrolled;
pub use interaction::PanelWidgetWriter;
pub use interaction::PointerScrollTarget;
pub use interaction::RequestPanelFocus;
pub use interaction::RequestWidgetFocus;
pub(crate) use interaction::SemanticWidgetIntent;
pub use interaction::SizeGateExempt;
use interaction::WidgetCaptures;
pub use interaction::WidgetControlSummary;
pub use interaction::WidgetDisabled;
pub use interaction::WidgetPrimaryPressOwnership;
pub use interaction::WidgetScreenSizeGate;
#[cfg(test)]
pub(crate) use tooltip::TooltipPointerCamera;
/// Dims a control's linked caption while leaving its appearance and interaction enabled.
///
/// Add this marker to a widget entity. Focus, input routing, and widget events
/// continue to use its resolved interactivity.
#[derive(Clone, Copy, Component, Debug, Default, Eq, PartialEq, Reflect)]
#[reflect(Component)]
pub struct WidgetDimmed;

pub(crate) use constants::TOOLTIP_SHADOW_EXTENT;
pub(crate) use interaction::WidgetFocusAuthority;
pub use interaction::WidgetFocusChangeCause;
pub use interaction::WidgetFocusChanged;
pub use interaction::WidgetFocusTarget;
pub(crate) use interaction::WidgetFocusVisible;
pub use interaction::WidgetFocusable;
pub use interaction::WidgetFocused;
pub use interaction::WidgetInput;
pub use interaction::WidgetInputBindings;
pub use interaction::WidgetInputBindingsBuilder;
pub use interaction::WidgetInputBindingsError;
pub use interaction::WidgetInputDisabled;
pub use interaction::WidgetInputMode;
pub use interaction::WidgetInputPlugin;
pub use interaction::WidgetInteractivity;
use interaction::WidgetPrimaryPresses;
pub use interaction::WidgetStepDirection;
pub(crate) use interaction::finalize_panel_focus;
pub use presentation::Appearance;
pub use presentation::BackgroundColor;
pub use presentation::BorderColor;
pub(crate) use presentation::ComputedVisualSlot;
pub use presentation::IntoAppearance;
pub use presentation::PathColor;
pub(crate) use presentation::ResolvedWidgetStateAppearances;
pub(crate) use presentation::StateAppearance;
pub use presentation::TextColor;
pub use presentation::Tint;
pub(crate) use presentation::VisualOverrideIndex;
pub(crate) use presentation::VisualSlotId;
pub(crate) use presentation::VisualSlotOverride;
pub use presentation::WidgetDisabledAppearance;
pub use presentation::WidgetFocusedAppearance;
pub use presentation::WidgetHoveredAppearance;
pub use presentation::WidgetPressedAppearance;
pub(crate) use presentation::WidgetState;
pub(crate) use presentation::WidgetVisualOverrides;
pub(crate) use presentation::WidgetVisualSlots;
pub(crate) use tooltip::ComputedTooltipRecord;
pub(crate) use tooltip::MaterializedTooltip;
pub use tooltip::MeshAnchorCommandsExt;
pub(crate) use tooltip::MeshAnchorWarnings;
pub use tooltip::MeshFace;
pub use tooltip::ScreenAnchorCommandsExt;
pub use tooltip::Tooltip;
pub use tooltip::TooltipCardOccupiedRegions;
pub use tooltip::TooltipCommandsExt;
pub(crate) use tooltip::TooltipControllerIndex;
pub use tooltip::TooltipDisabledPolicy;
pub use tooltip::TooltipFor;
pub use tooltip::TooltipHidden;
pub(crate) use tooltip::TooltipPaintInsideBorder;
use tooltip::TooltipPlugin;
pub use tooltip::TooltipShown;
pub use tooltip::TooltipTarget;
pub use tooltip::TooltipTargetEntity;
pub use tooltip::TooltipTargetSpace;
pub use tooltip::Tooltips;
pub(crate) use tooltip::remove_materialized_state as remove_materialized_tooltip_state;
pub(crate) use tooltip::select_window_presentation_camera;

use crate::PanelSystems;
use crate::cascade;
use crate::cascade::CascadeSet;
use crate::ime::ImeSystemSet;
use crate::panel;
use crate::screen_space::ScreenSpaceSystems;

/// Named scheduling points for semantic widget work.
#[derive(SystemSet, Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum WidgetSystems {
    /// Reifies widget identities from the latest computed panel output.
    Reify,
    /// Refreshes edited field parts after post-input layout.
    ReifyEdited,
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
    app.add_observer(interaction::request_panel_focus)
        .add_observer(interaction::request_widget_focus)
        .add_observer(interaction::clear_widget_focus)
        .add_observer(interaction::focus_from_pointer_press)
        .add_observer(controls::press_from_pointer)
        .add_observer(controls::pointer_over_button)
        .add_observer(controls::pointer_out_of_button)
        .add_observer(controls::click_from_pointer)
        .add_observer(controls::release_from_pointer)
        .add_observer(controls::cancel_from_pointer)
        .add_observer(controls::cancel_from_drag_end)
        .add_observer(controls::cancel_from_pointer_removal)
        .add_observer(controls::cancel_from_disabled)
        .add_observer(controls::cancel_from_focus_loss)
        .add_observer(controls::cancel_from_widget_removal)
        .add_observer(controls::cancel_before_widget_despawn)
        .add_observer(controls::handle_semantic_intent)
        .add_observer(controls::dispatch_click_callback)
        .add_observer(interaction::scroll_from_pointer)
        .add_observer(interaction::record_widget_primary_press);
}

fn add_mesh_anchor_systems(app: &mut App) {
    app.add_systems(
        PostUpdate,
        (
            anchor::update_world_anchor_geometry,
            tooltip::remove_mesh_anchor_geometry,
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
            WidgetSystems::ReifyEdited
                .after(PanelSystems::ComputeEditedLayout)
                .before(ImeSystemSet::UpdateEditorGeometry)
                .run_if(panel::late_editor_layout_needed),
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
        app.init_resource::<WidgetScreenSizeGate>()
            .init_resource::<MeshPickingSettings>()
            .init_resource::<WidgetFocusAuthority>()
            .init_resource::<WidgetCaptures>()
            .init_resource::<WidgetPrimaryPresses>()
            .init_resource::<WidgetPrimaryPressOwnership>()
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
                SwitchPlugin,
                TooltipPlugin,
            ));
        configure_widget_system_sets(app);
        app.add_systems(
            PreUpdate,
            interaction::update_hits
                .run_if(resource_exists::<RayMap>.and_then(resource_exists::<Assets<Mesh>>))
                .in_set(PickingSystems::Backend),
        )
        .add_systems(
            PreUpdate,
            interaction::reconcile_pointer_input
                .run_if(
                    resource_exists::<Messages<PointerInput>>
                        .and_then(resource_exists::<PointerState>)
                        .and_then(resource_exists::<HoverMap>)
                        .and_then(resource_exists::<PickingSettings>),
                )
                .in_set(PickingSystems::Last),
        )
        .add_systems(
            PreUpdate,
            interaction::release_widget_primary_presses
                .after(InputSystems)
                .before(PickingSystems::Hover),
        )
        .add_systems(
            Update,
            (
                reify::reify_widgets.in_set(WidgetSystems::Reify),
                (
                    reify::reify_widgets,
                    ApplyDeferred,
                    controls::present_editable_state,
                    ApplyDeferred,
                    presentation::dispatch_visual_overrides,
                )
                    .chain()
                    .in_set(WidgetSystems::ReifyEdited),
                ApplyDeferred.in_set(WidgetSystems::ReifyCommandsApplied),
                tooltip::reify_tooltip_controllers.in_set(TooltipSystems::ReifyControllers),
                ApplyDeferred.in_set(TooltipSystems::ControllerCommandsApplied),
                interaction::resolve_interactivity.in_set(WidgetSystems::ResolveInteractivity),
                ApplyDeferred.in_set(WidgetSystems::InteractivityCommandsApplied),
                interaction::cleanup_removed_focus_participants.in_set(WidgetSystems::Focus),
                interaction::route_semantic_input.in_set(WidgetSystems::SemanticInput),
                ApplyDeferred.in_set(WidgetSystems::FocusCommandsApplied),
                controls::present_button_state
                    .after(WidgetSystems::FocusCommandsApplied)
                    .after(WidgetSystems::ReifyCommandsApplied)
                    .after(CascadeSet::Propagate)
                    .before(WidgetSystems::PresentationCommandsApplied),
                controls::present_editable_state
                    .after(WidgetSystems::FocusCommandsApplied)
                    .after(WidgetSystems::ReifyCommandsApplied)
                    .after(CascadeSet::Propagate)
                    .before(WidgetSystems::PresentationCommandsApplied),
                controls::present_slider_state
                    .after(WidgetSystems::FocusCommandsApplied)
                    .after(WidgetSystems::ReifyCommandsApplied)
                    .after(CascadeSet::Propagate)
                    .before(WidgetSystems::PresentationCommandsApplied),
                ApplyDeferred.in_set(WidgetSystems::PresentationCommandsApplied),
                presentation::dispatch_visual_overrides
                    .after(WidgetSystems::PresentationCommandsApplied),
            ),
        );
        add_widget_observers(app);
        add_mesh_anchor_systems(app);
    }
}
