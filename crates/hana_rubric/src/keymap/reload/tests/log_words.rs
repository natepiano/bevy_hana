//! Real shortcut reload reports separate operator words from source diagnostics.

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::PoisonError;

use bevy::log::tracing;
use bevy::log::tracing::Event;
use bevy::log::tracing::Level;
use bevy::log::tracing::field::Field;
use bevy::log::tracing::field::Visit;
use bevy::log::tracing_subscriber::Layer;
use bevy::log::tracing_subscriber::Registry;
use bevy::log::tracing_subscriber::layer::Context;
use bevy::log::tracing_subscriber::layer::SubscriberExt;

use super::DEFAULTS_PATH;
use super::USER_KEYMAP_FIXTURE;
use super::commit_defaults;
use super::commit_reload;
use super::queue_user_snapshot;
use super::transaction_app;

#[derive(Clone, Default)]
struct ShortcutReports(Arc<Mutex<Vec<(Level, ShortcutReportText)>>>);

#[derive(Default)]
struct ShortcutReportText {
    words:   String,
    details: String,
}

impl Visit for ShortcutReportText {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        match field.name() {
            "message" => self.words = format!("{value:?}"),
            "diagnostic" => self.details = format!("{value:?}"),
            _ => {},
        }
    }
}

impl<S: tracing::Subscriber> Layer<S> for ShortcutReports {
    fn on_event(&self, event: &Event<'_>, _: Context<'_, S>) {
        if !event.metadata().target().starts_with("hana_rubric") {
            return;
        }
        let mut text = ShortcutReportText::default();
        event.record(&mut text);
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((*event.metadata().level(), text));
    }
}

#[test]
fn user_shortcut_reload_reports_plain_notices_and_keeps_source_details() -> Result<(), String> {
    let mut app = transaction_app(
        r#"{
        "bindings": [{ "bindings": { "g": "reload::first" } }],
        "bindngs": []
    }"#,
    )?;
    let reports = ShortcutReports::default();
    let subscriber = Registry::default().with(reports.clone());
    tracing::subscriber::with_default(subscriber, || {
        assert!(commit_defaults(app.world_mut()));
        queue_user_snapshot(&mut app, br#"{ "bindings": [}"#);
        commit_reload(app.world_mut());
    });
    let reports = reports.0.lock().unwrap_or_else(PoisonError::into_inner);
    assert!(reports.iter().any(|(level, report)| {
        *level == Level::WARN
            && report
                .words
                .contains("Your keyboard shortcut settings need attention.")
            && report.details.contains("bindngs")
            && report.details.contains(DEFAULTS_PATH)
    }));
    assert!(reports.iter().any(|(level, report)| {
        *level == Level::ERROR
            && report
                .words
                .contains("Your keyboard shortcut settings need attention.")
            && report.details.contains(USER_KEYMAP_FIXTURE)
            && report.details.contains("Syntax")
    }));
    for (_, report) in reports.iter() {
        assert!(!report.words.contains("bindngs"));
        assert!(!report.words.contains(DEFAULTS_PATH));
        assert!(!report.words.contains(USER_KEYMAP_FIXTURE));
        assert!(!report.words.contains("reload::"));
    }
    drop(reports);
    Ok(())
}
