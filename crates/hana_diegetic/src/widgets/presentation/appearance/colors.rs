use bevy::prelude::Color;

use super::Appearance;

/// A color that replaces a state appearance's background property.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BackgroundColor(
    /// The replacement color.
    pub Color,
);

impl From<Color> for BackgroundColor {
    fn from(color: Color) -> Self { Self(color) }
}

/// A color that replaces a state appearance's border property.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BorderColor(
    /// The replacement color.
    pub Color,
);

impl From<Color> for BorderColor {
    fn from(color: Color) -> Self { Self(color) }
}

/// A color that replaces a state appearance's text property.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextColor(
    /// The replacement color.
    pub Color,
);

impl From<Color> for TextColor {
    fn from(color: Color) -> Self { Self(color) }
}

/// A color that replaces a state appearance's panel-draw property.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PathColor(
    /// The replacement color.
    pub Color,
);

impl From<Color> for PathColor {
    fn from(color: Color) -> Self { Self(color) }
}

/// A color that replaces a state appearance's image tint property.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tint(
    /// The replacement color.
    pub Color,
);

impl From<Color> for Tint {
    fn from(color: Color) -> Self { Self(color) }
}

/// Converts a state appearance bundle or property-specific color into an [`Appearance`].
#[diagnostic::on_unimplemented(
    message = "a bare `Color` does not say which property it sets",
    label = "wrap it: `BackgroundColor({Self})`, `TextColor({Self})`, `BorderColor({Self})`, `PathColor({Self})`, or `Tint({Self})`"
)]
pub trait IntoAppearance {
    /// Returns the corresponding state appearance bundle.
    fn into_appearance(self) -> Appearance;
}

impl IntoAppearance for Appearance {
    fn into_appearance(self) -> Appearance { self }
}

impl IntoAppearance for BackgroundColor {
    fn into_appearance(self) -> Appearance { Appearance::new().background(self.0) }
}

impl IntoAppearance for BorderColor {
    fn into_appearance(self) -> Appearance { Appearance::new().border_color(self.0) }
}

impl IntoAppearance for TextColor {
    fn into_appearance(self) -> Appearance { Appearance::new().text_color(self.0) }
}

impl IntoAppearance for PathColor {
    fn into_appearance(self) -> Appearance { Appearance::new().path_color(self.0) }
}

impl IntoAppearance for Tint {
    fn into_appearance(self) -> Appearance { Appearance::new().tint(self.0) }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use bevy::ecs::system::RunSystemOnce;
    use bevy::prelude::App;
    use bevy::prelude::Color;
    use bevy::prelude::Entity;
    use bevy::prelude::MinimalPlugins;

    use crate::DiegeticPanel;
    use crate::DiegeticTextMeasurer;
    use crate::El;
    use crate::HeadlessLayoutPlugin;
    use crate::LayoutBuilder;
    use crate::Mm;
    use crate::PanelElementId;
    use crate::PanelWidgetReader;
    use crate::cascade::Resolved;
    use crate::widgets::WidgetsPlugin;
    use crate::widgets::presentation::appearance::Appearance;
    use crate::widgets::presentation::appearance::BackgroundColor;
    use crate::widgets::presentation::appearance::BorderColor;
    use crate::widgets::presentation::appearance::PathColor;
    use crate::widgets::presentation::appearance::TextColor;
    use crate::widgets::presentation::appearance::Tint;
    use crate::widgets::presentation::appearance::VisualChange;
    use crate::widgets::presentation::appearance::WidgetHoveredAppearance;

    const HOVER_FILL: Color = Color::srgb(0.1, 0.2, 0.3);
    const FOCUS_BORDER: Color = Color::srgb(0.9, 0.8, 0.2);
    const HOVER_TEXT: Color = Color::srgb(0.7, 0.5, 0.3);
    const FOCUS_PATH: Color = Color::srgb(0.3, 0.5, 0.7);
    const HOVER_TINT: Color = Color::srgb(0.6, 0.4, 0.8);

    macro_rules! assert_resolved_appearance {
        ($app:expr, $widget:expr, $attribute:ty, $expected:expr $(,)?) => {{
            let expected = $expected;
            assert_eq!(
                $app.world()
                    .get::<Resolved<$attribute>>($widget)
                    .map(|resolved| resolved.0.appearance()),
                Some(&expected),
            );
        }};
    }

