use bevy::ecs::system::SystemParam;
use bevy::ecs::world::DeferredWorld;
use bevy::prelude::ChildOf;
use bevy::prelude::Entity;
use bevy::prelude::Query;
use bevy::prelude::Res;
use bevy::prelude::World;
use hana_kana::CascadeRootResource;

use super::PanelTextLayout;
use crate::cascade::Cascade;
use crate::cascade::CascadeAttribute;
use crate::cascade::CascadeFrom;
use crate::cascade::CascadeRoot;
use crate::cascade::HdrTextCoverageBias;
use crate::cascade::Resolved;
use crate::cascade::TextAlpha;
use crate::cascade::TextMaterial;
use crate::layout::GlyphShadowMode;
use crate::layout::Lighting;
use crate::layout::ShadowCasting;
use crate::layout::Sidedness;
use crate::render::AntiAlias;

/// Connects a panel label to its panel's glyph-attribute cascades.
///
/// Reified labels publish their complete cascade bundle and skip this bridge.
/// For manually constructed labels, it fires on [`TextContent`](crate::TextContent) and queues
/// explicit [`CascadeFrom`] construction. After the surrounding construction commands
/// apply, it inserts inheriting components only for attributes that still have
/// no authored value. Label values from `TextStyle` therefore win on initial
/// construction. `CascadePlugin<A>` follows `CascadeFrom` to the panel and
/// maintains the matching `Resolved<A>` cache.
pub(super) fn seed_panel_text_child_glyph(world: &mut DeferredWorld, entity: Entity) {
    if world.get::<PanelTextLayout>(entity).is_some()
        && world.get::<CascadeFrom>(entity).is_some()
        && world.get::<Cascade<TextMaterial>>(entity).is_some()
        && world.get::<Cascade<Lighting>>(entity).is_some()
        && world.get::<Cascade<Sidedness>>(entity).is_some()
        && world.get::<Cascade<ShadowCasting>>(entity).is_some()
        && world.get::<Cascade<GlyphShadowMode>>(entity).is_some()
        && world.get::<Cascade<AntiAlias>>(entity).is_some()
        && world.get::<Cascade<HdrTextCoverageBias>>(entity).is_some()
    {
        return;
    }
    world.commands().queue(move |world: &mut World| {
        let Some(panel) = world.get::<ChildOf>(entity).map(ChildOf::parent) else {
            return;
        };
        world.entity_mut(entity).insert(CascadeFrom::new(panel));
        insert_inherit_if_missing::<TextMaterial>(world, entity);
        insert_inherit_if_missing::<Lighting>(world, entity);
        if world.get::<Cascade<Sidedness>>(entity).is_none() {
            world
                .entity_mut(entity)
                .insert(Cascade::Override(Sidedness::FrontOnly));
        }
        insert_inherit_if_missing::<ShadowCasting>(world, entity);
        insert_inherit_if_missing::<GlyphShadowMode>(world, entity);
        insert_inherit_if_missing::<AntiAlias>(world, entity);
        insert_inherit_if_missing::<HdrTextCoverageBias>(world, entity);
    });
}

fn insert_inherit_if_missing<A: CascadeAttribute>(world: &mut World, entity: Entity) {
    if world.get::<Cascade<A>>(entity).is_none() {
        world.entity_mut(entity).insert(Cascade::<A>::Inherit);
    }
}

/// Panel caches and root resources read once per changed panel, before spawning
/// its text children. Optional resources keep plain headless reification usable.
#[derive(SystemParam)]
pub(super) struct PanelTextCascadeSources<'w, 's> {
    alpha:                  TextCascadeSource<'w, 's, TextAlpha>,
    material:               TextCascadeSource<'w, 's, TextMaterial>,
    lighting:               TextCascadeSource<'w, 's, Lighting>,
    sidedness:              TextCascadeSource<'w, 's, Sidedness>,
    shadow_casting:         TextCascadeSource<'w, 's, ShadowCasting>,
    glyph_shadow_mode:      TextCascadeSource<'w, 's, GlyphShadowMode>,
    anti_alias:             TextCascadeSource<'w, 's, AntiAlias>,
    hdr_text_coverage_bias: TextCascadeSource<'w, 's, HdrTextCoverageBias>,
}

#[derive(SystemParam)]
struct TextCascadeSource<'w, 's, A: CascadeRoot> {
    panels: Query<'w, 's, &'static Resolved<A>>,
    root:   Option<Res<'w, <A as CascadeRoot>::Root>>,
}

impl<A: CascadeRoot> TextCascadeSource<'_, '_, A> {
    fn inherited(&self, panel: Entity) -> A {
        self.panels.get(panel).map_or_else(
            |_| {
                self.root
                    .as_deref()
                    .map_or_else(A::root_default, CascadeRootResource::root)
            },
            |resolved| resolved.0.clone(),
        )
    }
}

/// The inherited values for all children of one panel. Local `TextStyle`
/// overrides resolve against these before the entity's first publication.
pub(super) struct InheritedTextCascades {
    pub(super) alpha:                  TextAlpha,
    pub(super) material:               TextMaterial,
    pub(super) lighting:               Lighting,
    pub(super) sidedness:              Sidedness,
    pub(super) shadow_casting:         ShadowCasting,
    pub(super) glyph_shadow_mode:      GlyphShadowMode,
    pub(super) anti_alias:             AntiAlias,
    pub(super) hdr_text_coverage_bias: HdrTextCoverageBias,
}

impl PanelTextCascadeSources<'_, '_> {
    pub(super) fn for_panel(&self, panel: Entity) -> InheritedTextCascades {
        InheritedTextCascades {
            alpha:                  self.alpha.inherited(panel),
            material:               self.material.inherited(panel),
            lighting:               self.lighting.inherited(panel),
            sidedness:              self.sidedness.inherited(panel),
            shadow_casting:         self.shadow_casting.inherited(panel),
            glyph_shadow_mode:      self.glyph_shadow_mode.inherited(panel),
            anti_alias:             self.anti_alias.inherited(panel),
            hdr_text_coverage_bias: self.hdr_text_coverage_bias.inherited(panel),
        }
    }
}

/// All glyph attributes replace their inherited value. Keep the authored state
/// beside its initial cache; the ordinary cascade propagation still handles edits.
pub(super) fn seeded_text_cascade<A: CascadeRoot>(
    authored: Cascade<A>,
    inherited: &A,
) -> (Cascade<A>, Resolved<A>) {
    let value = authored.as_ref().map_or_else(
        || inherited.clone(),
        |local| A::combine(local.clone(), inherited),
    );
    (authored, Resolved(value))
}
