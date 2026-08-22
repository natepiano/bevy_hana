use bevy::prelude::Add;
use bevy::prelude::ChildOf;
use bevy::prelude::Commands;
use bevy::prelude::Entity;
use bevy::prelude::EntityEvent;
use bevy::prelude::On;
use bevy::prelude::World;

use crate::cascade::Cascade;
use crate::cascade::CascadeAttribute;
use crate::cascade::CascadeFrom;
use crate::cascade::HdrTextCoverageBias;
use crate::cascade::TextMaterial;
use crate::layout::GlyphShadowMode;
use crate::layout::Lighting;
use crate::layout::ShadowCasting;
use crate::layout::Sidedness;
use crate::render::AntiAlias;
use crate::render::world_text::TextContent;

/// Connects a panel label to its panel's glyph-attribute cascades.
///
/// Fires when a label first gains [`TextContent`] and queues explicit
/// [`CascadeFrom`] construction. After the surrounding construction commands
/// apply, it inserts inheriting components only for attributes that still have
/// no authored value. Label values from `TextStyle` therefore win on initial
/// construction. `CascadePlugin<A>` follows `CascadeFrom` to the panel and
/// maintains the matching `Resolved<A>` cache.
pub(super) fn seed_panel_text_child_glyph(trigger: On<Add, TextContent>, mut commands: Commands) {
    let entity = trigger.event_target();
    commands.queue(move |world: &mut World| {
        let Some(panel) = world.get::<ChildOf>(entity).map(ChildOf::parent) else {
            return;
        };
        world.entity_mut(entity).insert(CascadeFrom::new(panel));
        insert_inherit_if_missing::<TextMaterial>(world, entity);
        insert_inherit_if_missing::<Lighting>(world, entity);
        insert_inherit_if_missing::<Sidedness>(world, entity);
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
