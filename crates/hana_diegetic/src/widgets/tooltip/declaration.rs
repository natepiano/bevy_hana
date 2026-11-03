use std::sync::Arc;
use std::time::Duration;

use bevy::asset::Handle;
use bevy::color::Color;
use bevy::image::Image;
use bevy::prelude::Component;
use bevy::prelude::Timer;

use super::materialization;
use super::placement;
use crate::layout::Anchor;
use crate::layout::ChildLayoutState;
use crate::layout::Dimension;
use crate::layout::El;
use crate::layout::LayoutBuilder;
use crate::layout::LayoutOnly;
use crate::layout::LayoutTree;
use crate::layout::Text;
use crate::layout::TextStyle;
use crate::layout::Unit;
use crate::panel::PanelAnchorOffset;
use crate::panel::PanelAttachment;
use crate::panel::PanelSpace;
use crate::widgets::constants::DEFAULT_SHOW_DELAY;

/// Behavior when the described widget is disabled.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TooltipDisabledPolicy {
    /// Disabled targets may show their tooltip.
    Show,
    /// Disabled targets suppress their tooltip.
    Suppress,
}

/// Whether showing this help retires other materialized help panels.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum TooltipExclusivity {
    Independent,
    SoleHelp,
}

/// Deferred visual and behavior declaration for a tooltip.
///
/// Replace a standalone tooltip by despawning its controller and calling
/// [`crate::TooltipCommandsExt::spawn_tooltip`] again. Replacing this component on a
/// materialized controller is not an in-place content update.
#[derive(Component, Debug)]
#[require(TooltipPhase)]
pub struct Tooltip {
    pub(super) blueprint:                     Arc<LayoutTree>,
    authoring:                                TooltipAuthoring,
    pub(super) drawn_label_size:              Dimension,
    pub(super) font_unit:                     TooltipFontUnit,
    pub(super) ink_padding:                   TooltipInkPadding,
    pub(super) show_after:                    Duration,
    pub(super) hide_after:                    Duration,
    pub(super) disabled_policy:               TooltipDisabledPolicy,
    pub(super) exclusivity:                   TooltipExclusivity,
    pub(super) show_over_descendant_controls: bool,
    pub(super) source_anchor:                 Anchor,
    pub(super) target_anchor:                 Anchor,
    offset:                                   TooltipOffset,
}

/// Where a tooltip sits from the anchor point on its control.
#[derive(Clone, Copy, Debug, PartialEq)]
enum TooltipOffset {
    /// The default gap, out from the side the anchors name.
    DefaultGap,
    /// The offset the caller set, kept as set.
    Authored(PanelAnchorOffset),
}

/// How a world tooltip interprets text sizes without an authored unit.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum TooltipFontUnit {
    /// Select the layout unit for fixed world tooltips and points for fitting ones.
    FixedLayoutOrFitPoints,
    /// Use this unit even when the tooltip fits its content.
    Explicit(Unit),
}

/// Where the frame's clearance from the visible glyphs comes from.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum TooltipInkPadding {
    /// Use the widget's standard measured frame.
    WidgetFrame,
    /// Keep this distance between ink and frame on all four sides.
    Uniform(Dimension),
}

/// Sole authority for one tooltip controller's visibility lifecycle.
///
/// A wait owns the timer duration captured when that transition began. The
/// panel remains materialized while hidden; only `Visible` and
/// `WaitingToHide` represent an on-screen tooltip.
#[derive(Component, Debug, Default)]
pub(super) enum TooltipPhase {
    #[default]
    Hidden,
    /// A press retires help until the pointer leaves its target.
    HiddenUntilLeave,
    WaitingToShow(Timer),
    Visible,
    WaitingToHide(Timer),
}

impl TooltipPhase {
    pub(super) const fn is_visible(&self) -> bool {
        matches!(self, Self::Visible | Self::WaitingToHide(_))
    }
}

#[derive(Debug)]
struct TooltipAuthoring {
    parent_stack:  Vec<usize>,
    next_auto_id:  u32,
    unstyled_text: Vec<usize>,
}

