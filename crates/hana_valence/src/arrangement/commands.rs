use core::fmt::Debug;
use core::hash::Hash;

use bevy_ecs::entity::Entity;
use bevy_ecs::error::warn;
use bevy_ecs::prelude::Commands;
use bevy_ecs::world::World;
use bevy_scene::Scene;

use super::ArrangementError;
use super::ArrangementMemberEntities;
use super::ArrangementProvider;
use super::MemberBinding;
use super::materialize;
use crate::FoldRecipe;
use crate::fold;

/// Arrangement-construction commands available on Bevy [`Commands`].
///
/// The two commands consume one provider member enumeration in its supplied
/// deterministic order, construct the crate-owned
/// [`ArrangementMemberEntities`] association exactly once, and validate the
/// provider's typed plan before they write relationships or run member-scene
/// factories. The controller and, for [`Self::spawn_arrangement`], member-root
/// entity IDs are reserved up front so a successful return can identify the
/// controller immediately.
///
/// Synchronous association, provider, and plan failures preserve their
/// [`ArrangementError`] variant and provider source. They queue cleanup for
/// every reserved entity and never call a member-scene factory. Once deferred
/// commands begin applying, an application-owned entity that has since been
/// removed is skipped: an existing binding that is gone by then is neither
/// recreated nor checked in advance.
pub trait ArrangementCommandsExt {
    /// Reserves and materializes an arrangement controller plus one root per provider member.
    ///
    /// `P` supplies logical member values and one typed connection forest.
    /// `F` receives each logical member in provider order only after the
    /// complete association and plan are valid. It returns `S`, a Bevy
    /// [`Scene`] applied onto that member's already-reserved root with
    /// `queue_apply_scene`; this command never spawns a second scene root.
    /// `()` is therefore the empty-scene choice. Entities related by a scene
    /// are not arrangement members unless they separately receive [`super::Member`].
    ///
    /// Queued scenes apply in the same frame only when their dependencies are
    /// ready and deferred commands run before Bevy's scene-spawn system.
    /// Otherwise Bevy retains the queued scene and applies it after its
    /// dependencies become ready.
    ///
    /// # Errors
    ///
    /// Returns the exact association, provider, or plan error. On any such
    /// synchronous failure, every controller/member ID reserved by this call
    /// is queued for cleanup and `scene_for_member` is never invoked.
    fn spawn_arrangement<P, F, S>(
        &mut self,
        provider: P,
        scene_for_member: F,
    ) -> Result<Entity, ArrangementError>
    where
        P: ArrangementProvider,
        F: FnMut(&P::Member) -> S,
        S: Scene;

    /// Materializes an arrangement controller around entities the application already owns.
    ///
    /// `P` has the same logical-member and typed-plan role as in
    /// [`Self::spawn_arrangement`]. `F` is called once for every listed logical
    /// member before any relationship is inserted. It returns [`MemberBinding`]:
    /// [`MemberBinding::Bound`] names the existing root to receive membership,
    /// while [`MemberBinding::Missing`] is a required-member failure, not an
    /// optional omission. All bindings are consumed before association or plan
    /// validation begins, so a failure cannot leave a partial membership write.
    ///
    /// Bound entities are not checked in advance. A bound entity removed
    /// by application code before deferred commands apply remains absent; this
    /// command neither restores it nor creates a replacement member or scene.
    ///
    /// # Errors
    ///
    /// Returns the first missing binding, duplicate bound entity, association,
    /// provider, or plan error. The reserved controller is queued for cleanup
    /// on every synchronous failure.
    fn spawn_arrangement_from_members<P, F>(
        &mut self,
        provider: P,
        binding_for_member: F,
    ) -> Result<Entity, ArrangementError>
    where
        P: ArrangementProvider,
        F: FnMut(&P::Member) -> MemberBinding;

