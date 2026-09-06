//! Compile guard for macOS production display backend selection.
#![cfg(all(target_os = "macos", feature = "test"))]

const _: usize = std::mem::size_of::<hana_clerestory::LiveWinitProductionBackendSelection>();
