//! Queued consumer changes reconciled at the service's first boundary.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;

use bevy_diagnostic::DiagnosticPath;
use bevy_diagnostic::DiagnosticsStore;
use bevy_ecs::prelude::Res;
use bevy_ecs::prelude::ResMut;
use bevy_ecs::prelude::Resource;

use super::ActiveMetricSource;
use super::CounterBaseline;
use super::RetentionCapacity;
use super::SourceKey;
use super::SubscriptionStatus;
use crate::RunOffset;
use crate::definition;
use crate::recording;
use crate::recording::CounterState;
use crate::recording::MetricRegistry;
use crate::recording::MetricSource;
use crate::recording::ObservationClock;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct SubscriptionId(u64);

#[derive(Default, Resource)]
pub(crate) struct DemandLedger {
    pub(super) shared: Arc<DemandChanges>,
    subscriptions:     HashMap<SubscriptionId, ActiveSubscription>,
}

#[derive(Default)]
pub(super) struct DemandChanges {
    changed: AtomicBool,
    next_id: AtomicU64,
    queue:   Mutex<Vec<DemandChange>>,
}

pub(super) struct ActiveSubscription {
    pub(super) paths:      Vec<(DiagnosticPath, RetentionCapacity)>,
    pub(super) status:     Arc<Mutex<SubscriptionStatus>>,
    pub(super) boundaries: super::SubscriptionBoundaries,
}

enum DemandChange {
    Acquire {
        id:           SubscriptionId,
        subscription: ActiveSubscription,
    },
    Release(SubscriptionId),
}

impl DemandChanges {
    pub(super) fn acquire(&self, subscription: ActiveSubscription) -> SubscriptionId {
        let id = SubscriptionId(self.next_id.fetch_add(1, Ordering::Relaxed));
        recording::lock(&self.queue).push(DemandChange::Acquire { id, subscription });
        self.changed.store(true, Ordering::Release);
        id
    }

    pub(super) fn release(&self, id: SubscriptionId) {
        recording::lock(&self.queue).push(DemandChange::Release(id));
        self.changed.store(true, Ordering::Release);
    }
}

pub(crate) fn reconcile(
    mut ledger: ResMut<DemandLedger>,
    mut registry: ResMut<MetricRegistry>,
    mut store: ResMut<DiagnosticsStore>,
    clock: Res<ObservationClock>,
) {
    if !ledger.shared.changed.load(Ordering::Acquire) {
        return;
    }
    let changes = {
        let mut queue = recording::lock(&ledger.shared.queue);
        ledger.shared.changed.store(false, Ordering::Release);
        std::mem::take(&mut *queue)
    };
    let now = clock.now();
    let (mut touched, joining) = apply_changes(&mut ledger, &mut registry, &store, changes);
    touched.sort_by(|left, right| left.as_str().cmp(right.as_str()));
    touched.dedup();
    reconcile_sources(&mut registry, &mut store, &clock, now, touched);
    establish_boundaries(&ledger, &registry, now, joining);
    publish_statuses(&ledger, &registry);
}

fn apply_changes(
    ledger: &mut DemandLedger,
    registry: &mut MetricRegistry,
    store: &DiagnosticsStore,
    changes: Vec<DemandChange>,
) -> (Vec<DiagnosticPath>, Vec<SubscriptionId>) {
    let mut touched = Vec::new();
    let mut joining = Vec::new();
    for change in changes {
        match change {
            DemandChange::Acquire { id, subscription } => {
                for (path, capacity) in &subscription.paths {
                    if !registry.entries.contains_key(path)
                        && store.get(path).is_some()
                        && definition::validate_native_path(path.as_str()).is_ok()
                    {
                        registry.entries.insert(
                            path.clone(),
                            crate::recording::RegisteredMetric::discovered(
                                crate::MetricDescriptor::discovered(path.clone()),
                            ),
                        );
                    }
                    if let Some(entry) = registry.entries.get_mut(path) {
                        entry.demands.insert(id, *capacity);
                        touched.push(path.clone());
                    }
                }
                joining.push(id);
                ledger.subscriptions.insert(id, subscription);
            },
            DemandChange::Release(id) => {
                if let Some(subscription) = ledger.subscriptions.remove(&id) {
                    for (path, _) in subscription.paths {
                        if let Some(entry) = registry.entries.get_mut(&path) {
                            entry.demands.remove(&id);
                        }
                        touched.push(path);
                    }
                }
            },
        }
    }
    (touched, joining)
}

fn reconcile_sources(
    registry: &mut MetricRegistry,
    store: &mut DiagnosticsStore,
    clock: &ObservationClock,
    now: RunOffset,
    touched: Vec<DiagnosticPath>,
) {
    for path in touched {
        let Some(entry) = registry.entries.get_mut(&path) else {
            continue;
        };
        let was_active = entry.is_active();
        let active = !entry.demands.is_empty();
        if active {
            let capacity = entry
                .demands
                .values()
                .copied()
                .max()
                .unwrap_or_else(RetentionCapacity::latest);
            if was_active {
                recording::drain_metric(entry, store, clock);
                entry.resize_retention(capacity);
            } else {
                entry.activate(now, capacity);
            }
        } else if was_active {
            entry.release(store, clock);
        }
        if matches!(&entry.source, MetricSource::Owned(_))
            && let Some(diagnostic) = store.get_mut(&path)
        {
            diagnostic.is_enabled = active;
        }
        if active && !was_active {
            registry.active.push(path);
        } else if !active && was_active {
            registry.active.retain(|active| *active != path);
        }
    }
}

fn establish_boundaries(
    ledger: &DemandLedger,
    registry: &MetricRegistry,
    now: RunOffset,
    joining: Vec<SubscriptionId>,
) {
    for id in joining {
        let Some(subscription) = ledger.subscriptions.get(&id) else {
            continue;
        };
        let mut boundaries = recording::lock(&subscription.boundaries);
        for (path, _) in &subscription.paths {
            let Some(entry) = registry.entries.get(path) else {
                continue;
            };
            let baseline = match &entry.source {
                MetricSource::Owned(owned) => match owned.counter {
                    CounterState::Increments(total) => CounterBaseline::Measured {
                        total,
                        at: now,
                        sequence: owned.sequence,
                    },
                    CounterState::External {
                        total,
                        at,
                        sequence,
                        ..
                    } => CounterBaseline::Measured {
                        total,
                        at,
                        sequence,
                    },
                    _ => CounterBaseline::AwaitingSnapshot,
                },
                _ => CounterBaseline::AwaitingSnapshot,
            };
            boundaries.insert(
                path.clone(),
                super::SubscriptionBoundary {
                    epoch: entry.epoch,
                    effective_at: now,
                    baseline,
                    activation: Arc::clone(&entry.activation),
                },
            );
        }
    }
}

fn publish_statuses(ledger: &DemandLedger, registry: &MetricRegistry) {
    for subscription in ledger.subscriptions.values() {
        let sources = subscription
            .paths
            .iter()
            .filter_map(|(path, _)| {
                let entry = registry.entries.get(path)?;
                Some(ActiveMetricSource {
                    source:       SourceKey(path.as_str().to_owned()),
                    epoch:        entry.epoch,
                    effective_at: recording::lock(&subscription.boundaries)
                        .get(path)
                        .map_or(entry.effective_at, |boundary| boundary.effective_at),
                })
            })
            .collect();
        *recording::lock(&subscription.status) = SubscriptionStatus::Active { sources };
    }
}
