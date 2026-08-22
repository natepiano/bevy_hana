//! Candidate validation for screen-space panel attachments.

use bevy::platform::collections::HashMap;
use bevy::prelude::Entity;
use bevy::prelude::Query;
use bevy::prelude::Reflect;
use bevy::prelude::Vec2;
use bevy::window::WindowRef;
use hana_valence::AnchorSite;
use hana_valence::AnchoredTo as ValenceAnchoredTo;
use hana_valence::AttachmentResolveCandidate;

use super::window;
use super::window::WindowResolveFailure;
use crate::panel::CoordinateSpace;
use crate::panel::DiegeticPanel;
use crate::panel::PanelAnchorOffset;
use crate::panel::PanelAttachmentAuthored;
use crate::panel::PanelSpace;
use crate::panel::WidgetOwnerLayout;
use crate::screen_space::CandidateQueries;
use crate::screen_space::ScreenAnchorTarget;
use crate::widgets::ScreenWidgetAnchorProxy;
use crate::widgets::ScreenWidgetAnchoredHere;
use crate::widgets::WidgetAnchorRect;
use crate::widgets::WidgetOf;

type WidgetTargetState<'a> = (
    Option<&'a WidgetOf>,
    Option<&'a WidgetAnchorRect>,
    Option<&'a ScreenWidgetAnchoredHere>,
    Option<&'a ScreenWidgetAnchorProxy>,
);

pub(super) fn classify_candidates(
    attachments: &Query<(Entity, &PanelAttachmentAuthored, &PanelAnchorOffset)>,
    queries: &CandidateQueries<'_, '_>,
    window_sizes: &HashMap<Entity, Vec2>,
    candidates: &mut Vec<AttachmentResolveCandidate<ScreenAttachmentResolveSkip>>,
) {
    for (source, attachment, _) in attachments.iter() {
        if let Some(candidate) = classify_candidate(source, *attachment, queries, window_sizes) {
            candidates.push(candidate);
        }
    }
}

fn classify_candidate(
    source: Entity,
    attachment: PanelAttachmentAuthored,
    queries: &CandidateQueries<'_, '_>,
    window_sizes: &HashMap<Entity, Vec2>,
) -> Option<AttachmentResolveCandidate<ScreenAttachmentResolveSkip>> {
    let target = attachment.target();
    let source_panel = queries.panels.get(source).ok().map(|(_, panel)| panel);
    if source_panel
        .is_some_and(|panel| matches!(panel.coordinate_space(), CoordinateSpace::World { .. }))
    {
        return None;
    }
    Some(
        match validate_candidate(source, attachment, queries, window_sizes) {
            Ok(()) => AttachmentResolveCandidate::Active {
                source,
                target,
                attachment: attachment.valence_relation(),
            },
            Err(reason) => AttachmentResolveCandidate::Skipped {
                source,
                target,
                reason,
            },
        },
    )
}

fn validate_candidate(
    source: Entity,
    attachment: PanelAttachmentAuthored,
    queries: &CandidateQueries<'_, '_>,
    window_sizes: &HashMap<Entity, Vec2>,
) -> Result<(), ScreenAttachmentResolveSkip> {
    let target = attachment.target();
    let Ok((_, source_panel)) = queries.panels.get(source) else {
        return Err(ScreenAttachmentResolveSkip::SourceWithoutPanel);
    };
    if source == target {
        return Err(ScreenAttachmentResolveSkip::SelfAttachment);
    }
    if !queries.entities.contains(target) {
        return Err(ScreenAttachmentResolveSkip::TargetMissing);
    }
    if queries.transforms.get(source).is_err() {
        return Err(ScreenAttachmentResolveSkip::SourceTransformMissing);
    }
    let CoordinateSpace::Screen {
        window: source_window,
        ..
    } = source_panel.coordinate_space()
    else {
        return Err(ScreenAttachmentResolveSkip::MixedCoordinateSpace);
    };
    let source_window = resolve_source_window(*source_window, queries, window_sizes)?;

    match (
        queries.panels.get(target),
        queries.widgets.get(target),
        queries.screen_targets.get(target),
    ) {
        (Ok((_, target_panel)), _, _) => {
            validate_panel_target(target_panel, target, source_window, queries, window_sizes)
        },
        (Err(_), Ok((widget_of, anchor_rect, demand, proxy)), _) => validate_widget_target(
            source,
            target,
            (widget_of, anchor_rect, demand, proxy),
            source_window,
            queries,
            window_sizes,
        ),
        (Err(_), Err(_), Ok((_, screen_target))) => {
            validate_screen_target(screen_target, source_window, window_sizes)
        },
        (Err(_), Err(_), Err(_)) => Err(ScreenAttachmentResolveSkip::TargetWithoutPanel),
    }
}

fn validate_screen_target(
    target: &ScreenAnchorTarget,
    source_window: Entity,
    window_sizes: &HashMap<Entity, Vec2>,
) -> Result<(), ScreenAttachmentResolveSkip> {
    let Some(size) = window_sizes.get(&target.window()) else {
        return Err(ScreenAttachmentResolveSkip::TargetWindowMissing);
    };
    if size.x <= 0.0 || size.y <= 0.0 {
        return Err(ScreenAttachmentResolveSkip::TargetWindowZeroSized);
    }
    if source_window != target.window() {
        return Err(ScreenAttachmentResolveSkip::CrossWindow);
    }
    Ok(())
}

