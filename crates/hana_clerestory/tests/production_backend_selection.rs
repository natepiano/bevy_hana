//! Compile guard for production display backend selection.
#![cfg(feature = "test")]

const _: usize = std::mem::size_of::<hana_clerestory::LiveWinitProductionBackendSelection>();