impl Clone for Tooltip {
    fn clone(&self) -> Self {
        Self {
            blueprint:                     Arc::clone(&self.blueprint),
            // `parent_stack` exists only for the active `Tooltip::with` call;
            // an independent blueprint clone resumes authoring at the tree root.
            authoring:                     TooltipAuthoring {
                parent_stack:  vec![0],
                next_auto_id:  self.authoring.next_auto_id,
                unstyled_text: self.authoring.unstyled_text.clone(),
            },
            drawn_label_size:              self.drawn_label_size,
            font_unit:                     self.font_unit,
            ink_padding:                   self.ink_padding,
            show_after:                    self.show_after,
            hide_after:                    self.hide_after,
            disabled_policy:               self.disabled_policy,
            exclusivity:                   self.exclusivity,
            show_over_descendant_controls: self.show_over_descendant_controls,
            source_anchor:                 self.source_anchor,
            target_anchor:                 self.target_anchor,
            offset:                        self.offset,
        }
    }
}

impl PartialEq for Tooltip {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.blueprint, &other.blueprint)
            && self.show_after == other.show_after
            && self.hide_after == other.hide_after
            && self.disabled_policy == other.disabled_policy
            && self.exclusivity == other.exclusivity
            && self.show_over_descendant_controls == other.show_over_descendant_controls
            && self.source_anchor == other.source_anchor
            && self.target_anchor == other.target_anchor
            && self.offset == other.offset
            && self.drawn_label_size == other.drawn_label_size
            && self.font_unit == other.font_unit
            && self.ink_padding == other.ink_padding
    }
}

impl Tooltip {
    /// Creates a deferred `Fit` by `Fit` tooltip with `root` as its visible root.
    ///
    /// A tooltip describes a widget; it does not contain one. Its root, its
    /// containers, and its leaves are ordinary layout:
    ///
    /// ```
    /// use bevy::asset::Handle;
    /// use bevy::color::Color;
    /// use bevy::image::Image;
    /// use hana_diegetic::El;
    /// use hana_diegetic::TextStyle;
    /// use hana_diegetic::Tooltip;
    ///
    /// fn described() -> Tooltip {
    ///     let mut tooltip = Tooltip::new(El::column());
    ///     tooltip.with(El::new(), |tooltip| {
    ///         tooltip.text("plain");
    ///         tooltip.text(("styled", TextStyle::default()));
    ///     });
    ///     tooltip.image(El::new(), Handle::<Image>::default(), Color::WHITE);
    ///     tooltip
    /// }
    /// ```
    ///
    /// The tooltip above is what keeps this case and the ones on
    /// [`Self::with`], [`Self::text`], and [`Self::image`] meaningful — a
    /// rename would break that one loudly rather than leaving these succeeding
    /// for an unrelated reason:
    ///
    /// ```compile_fail,E0308
    /// use hana_diegetic::{Button, El, Tooltip};
    ///
    /// let widget = El::new().button("nested");
    /// let _ = Tooltip::new(widget);
    /// ```
    pub fn new<L>(root: El<L, LayoutOnly>) -> Self
    where
        L: ChildLayoutState,
    {
        let mut tooltip = Self {
            blueprint:                     Arc::new(
                LayoutBuilder::with_root(
                    materialization::resolve_tooltip_body_look(root).opaque_tooltip_surface(),
                )
                .build(),
            ),
            authoring:                     TooltipAuthoring {
                parent_stack:  vec![0],
                next_auto_id:  0,
                unstyled_text: Vec::new(),
            },
            drawn_label_size:              Dimension::from(TextStyle::default().size()),
            font_unit:                     TooltipFontUnit::FixedLayoutOrFitPoints,
            ink_padding:                   TooltipInkPadding::WidgetFrame,
            show_after:                    DEFAULT_SHOW_DELAY,
            hide_after:                    Duration::ZERO,
            disabled_policy:               TooltipDisabledPolicy::Suppress,
            exclusivity:                   TooltipExclusivity::Independent,
            show_over_descendant_controls: false,
            source_anchor:                 Anchor::TopCenter,
            target_anchor:                 Anchor::BottomCenter,
            offset:                        TooltipOffset::DefaultGap,
        };
        materialization::resolve_tooltip_look(
            Arc::make_mut(&mut tooltip.blueprint),
            &[],
            tooltip.drawn_label_size,
        );
        tooltip
    }

