#![allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
//! Integration tests for `hana_lading` startup asset loading.
//!
//! Every app loads from `tests/assets` under this crate's manifest directory,
//! so results do not depend on the invoking directory. Missing-file tests keep
//! the production failure path: the registered `ImageLoader` returns a
//! `Loading` handle, the asynchronous read produces
//! `AssetReaderError::NotFound`, and Bevy records the failed states before
//! `hana_lading` polls in `Update`.

mod failure;
mod recursive;
mod success;
mod support;
