//! Typed Lucide icons for panel and screen text rendering.
//!
//! [`Icon::size`] and [`DiegeticIconBuilder::size`] take points, so an icon can
//! use the same size as its adjacent text. The fixed square carries its own
//! point unit; panel layout and font units affect neither its size nor its
//! glyph em. Screen positions remain in pixels. Rendering resolves the glyph
//! em from the solved square, including projected screen copies of a panel.
//! An icon keeps that square before its bundled font registers.
//! Its measured outline ink is centred in the square on both axes. The font
//! caches each glyph and solved size, so repainting does not measure it again.

mod lucide_generated;

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::PoisonError;

use bevy::asset::AssetServer;
use bevy::asset::Handle;
use bevy::asset::embedded_asset;
use bevy::asset::load_embedded_asset;
use bevy::prelude::App;
use bevy::prelude::Color;
use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::DetectChangesMut;
use bevy::prelude::Entity;
use bevy::prelude::On;
use bevy::prelude::Plugin;
use bevy::prelude::Query;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::Resource;
use bevy::prelude::Startup;
use bevy::prelude::Transform;
use bevy::prelude::Vec2;
use bevy::prelude::debug;
use bevy::prelude::warn;
pub use lucide_generated::LucideIcon;

use crate::constants::DIAGNOSTIC_LOG_TARGET;
use crate::layout::El;
use crate::layout::LayoutBuilder;
use crate::layout::Sizing;
use crate::layout::TextInkMeasurement;
use crate::layout::TextStyle;
use crate::panel::ComputedDiegeticPanel;
use crate::panel::DiegeticPanel;
use crate::panel::Fit;
use crate::text::Font;
use crate::text::FontId;
use crate::text::FontRegistered;
use crate::text::FontRegistry;

const LUCIDE_FONT_FAMILY: &str = "lucide";

/// One Lucide glyph drawn through the panel-text renderer.
///
/// The icon owns its square size in points and visual style. Its bundled font is
/// selected internally, so callers never handle a codepoint or font id.
#[must_use]
#[derive(Clone, Debug, PartialEq)]
pub struct Icon {
    glyph: LucideIcon,
    style: TextStyle,
}

impl Icon {
    /// Creates an icon with the default text size and white theme colour.
    pub fn new(glyph: LucideIcon) -> Self {
        Self {
            glyph,
            style: TextStyle::default(),
        }
    }

    /// Sets both the glyph size and its square layout box in points, like text.
    ///
    /// The panel's layout and font units do not change this size.
    pub const fn size(mut self, size: f32) -> Self {
        self.style.set_size(size);
        self
    }

    /// Sets the glyph colour.
    pub const fn color(mut self, color: Color) -> Self {
        self.style.set_color(color);
        self
    }

    pub(crate) const fn size_value(&self) -> f32 { self.style.size() }

    pub(crate) fn resolved_text_for_box(
        &self,
        font_id: FontId,
        box_size_in_points: f32,
    ) -> (String, TextStyle) {
        let mut style = self.style.clone();
        style.set_font_id(font_id.0);
        style.set_size(box_size_in_points);
        (self.glyph.codepoint().to_string(), style)
    }
}

/// Marker on the screen panel spawned by [`DiegeticIcon::screen`].
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct DiegeticIcon;

impl DiegeticIcon {
    /// Starts a screen-space icon builder.
    pub fn screen(glyph: LucideIcon) -> DiegeticIconBuilder {
        DiegeticIconBuilder {
            icon:     Icon::new(glyph),
            position: ScreenIconPosition::PanelDefault,
        }
    }
}

/// Fluent builder for one screen-space [`DiegeticIcon`].
#[must_use]
pub struct DiegeticIconBuilder {
    icon:     Icon,
    position: ScreenIconPosition,
}

#[derive(Clone, Copy, Debug, Default)]
enum ScreenIconPosition {
    #[default]
    PanelDefault,
    Explicit(Vec2),
}

impl DiegeticIconBuilder {
    /// Sets both the glyph size and its square screen-space box in points.
    pub fn size(mut self, size: f32) -> Self {
        self.icon = self.icon.size(size);
        self
    }

    /// Sets the glyph colour.
    pub fn color(mut self, color: Color) -> Self {
        self.icon = self.icon.color(color);
        self
    }

    /// Places the icon in pixels from the screen's top-left corner.
    pub const fn screen_position(mut self, position: Vec2) -> Self {
        self.position = ScreenIconPosition::Explicit(position);
        self
    }

