//! External-client coverage for headless diegetic widget behavior.

use bevy::ecs::system::RunSystemOnce;
use bevy::prelude::App;
use bevy::prelude::Entity;
use bevy::prelude::EntityEvent;
use bevy::prelude::MinimalPlugins;
use bevy::prelude::On;
use bevy::prelude::ResMut;
use bevy::prelude::Resource;
use bevy::prelude::Window;
use bevy::render::RenderApp;
use hana_diegetic::ButtonClicked;
use hana_diegetic::DiegeticPanel;
use hana_diegetic::DiegeticTextMeasurer;
use hana_diegetic::El;
use hana_diegetic::HeadlessDiegeticUiPlugin;
use hana_diegetic::HeadlessLayoutPlugin;
use hana_diegetic::ImeAppOwnedFieldSpec;
use hana_diegetic::ImeEditableFieldSpec;
use hana_diegetic::ImeInputBlocker;
use hana_diegetic::ImeOpenSession;
use hana_diegetic::ImeTarget;
use hana_diegetic::LayoutBuilder;
use hana_diegetic::Mm;
use hana_diegetic::PanelElementId;
use hana_diegetic::PanelWidgetReader;
use hana_diegetic::RequestSliderAdjustment;
use hana_diegetic::RequestWidgetFocus;
use hana_diegetic::Slider;
use hana_diegetic::SliderAdjustment;
use hana_diegetic::SliderChangeRequested;
use hana_diegetic::WidgetInput;

#[derive(Default, Resource)]
struct ObservedBehavior {
    clicked:        Option<Entity>,
    proposed_value: Option<f32>,
}

fn record_button_click(click: On<ButtonClicked>, mut observed: ResMut<ObservedBehavior>) {
    observed.clicked = Some(click.event_target());
}

fn record_slider_change(change: On<SliderChangeRequested>, mut observed: ResMut<ObservedBehavior>) {
    observed.proposed_value = Some(change.value);
}

fn resolve_widget(app: &mut App, panel: Entity, id: &'static str) -> Option<Entity> {
    let id = PanelElementId::named(id);
    app.world_mut()
        .run_system_once(move |reader: PanelWidgetReader| reader.entity(panel, &id))
        .ok()
        .flatten()
}

#[test]
fn client_can_test_button_slider_and_ime_behavior_without_rendering() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .insert_resource(DiegeticTextMeasurer::default())
        .init_resource::<ObservedBehavior>()
        .add_plugins(HeadlessDiegeticUiPlugin)
        .add_observer(record_button_click)
        .add_observer(record_slider_change);

    assert!(app.is_plugin_added::<HeadlessLayoutPlugin>());
    assert!(app.get_sub_app(RenderApp).is_none());

    let slider = Slider::new(0.0..=1.0).value(0.5);

    let mut builder = LayoutBuilder::new(100.0, 50.0);
    builder.with(El::new().button("action"), |_| {});
    builder.with(El::new().widget("amount", slider), |_| {});
    let panel = DiegeticPanel::world()
        .size(Mm(100.0), Mm(50.0))
        .with_tree(builder.build())
        .build();
    assert!(panel.is_ok());
    let Ok(panel) = panel else {
        return;
    };

    let window = app.world_mut().spawn(Window::default()).id();
    let panel = app.world_mut().spawn(panel).id();
    app.update();

    let button = resolve_widget(&mut app, panel, "action");
    assert!(button.is_some());
    let Some(button) = button else {
        return;
    };
    let slider = resolve_widget(&mut app, panel, "amount");
    assert!(slider.is_some());
    let Some(slider) = slider else {
        return;
    };

    app.world_mut().trigger(RequestWidgetFocus {
        window,
        widget: button,
    });
    app.world_mut()
        .write_message(WidgetInput::Activate { window });
    app.update();
    assert_eq!(
        app.world().resource::<ObservedBehavior>().clicked,
        Some(button)
    );

    app.world_mut().trigger(RequestSliderAdjustment {
        entity:     slider,
        adjustment: SliderAdjustment::Absolute(0.75),
    });
    app.world_mut().flush();
    assert_eq!(
        app.world().resource::<ObservedBehavior>().proposed_value,
        Some(0.75),
    );

    let owner = app.world_mut().spawn_empty().id();
    app.world_mut().trigger(ImeOpenSession {
        target: ImeTarget::AppOwned {
            owner,
            field_id: PanelElementId::named("client-field"),
        },
        window,
        initial_text: "editable".to_owned(),
        field_spec: ImeEditableFieldSpec::AppOwned(ImeAppOwnedFieldSpec::new("client-field")),
        anchor: None,
    });
    app.world_mut().flush();
    assert_eq!(
        app.world().resource::<ImeInputBlocker>().window(),
        Some(window)
    );
    assert!(app.get_sub_app(RenderApp).is_none());
}