fn validate_panel_target(
    target_panel: &DiegeticPanel,
    target: Entity,
    source_window: Entity,
    queries: &CandidateQueries<'_, '_>,
    window_sizes: &HashMap<Entity, Vec2>,
) -> Result<(), ScreenAttachmentResolveSkip> {
    let CoordinateSpace::Screen {
        window: target_window,
        ..
    } = target_panel.coordinate_space()
    else {
        return Err(ScreenAttachmentResolveSkip::MixedCoordinateSpace);
    };
    if queries.transforms.get(target).is_err() {
        return Err(ScreenAttachmentResolveSkip::TargetTransformMissing);
    }
    let target_window = resolve_target_window(*target_window, queries, window_sizes)?;
    if source_window != target_window {
        return Err(ScreenAttachmentResolveSkip::CrossWindow);
    }
    Ok(())
}

fn validate_widget_target(
    source: Entity,
    target: Entity,
    target_state: WidgetTargetState<'_>,
    source_window: Entity,
    queries: &CandidateQueries<'_, '_>,
    window_sizes: &HashMap<Entity, Vec2>,
) -> Result<(), ScreenAttachmentResolveSkip> {
    let (widget_of, anchor_rect, demand, proxy) = target_state;
    let Some(widget_of) = widget_of else {
        return Err(ScreenAttachmentResolveSkip::TargetOwnerMissing);
    };
    let Ok((_, owner_panel)) = queries.panels.get(widget_of.panel()) else {
        return Err(ScreenAttachmentResolveSkip::TargetOwnerMissing);
    };
    let owner_layout = WidgetOwnerLayout::from(owner_panel);
    if owner_layout.panel_space() != PanelSpace::Screen {
        return Err(ScreenAttachmentResolveSkip::MixedCoordinateSpace);
    }
    if anchor_rect.is_none()
        || queries.geometry.get(target).is_err()
        || proxy.is_none()
        || demand.is_none_or(|demand| !demand.contains(&source))
    {
        return Err(ScreenAttachmentResolveSkip::TargetGeometryMissing);
    }
    if queries.transforms.get(widget_of.panel()).is_err() {
        return Err(ScreenAttachmentResolveSkip::TargetTransformMissing);
    }
    let CoordinateSpace::Screen {
        window: target_window,
        ..
    } = owner_panel.coordinate_space()
    else {
        return Err(ScreenAttachmentResolveSkip::MixedCoordinateSpace);
    };
    let target_window = resolve_target_window(*target_window, queries, window_sizes)?;
    if source_window != target_window {
        return Err(ScreenAttachmentResolveSkip::CrossWindow);
    }
    Ok(())
}

pub(super) fn proxy_candidates(
    queries: &CandidateQueries<'_, '_>,
    candidates: &mut Vec<AttachmentResolveCandidate<ScreenAttachmentResolveSkip>>,
) {
    for (widget, widget_of, anchor_rect, demand) in &queries.proxy_candidates {
        let owner = widget_of.panel();
        let Ok((_, owner_panel)) = queries.panels.get(owner) else {
            continue;
        };
        let owner_layout = WidgetOwnerLayout::from(owner_panel);
        if demand.is_empty()
            || anchor_rect.space() != PanelSpace::Screen
            || owner_layout.panel_space() != PanelSpace::Screen
            || queries.transforms.get(owner).is_err()
        {
            continue;
        }
        candidates.push(AttachmentResolveCandidate::Active {
            source:     widget,
            target:     owner,
            attachment: ValenceAnchoredTo::new(owner, AnchorSite::Center, AnchorSite::Center),
        });
    }
}

fn resolve_source_window(
    window_ref: WindowRef,
    queries: &CandidateQueries<'_, '_>,
    window_sizes: &HashMap<Entity, Vec2>,
) -> Result<Entity, ScreenAttachmentResolveSkip> {
    window::resolve_window(window_ref, &queries.primary, window_sizes)
        .map(|(entity, _)| entity)
        .map_err(|failure| match failure {
            WindowResolveFailure::Missing => ScreenAttachmentResolveSkip::SourceWindowMissing,
            WindowResolveFailure::ZeroSized => ScreenAttachmentResolveSkip::SourceWindowZeroSized,
        })
}

fn resolve_target_window(
    window_ref: WindowRef,
    queries: &CandidateQueries<'_, '_>,
    window_sizes: &HashMap<Entity, Vec2>,
) -> Result<Entity, ScreenAttachmentResolveSkip> {
    window::resolve_window(window_ref, &queries.primary, window_sizes)
        .map(|(entity, _)| entity)
        .map_err(|failure| match failure {
            WindowResolveFailure::Missing => ScreenAttachmentResolveSkip::TargetWindowMissing,
            WindowResolveFailure::ZeroSized => ScreenAttachmentResolveSkip::TargetWindowZeroSized,
        })
}

/// Why a screen-space attachment did not resolve in the current frame.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Reflect)]
pub(crate) enum ScreenAttachmentResolveSkip {
    SourceWithoutPanel,
    SourceGeometryMissing,
    SourceTransformMissing,
    TargetMissing,
    TargetWithoutPanel,
    TargetOwnerMissing,
    TargetGeometryMissing,
    TargetTransformMissing,
    SelfAttachment,
    SourceWindowMissing,
    SourceWindowZeroSized,
    TargetWindowMissing,
    TargetWindowZeroSized,
    CrossWindow,
    MixedCoordinateSpace,
    Cycle,
    BlockedByCycle,
    BlockedBySkippedDependency,
}
