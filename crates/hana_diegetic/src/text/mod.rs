//! Font loading and text measurement.
//!
//! [`FontRegistry`] manages font loading via parley's `FontContext`.
//! The registry embeds `JetBrains Mono` as the default font and provides
//! a [`MeasureTextFn`](crate::MeasureTextFn) backed by parley's layout engine.
//!
//! [`Font`] provides access to font-level typographic metrics via
//! [`Font::metrics`], which returns a [`FontMetrics`] struct scaled to
//! any requested font size.
//!
//! Glyph rendering is handled by the slug analytic Bézier renderer; this
//! module owns only font infrastructure and its slug shader/material setup.

mod font;
mod slug;

use bevy::asset::AssetLoadFailedEvent;
use bevy::prelude::App;
use bevy::prelude::AssetApp;
use bevy::prelude::AssetEvent;
use bevy::prelude::AssetServer;
use bevy::prelude::Assets;
use bevy::prelude::Commands;
use bevy::prelude::MessageReader;
use bevy::prelude::Mut;
use bevy::prelude::Plugin;
use bevy::prelude::PostUpdate;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::Startup;
use bevy::prelude::warn;
use bevy::render::storage::ShaderBuffer;
pub(crate) use font::DEFAULT_FAMILY;
pub use font::DecorationLineMetrics;
pub use font::DiegeticTextMeasurer;
pub use font::EmbeddedFace;
pub use font::Font;
pub use font::FontFailureSource;
pub use font::FontId;
pub use font::FontLoadError;
pub use font::FontLoadFailed;
use font::FontLoader;
pub use font::FontLookupError;
pub use font::FontMetrics;
pub use font::FontParseError;
pub use font::FontRegistered;
pub use font::FontRegistrationError;
pub use font::FontRegistry;
pub use font::FontRegistryError;
pub use font::FontSource;
#[cfg(feature = "typography_overlay")]
pub use font::GlyphBounds;
#[cfg(feature = "typography_overlay")]
pub use font::GlyphInkMeasurement;
#[cfg(feature = "typography_overlay")]
pub use font::GlyphMetricsError;
pub use font::GlyphOutline;
#[cfg(feature = "typography_overlay")]
pub use font::GlyphTypographyMetrics;
pub use font::IntegralAdvanceSizeError;
pub(crate) use font::RichTextMeasurer;
pub use font::TextOutline;
pub use font::TextOutlineError;
pub use font::TextOutlineMeasurement;
pub(crate) use font::create_cap_height_measurer;
pub use font::create_parley_measurer;
pub(crate) use font::create_parley_rich_measurer;
pub(crate) use font::glyph_ink_extents;
pub(crate) use slug::GlyphCache;
pub(crate) use slug::GlyphQuadExtents;
pub(crate) use slug::PositionedGlyph;
pub(crate) use slug::PreparedTextRun;
pub(crate) use slug::RunStorageKey;
pub(in crate::text) use slug::font_glyph_id_has_visible_outline;
pub(crate) use slug::glyph_quad_extents;
pub(in crate::text) use slug::load_glyph_by_id_from_face;

/// Clears cached glyph paths and their atlas buffers between drawn test scenes.
pub fn reset_drawn_glyph_cache(app: &mut App) {
    app.world_mut()
        .resource_scope(|world, mut buffers: Mut<Assets<ShaderBuffer>>| {
            world
                .resource_mut::<GlyphCache>()
                .reset_to_post_startup(&mut buffers);
        });
}

use self::slug::SlugPlugin;
use crate::constants::DIAGNOSTIC_LOG_TARGET;
use crate::layout::ShapedTextCache;
#[cfg(test)]
use crate::render::PathExtendedMaterial;

pub(crate) struct TextPlugin;

#[cfg(test)]
pub(super) const fn text_material_oit_paint_order(material: &PathExtendedMaterial) -> f32 {
    slug::text_material_oit_paint_order(material)
}

impl Plugin for TextPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(SlugPlugin);

        // If the embedded font fails to parse, skip the rest of text setup:
        // no `FontRegistry`, no `DiegeticTextMeasurer`, no `Font` asset or
        // loader, and none of the font systems get registered.
        let font_registry = match FontRegistry::new() {
            Ok(registry) => registry,
            Err(error) => {
                warn!(target: DIAGNOSTIC_LOG_TARGET, "hana_diegetic: {error} — text plugin disabled");
                return;
            },
        };

        let measurer = DiegeticTextMeasurer {
            measure_fn: create_parley_measurer(
                font_registry.font_context(),
                font_registry.family_names(),
            ),
        };

        app.init_resource::<ShapedTextCache>();
        let rich_measurer = RichTextMeasurer {
            measure_fn: create_parley_rich_measurer(
                font_registry.font_context(),
                font_registry.family_names(),
                app.world().resource::<ShapedTextCache>().clone(),
            ),
        };
        app.insert_resource(rich_measurer);
        app.insert_resource(font_registry)
            .insert_resource(measurer)
            .init_asset::<Font>()
            .init_asset_loader::<FontLoader>()
            .add_systems(Startup, register_embedded_font)
            .add_systems(PostUpdate, (consume_loaded_fonts, watch_font_failures));
    }
}

