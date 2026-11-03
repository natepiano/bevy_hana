//! Authored title positions returned by the public palette query.

#![allow(
    clippy::single_range_in_vec_init,
    reason = "fixtures store lists of title runs, not lists of positions within a run"
)]

use bevy::prelude::App;
use bevy::prelude::Event;
use bevy::prelude::Reflect;
use bevy::prelude::ReflectEvent;
use hana_rubric::ActiveKeymapContext;
use hana_rubric::CommandRegistry;
use hana_rubric::EffectiveKeymapStatus;
use hana_rubric::KeymapBindings;
use hana_rubric::KeymapCommand;
use hana_rubric::PaletteTitleMatch;
use hana_rubric::ReflectKeymapCommand;
use hana_rubric::query_command_palette;

macro_rules! fixture_command {
    ($event:ident, $id:literal, $title:literal, $description:literal) => {
        hana_rubric::event!(#[doc = $description] $event, reflect: KeymapCommand);
        impl KeymapCommand for $event {
            const ID: &'static str = $id;
            const TITLE: &'static str = $title;
            const DESCRIPTION: &'static str = $description;
            const CAPABILITY: hana_rubric::Capability = hana_rubric::Capability::OneShot;
            fn build() -> Self { Self }
            fn hold_phase(&self) -> Option<hana_rubric::HoldPhase> { None }
        }
    };
}

fixture_command!(
    Reset,
    "title_match_test::restore",
    "Reset everything",
    "Resets the fixture."
);
fixture_command!(
    PanTilt,
    "title_match_test::move",
    "Pan-tilt camera",
    "Moves the fixture camera."
);
fixture_command!(
    AddCamera,
    "title_match_test::add",
    "Añadir cámara",
    "Adds the fixture camera."
);
fixture_command!(
    ExpandedLowercase,
    "title_match_test::unicode",
    "İo",
    "Tests an expanding lowercase mapping."
);
fixture_command!(
    IdOnly,
    "title_match_test::identifier_only",
    "Unrelated title",
    "Tests a match on the command identifier."
);
fixture_command!(
    AddViewfinder,
    "fixture::finder",
    "Add viewfinder",
    "Adds a viewfinder."
);

#[allow(
    clippy::expect_used,
    reason = "invalid fixture declarations must fail the test"
)]
fn matching_titles(query: &str) -> Vec<(String, PaletteTitleMatch)> {
    let mut app = App::new();
    app.register_type::<Reset>()
        .register_type::<PanTilt>()
        .register_type::<AddCamera>()
        .register_type::<ExpandedLowercase>()
        .register_type::<IdOnly>()
        .register_type::<AddViewfinder>();
    let registry = CommandRegistry::initialize(app.world_mut()).expect("valid fixture commands");
    let context = ActiveKeymapContext::default();
    let bindings = KeymapBindings::default();
    query_command_palette(
        &registry,
        &context,
        &EffectiveKeymapStatus::AwaitingAcceptedDocument,
        &bindings,
        query,
    )
    .rows()
    .iter()
    .map(|row| (row.command().title().to_owned(), row.title_match().clone()))
    .collect()
}

fn assert_title_match(query: &str, title: &str, expected: PaletteTitleMatch) {
    let rows = matching_titles(query);
    let matches = rows
        .iter()
        .filter(|(text, _)| text == title)
        .collect::<Vec<_>>();
    assert_eq!(
        matches.len(),
        1,
        "query {query:?} must list {title:?} once: {rows:?}"
    );
    assert_eq!(matches[0].1, expected, "query {query:?}, title {title:?}");
}

#[test]
fn a_title_prefix_match_starts_at_the_first_character() {
    assert_title_match(
        "rEs",
        "Reset everything",
        PaletteTitleMatch::from_runs([0..3]),
    );
}

#[test]
fn a_later_word_prefix_match_gives_that_words_positions() {
    assert_title_match(
        "ever",
        "Reset everything",
        PaletteTitleMatch::from_runs([6..10]),
    );
}

#[test]
fn an_inner_match_gives_the_first_place_the_title_contains_it() {
    assert_title_match(
        "set",
        "Reset everything",
        PaletteTitleMatch::from_runs([2..5]),
    );
    assert_title_match(
        "t",
        "Reset everything",
        PaletteTitleMatch::from_runs([4..5]),
    );
}

