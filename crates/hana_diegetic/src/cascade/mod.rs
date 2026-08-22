//! Diegetic cascade attributes and typed public commands.
//!
//! `bevy_kana` owns authored [`Cascade`], the explicit [`CascadeFrom`]
//! relationship, propagation, and [`Resolved`] caches. This module chooses
//! diegetic attributes and exposes domain-specific command and reader names.

mod attributes;
mod cascade_set;
mod constants;
mod defaults;
mod resolved;

pub use attributes::CascadeEntityCommandsExt;
pub use attributes::FontUnit;
pub use attributes::HdrTextCoverageBias;
pub use attributes::SdfMaterial;
pub use attributes::ShapeMaterial;
pub use attributes::TextAlpha;
pub use attributes::TextMaterial;
pub(crate) use attributes::apply_cascade_override;
pub(crate) use attributes::remove_cascade_override;
pub use attributes::resolved_anti_alias;
pub use attributes::resolved_font_unit;
pub use attributes::resolved_glyph_shadow_mode;
pub use attributes::resolved_hairline_fade;
pub use attributes::resolved_hdr_text_coverage_bias;
pub use attributes::resolved_lighting;
pub use attributes::resolved_sdf_material;
pub use attributes::resolved_shadow_casting;
pub use attributes::resolved_shape_material;
pub use attributes::resolved_sidedness;
pub use attributes::resolved_text_alpha;
pub use attributes::resolved_text_material;
pub(crate) use bevy_kana::Cascade;
pub(crate) use bevy_kana::CascadeAttribute;
pub(crate) use bevy_kana::CascadeFrom;
pub(crate) use bevy_kana::CascadePlugin;
pub(crate) use bevy_kana::Resolved;
pub use cascade_set::CascadeSet;
pub(crate) use constants::CASCADE_ATTRIBUTE_BYTES;
pub use defaults::PanelDefaults;
pub(crate) use resolved::CascadeRoot;

pub(crate) fn cascade_plugin<A: CascadeRoot>() -> CascadePlugin<A, A::Root> {
    CascadePlugin::new(A::root_default())
        .with_combine(A::combine)
        .with_root_resource()
}