    /// Spawns the icon's one-element screen panel and returns its entity.
    pub fn spawn(self, commands: &mut Commands) -> Entity {
        let mut builder =
            LayoutBuilder::with_root(El::new().width(Sizing::FIT).height(Sizing::FIT));
        builder.icon(self.icon);
        let panel = DiegeticPanel::screen().size(Fit, Fit);
        let panel = match self.position {
            ScreenIconPosition::PanelDefault => panel,
            ScreenIconPosition::Explicit(position) => panel.screen_position(position.x, position.y),
        };
        let panel = panel.with_tree(builder.build());
        let panel = panel.build().unwrap_or_else(|error| {
            warn!(target: DIAGNOSTIC_LOG_TARGET, "DiegeticIcon::screen: {error}; falling back to a default panel");
            DiegeticPanel::default()
        });
        commands
            .spawn((panel, DiegeticIcon, Transform::IDENTITY))
            .id()
    }
}

#[derive(Resource, Default)]
pub(crate) struct LucideFont {
    state:       LucideFontState,
    ink_centres: Mutex<HashMap<(char, u32), Vec2>>,
}

#[derive(Default)]
enum LucideFontState {
    #[default]
    LoadNotRequested,
    Loading(Handle<Font>),
    Registered {
        handle: Handle<Font>,
        id:     FontId,
    },
}

#[derive(Clone, Copy)]
pub(crate) enum LucideFontReadiness {
    Loading,
    Registered(FontId),
}

impl LucideFont {
    pub(crate) const fn readiness(&self) -> LucideFontReadiness {
        match &self.state {
            LucideFontState::LoadNotRequested | LucideFontState::Loading(_) => {
                LucideFontReadiness::Loading
            },
            LucideFontState::Registered { id, .. } => LucideFontReadiness::Registered(*id),
        }
    }

    /// Measures the drawn outline's centre from the text run's origin.
    /// Each glyph and solved size is measured once per font registration. Colour,
    /// focus and hover changes reuse the same measurement.
    pub(crate) fn ink_centre(&self, glyph: char, style: &TextStyle, fonts: &FontRegistry) -> Vec2 {
        let mut centres = self
            .ink_centres
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let key = (glyph, style.size().to_bits());
        if let Some(centre) = centres.get(&key) {
            return *centre;
        }
        let centre = match fonts.measure_ink(&glyph.to_string(), &style.as_measure()) {
            Ok(TextInkMeasurement::Visible(ink)) => {
                Vec2::new(ink.left + ink.right, ink.top + ink.bottom) * 0.5
            },
            Ok(TextInkMeasurement::NoVisibleInk) => Vec2::splat(style.size() * 0.5),
            Err(error) => {
                debug!("Lucide ink is not ready: {error}");
                return Vec2::splat(style.size() * 0.5);
            },
        };
        centres.insert(key, centre);
        centre
    }
}

pub(crate) struct IconPlugin;

impl Plugin for IconPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "../../assets/fonts/lucide.ttf");
        app.init_resource::<LucideFont>()
            .add_systems(Startup, load_lucide_font)
            .add_observer(record_lucide_font_registration);
    }
}

fn load_lucide_font(asset_server: Res<AssetServer>, mut lucide_font: ResMut<LucideFont>) {
    lucide_font.state = LucideFontState::Loading(load_embedded_asset!(
        asset_server.as_ref(),
        "../../assets/fonts/lucide.ttf"
    ));
}

fn record_lucide_font_registration(
    trigger: On<FontRegistered>,
    mut lucide_font: ResMut<LucideFont>,
    mut panels: Query<&mut ComputedDiegeticPanel>,
) {
    if trigger.name != LUCIDE_FONT_FAMILY {
        return;
    }
    let handle = match &lucide_font.state {
        LucideFontState::Loading(handle) | LucideFontState::Registered { handle, .. } => {
            handle.clone()
        },
        LucideFontState::LoadNotRequested => {
            debug!("Lucide registered before hana_diegetic requested its font asset");
            return;
        },
    };
    lucide_font.state = LucideFontState::Registered {
        handle,
        id: trigger.id,
    };
    lucide_font
        .ink_centres
        .get_mut()
        .unwrap_or_else(PoisonError::into_inner)
        .clear();
    for mut panel in &mut panels {
        panel.set_changed();
    }
}

#[cfg(test)]
mod tests;