/// Fires [`FontRegistered`] for the embedded default font at startup.
fn register_embedded_font(mut commands: Commands) {
    commands.trigger(FontRegistered {
        id:     FontId::MONOSPACE,
        name:   DEFAULT_FAMILY.to_string(),
        source: FontSource::Embedded,
    });
}

/// Watches for newly loaded [`Font`] assets and registers them with
/// [`FontRegistry`]. Fires [`FontRegistered`] for each successful
/// registration and [`FontLoadFailed`] for each refused one.
fn consume_loaded_fonts(
    mut events: MessageReader<AssetEvent<Font>>,
    font_assets: Res<Assets<Font>>,
    asset_server: Res<AssetServer>,
    mut font_registry: ResMut<FontRegistry>,
    mut commands: Commands,
    cache: Option<Res<ShapedTextCache>>,
) {
    for event in events.read() {
        if let AssetEvent::Added { id } = event
            && let Some(font) = font_assets.get(*id)
        {
            if font_registry.font_id_by_name(font.name()).is_ok() {
                continue;
            }
            match font_registry.register_font(font.name(), font.data()) {
                Ok(font_id) => {
                    // A loaded family was absent when the measurer was built.
                    // Rebuild its family list so it measures with its own face.
                    commands.insert_resource(DiegeticTextMeasurer {
                        measure_fn: create_parley_measurer(
                            font_registry.font_context(),
                            font_registry.family_names(),
                        ),
                    });
                    commands.insert_resource(RichTextMeasurer {
                        measure_fn: create_parley_rich_measurer(
                            font_registry.font_context(),
                            font_registry.family_names(),
                            cache.as_deref().cloned().unwrap_or_default(),
                        ),
                    });
                    commands.trigger(FontRegistered {
                        id:     font_id,
                        name:   (*font.name()).to_string(),
                        source: FontSource::Loaded,
                    });
                },
                Err(error) => {
                    let source = asset_server.get_path(*id).map_or_else(
                        || FontFailureSource::FamilyName(font.name().to_string()),
                        |path| FontFailureSource::AssetPath(path.into_owned()),
                    );
                    warn!(target: DIAGNOSTIC_LOG_TARGET, "hana_diegetic: font {source}: {error}");
                    commands.trigger(FontLoadFailed {
                        source,
                        error: FontLoadError::Registration(error),
                    });
                },
            }
        }
    }
}

/// Watches for failed [`Font`] asset loads and fires [`FontLoadFailed`].
fn watch_font_failures(
    mut failures: MessageReader<AssetLoadFailedEvent<Font>>,
    mut commands: Commands,
) {
    for event in failures.read() {
        commands.trigger(FontLoadFailed {
            source: FontFailureSource::AssetPath(event.path.clone()),
            error:  FontLoadError::Asset(event.error.clone()),
        });
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    reason = "tests use unwrap for clearer failure messages"
)]
mod tests {
    use std::sync::Arc;

    use bevy::asset::AssetEventSystems;
    use bevy::asset::AssetPlugin;
    use bevy::prelude::App;
    use bevy::prelude::AssetApp;
    use bevy::prelude::Assets;
    use bevy::prelude::IntoScheduleConfigs;
    use bevy::prelude::MinimalPlugins;
    use bevy::prelude::On;
    use bevy::prelude::PostUpdate;
    use bevy::prelude::ResMut;
    use bevy::prelude::Resource;

    use super::DiegeticTextMeasurer;
    use super::Font;
    use super::FontFailureSource;
    use super::FontLoadError;
    use super::FontLoadFailed;
    use super::FontLookupError;
    use super::FontParseError;
    use super::FontRegistered;
    use super::FontRegistrationError;
    use super::FontRegistry;
    use super::consume_loaded_fonts;
    use super::create_parley_measurer;
    use crate::TextStyle;

    /// Noto Sans CJK SC — the exact font the typography example loads on Digit7,
    /// whose descenders clipped because its tall CJK line box was measured with
    /// the monospace default's metrics instead.
    const NOTO_CJK_DATA: &[u8] = include_bytes!("../../assets/fonts/NotoSansCJKsc-Regular.otf");

    #[derive(Resource, Default)]
    struct RegistrationCount(usize);