    fn cascade_test_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(DiegeticTextMeasurer::default())
            .add_plugins((HeadlessLayoutPlugin, WidgetsPlugin));
        app
    }

    fn resolve_widget(app: &mut App, panel: Entity, id: &'static str) -> Entity {
        let id = PanelElementId::named(id);
        let result = app
            .world_mut()
            .run_system_once(move |reader: PanelWidgetReader| reader.entity(panel, &id));
        assert!(result.is_ok());
        let Ok(widget) = result else {
            return Entity::PLACEHOLDER;
        };
        assert!(widget.is_some());
        let Some(widget) = widget else {
            return Entity::PLACEHOLDER;
        };
        widget
    }

    #[test]
    fn background_color_sets_only_the_background_appearance() {
        let mut app = cascade_test_app();
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.with(
            El::new()
                .button("background")
                .hovered(BackgroundColor(HOVER_FILL)),
            |_| {},
        );
        let panel = DiegeticPanel::world()
            .size(Mm(100.0), Mm(50.0))
            .with_tree(builder.build())
            .build()
            .expect("a sized panel should build");
        let panel = app.world_mut().spawn(panel).id();

        app.update();

        let widget = resolve_widget(&mut app, panel, "background");
        assert_resolved_appearance!(
            &app,
            widget,
            WidgetHoveredAppearance,
            Appearance {
                background:   VisualChange::To(HOVER_FILL),
                border_color: VisualChange::Unchanged,
                border_width: VisualChange::Unchanged,
                text_color:   VisualChange::Unchanged,
                path_color:   VisualChange::Unchanged,
                tint:         VisualChange::Unchanged,
                material:     VisualChange::Unchanged,
            },
        );
    }

    #[test]
    fn border_color_sets_only_the_border_appearance() {
        let mut app = cascade_test_app();
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.with(
            El::new()
                .button("border")
                .hovered(BorderColor(FOCUS_BORDER)),
            |_| {},
        );
        let panel = DiegeticPanel::world()
            .size(Mm(100.0), Mm(50.0))
            .with_tree(builder.build())
            .build()
            .expect("a sized panel should build");
        let panel = app.world_mut().spawn(panel).id();

        app.update();

        let widget = resolve_widget(&mut app, panel, "border");
        assert_resolved_appearance!(
            &app,
            widget,
            WidgetHoveredAppearance,
            Appearance {
                background:   VisualChange::Unchanged,
                border_color: VisualChange::To(FOCUS_BORDER),
                border_width: VisualChange::Unchanged,
                text_color:   VisualChange::Unchanged,
                path_color:   VisualChange::Unchanged,
                tint:         VisualChange::Unchanged,
                material:     VisualChange::Unchanged,
            },
        );
    }

    #[test]
    fn text_color_sets_only_the_text_appearance() {
        let mut app = cascade_test_app();
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.with(
            El::new().button("text").hovered(TextColor(HOVER_TEXT)),
            |_| {},
        );
        let panel = DiegeticPanel::world()
            .size(Mm(100.0), Mm(50.0))
            .with_tree(builder.build())
            .build()
            .expect("a sized panel should build");
        let panel = app.world_mut().spawn(panel).id();

        app.update();

        let widget = resolve_widget(&mut app, panel, "text");
        assert_resolved_appearance!(
            &app,
            widget,
            WidgetHoveredAppearance,
            Appearance {
                background:   VisualChange::Unchanged,
                border_color: VisualChange::Unchanged,
                border_width: VisualChange::Unchanged,
                text_color:   VisualChange::To(HOVER_TEXT),
                path_color:   VisualChange::Unchanged,
                tint:         VisualChange::Unchanged,
                material:     VisualChange::Unchanged,
            },
        );
    }

    #[test]
    fn path_color_sets_only_the_panel_draw_appearance() {
        let mut app = cascade_test_app();
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.with(
            El::new().button("path").hovered(PathColor(FOCUS_PATH)),
            |_| {},
        );
        let panel = DiegeticPanel::world()
            .size(Mm(100.0), Mm(50.0))
            .with_tree(builder.build())
            .build()
            .expect("a sized panel should build");
        let panel = app.world_mut().spawn(panel).id();

        app.update();

        let widget = resolve_widget(&mut app, panel, "path");
        assert_resolved_appearance!(
            &app,
            widget,
            WidgetHoveredAppearance,
            Appearance {
                background:   VisualChange::Unchanged,
                border_color: VisualChange::Unchanged,
                border_width: VisualChange::Unchanged,
                text_color:   VisualChange::Unchanged,
                path_color:   VisualChange::To(FOCUS_PATH),
                tint:         VisualChange::Unchanged,
                material:     VisualChange::Unchanged,
            },
        );
    }

    #[test]
    fn tint_sets_only_the_image_appearance() {
        let mut app = cascade_test_app();
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.with(El::new().button("tint").hovered(Tint(HOVER_TINT)), |_| {});
        let panel = DiegeticPanel::world()
            .size(Mm(100.0), Mm(50.0))
            .with_tree(builder.build())
            .build()
            .expect("a sized panel should build");
        let panel = app.world_mut().spawn(panel).id();

        app.update();

        let widget = resolve_widget(&mut app, panel, "tint");
        assert_resolved_appearance!(
            &app,
            widget,
            WidgetHoveredAppearance,
            Appearance {
                background:   VisualChange::Unchanged,
                border_color: VisualChange::Unchanged,
                border_width: VisualChange::Unchanged,
                text_color:   VisualChange::Unchanged,
                path_color:   VisualChange::Unchanged,
                tint:         VisualChange::To(HOVER_TINT),
                material:     VisualChange::Unchanged,
            },
        );
    }
}
