use bevy::app::App;
use bevy::prelude::MinimalPlugins;
use hana_diegetic::HeadlessLayoutPlugin;

use super::measurement::monospace_measurer;

#[must_use = "the benchmark app must be spawned into or updated"]
pub fn create_bench_app() -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.insert_resource(monospace_measurer());
    app.add_plugins(HeadlessLayoutPlugin);
    app.update();
    app
}
