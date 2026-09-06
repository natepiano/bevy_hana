//! Shared constants for the panel-anchoring example: panel sizes, colors, info
//! panel and menu layout metrics, animation timings, and arrangement controls.

use bevy::prelude::Color;
use bevy::prelude::Vec3;
use hana_diegetic::Anchor;

pub(crate) const PANEL_WIDTH: f32 = 108.0;
pub(crate) const PANEL_HEIGHT: f32 = 78.0;
pub(crate) const BORDER_WIDTH: f32 = 1.2;
pub(crate) const HOME_MARGIN: f32 = 0.4;
pub(crate) const ANCHOR_MARKER_SIZE: f32 = 10.0;
pub(crate) const MARKER_ALPHA: f32 = 0.72;
/// Font size of the depth label drawn on the anchored panel, in millimeters.
pub(crate) const DEPTH_LABEL_SIZE: f32 = 7.0;
/// Bottom padding (mm) that raises the depth label to just above where the
/// bottom-center anchor marker would sit.
pub(crate) const DEPTH_LABEL_BOTTOM_PAD: f32 = ANCHOR_MARKER_SIZE + 2.0;
/// Minimum change (mm) in the displayed depth before the anchored panel is
/// rebuilt, so a live depth change updates the label without a rebuild every
/// frame.
pub(crate) const DEPTH_LABEL_STEP_MM: f32 = 0.5;
/// Minimum marker movement (mm) before a panel is rebuilt during an anchor ease.
pub(crate) const MARKER_STEP_MM: f32 = 0.25;
pub(crate) const ANCHOR_LINK_MIN_LENGTH: f32 = 0.001;
pub(crate) const INFO_PANEL_WIDTH: f32 = 230.0;
pub(crate) const INFO_TITLE_SIZE: f32 = 16.0;
pub(crate) const INFO_BODY_SIZE: f32 = 12.0;
pub(crate) const INFO_SECTION_GAP: f32 = 8.0;
pub(crate) const INFO_ROW_GAP: f32 = 4.0;
pub(crate) const INFO_COL_GAP: f32 = 10.0;
pub(crate) const INFO_GRID_CELL_SIZE: f32 = 9.0;
pub(crate) const INFO_GRID_GAP: f32 = 2.0;
pub(crate) const INFO_GRID_BORDER_WIDTH: f32 = 1.0;
pub(crate) const INFO_GRID_SIDE: usize = 3;
/// Gap (px) between the `Navigation` title and the `Tab to change Panel` hint.
pub(crate) const NAV_TITLE_GAP: f32 = 8.0;
/// Vertical gap (mm) between the cross rows of the Navigation legend.
pub(crate) const NAV_GAP: f32 = 8.0;
/// Horizontal gap (mm) between the arrows and the centered `R Reset`.
pub(crate) const NAV_MIDDLE_GAP: f32 = 12.0;
/// Arrow glyph size (mm) in the Navigation cross.
pub(crate) const NAV_GLYPH_SIZE: f32 = 18.0;
/// Size (mm) of the centered `R Reset` label.
pub(crate) const NAV_CENTER_SIZE: f32 = 13.0;
/// Size (mm) of the `Tab to change Panel` hint.
pub(crate) const NAV_HINT_SIZE: f32 = 11.0;
/// Gap (mm) standing in for the space after `Tab` in the hint, so `Tab` can
/// light on its own.
pub(crate) const NAV_HINT_WORD_GAP: f32 = 4.0;
/// Vertical gap (mm) between the two hint sentences above the arrow cross.
pub(crate) const NAV_HINT_LINE_GAP: f32 = 9.0;
pub(crate) const MENU_TITLE_SIZE: f32 = 14.0;
pub(crate) const MENU_ROW_SIZE: f32 = 13.0;
pub(crate) const MENU_SECTION_GAP: f32 = 6.0;
pub(crate) const MENU_ROW_GAP: f32 = 3.0;
pub(crate) const MENU_ROW_COL_GAP: f32 = 8.0;
pub(crate) const MENU_ROW_PADDING: f32 = 4.0;
pub(crate) const MENU_ROW_CORNER: f32 = 3.0;
pub(crate) const MENU_HIGHLIGHT_ALPHA: f32 = 0.26;

pub(crate) const TARGET_POSITION: Vec3 = Vec3::ZERO;
pub(crate) const CAMERA_FOCUS: Vec3 = Vec3::ZERO;
pub(crate) const CAMERA_RADIUS: f32 = 1.75;
pub(crate) const CAMERA_YAW: f32 = 0.0;
pub(crate) const CAMERA_PITCH: f32 = 0.0;