    /// Replaces every folded endpoint of one selected fold-group set from a recipe.
    ///
    /// `selection` names the retained fold-group alternative the provider filed
    /// under its own selection type, and `recipe` derives one endpoint per
    /// selected member. The recipe is transient: it is consumed once, here, and
    /// the arrangement retains only the resulting hinge endpoints.
    ///
    /// Applying the recipe runs entirely inside a deferred command, so this
    /// call may be issued in the same
    /// `Commands` batch as [`Self::spawn_arrangement`] or
    /// [`Self::spawn_arrangement_from_members`] on the controller they return.
    /// Every read, capability retrieval, and validation happens before the first
    /// hinge write, so a selection, capability, coverage, or base-endpoint
    /// failure warns through Bevy's command-error handler and leaves every
    /// current hinge unchanged. Once writing begins, an application-owned member
    /// that has since disappeared is skipped rather than recreated.
    fn apply_fold_recipe<S, R>(&mut self, arrangement_entity: Entity, selection: S, recipe: R)
    where
        S: Eq + Hash + Debug + Send + Sync + 'static,
        R: FoldRecipe + Send + 'static;
}

impl ArrangementCommandsExt for Commands<'_, '_> {
    fn spawn_arrangement<P, F, S>(
        &mut self,
        provider: P,
        mut scene_for_member: F,
    ) -> Result<Entity, ArrangementError>
    where
        P: ArrangementProvider,
        F: FnMut(&P::Member) -> S,
        S: Scene,
    {
        let controller = self.spawn_empty().id();
        let reserved_members = provider
            .members()
            .map(|member| (member, self.spawn_empty().id()))
            .collect::<Vec<_>>();
        let reserved_entities = reserved_members
            .iter()
            .map(|(_, entity)| *entity)
            .collect::<Vec<_>>();
        let members = match ArrangementMemberEntities::try_new(reserved_members) {
            Ok(members) => members,
            Err(error) => {
                return Err(clean_up_reserved(
                    self,
                    controller,
                    &reserved_entities,
                    error,
                ));
            },
        };
        let plan = match provider.generate_plan(&members) {
            Ok(plan) => plan,
            Err(error) => {
                return Err(clean_up_reserved(
                    self,
                    controller,
                    &reserved_entities,
                    error,
                ));
            },
        };
        let scenes = members
            .iter()
            .map(|(member, _)| scene_for_member(member))
            .collect::<Vec<_>>();

        materialize::queue_arrangement(self, controller, members, plan, scenes);
        Ok(controller)
    }

    fn spawn_arrangement_from_members<P, F>(
        &mut self,
        provider: P,
        mut binding_for_member: F,
    ) -> Result<Entity, ArrangementError>
    where
        P: ArrangementProvider,
        F: FnMut(&P::Member) -> MemberBinding,
    {
        let controller = self.spawn_empty().id();
        let bindings = provider
            .members()
            .map(|member| {
                let binding = binding_for_member(&member);
                (member, binding)
            })
            .collect::<Vec<_>>();
        let first_missing = bindings.iter().find_map(|(member, binding)| {
            matches!(binding, MemberBinding::Missing).then(|| format!("{member:?}"))
        });
        if let Some(member) = first_missing {
            return Err(clean_up_reserved(
                self,
                controller,
                &[],
                ArrangementError::MissingMemberBinding { member },
            ));
        }
        let members = bindings
            .into_iter()
            .filter_map(|(member, binding)| match binding {
                MemberBinding::Bound(entity) => Some((member, entity)),
                MemberBinding::Missing => None,
            })
            .collect::<Vec<_>>();
        let members = match ArrangementMemberEntities::try_new(members) {
            Ok(members) => members,
            Err(error) => return Err(clean_up_reserved(self, controller, &[], error)),
        };
        let plan = match provider.generate_plan(&members) {
            Ok(plan) => plan,
            Err(error) => return Err(clean_up_reserved(self, controller, &[], error)),
        };

        materialize::queue_arrangement_without_scenes(self, controller, members, plan);
        Ok(controller)
    }

    fn apply_fold_recipe<S, R>(&mut self, arrangement_entity: Entity, selection: S, recipe: R)
    where
        S: Eq + Hash + Debug + Send + Sync + 'static,
        R: FoldRecipe + Send + 'static,
    {
        self.queue_handled(
            move |world: &mut World| {
                fold::apply_fold_recipe(world, arrangement_entity, &selection, &recipe)
            },
            warn,
        );
    }
}

fn clean_up_reserved(
    commands: &mut Commands<'_, '_>,
    controller: Entity,
    members: &[Entity],
    error: ArrangementError,
) -> ArrangementError {
    commands.entity(controller).despawn();
    for member in members {
        commands.entity(*member).despawn();
    }
    error
}
