use fairy_dust::NoOrbitCam;
use fairy_dust::SprinkleBuilder;

fn accepts_installed_builder(builder: SprinkleBuilder<NoOrbitCam>) { drop(builder); }

fn accepts_app(_: &mut bevy::prelude::App) {}

fn asset_root_first() {
    let mut builder = fairy_dust::sprinkle_example().with_asset_root("assets");
    accepts_app(builder.app_mut());
    accepts_installed_builder(builder);
}

fn main() {}
