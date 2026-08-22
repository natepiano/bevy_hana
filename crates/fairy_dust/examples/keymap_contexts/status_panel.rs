use bevy::ecs::change_detection::DetectChanges;
use bevy::log::error;
use bevy::prelude::Assets;
use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::Entity;
use bevy::prelude::Query;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::StandardMaterial;
use bevy::prelude::Transform;
use bevy::prelude::With;
use fairy_dust::StatsPanelRow;
use fairy_dust::StatsPanelSection;
use fairy_dust::diegetic_stats_sections_panel;
use fairy_dust::palette_binding_presentation;
use hana_rubric::ActiveKeymapContext;
use hana_rubric::ActiveKeymapContextState;
use hana_rubric::CommandId;
use hana_rubric::CommandRegistry;
use hana_rubric::EffectiveKeymapPublication;
use hana_rubric::EffectiveKeymapStatus;
use hana_rubric::KeymapBindings;
use hana_rubric::query_command_palette;

use super::ContextSceneEffect;
use super::ExampleInvocationSummary;
use super::commands;
use super::palette_control;

/// Marks the example-owned panel that presents Rubric's public routing state.
#[derive(Component)]
pub(super) struct ContextStatusPanel;

/// Spawns the single example-owned panel that reads the public Rubric snapshot.
pub(super) fn spawn_context_status_panel(
    active_context: Res<ActiveKeymapContext>,
    effective_keymap_status: Res<EffectiveKeymapStatus>,
    keymap_bindings: Res<KeymapBindings>,
    command_registry: Res<CommandRegistry>,
    scene_effect: Res<ContextSceneEffect>,
    invocation_summary: Res<ExampleInvocationSummary>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut commands: Commands,
) {
    let status_sections = context_status_sections(
        &active_context,
        &effective_keymap_status,
        &keymap_bindings,
        &command_registry,
        *scene_effect,
        &invocation_summary,
    );
    match diegetic_stats_sections_panel(&status_sections, &mut materials) {
        Ok(panel) => {
            commands.spawn((ContextStatusPanel, panel, Transform::default()));
        },
        Err(panel_build_error) => {
            error!(
                "keymap_contexts: failed to build the context status panel: {panel_build_error}"
            );
        },
    }
}

/// Refreshes the authoritative panel only when one of its published inputs changes.
pub(super) fn refresh_context_status_panel(
    panels: Query<Entity, With<ContextStatusPanel>>,
    active_context: Res<ActiveKeymapContext>,
    effective_keymap_status: Res<EffectiveKeymapStatus>,
    keymap_bindings: Res<KeymapBindings>,
    command_registry: Res<CommandRegistry>,
    scene_effect: Res<ContextSceneEffect>,
    invocation_summary: Res<ExampleInvocationSummary>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut commands: Commands,
) {
    if !active_context.is_changed()
        && !effective_keymap_status.is_changed()
        && !keymap_bindings.is_changed()
        && !scene_effect.is_changed()
        && !invocation_summary.is_changed()
    {
        return;
    }

    let status_sections = context_status_sections(
        &active_context,
        &effective_keymap_status,
        &keymap_bindings,
        &command_registry,
        *scene_effect,
        &invocation_summary,
    );
    match diegetic_stats_sections_panel(&status_sections, &mut materials) {
        Ok(panel) => {
            for context_status_panel in &panels {
                commands.entity(context_status_panel).despawn();
            }
            commands.spawn((ContextStatusPanel, panel, Transform::default()));
        },
        Err(panel_build_error) => {
            error!(
                "keymap_contexts: failed to refresh the context status panel: {panel_build_error}"
            );
        },
    }
}

/// Builds the panel rows from Rubric's published context, binding, and palette observations.
pub(super) fn context_status_sections(
    active_context: &ActiveKeymapContext,
    effective_keymap_status: &EffectiveKeymapStatus,
    keymap_bindings: &KeymapBindings,
    command_registry: &CommandRegistry,
    scene_effect: ContextSceneEffect,
    invocation_summary: &ExampleInvocationSummary,
) -> Vec<StatsPanelSection> {
    vec![
        StatsPanelSection::new("Active state", active_context_rows(active_context.state())),
        StatsPanelSection::new(
            "Effective keymap",
            effective_keymap_rows(effective_keymap_status),
        ),
        StatsPanelSection::new(
            "Palette agreement",
            [
                StatsPanelRow::new(
                    "global route",
                    canonical_palette_binding(
                        command_registry,
                        active_context,
                        effective_keymap_status,
                        keymap_bindings,
                    ),
                ),
                StatsPanelRow::new("recovery", format!("protected {}", palette_control())),
            ],
        ),
        StatsPanelSection::new(
            "Visible effect",
            [StatsPanelRow::new(
                "scene",
                context_scene_effect_label(scene_effect),
            )],
        ),
        StatsPanelSection::new(
            "Invocations",
            [StatsPanelRow::new(
                "summary",
                invocation_summary_label(invocation_summary),
            )],
        ),
        StatsPanelSection::new("Permanent controls", permanent_control_rows()),
    ]
}

