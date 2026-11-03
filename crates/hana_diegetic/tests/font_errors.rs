//! The public font error contract: every font failure reaches its caller as a
//! named error, through the crate's public API only.
#![allow(
    clippy::expect_used,
    reason = "a font fixture that fails to build should fail the test immediately"
)]

use bevy::asset::AssetApp;
use bevy::asset::AssetPath;
use bevy::asset::AssetPlugin;
use bevy::asset::AssetServer;
use bevy::gizmos::GizmoAsset;
use bevy::prelude::App;
use bevy::prelude::Image;
use bevy::prelude::Mesh;
use bevy::prelude::MinimalPlugins;
use bevy::prelude::On;
use bevy::prelude::ResMut;
use bevy::prelude::Resource;
use bevy::render::storage::ShaderBuffer;
use bevy::shader::Shader;
use hana_diegetic::DecorationLineMetrics;
use hana_diegetic::DiegeticUiPlugin;
use hana_diegetic::Font;
use hana_diegetic::FontFailureSource;
use hana_diegetic::FontId;
use hana_diegetic::FontLoadError;
use hana_diegetic::FontLoadFailed;
use hana_diegetic::FontLookupError;
use hana_diegetic::FontParseError;
use hana_diegetic::FontRegistrationError;
use hana_diegetic::FontRegistry;
use hana_diegetic::TextInkMeasurement;
use hana_diegetic::TextStyle;

/// Bytes no font parser accepts.
const UNREADABLE_FONT_DATA: &[u8] = b"not a font";

/// The name the unreadable bytes are offered under.
const UNREADABLE_FONT_NAME: &str = "Unreadable Face";

/// A family name no font is registered under.
const UNREGISTERED_FAMILY: &str = "Unregistered Family";

/// A font ID no family is registered under.
const UNREGISTERED_FONT_ID: FontId = FontId(u16::MAX);

/// A font file that does not exist under the crate's asset root.
const MISSING_FONT_PATH: &str = "fonts/MissingFace-Regular.ttf";

/// Font size every measurement in this file uses.
const MEASURE_SIZE: f32 = 12.0;

fn embedded_registry() -> FontRegistry {
    FontRegistry::new().expect("the embedded font family builds")
}

#[test]
fn unreadable_bytes_fail_to_parse_as_a_font() {
    let error = Font::from_bytes(UNREADABLE_FONT_NAME, UNREADABLE_FONT_DATA).err();

    assert!(
        matches!(&error, Some(FontParseError::Unreadable { name }) if name == UNREADABLE_FONT_NAME),
        "unreadable bytes should fail to parse under their name, got {error:?}"
    );
}

#[test]
fn registering_unreadable_bytes_carries_the_parse_error() {
    let mut registry = embedded_registry();

    let result = registry.register_font(UNREADABLE_FONT_NAME, UNREADABLE_FONT_DATA);

    assert!(
        matches!(
            &result,
            Err(FontRegistrationError::Unreadable(FontParseError::Unreadable { name }))
                if name == UNREADABLE_FONT_NAME
        ),
        "registering unreadable bytes should carry the parse error, got {result:?}"
    );
}

#[test]
fn the_embedded_family_builds() {
    let result = FontRegistry::new();

    assert!(
        result.is_ok(),
        "the embedded faces should parse, got {:?}",
        result.err()
    );
}

#[test]
fn an_unregistered_font_id_is_an_unknown_family_id() {
    let registry = embedded_registry();

    let font_error = registry.font(UNREGISTERED_FONT_ID).err();
    let family_name = registry.family_name(UNREGISTERED_FONT_ID);

    assert!(
        matches!(
            font_error,
            Some(FontLookupError::UnknownFamilyId(id)) if id == UNREGISTERED_FONT_ID
        ),
        "looking up an unregistered font ID should name that ID, got {font_error:?}"
    );
    assert!(
        matches!(
            family_name,
            Err(FontLookupError::UnknownFamilyId(id)) if id == UNREGISTERED_FONT_ID
        ),
        "naming an unregistered font ID should name that ID, got {family_name:?}"
    );
}

#[test]
fn an_unregistered_family_name_is_an_unknown_family_name() {
    let registry = embedded_registry();

    let result = registry.font_id_by_name(UNREGISTERED_FAMILY);

    assert!(
        matches!(
            &result,
            Err(FontLookupError::UnknownFamilyName(name)) if name == UNREGISTERED_FAMILY
        ),
        "looking up an unregistered family should name that family, got {result:?}"
    );
}

#[test]
fn blank_text_measures_no_visible_ink() {
    let registry = embedded_registry();

    let result = registry.measure_ink("   ", &TextStyle::new(MEASURE_SIZE).as_measure());

    assert!(
        matches!(result, Ok(TextInkMeasurement::NoVisibleInk)),
        "blank text should draw no ink, got {result:?}"
    );
}

#[test]
fn a_word_measures_visible_ink() {
    let registry = embedded_registry();

    let result = registry.measure_ink("Hana", &TextStyle::new(MEASURE_SIZE).as_measure());

    assert!(
        matches!(
            result,
            Ok(TextInkMeasurement::Visible(ink)) if ink.right > ink.left && ink.bottom > ink.top
        ),
        "a word should draw ink with an area, got {result:?}"
    );
}