    /// Sets the size at which the surrounding row labels are drawn.
    /// Plain tooltip text uses 85% of this size in the same layout unit.
    #[must_use]
    pub fn with_label_size(mut self, size: impl Into<Dimension>) -> Self {
        self.drawn_label_size = size.into();
        materialization::resolve_tooltip_look(
            Arc::make_mut(&mut self.blueprint),
            &self.authoring.unstyled_text,
            self.drawn_label_size,
        );
        self
    }

    /// Interprets unqualified text sizes in this unit, including on a fitting tooltip.
    #[must_use]
    pub const fn font_unit(mut self, unit: Unit) -> Self {
        self.font_unit = TooltipFontUnit::Explicit(unit);
        self
    }

    /// Pads an otherwise unpadded root from its visible words instead of its line box.
    /// Unitless gaps use the tooltip's font unit.
    #[must_use]
    pub fn ink_padding(mut self, gap: impl Into<Dimension>) -> Self {
        self.ink_padding = TooltipInkPadding::Uniform(gap.into());
        self
    }

    /// Keeps this tooltip eligible while an interactive descendant is hovered.
    /// Useful when that descendant is another part of the same described item.
    #[must_use]
    pub const fn show_over_descendant_controls(mut self) -> Self {
        self.show_over_descendant_controls = true;
        self
    }

    /// Keeps this help as the sole materialized tooltip while it is shown.
    #[must_use]
    pub const fn one_help_at_a_time(mut self) -> Self {
        self.exclusivity = TooltipExclusivity::SoleHelp;
        self
    }

    /// Adds a visual container and authors its descendants through this tooltip.
    ///
    /// A container is ordinary layout. See [`Self::new`] for the passing case
    /// that keeps this one meaningful.
    ///
    /// ```compile_fail,E0308
    /// use hana_diegetic::{Button, El, Tooltip};
    ///
    /// let mut tooltip = Tooltip::new(El::new());
    /// tooltip.with(El::new().button("nested"), |_| {});
    /// ```
    pub fn with<L>(
        &mut self,
        element: El<L, LayoutOnly>,
        children: impl FnOnce(&mut Self),
    ) -> &mut Self
    where
        L: ChildLayoutState,
    {
        let parent = self.current_parent();
        let child = Arc::make_mut(&mut self.blueprint).tooltip_add_container(parent, element);
        self.authoring.parent_stack.push(child);
        children(self);
        self.authoring.parent_stack.pop();
        self
    }

    /// Adds a text leaf under the current tooltip container.
    ///
    /// The run's layout element is ordinary layout, like every other part of a
    /// tooltip; a run laid out on a widget cannot become one. See
    /// [`Self::new`] for the passing case that keeps this one meaningful.
    ///
    /// ```compile_fail,E0277
    /// use hana_diegetic::{Button, El, Text, TextStyle, Tooltip};
    ///
    /// let widget_text =
    ///     Text::new("nested", TextStyle::default()).layout(El::new().button("nested"));
    /// let mut tooltip = Tooltip::new(El::new());
    /// tooltip.text(widget_text);
    /// ```
    pub fn text(&mut self, text: impl Into<Text<LayoutOnly>>) -> &mut Self {
        let parent = self.current_parent();
        let text = text.into();
        let unstyled = text.is_unstyled();
        let index = Arc::make_mut(&mut self.blueprint).tooltip_add_text(
            parent,
            text,
            &mut self.authoring.next_auto_id,
        );
        if unstyled {
            self.authoring.unstyled_text.push(index);
            materialization::resolve_tooltip_look(
                Arc::make_mut(&mut self.blueprint),
                &self.authoring.unstyled_text,
                self.drawn_label_size,
            );
        }
        self
    }