    #[derive(Resource, Default)]
    struct LoadFailures(Vec<FontLoadFailed>);

    fn count_registration(_: On<FontRegistered>, mut count: ResMut<RegistrationCount>) {
        count.0 += 1;
    }

    fn record_load_failure(failure: On<FontLoadFailed>, mut failures: ResMut<LoadFailures>) {
        failures.0.push(failure.event().clone());
    }

    #[test]
    fn unreadable_loaded_font_fires_load_failed_and_does_not_register() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_plugins(AssetPlugin::default());
        app.init_asset::<Font>();
        app.insert_resource(FontRegistry::new().unwrap());
        app.init_resource::<RegistrationCount>();
        app.init_resource::<LoadFailures>();
        app.add_observer(count_registration);
        app.add_observer(record_load_failure);
        app.add_systems(PostUpdate, consume_loaded_fonts.after(AssetEventSystems));
        app.finish();
        app.cleanup();

        let font = Font::from_bytes("Broken family", NOTO_CJK_DATA)
            .unwrap()
            .with_unreadable_data_for_test();
        let _handle = app.world_mut().resource_mut::<Assets<Font>>().add(font);
        app.update();

        assert_eq!(app.world().resource::<RegistrationCount>().0, 0);
        assert_eq!(
            app.world()
                .resource::<FontRegistry>()
                .font_id_by_name("Broken family"),
            Err(FontLookupError::UnknownFamilyName(
                "Broken family".to_string()
            ))
        );
        let failures = &app.world().resource::<LoadFailures>().0;
        assert_eq!(failures.len(), 1);
        assert!(matches!(
            &failures[0],
            FontLoadFailed {
                source: FontFailureSource::FamilyName(name),
                error: FontLoadError::Registration(FontRegistrationError::Unreadable(
                    FontParseError::Unreadable { name: parsed_name }
                )),
            } if name == "Broken family" && parsed_name == "Broken family"
        ));
    }

    #[test]
    fn loading_a_font_rebuilds_the_measurer_to_resolve_it() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_plugins(AssetPlugin::default());
        app.init_asset::<Font>();

        // Seed the registry and a measurer built from its startup family list
        // — at this point only the embedded monospace font exists, so that
        // list has a single entry.
        let registry = FontRegistry::new().unwrap();
        let measurer = DiegeticTextMeasurer {
            measure_fn: create_parley_measurer(registry.font_context(), registry.family_names()),
        };
        app.insert_resource(registry);
        app.insert_resource(measurer);
        // Ordered after the asset-event drain so the `Added` event is in the
        // message buffer when it runs — the same `PostUpdate` slot the real
        // plugin uses.
        app.add_systems(PostUpdate, consume_loaded_fonts.after(AssetEventSystems));

        // AssetPlugin finishes its setup in `finish`/`cleanup`, which a
        // hand-built app must call before the first update.
        app.finish();
        app.cleanup();

        // Load the CJK font the example switches to. `Assets::add` queues an
        // `Added` event the asset system drains into the message buffer
        // `consume_loaded_fonts` reads; it registers the font (font 1) and
        // rebuilds the measurer with the now-current family list.
        let font = Font::from_bytes("Noto Sans CJK SC", NOTO_CJK_DATA).unwrap();
        // Hold the strong handle so the asset is not dropped before
        // `consume_loaded_fonts` reads the `Added` event.
        let _handle = app.world_mut().resource_mut::<Assets<Font>>().add(font);
        app.update();

        // The font registered through the event the system consumes.
        assert!(
            app.world()
                .resource::<FontRegistry>()
                .font_id_by_name("Noto Sans CJK SC")
                .is_ok(),
            "consume_loaded_fonts should register the loaded font",
        );

        // Measure the exact word that clipped in the example. The rebuilt
        // measurer must resolve font 1 to the real CJK face, whose line box is
        // taller than the monospace default's (≈1.45em vs ≈1.32em) — the height
        // the panel fits to. Without the rebuild, font 1 falls back to the
        // default and the two heights collapse to equal, which is exactly the
        // clipping bug: the panel sized to the wrong (shorter) line box.
        let measure_fn = Arc::clone(&app.world().resource::<DiegeticTextMeasurer>().measure_fn);
        let default_height = measure_fn("Typography", &TextStyle::new(32.0).as_measure()).height;
        let mut cjk = TextStyle::new(32.0).as_measure();
        cjk.font_id = 1;
        let cjk_height = measure_fn("Typography", &cjk).height;
        assert!(
            cjk_height > default_height + 1.0,
            "loading the CJK font must rebuild the measurer so \"Typography\" \
             measures the font's own (taller) line box, not the monospace \
             default: default {default_height} vs cjk {cjk_height}",
        );
    }
}
