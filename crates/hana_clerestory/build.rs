//! Enlarge the example executables' stack reserve on Windows (MSVC).
//!
//! On a host without a GPU (for example the `VMware` test guest), `wgpu`'s `dx12`
//! backend falls back to the WARP software rasterizer. WARP recurses deeply
//! while reconfiguring its swapchain/surfaces during a monitor reconnect, and
//! its worker threads inherit the executable's default 1 MB stack reserve,
//! which the deep-but-finite recursion overflows. Reserving 256 MB (virtual
//! address space only, committed lazily) gives the recursion room to complete.
//!
//! This is a linker flag only; it changes no code and is inert on hardware
//! renderers. `/STACK` is MSVC-specific, so it is gated to that target.

fn main() {
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let target_env = std::env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    if target_os == "windows" && target_env == "msvc" {
        // 256 MiB = 0x1000_0000.
        println!("cargo::rustc-link-arg-examples=/STACK:268435456");
    }
}
