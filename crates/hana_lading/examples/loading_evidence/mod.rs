//! Shared reflected evidence and failure-panel support for the loading examples.
//!
//! Together the examples exhibit these protections:
//! 1. every handle returned by `DiskAssetLoader` is tracked;
//! 2. a set resolves only after recursive dependencies resolve (proved by the
//!    `recursive_dependencies_gate_and_fail` contract test because PNG has no recursive child);
//! 3. failures are reported instead of leaving startup waiting forever;
//! 4. `AssetSetLoadFailed` records failures without the set's concrete type;
//! 5. direct failure-record writes are visible at global completion; and
//! 6. whether startup remains blocked or degrades is left to application code.

use bevy::prelude::App;
use bevy::prelude::AppExtStates;
use bevy::prelude::Assets;
use bevy::prelude::Color;
use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::On;
use bevy::prelude::Plugin;
use bevy::prelude::Reflect;
use bevy::prelude::ReflectComponent;
use bevy::prelude::ReflectResource;
use bevy::prelude::ResMut;
use bevy::prelude::Resource;
use bevy::prelude::StandardMaterial;
use bevy::prelude::State;
use bevy::prelude::States;
use bevy::prelude::Transform;
use bevy::prelude::error;
use fairy_dust::LABEL_SIZE;
use fairy_dust::screen_panel_frame;
use fairy_dust::screen_panel_material_handle;
use hana_diegetic::Anchor;
use hana_diegetic::DiegeticPanel;
use hana_diegetic::Fit;
use hana_diegetic::Sizing;
use hana_diegetic::TextStyle;
use hana_lading::AssetSetLoadFailed;

pub(super) struct LoadingEvidencePlugin;

impl Plugin for LoadingEvidencePlugin {
    fn build(&self, app: &mut App) {
        app.init_state::<ExampleState>()
            .init_resource::<FailureRecord>()
            .register_type::<State<ExampleState>>()
            .add_observer(record_failure);
    }
}

/// Application-owned startup state used by both loading examples.
#[derive(States, Reflect, Default, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum ExampleState {
    /// Startup assets are still loading or application policy blocks readiness.
    #[default]
    Loading,
    /// Required startup content is usable.
    Ready,
}

/// Durable, reflected evidence copied from a generic loading failure event.
#[derive(Resource, Reflect, Default)]
#[reflect(Resource)]
pub(super) struct FailureRecord {
    pub(super) set_name:     String,
    pub(super) tracked_path: String,
    pub(super) error:        String,
}

/// Reflected copy of the exact text rendered by the failure panel.
#[derive(Component, Reflect)]
#[reflect(Component)]
pub(super) struct FailurePanelContent {
    pub(super) text: String,
}

/// Marks scene content whose required startup assets resolved successfully.
#[derive(Component, Reflect)]
#[reflect(Component)]
pub(super) struct RequiredSceneContent;

fn record_failure(event: On<AssetSetLoadFailed>, mut record: ResMut<FailureRecord>) {
    event.set_name().clone_into(&mut record.set_name);
    record.tracked_path = event.tracked_path().to_string();
    record.error = event.error().to_string();
}

pub(super) fn spawn_failure_panel(
    commands: &mut Commands,
    materials: &mut Assets<StandardMaterial>,
    record: &FailureRecord,
    display_name: &str,
    decision: &str,
    background: Color,
) {
    let text = format!(
        "{display_name}\nSET: {}\nPATH: {}\nERROR: {}\nDECISION: {decision}",
        record.set_name, record.tracked_path, record.error
    );
    let material = screen_panel_material_handle(materials);
    let built = DiegeticPanel::screen()
        .size(Fit, Fit)
        .anchor(Anchor::Center)
        .material(material.clone())
        .text_material(material)
        .layout(|builder| {
            screen_panel_frame(builder, Sizing::FIT, Sizing::FIT, background, |builder| {
                builder.text((text.as_str(), TextStyle::new(LABEL_SIZE)));
            });
        })
        .build();

    match built {
        Ok(panel) => {
            commands.spawn((FailurePanelContent { text }, panel, Transform::default()));
        },
        Err(error) => {
            error!("failed to build loading failure panel: {error}");
        },
    }
}