    /// Adds an image leaf under the current tooltip container.
    ///
    /// An image leaf is ordinary layout. See [`Self::new`] for the passing case
    /// that keeps this one meaningful.
    ///
    /// ```compile_fail,E0308
    /// use bevy::asset::Handle;
    /// use bevy::color::Color;
    /// use bevy::image::Image;
    /// use hana_diegetic::{Button, El, Tooltip};
    ///
    /// let mut tooltip = Tooltip::new(El::new());
    /// tooltip.image(
    ///     El::new().button("nested"),
    ///     Handle::<Image>::default(),
    ///     Color::WHITE,
    /// );
    /// ```
    pub fn image<L>(
        &mut self,
        element: El<L, LayoutOnly>,
        handle: Handle<Image>,
        tint: Color,
    ) -> &mut Self
    where
        L: ChildLayoutState,
    {
        let parent = self.current_parent();
        Arc::make_mut(&mut self.blueprint).tooltip_add_image(parent, element, handle, tint);
        self
    }

    /// Sets the delay before an eligible tooltip is shown.
    #[must_use]
    pub const fn show_after(mut self, delay: Duration) -> Self {
        self.show_after = delay;
        self
    }

    /// Sets the delay before an ineligible tooltip is hidden.
    #[must_use]
    pub const fn hide_after(mut self, delay: Duration) -> Self {
        self.hide_after = delay;
        self
    }

    /// Sets disabled-target behavior.
    #[must_use]
    pub const fn disabled_policy(mut self, policy: TooltipDisabledPolicy) -> Self {
        self.disabled_policy = policy;
        self
    }

    /// Sets the tooltip anchor used for placement.
    #[must_use]
    pub const fn source_anchor(mut self, anchor: Anchor) -> Self {
        self.source_anchor = anchor;
        self
    }

    /// Sets the described target anchor used for placement.
    #[must_use]
    pub const fn target_anchor(mut self, anchor: Anchor) -> Self {
        self.target_anchor = anchor;
        self
    }

    /// Sets the offset from the described target. Without one, the tooltip keeps
    /// a gap out from the side its anchors name.
    #[must_use]
    pub const fn offset(mut self, offset: PanelAnchorOffset) -> Self {
        self.offset = TooltipOffset::Authored(offset);
        self
    }

    #[cfg(test)]
    fn tree(&self) -> &LayoutTree { &self.blueprint }

    pub(super) const fn blueprint(&self) -> &Arc<LayoutTree> { &self.blueprint }

    /// Default-styled text follows the source label's drawn world scale.
    pub(super) const fn has_source_label_sized_text(&self) -> bool {
        !self.authoring.unstyled_text.is_empty()
    }

    pub(super) fn attachment(&self) -> PanelAttachment {
        let offset = match self.offset {
            TooltipOffset::DefaultGap => {
                placement::default_gap_offset(self.source_anchor, self.target_anchor)
            },
            TooltipOffset::Authored(offset) => offset,
        };
        PanelAttachment::new(self.source_anchor, self.target_anchor).with_offset(offset)
    }

    fn current_parent(&self) -> usize { self.authoring.parent_stack.last().copied().unwrap_or(0) }
}

#[derive(Clone, Copy, Component)]
pub(super) struct PrepareTooltip;