/// Background of every anchor-chain tile. The tile's identity comes from its
/// accent (the hinge color wheel), so all tiles share one neutral background.
pub(crate) const TILE_BACKGROUND: Color = Color::srgba(0.09, 0.11, 0.16, 0.92);
pub(crate) const BODY_COLOR: Color = Color::srgba(0.82, 0.88, 0.96, 0.92);
pub(crate) const INFO_LABEL_COLOR: Color = Color::srgba(0.66, 0.72, 0.82, 0.94);
pub(crate) const INFO_GRID_INACTIVE: Color = Color::srgba(0.12, 0.14, 0.18, 0.95);
pub(crate) const INFO_GRID_BORDER: Color = Color::srgba(0.55, 0.62, 0.72, 0.90);
/// Thin rule separating the anchor sections from the Navigation section.
pub(crate) const INFO_DIVIDER_THICKNESS: f32 = 1.0;
pub(crate) const INFO_DIVIDER_COLOR: Color = Color::srgba(0.42, 0.48, 0.58, 0.55);
/// Vertical space (mm) above and below the divider rule.
pub(crate) const INFO_DIVIDER_PAD: f32 = 9.0;
/// Alpha on the unselected panel's section title; `Tab` selects which panel the
/// arrow keys move, and the selected title shows at full strength.
pub(crate) const INFO_TITLE_DIM_ALPHA: f32 = 0.38;
/// Highlight for the active direction glyph during an anchor transition: the same
/// yellow the title bar uses for an active control.
pub(crate) const INFO_LEGEND_ACTIVE: Color = Color::srgb(1.0, 0.9, 0.25);
pub(crate) const MENU_HEADER_COLOR: Color = Color::srgba(0.72, 0.78, 0.88, 0.96);
pub(crate) const MENU_IDLE_COLOR: Color = Color::srgba(0.78, 0.84, 0.94, 0.90);
pub(crate) const MENU_ACTIVE_COLOR: Color = Color::WHITE;
pub(crate) const MENU_HIGHLIGHT: Color = Color::srgb(0.95, 0.78, 0.32);

/// Segment id of the title-bar `Pause` word; highlights while the spin
/// animation is paused.
pub(crate) const PAUSE_CONTROL: &str = "pause";

/// Segment id of the title-bar `Show Anchor` word; highlights while the anchor
/// marker discs are drawn (`O` toggles, on by default).
pub(crate) const SHOW_ANCHOR_CONTROL: &str = "show_anchor";

/// Segment id of the title-bar `+` word; flashes while a tile is added.
pub(crate) const ADD_TILE_CONTROL: &str = "add_tile";
/// Segment id of the title-bar `-` word; flashes while a tile is removed.
pub(crate) const REMOVE_TILE_CONTROL: &str = "remove_tile";
/// Segment id of the title-bar `[` word; flashes while depth moves out (`[` held).
pub(crate) const DEPTH_OUT_CONTROL: &str = "depth_out";
/// Segment id of the title-bar `]` word; flashes while depth moves in (`]` held).
pub(crate) const DEPTH_IN_CONTROL: &str = "depth_in";

/// Segment id of the title-bar `Autofit` word; highlights while the camera
/// reframes the panel union when it overflows the viewport (`I` toggles, on by
/// default).
pub(crate) const AUTOFIT_CONTROL: &str = "autofit";

pub(crate) const CAPABILITY_NAMES: [&str; 3] = ["Anchor", "Spin", "Hinge Chain"];

/// Capability index of plain anchoring (menu entry `1`): the anchor scene with no
/// spin envelope — arrows and depth only.
pub(crate) const ANCHOR_INDEX: usize = 0;
/// Capability index of the spin animation (menu entry `2`): the same anchor
/// scene, with the spin envelope toggled by its number key.
pub(crate) const SPIN_INDEX: usize = 1;
/// Capability index of the hinge-chain unwrap animation (menu entry `3`).
pub(crate) const HINGE_CHAIN_INDEX: usize = 2;
/// Seconds the spin takes to accelerate from rest on start, and to decelerate
/// back to rest on stop — symmetric, so the wind-down matches the wind-up.
pub(crate) const SPIN_EASE_SECS: f32 = 0.8;
/// Spin rate about the plane normal during the holding phase, radians/second.
pub(crate) const SPIN_RATE_RAD: f32 = 0.96;
/// Seconds an anchor-selection change (arrow keys or reset) takes to ease from
/// the previously resolved position to the new one — fast, but eased.
pub(crate) const ANCHOR_TRANSITION_SECS: f32 = 0.22;
/// Squared length below which a transition offset is treated as zero (meters²),
/// so a change that resolves to the same spot starts no animation.
pub(crate) const ANCHOR_TRANSITION_EPS_SQ: f32 = 1e-12;
/// Extra seconds a Navigation glow holds after its key is released, so the
/// yellow flash stays visible past the keypress.
pub(crate) const LEGEND_GLOW_TAIL_SECS: f32 = 0.2;
/// Total glow time for a tapped direction (arrow / `R Reset`): the slide ease
/// plus the release tail.
pub(crate) const LEGEND_TAP_GLOW_SECS: f32 = ANCHOR_TRANSITION_SECS + LEGEND_GLOW_TAIL_SECS;

