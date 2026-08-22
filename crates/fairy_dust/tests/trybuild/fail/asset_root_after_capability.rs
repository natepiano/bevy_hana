fn main() {
    let builder = fairy_dust::sprinkle_example()
        .with_brp_extras()
        .with_asset_root("assets");
    drop(builder);
}