#[test]
fn measuring_with_an_unregistered_font_id_is_an_unknown_family_id() {
    let registry = embedded_registry();
    let measure = TextStyle::new(MEASURE_SIZE)
        .with_font(UNREGISTERED_FONT_ID.0)
        .as_measure();

    let result = registry.measure_ink("Hana", &measure);

    assert!(
        matches!(result, Err(FontLookupError::UnknownFamilyId(id)) if id == UNREGISTERED_FONT_ID),
        "measuring with an unregistered font ID should name that ID, got {result:?}"
    );
}

#[test]
fn the_embedded_face_specifies_underline_and_strikeout() {
    let registry = embedded_registry();
    let font = registry
        .font(FontId::MONOSPACE)
        .expect("the embedded family is registered");

    let metrics = font.metrics(MEASURE_SIZE);

    assert!(
        matches!(
            metrics.underline,
            DecorationLineMetrics::Specified { thickness, .. } if thickness > 0.0
        ),
        "the embedded face's post table gives an underline, got {:?}",
        metrics.underline
    );
    assert!(
        matches!(
            metrics.strikeout,
            DecorationLineMetrics::Specified { thickness, .. } if thickness > 0.0
        ),
        "the embedded face's OS/2 table gives a strikeout, got {:?}",
        metrics.strikeout
    );
}

/// Every [`FontLoadFailed`] the app fired, in order.
#[derive(Resource, Default)]
struct FontLoadFailures(Vec<FontLoadFailed>);

fn record_font_load_failure(failure: On<FontLoadFailed>, mut failures: ResMut<FontLoadFailures>) {
    failures.0.push(failure.event().clone());
}

#[test]
fn loading_a_missing_font_file_fires_font_load_failed_naming_its_path() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, AssetPlugin::default()))
        .init_asset::<Shader>()
        .init_asset::<Mesh>()
        .init_asset::<GizmoAsset>()
        .init_asset::<Image>()
        .init_asset::<ShaderBuffer>()
        .add_plugins(DiegeticUiPlugin)
        .init_resource::<FontLoadFailures>()
        .add_observer(record_font_load_failure);

    let font = app
        .world()
        .resource::<AssetServer>()
        .load::<Font>(MISSING_FONT_PATH);
    // The asset server reads the file off-thread, so drive frames until it
    // reports the load failed. The failure and `FontLoadFailed` land in the
    // same frame.
    while !app
        .world()
        .resource::<AssetServer>()
        .load_state(font.id())
        .is_failed()
    {
        app.update();
    }

    let failures = &app.world().resource::<FontLoadFailures>().0;
    assert!(
        matches!(
            failures.as_slice(),
            [FontLoadFailed {
                error: FontLoadError::Asset(_),
                source: FontFailureSource::AssetPath(path),
            }] if *path == AssetPath::from(MISSING_FONT_PATH)
        ),
        "a missing font file should fire one asset failure naming its path, got {failures:?}"
    );
}

/// Glyph measurement, published with the typography overlay.
#[cfg(feature = "typography_overlay")]
mod glyph_ink {
    use hana_diegetic::FontId;
    use hana_diegetic::GlyphInkMeasurement;
    use hana_diegetic::GlyphMetricsError;

    use super::MEASURE_SIZE;
    use super::embedded_registry;

    /// An ideograph the embedded monospace face has no glyph for.
    const ABSENT_IDEOGRAPH: char = '漢';

    /// A glyph ID at or past any face's glyph count.
    const ABSENT_GLYPH_ID: u16 = u16::MAX;

    #[test]
    fn a_letter_draws_visible_ink() {
        let registry = embedded_registry();
        let font = registry
            .font(FontId::MONOSPACE)
            .expect("the embedded family is registered");

        let result = font.glyph_metrics('H', MEASURE_SIZE);

        assert!(
            matches!(result, Ok(GlyphInkMeasurement::Visible(_))),
            "H should draw ink, got error {:?}",
            result.err()
        );
    }

    #[test]
    fn a_space_draws_no_visible_ink() {
        let registry = embedded_registry();
        let font = registry
            .font(FontId::MONOSPACE)
            .expect("the embedded family is registered");

        let result = font.glyph_metrics(' ', MEASURE_SIZE);

        assert!(
            matches!(result, Ok(GlyphInkMeasurement::NoVisibleInk)),
            "a space has a glyph with no outline, got error {:?}",
            result.err()
        );
    }

    #[test]
    fn a_character_the_face_lacks_is_a_missing_glyph() {
        let registry = embedded_registry();
        let font = registry
            .font(FontId::MONOSPACE)
            .expect("the embedded family is registered");

        let error = font.glyph_metrics(ABSENT_IDEOGRAPH, MEASURE_SIZE).err();

        assert!(
            matches!(error, Some(GlyphMetricsError::MissingGlyph(ch)) if ch == ABSENT_IDEOGRAPH),
            "the embedded face has no CJK ideographs, got {error:?}"
        );
    }

    #[test]
    fn a_glyph_id_past_the_glyph_count_is_unknown() {
        let registry = embedded_registry();
        let font = registry
            .font(FontId::MONOSPACE)
            .expect("the embedded family is registered");

        let error = font
            .glyph_metrics_by_id(ABSENT_GLYPH_ID, MEASURE_SIZE)
            .err();

        assert!(
            matches!(error, Some(GlyphMetricsError::UnknownGlyphId(id)) if id == ABSENT_GLYPH_ID),
            "a glyph ID past the face's glyph count should be unknown, got {error:?}"
        );
    }
}
