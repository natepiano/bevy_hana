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
use bevy::prelude::Assets;
use bevy::prelude::Commands;
use bevy::prelude::MessageReader;
use bevy::prelude::Plugin;
use bevy::prelude::PostUpdate;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::Startup;
use bevy::prelude::warn;
pub(crate) use font::DEFAULT_FAMILY;
pub use font::DiegeticTextMeasurer;
pub use font::Font;
pub use font::FontId;
pub use font::FontLoadFailed;
use font::FontLoader;
pub use font::FontMetrics;
pub use font::FontRegistered;
pub use font::FontRegistry;
pub use font::FontSource;
#[cfg(feature = "typography_overlay")]
pub use font::GlyphBounds;
#[cfg(feature = "typography_overlay")]
pub use font::GlyphTypographyMetrics;
pub use font::IntegralAdvanceSizeError;
pub use font::create_parley_measurer;
pub(crate) use font::glyph_ink_extents;
pub(crate) use slug::GlyphCache;
pub(crate) use slug::GlyphQuadExtents;
pub(crate) use slug::PositionedGlyph;
pub(crate) use slug::PreparedTextRun;
pub(crate) use slug::RunStorageKey;
pub(crate) use slug::glyph_quad_extents;

use self::slug::SlugPlugin;
#[cfg(test)]
use crate::render::PathExtendedMaterial;

pub(crate) struct TextPlugin;

#[cfg(test)]
pub(super) const fn text_material_oit_depth_offset(material: &PathExtendedMaterial) -> f32 {
    slug::text_material_oit_depth_offset(material)
}

impl Plugin for TextPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(SlugPlugin);

        // If the embedded font fails to parse, skip the rest of text setup:
        // no `FontRegistry`, no `DiegeticTextMeasurer`, no `Font` asset or
        // loader, and none of the font systems get registered.
        let Some(font_registry) = FontRegistry::new() else {
            warn!("hana_diegetic: embedded font failed to parse — text plugin disabled");
            return;
        };

        let measurer = DiegeticTextMeasurer {
            measure_fn: create_parley_measurer(
                font_registry.font_context(),
                font_registry.family_names(),
            ),
        };

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
/// registration.
fn consume_loaded_fonts(
    mut events: MessageReader<AssetEvent<Font>>,
    font_assets: Res<Assets<Font>>,
    mut font_registry: ResMut<FontRegistry>,
    mut commands: Commands,
) {
    for event in events.read() {
        if let AssetEvent::Added { id } = event
            && let Some(font) = font_assets.get(*id)
        {
            if font_registry.font_id_by_name(font.name()).is_some() {
                continue;
            }
            if let Some(font_id) = font_registry.register_font(font.name(), font.data()) {
                // The parley measurer maps `font_id` → family name through the
                // family list it was built from. A font registered after
                // startup is absent from that list, so its measure falls back
                // to the default family and mis-sizes the panel. Rebuild the
                // measurer with the now-current family list so the new font
                // measures as itself.
                commands.insert_resource(DiegeticTextMeasurer {
                    measure_fn: create_parley_measurer(
                        font_registry.font_context(),
                        font_registry.family_names(),
                    ),
                });
                commands.trigger(FontRegistered {
                    id:     font_id,
                    name:   (*font.name()).to_string(),
                    source: FontSource::Loaded,
                });
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
            path:  event.path.to_string(),
            error: event.error.to_string(),
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
    use bevy::prelude::PostUpdate;

    use super::DiegeticTextMeasurer;
    use super::Font;
    use super::FontRegistry;
    use super::consume_loaded_fonts;
    use super::create_parley_measurer;
    use crate::TextStyle;

    /// Noto Sans CJK SC — the exact font the typography example loads on Digit7,
    /// whose descenders clipped because its tall CJK line box was measured with
    /// the monospace default's metrics instead.
    const NOTO_CJK_DATA: &[u8] = include_bytes!("../../assets/fonts/NotoSansCJKsc-Regular.otf");

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
                .is_some(),
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