#[derive(Clone, Copy, Component)]
pub(super) struct AuthoredTooltipTargetSpace(pub(super) PanelSpace);

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use bevy::asset::Handle;
    use bevy::color::Color;
    use bevy::image::Image;

    use super::*;
    use crate::PanelElementId;
    use crate::layout::Anchor;
    use crate::layout::Border;
    use crate::layout::CornerRadius;
    use crate::layout::El;
    use crate::layout::LayoutBuilder;
    use crate::layout::LayoutTreeChange;
    use crate::layout::Padding;
    use crate::layout::Px;
    use crate::layout::Text;
    use crate::panel::PanelAnchorOffset;
    use crate::widgets::constants::DEFAULT_TOOLTIP_GAP;

    #[test]
    fn defaults_and_clone_identity_match_the_public_contract() {
        let tooltip = Tooltip::new(El::new());
        let clone = tooltip.clone();
        let policy = clone.clone().show_after(Duration::from_secs(1));

        assert_eq!(tooltip, clone);
        assert!(Arc::ptr_eq(&tooltip.blueprint, &clone.blueprint));
        assert!(Arc::ptr_eq(&tooltip.blueprint, &policy.blueprint));
        assert_eq!(tooltip.show_after, DEFAULT_SHOW_DELAY);
        assert_eq!(tooltip.hide_after, Duration::ZERO);
        assert_eq!(tooltip.disabled_policy, TooltipDisabledPolicy::Suppress);
        assert_eq!(tooltip.source_anchor, Anchor::TopCenter);
        assert_eq!(tooltip.target_anchor, Anchor::BottomCenter);
        assert_eq!(
            tooltip.attachment().offset(),
            PanelAnchorOffset::new(Px(0.0), Px(DEFAULT_TOOLTIP_GAP))
        );
    }

    #[test]
    fn default_gap_follows_the_side_the_anchors_name() {
        let gap = DEFAULT_TOOLTIP_GAP;
        let cases = [
            (Anchor::TopCenter, Anchor::BottomCenter, Px(0.0), Px(gap)),
            (Anchor::BottomCenter, Anchor::TopCenter, Px(0.0), Px(-gap)),
            (Anchor::CenterLeft, Anchor::CenterRight, Px(gap), Px(0.0)),
            (Anchor::CenterRight, Anchor::CenterLeft, Px(-gap), Px(0.0)),
            (Anchor::Center, Anchor::Center, Px(0.0), Px(gap)),
        ];
        for (source, target, x, y) in cases {
            let tooltip = Tooltip::new(El::new())
                .source_anchor(source)
                .target_anchor(target);
            assert_eq!(
                tooltip.attachment().offset(),
                PanelAnchorOffset::new(x, y),
                "{source:?} to {target:?}"
            );
        }
    }

    #[test]
    fn authored_offset_stays_as_authored_on_any_side() {
        let authored = PanelAnchorOffset::new(Px(3.0), Px(5.0));
        let tooltip = Tooltip::new(El::new())
            .offset(authored)
            .source_anchor(Anchor::CenterRight)
            .target_anchor(Anchor::CenterLeft);
        assert_eq!(tooltip.attachment().offset(), authored);
    }

    #[test]
    fn blueprint_mutation_uses_copy_on_write() {
        let tooltip = Tooltip::new(El::new());
        let mut changed = tooltip.clone();
        assert!(Arc::ptr_eq(&tooltip.blueprint, &changed.blueprint));

        changed.text("details");

        assert_ne!(tooltip, changed);
        assert!(!Arc::ptr_eq(&tooltip.blueprint, &changed.blueprint));
        assert_eq!(tooltip.tree().len(), 1);
        assert_eq!(changed.tree().len(), 2);
    }

    #[test]
    fn clone_captured_during_nested_authoring_resumes_at_the_root() {
        let mut tooltip = Tooltip::new(El::column());
        let mut captured = None;
        tooltip.with(El::row(), |tooltip| {
            tooltip.text("nested");
            captured = Some(tooltip.clone());
        });
        assert!(captured.is_some());
        let Some(mut captured) = captured else {
            return;
        };

        captured.text("captured root child");
        tooltip.text("original root child");

        let mut expected_captured = LayoutBuilder::with_root(
            materialization::resolve_tooltip_body_look(El::column()).opaque_tooltip_surface(),
        );
        expected_captured.with(El::row(), |builder| {
            builder.text("nested");
        });
        expected_captured.text("captured root child");
        let mut expected_captured = expected_captured.build();
        materialization::resolve_tooltip_look(
            &mut expected_captured,
            &[2, 3],
            captured.drawn_label_size,
        );

        let mut expected_original = LayoutBuilder::with_root(
            materialization::resolve_tooltip_body_look(El::column()).opaque_tooltip_surface(),
        );
        expected_original.with(El::row(), |builder| {
            builder.text("nested");
        });
        expected_original.text("original root child");
        let mut expected_original = expected_original.build();
        materialization::resolve_tooltip_look(
            &mut expected_original,
            &[2, 3],
            tooltip.drawn_label_size,
        );

        assert_eq!(
            captured.tree().classify_change(&expected_captured),
            LayoutTreeChange::Identical
        );
        assert_eq!(
            tooltip.tree().classify_change(&expected_original),
            LayoutTreeChange::Identical
        );
        assert_eq!(
            captured.tree().element_id(2),
            Some(&PanelElementId::auto(0))
        );
        assert_eq!(
            captured.tree().element_id(3),
            Some(&PanelElementId::auto(1))
        );
        assert!(!Arc::ptr_eq(&tooltip.blueprint, &captured.blueprint));
    }

    #[test]
    fn nested_authoring_restores_the_parent_cursor() {
        let mut tooltip = Tooltip::new(El::column());
        tooltip.with(El::row(), |tooltip| {
            tooltip.text("nested");
        });
        tooltip.text("root child");

        assert_eq!(tooltip.tree().len(), 4);
        assert_eq!(tooltip.tree().field_display_text(1), Some("nested"));
        assert_eq!(tooltip.tree().element_text(2), Some("nested"));
        assert_eq!(tooltip.tree().element_text(3), Some("root child"));
    }

    #[test]
    fn tooltip_visual_authoring_matches_a_default_styled_layout_tree() {
        let root = El::column()
            .gap(Px(3.0))
            .background(Color::BLACK)
            .corner_radius(4.0);
        let row = El::row()
            .gap(Px(2.0))
            .background(Color::srgb(0.1, 0.2, 0.3));
        let overlay = El::overlay()
            .size(Px(18.0), Px(12.0))
            .background(Color::srgb(0.3, 0.2, 0.1));
        let image = El::new().size(Px(8.0), Px(6.0)).corner_radius(2.0);
        let text_style = crate::TextStyle::new(Px(11.0)).with_color(Color::srgb(0.8, 0.9, 1.0));
        let image_handle = Handle::<Image>::default();
        let image_tint = Color::srgb(0.7, 0.8, 0.9);

        let mut tooltip = Tooltip::new(root.clone());
        tooltip.with(row.clone(), |tooltip| {
            tooltip.text(Text::new("details", text_style.clone()));
            tooltip.with(overlay.clone(), |tooltip| {
                tooltip.image(image.clone(), image_handle.clone(), image_tint);
            });
        });

        let mut builder = LayoutBuilder::with_root(
            materialization::resolve_tooltip_body_look(root).opaque_tooltip_surface(),
        );
        builder.with(row, |builder| {
            builder.text(Text::new("details", text_style));
            builder.with(overlay, |builder| {
                builder.image(image, image_handle, image_tint);
            });
        });
        let mut ordinary = builder.build();
        materialization::resolve_tooltip_look(&mut ordinary, &[], tooltip.drawn_label_size);

        assert_eq!(
            tooltip.tree().classify_change(&ordinary),
            LayoutTreeChange::Identical
        );
        assert_eq!(
            ordinary.classify_change(tooltip.tree()),
            LayoutTreeChange::Identical
        );
    }

    #[test]
    fn authored_root_properties_survive_default_resolution_independently() {
        let red = Color::srgb(1.0, 0.0, 0.0);
        let blue = Color::srgb(0.0, 0.0, 1.0);
        let border = Border::all(Px(2.0), blue);
        let root = El::column()
            .background(red)
            .padding(Padding::all(Px(20.0)))
            .border(border)
            .corner_radius(CornerRadius::ZERO);
        let tooltip = Tooltip::new(root.clone());
        let expected = LayoutBuilder::with_root(root.opaque_tooltip_surface()).build();
        assert_eq!(
            tooltip.tree().classify_change(&expected),
            LayoutTreeChange::Identical
        );
    }
}