/// Seconds a mode switch takes to ease the persistent tiles from the outgoing
/// mode's layout to the incoming mode's layout. Every capability shares the one
/// tile set, so a switch re-anchors and re-poses the same tiles and eases the
/// resolved transform across this window rather than despawning and re-flying a
/// scene.
pub(crate) const MODE_MORPH_SECS: f32 = 0.6;

/// Panel height in world meters (`1 mm = 0.001 m`), the edge-to-edge spacing
/// between stacked hinge-strip links.
pub(crate) const PANEL_HEIGHT_M: f32 = PANEL_HEIGHT * 0.001;
/// Stroke width (mm) of a hinge link's border.
pub(crate) const HINGE_BORDER_WIDTH: f32 = 1.2;
/// Folded hinge angle (radians) each crease bends about its pinned edge: a full
/// half-turn so every link folds flat back onto its neighbour, collapsing the
/// chain to a single coplanar stack. The unwrap eases this to zero (coplanar
/// strip spread out). This example's direct hinge calculation alternates the
/// sign for accordion-style links and keeps it constant for coil-style links.
pub(crate) const HINGE_FOLD_ANGLE_RAD: f32 = std::f32::consts::PI;
/// Seconds for a full fold/unwrap ease, independent of link count: every
/// transition takes the same wall-clock time whether the chain has three links
/// or a hundred, so a long coil and a short accordion read at the same pace.
pub(crate) const HINGE_EASE_SECS: f32 = 3.0;
/// Font size (mm) of the number drawn on each hinge link.
pub(crate) const HINGE_LABEL_SIZE: f32 = 26.0;
/// Hue (degrees) of the first link's accent; each later link steps around the
/// color wheel by its index over the current link count, so a fuller chain
/// spreads its accents across more of the wheel.
pub(crate) const HINGE_ACCENT_HUE_START: f32 = 210.0;
/// Saturation and lightness every hinge accent is built at; only the hue walks
/// the wheel.
pub(crate) const HINGE_ACCENT_SATURATION: f32 = 0.72;
pub(crate) const HINGE_ACCENT_LIGHTNESS: f32 = 0.60;
/// Seconds a `+`/`-` key is held before it begins auto-repeating, then the
/// seconds between repeats while it stays held. Shared by the hinge chain and
/// the anchor chain.
pub(crate) const TILE_COUNT_REPEAT_DELAY: f32 = 0.35;
pub(crate) const TILE_COUNT_REPEAT_RATE: f32 = 0.08;
pub(crate) const HINGE_BACKGROUND: Color = Color::srgba(0.09, 0.11, 0.16, 0.92);

pub(crate) const ANCHOR_POINTS: [Anchor; 9] = [
    Anchor::TopLeft,
    Anchor::TopCenter,
    Anchor::TopRight,
    Anchor::CenterLeft,
    Anchor::Center,
    Anchor::CenterRight,
    Anchor::BottomLeft,
    Anchor::BottomCenter,
    Anchor::BottomRight,
];

pub(crate) const ANCHOR_NAMES: [&str; 9] = [
    "Top Left",
    "Top Center",
    "Top Right",
    "Center Left",
    "Center",
    "Center Right",
    "Bottom Left",
    "Bottom Center",
    "Bottom Right",
];

pub(crate) const DEFAULT_SOURCE_INDEX: usize = 0;
pub(crate) const DEFAULT_TARGET_INDEX: usize = 8;
pub(crate) const DEPTH_RATE_MM_PER_SEC: f32 = 95.0;
/// Multiplier applied to the depth rate while either `Ctrl` is held with `[`/`]`,
/// for a coarse fast-move.
pub(crate) const DEPTH_FAST_MULTIPLIER: f32 = 4.0;

/// Fewest tiles the anchor chain may shrink to (`-`): a single anchored pair —
/// one target tile and one dependent.
pub(crate) const ANCHOR_MIN_TILES: usize = 2;
/// Most tiles the chain may grow to (`+`); shared by every mode, it bounds the
/// cumulative depth fan, the fold link count, and the transparent draw-call count.
pub(crate) const ANCHOR_MAX_TILES: usize = 100;
