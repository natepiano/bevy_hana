use bevy::prelude::Add;
use bevy::prelude::ChildOf;
use bevy::prelude::Commands;
use bevy::prelude::EntityEvent;
use bevy::prelude::On;
use bevy::prelude::World;

use crate::cascade::Cascade;
use crate::cascade::CascadeFrom;
use crate::cascade::TextAlpha;
use crate::render::world_text::TextContent;

/// Connects a panel label to its panel's text-alpha cascade.
///
/// A panel label (a `TextContent` run) is depth-2 under its panel. This
/// observer fires when a label first gains [`TextContent`] and queues explicit
/// [`CascadeFrom`] construction. After the surrounding construction commands
/// apply, it inserts inheriting `Cascade<TextAlpha>` only when the label still
/// has no authored value, so `TextStyle::with_alpha_mode` always wins. The standalone
/// `seed_world_text_overrides` bridge skips labels (its `Without<TextContent>`
/// filter), so labels participate only through this bridge.
pub(super) fn seed_panel_text_child_alpha(trigger: On<Add, TextContent>, mut commands: Commands) {
    let entity = trigger.event_target();
    commands.queue(move |world: &mut World| {
        let Some(panel) = world.get::<ChildOf>(entity).map(ChildOf::parent) else {
            return;
        };
        world.entity_mut(entity).insert(CascadeFrom::new(panel));
        if world.get::<Cascade<TextAlpha>>(entity).is_none() {
            world
                .entity_mut(entity)
                .insert(Cascade::<TextAlpha>::Inherit);
        }
    });
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::sync::Arc;

    use bevy::prelude::AlphaMode;
    use bevy::prelude::App;
    use bevy::prelude::Entity;
    use bevy::prelude::IntoScheduleConfigs;
    use bevy::prelude::MinimalPlugins;
    use bevy::prelude::PostUpdate;
    use bevy::prelude::Query;
    use bevy::prelude::Res;
    use bevy::prelude::ResMut;
    use bevy::prelude::Resource;
    use bevy::prelude::Update;
    use bevy::prelude::With;

    use super::*;
    use crate::CascadeEntityCommandsExt as _;
    use crate::Fit;
    use crate::LayoutBuilder;
    use crate::Mm;
    use crate::TextContent;
    use crate::TextStyle;
    use crate::cascade;
    use crate::cascade::CascadeSet;
    use crate::cascade::Resolved;
    use crate::cascade::TextAlpha;
    use crate::layout::LayoutTree;
    use crate::layout::TextDimensions;
    use crate::layout::TextMeasure;
    use crate::panel::DiegeticPanel;
    use crate::panel::DiegeticPanelCommands;
    use crate::panel::HeadlessLayoutPlugin;
    use crate::render::panel_text::reify;
    use crate::text::DiegeticTextMeasurer;

    /// Panel that the label-spawning system parents its label under.
    #[derive(Resource)]
    struct TestPanel(Entity);

    /// Records the alpha the reader observed for the label on the first frame
    /// the label exists.
    #[derive(Resource, Default)]
    struct SeenLabelAlpha(Option<AlphaMode>);

    fn measurer() -> DiegeticTextMeasurer {
        DiegeticTextMeasurer {
            measure_fn: Arc::new(|_text: &str, measure: &TextMeasure| TextDimensions {
                width:       measure.size,
                height:      measure.size,
                line_height: measure.size,
            }),
        }
    }

    /// Stand-in for `reify_text_entities`: parents one label under
    /// the panel during a command flush, once the panel carries its
    /// `Cascade<TextAlpha>`.
    fn spawn_label_once(
        panel: Res<TestPanel>,
        ready: Query<(), (With<DiegeticPanel>, With<Cascade<TextAlpha>>)>,
        labels: Query<(), With<TextContent>>,
        mut commands: Commands,
    ) {
        if !labels.is_empty() || ready.get(panel.0).is_err() {
            return;
        }
        commands
            .entity(panel.0)
            .with_child(TextContent::new("label"));
    }

    /// Stand-in for `update_panel_text_batches`' alpha read: the cached
    /// `Resolved<TextAlpha>` else the global-default fallback. Records the
    /// first observation so the assertion sees the spawn frame, not a later
    /// settled one.
    fn read_label_alpha(
        resolved_alphas: Query<&Resolved<TextAlpha>, With<TextContent>>,
        labels: Query<Entity, With<TextContent>>,
        default: Res<TextAlpha>,
        mut seen: ResMut<SeenLabelAlpha>,
    ) {
        if seen.0.is_some() {
            return;
        }
        for entity in &labels {
            seen.0 = Some(
                resolved_alphas
                    .get(entity)
                    .map_or(default.0, |resolved| resolved.0.0),
            );
        }
    }

    #[test]
    fn label_inherits_overridden_panel_alpha_same_frame_as_spawn() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(measurer())
            .add_plugins(HeadlessLayoutPlugin)
            .add_plugins(cascade::cascade_plugin::<TextAlpha>())
            .add_observer(seed_panel_text_child_alpha)
            .init_resource::<SeenLabelAlpha>();

        // Panel override is `Add`; the global default stays `Blend`, so the
        // reader's fallback would mask an ordering regression only if the two
        // were equal — they are not.
        let panel = DiegeticPanel::world()
            .size(Mm(50.0), Mm(30.0))
            .text_alpha_mode(AlphaMode::Add)
            .layout(|b| {
                b.text(("Hi", TextStyle::new(Mm(6.0))));
            })
            .build()
            .expect("test panel should build");
        let panel_entity = app.world_mut().spawn(panel).id();
        app.insert_resource(TestPanel(panel_entity));
        app.add_systems(
            Update,
            (
                spawn_label_once.before(CascadeSet::Propagate),
                read_label_alpha.after(CascadeSet::Propagate),
            ),
        );

        for _ in 0..5 {
            app.update();
        }

        // `spawn_label_once` runs before `CascadeSet::Propagate`, so the reader
        // sees the panel's overridden `Add`, not the `Blend` fallback.
        assert_eq!(
            app.world().resource::<SeenLabelAlpha>().0,
            Some(AlphaMode::Add)
        );
    }

    #[test]
    fn panel_inheritance_survives_layout_tree_replacement() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(measurer())
            .add_plugins(HeadlessLayoutPlugin)
            .add_plugins(cascade::cascade_plugin::<TextAlpha>())
            .insert_resource(TextAlpha(AlphaMode::Blend))
            .add_observer(seed_panel_text_child_alpha)
            .add_systems(PostUpdate, reify::reify_text_entities);

        let panel = app
            .world_mut()
            .spawn(
                DiegeticPanel::world()
                    .size(Mm(50.0), Mm(30.0))
                    .text_alpha_mode(AlphaMode::Add)
                    .with_tree(alpha_panel_tree("initial"))
                    .build()
                    .expect("panel should build"),
            )
            .id();
        for _ in 0..3 {
            app.update();
        }

        assert_eq!(
            app.world()
                .get::<Cascade<TextAlpha>>(panel)
                .expect("panel should carry authored text alpha"),
            &Cascade::Override(TextAlpha(AlphaMode::Add))
        );
        assert_eq!(
            app.world()
                .get::<Resolved<TextAlpha>>(panel)
                .expect("panel should resolve its construction seed")
                .0
                .0,
            AlphaMode::Add
        );
        assert_eq!(single_label_alpha(&mut app), AlphaMode::Add);

        app.world_mut().resource_mut::<TextAlpha>().0 = AlphaMode::AlphaToCoverage;
        {
            let mut commands = app.world_mut().commands();
            commands.entity(panel).inherit_text_alpha();
            assert!(
                commands
                    .set_tree(panel, alpha_panel_tree("replacement"))
                    .is_ok()
            );
        }
        for _ in 0..2 {
            app.update();
        }

        assert_eq!(
            app.world()
                .get::<Cascade<TextAlpha>>(panel)
                .expect("panel should retain runtime inheritance"),
            &Cascade::Inherit
        );
        assert_eq!(
            app.world()
                .get::<Resolved<TextAlpha>>(panel)
                .expect("panel should resolve the changed global default")
                .0
                .0,
            AlphaMode::AlphaToCoverage
        );
        let label = single_label_entity(&mut app);
        assert_eq!(
            app.world()
                .get::<Cascade<TextAlpha>>(label)
                .expect("label should retain authored inheritance"),
            &Cascade::Inherit
        );
        assert_eq!(single_label_alpha(&mut app), AlphaMode::AlphaToCoverage);
    }

    #[test]
    fn label_resolves_to_global_default_when_panel_has_no_override() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(measurer())
            .add_plugins(HeadlessLayoutPlugin)
            .add_plugins(cascade::cascade_plugin::<TextAlpha>())
            .insert_resource(TextAlpha(AlphaMode::Multiply))
            .add_observer(seed_panel_text_child_alpha);

        // Panel authors no `text_alpha_mode`, so it carries no `Cascade<TextAlpha>`.
        let panel = DiegeticPanel::world()
            .size(Mm(50.0), Mm(30.0))
            .layout(|b| {
                b.text(("Hi", TextStyle::new(Mm(6.0))));
            })
            .build()
            .expect("test panel should build");
        let panel_entity = app.world_mut().spawn(panel).id();

        // `seed_panel_text_child_alpha` inserts `CascadeFrom(panel)`. Because
        // the panel has no override, `CascadePlugin<TextAlpha>` uses
        // `TextAlpha`.
        let label = app
            .world_mut()
            .spawn((TextContent::new("label"), ChildOf(panel_entity)))
            .id();
        app.update();

        assert_eq!(
            app.world()
                .get::<Resolved<TextAlpha>>(label)
                .expect("label should carry Resolved<TextAlpha>")
                .0
                .0,
            AlphaMode::Multiply
        );
    }

    #[test]
    fn screen_panel_labels_keep_the_screen_alpha_default() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(measurer())
            .add_plugins(HeadlessLayoutPlugin)
            .add_plugins(cascade::cascade_plugin::<TextAlpha>())
            .insert_resource(TextAlpha(AlphaMode::AlphaToCoverage))
            .add_observer(seed_panel_text_child_alpha);

        let panel = DiegeticPanel::screen()
            .size(Fit, Fit)
            .build()
            .expect("screen panel should build");
        let panel = app.world_mut().spawn(panel).id();
        let label = app
            .world_mut()
            .spawn((TextContent::new("label"), ChildOf(panel)))
            .id();
        app.update();
        app.update();

        assert_eq!(
            app.world()
                .get::<Resolved<TextAlpha>>(panel)
                .expect("screen panel should resolve text alpha")
                .0
                .0,
            AlphaMode::Blend
        );
        assert_eq!(
            app.world()
                .get::<Resolved<TextAlpha>>(label)
                .expect("screen label should resolve text alpha")
                .0
                .0,
            AlphaMode::Blend
        );
    }

    #[test]
    fn label_with_own_alpha_overrides_inherited_panel_alpha() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(measurer())
            .add_plugins(HeadlessLayoutPlugin)
            .add_plugins(cascade::cascade_plugin::<TextAlpha>())
            .add_observer(seed_panel_text_child_alpha)
            .add_systems(PostUpdate, reify::reify_text_entities);

        // Panel sets `Add` for its labels; the one label authors its own
        // `Multiply` via `TextStyle::with_alpha_mode`. Reification inserts
        // the label's `Cascade<TextAlpha>`, which the walk resolves ahead of
        // the panel's inherited alpha.
        let panel = DiegeticPanel::world()
            .size(Mm(50.0), Mm(30.0))
            .text_alpha_mode(AlphaMode::Add)
            .layout(|b| {
                b.text((
                    "Hi",
                    TextStyle::new(Mm(6.0)).with_alpha_mode(AlphaMode::Multiply),
                ));
            })
            .build()
            .expect("test panel should build");
        app.world_mut().spawn(panel);

        for _ in 0..3 {
            app.update();
        }

        assert_eq!(single_label_alpha(&mut app), AlphaMode::Multiply);
    }

    #[test]
    fn label_alpha_change_reinherits_panel_alpha_through_reification() {
        fn label_tree(alpha: Option<AlphaMode>) -> LayoutTree {
            let mut style = TextStyle::new(13.0);
            if let Some(mode) = alpha {
                style = style.with_alpha_mode(mode);
            }
            let mut builder = LayoutBuilder::new(100.0, 50.0);
            builder.text(("Hi", style));
            builder.build()
        }

        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(measurer())
            .add_plugins(HeadlessLayoutPlugin)
            .add_plugins(cascade::cascade_plugin::<TextAlpha>())
            .add_observer(seed_panel_text_child_alpha)
            .add_systems(PostUpdate, reify::reify_text_entities);

        // Panel inherits `Add`; the label initially authors its own `Multiply`.
        let panel = DiegeticPanel::world()
            .size(Mm(50.0), Mm(30.0))
            .text_alpha_mode(AlphaMode::Add)
            .with_tree(label_tree(Some(AlphaMode::Multiply)))
            .build()
            .expect("test panel should build");
        let panel_entity = app.world_mut().spawn(panel).id();

        for _ in 0..3 {
            app.update();
        }
        assert_eq!(single_label_alpha(&mut app), AlphaMode::Multiply);

        // The label drops its own alpha. Reification removes the label's
        // `Cascade<TextAlpha>` (its update arm), and the propagation pass
        // re-inherits the panel's `Add`.
        assert!(
            app.world_mut()
                .commands()
                .set_tree(panel_entity, label_tree(None))
                .is_ok()
        );
        for _ in 0..3 {
            app.update();
        }
        assert_eq!(single_label_alpha(&mut app), AlphaMode::Add);
    }

    fn alpha_panel_tree(text: &str) -> LayoutTree {
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.text((text, TextStyle::new(13.0)));
        builder.build()
    }

    fn single_label_entity(app: &mut App) -> Entity {
        let mut query = app
            .world_mut()
            .query_filtered::<Entity, With<TextContent>>();
        let labels: Vec<Entity> = query.iter(app.world()).collect();
        assert_eq!(labels.len(), 1, "expected exactly one panel label");
        labels[0]
    }

    /// Resolved alpha of the scene's single panel label.
    fn single_label_alpha(app: &mut App) -> AlphaMode {
        let label = single_label_entity(app);
        app.world()
            .get::<Resolved<TextAlpha>>(label)
            .expect("label should resolve text alpha")
            .0
            .0
    }
}