#[test]
fn a_repeated_letter_highlights_only_the_occurrence_that_ranked_the_row() {
    assert_title_match(
        "e",
        "Reset everything",
        PaletteTitleMatch::from_runs([6..7]),
    );
}

#[test]
fn separate_words_leave_the_authored_separator_plain() {
    assert_title_match(
        "pan tilt",
        "Pan-tilt camera",
        PaletteTitleMatch::from_runs([0..3, 4..8]),
    );
}

#[test]
fn scattered_letters_do_not_match() {
    assert_eq!(matching_titles("rst"), []);
}

#[test]
fn a_query_no_title_contains_lists_no_row() {
    assert_eq!(matching_titles("absent query"), []);
}

#[test]
fn a_match_on_the_id_alone_lists_the_row_with_no_title_run() {
    assert_eq!(
        matching_titles("identifier only"),
        vec![("Unrelated title".to_owned(), PaletteTitleMatch::None)]
    );
}

#[test]
fn an_empty_query_gives_no_run_on_any_row() {
    for query in ["", " :_-\t "] {
        let rows = matching_titles(query);
        assert_eq!(rows.len(), 6);
        assert!(
            rows.iter()
                .all(|(_, title_match)| *title_match == PaletteTitleMatch::None)
        );
    }
}

#[test]
fn positions_count_characters_in_a_non_ascii_title() {
    assert_title_match(
        "cám",
        "Añadir cámara",
        PaletteTitleMatch::from_runs([7..10]),
    );
}

#[test]
fn a_letter_whose_lowercase_is_two_characters_counts_once() {
    assert_title_match("o", "İo", PaletteTitleMatch::from_runs([1..2]));
    assert_title_match("i", "İo", PaletteTitleMatch::from_runs([0..1]));
    assert_title_match("İo", "İo", PaletteTitleMatch::from_runs([0..2]));
}

fn title_runs(query: &str, title: &str) -> Vec<std::ops::Range<usize>> {
    let rows = matching_titles(query);
    let matches = rows
        .iter()
        .filter(|(text, _)| text == title)
        .collect::<Vec<_>>();
    assert_eq!(matches.len(), 1, "{query:?} must find {title:?}: {rows:?}");
    matches[0].1.runs().to_vec()
}

#[test]
fn each_query_word_matches_a_separate_title_run() {
    assert_eq!(title_runs("add fin", "Add viewfinder"), [0..3, 8..11]);
}

#[test]
fn reversing_query_words_keeps_the_title_runs() {
    assert_eq!(title_runs("fin add", "Add viewfinder"), [0..3, 8..11]);
}

#[test]
fn every_query_word_is_required() {
    assert_eq!(
        matching_titles("add zzz"),
        [] as [(std::string::String, hana_rubric::PaletteTitleMatch); 0]
    );
    assert_eq!(title_runs("add fin", "Add viewfinder"), [0..3, 8..11]);
}

#[test]
fn one_word_keeps_its_results_and_order_when_multiword_search_is_added() {
    let titles = |query| {
        matching_titles(query)
            .into_iter()
            .map(|(title, _)| title)
            .collect::<Vec<_>>()
    };
    assert_eq!(
        titles("t"),
        [
            "Pan-tilt camera",
            "Unrelated title",
            "Add viewfinder",
            "Añadir cámara",
            "İo",
            "Reset everything"
        ]
    );
    assert_eq!(titles("fin add"), ["Add viewfinder"]);
}

#[test]
fn overlapping_words_merge_into_one_title_run() {
    assert_eq!(title_runs("view iewfin", "Add viewfinder"), [4..11]);
}

#[test]
fn touching_and_duplicate_words_merge_into_one_title_run() {
    assert_eq!(title_runs("view fin view", "Add viewfinder"), [4..11]);
}

#[test]
fn a_word_can_match_the_id_while_another_matches_the_title() {
    assert_eq!(title_runs("identifier title", "Unrelated title"), [10..15]);
}

#[test]
fn word_runs_keep_authored_unicode_character_positions() {
    assert_eq!(title_runs("cám aña", "Añadir cámara"), [0..3, 7..10]);
}