/// Uses the context state Rubric has already observed instead of reading application state
/// directly.
fn active_context_rows(active_context: &ActiveKeymapContextState) -> Vec<StatsPanelRow> {
    match active_context {
        ActiveKeymapContextState::GlobalRouting => {
            vec![StatsPanelRow::new("routing", "global")]
        },
        ActiveKeymapContextState::AwaitingStateDimensions => {
            vec![StatsPanelRow::new("routing", "awaiting state dimensions")]
        },
        ActiveKeymapContextState::StateDimensionsUnavailable { missing } => vec![
            StatsPanelRow::new("routing", "state dimensions unavailable").details(
                missing
                    .iter()
                    .map(hana_rubric::ContextDimensionName::as_str),
            ),
        ],
        ActiveKeymapContextState::Resolved(context_snapshot) => context_snapshot
            .values()
            .map(|(dimension, value)| StatsPanelRow::new(dimension.as_str(), value.as_str()))
            .collect(),
    }
}

/// Renders publication provenance in the exact order Rubric matched document layers.
fn effective_keymap_rows(effective_keymap_status: &EffectiveKeymapStatus) -> Vec<StatsPanelRow> {
    match effective_keymap_status {
        EffectiveKeymapStatus::Loaded(publication) => loaded_keymap_rows(publication),
        EffectiveKeymapStatus::AwaitingAcceptedDocument => {
            vec![StatsPanelRow::new("status", "awaiting accepted document")]
        },
        EffectiveKeymapStatus::RejectedInitialDocument => {
            vec![StatsPanelRow::new("status", "initial document rejected")]
        },
        EffectiveKeymapStatus::AwaitingStateDimensions => {
            vec![StatsPanelRow::new("status", "awaiting state dimensions")]
        },
        EffectiveKeymapStatus::StateDimensionsUnavailable { missing } => vec![
            StatsPanelRow::new("status", "state dimensions unavailable").details(
                missing
                    .iter()
                    .map(hana_rubric::ContextDimensionName::as_str),
            ),
        ],
        EffectiveKeymapStatus::UnmaterializableStateDimensions => vec![StatsPanelRow::new(
            "status",
            "state dimensions cannot materialize an effective keymap",
        )],
    }
}

/// Renders one accepted materialization and its ordered matched-layer provenance.
fn loaded_keymap_rows(publication: &EffectiveKeymapPublication) -> Vec<StatsPanelRow> {
    let mut rows = vec![
        StatsPanelRow::new("status", "loaded"),
        StatsPanelRow::new("generation", publication.generation.to_string()),
    ];
    if publication.matched_layers.is_empty() {
        rows.push(StatsPanelRow::new("matched layers", "none"));
        return rows;
    }
    rows.extend(publication.matched_layers.iter().map(|matched_layer| {
        StatsPanelRow::new(
            "matched layer",
            format!("#{}", matched_layer.document_order),
        )
        .detail(predicate_label(&matched_layer.predicate.terms))
    }));
    rows
}

/// Formats a predicate identity that Rubric published with one matched layer.
fn predicate_label(
    terms: &[(
        hana_rubric::ContextDimensionName,
        hana_rubric::ContextValueName,
    )],
) -> String {
    terms
        .iter()
        .map(|(dimension, value)| format!("{}={}", dimension.as_str(), value.as_str()))
        .collect::<Vec<_>>()
        .join(" and ")
}

/// Queries Rubric's shared palette result for the global example command.
fn canonical_palette_binding(
    command_registry: &CommandRegistry,
    active_context: &ActiveKeymapContext,
    effective_keymap_status: &EffectiveKeymapStatus,
    keymap_bindings: &KeymapBindings,
) -> String {
    let command_id = CommandId::declared::<commands::ShowGlobalRoute>();
    let query_result = query_command_palette(
        command_registry,
        active_context,
        effective_keymap_status,
        keymap_bindings,
        command_id.as_str(),
    );
    query_result
        .rows()
        .iter()
        .find(|row| row.command().id() == &command_id)
        .map_or_else(
            || String::from("declared command unavailable to the palette"),
            |row| palette_binding_presentation(row.binding()).to_string(),
        )
}

/// Names the command result visible in the world without rebuilding routing state.
const fn context_scene_effect_label(context_scene_effect: ContextSceneEffect) -> &'static str {
    match context_scene_effect {
        ContextSceneEffect::Initial => "initial",
        ContextSceneEffect::GlobalRoute => "global route",
        ContextSceneEffect::MainMenuRoute => "main-menu route",
        ContextSceneEffect::RunningRoute => "running route",
        ContextSceneEffect::RestingRoute => "resting route",
        ContextSceneEffect::DimensionLockRoute => "dimension-lock route",
        ContextSceneEffect::CombinedRoute => "combined route",
        ContextSceneEffect::CombinedRouteBeforeOverride => "combined route before override",
        ContextSceneEffect::CombinedRouteAfterOverride => "combined route after override",
        ContextSceneEffect::TombstonedRoute => "tombstoned route",
    }
}

/// Renders the semantic invocation state without making an absent command an `Option`.
fn invocation_summary_label(invocation_summary: &ExampleInvocationSummary) -> String {
    match invocation_summary {
        ExampleInvocationSummary::NoInvocations => String::from("no invocations"),
        ExampleInvocationSummary::Invoked {
            last_command,
            count,
        } => format!("{count} invocation(s); last {last_command}"),
    }
}

/// Lists controls implemented through `SprinkleBuilder::with_shortcut`, outside authored routing.
fn permanent_control_rows() -> [StatsPanelRow; 5] {
    [
        StatsPanelRow::new("1", "main menu"),
        StatsPanelRow::new("2", "running"),
        StatsPanelRow::new("3", "resting"),
        StatsPanelRow::new("4", "dimension lock"),
        StatsPanelRow::new(palette_control(), "command palette recovery"),
    ]
}
