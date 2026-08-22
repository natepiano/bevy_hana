//! Convenience traits for common numeric casts.
//!
//! Rust's `as` casts between numeric types trigger a family of clippy pedantic lints
//! (`cast_precision_loss`, `cast_possible_truncation`, `cast_possible_wrap`,
//! `cast_sign_loss`) that are noisy in geometry and game code where the values are
//! known to be small. These traits centralize the `#[allow]` in one place so call
//! sites stay clean and the cast intent is explicit.
//!
//! # Safety contract
//!
//! **These conversions are deliberately lossy.** They silence clippy warnings for
//! precision loss, truncation, sign loss, and wrapping — which means they will
//! silently produce wrong results if the input exceeds the target type's
//! representable range. It is the caller's responsibility to ensure values are
//! in bounds. Typical safe usage: loop indices, mesh vertex counts, ring/side
//! counts, and other small geometry values that will never approach the limits.

/// Lossy conversion to `f32`.
///
/// Precision is lost for values above 2^24 (16,777,216). The caller must ensure
/// the input is small enough that precision loss is acceptable.
pub trait ToF32 {
    /// Convert to `f32`, potentially losing precision for large values.
    fn to_f32(self) -> f32;
}

impl ToF32 for i32 {
    #[allow(
        clippy::cast_precision_loss,
        reason = "intentionally lossy — callers ensure values are in range"
    )]
    fn to_f32(self) -> f32 { self as f32 }
}

impl ToF32 for u32 {
    #[allow(
        clippy::cast_precision_loss,
        reason = "intentionally lossy — callers ensure values are in range"
    )]
    fn to_f32(self) -> f32 { self as f32 }
}

impl ToF32 for usize {
    #[allow(
        clippy::cast_precision_loss,
        reason = "intentionally lossy — callers ensure values are in range"
    )]
    fn to_f32(self) -> f32 { self as f32 }
}

impl ToF32 for f64 {
    #[allow(
        clippy::cast_possible_truncation,
        reason = "intentionally lossy — callers ensure values are in range"
    )]
    fn to_f32(self) -> f32 { self as f32 }
}

/// Narrowing conversion to `i32`.
///
/// May truncate (`usize`, `f32`) or wrap (`u32` values above `i32::MAX`).
/// The caller must ensure the value fits in `i32`'s range.
pub trait ToI32 {
    /// Convert to `i32`, potentially truncating or wrapping.
    fn to_i32(self) -> i32;
}

impl ToI32 for usize {
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_possible_wrap,
        reason = "intentionally lossy — callers ensure values are in range"
    )]
    fn to_i32(self) -> i32 { self as i32 }
}

impl ToI32 for u32 {
    #[allow(
        clippy::cast_possible_wrap,
        reason = "intentionally lossy — callers ensure values are in range"
    )]
    fn to_i32(self) -> i32 { self as i32 }
}

impl ToI32 for f32 {
    #[allow(
        clippy::cast_possible_truncation,
        reason = "intentionally lossy — callers ensure values are in range"
    )]
    fn to_i32(self) -> i32 { self as i32 }
}

impl ToI32 for f64 {
    #[allow(
        clippy::cast_possible_truncation,
        reason = "intentionally lossy — callers ensure values are in range"
    )]
    fn to_i32(self) -> i32 { self as i32 }
}

/// Narrowing conversion to `u32`.
///
/// May truncate (`usize` on 64-bit) or discard fractional/negative parts
/// (`f32`, `f64`). The caller must ensure the value is non-negative and
/// fits in `u32`'s range.
pub trait ToU32 {
    /// Convert to `u32`, potentially truncating or losing sign.
    fn to_u32(self) -> u32;
}

impl ToU32 for usize {
    #[allow(
        clippy::cast_possible_truncation,
        reason = "intentionally lossy — callers ensure values are in range"
    )]
    fn to_u32(self) -> u32 { self as u32 }
}

impl ToU32 for f32 {
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "intentionally lossy — callers ensure values are in range"
    )]
    fn to_u32(self) -> u32 { self as u32 }
}

impl ToU32 for f64 {
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "intentionally lossy — callers ensure values are in range"
    )]
    fn to_u32(self) -> u32 { self as u32 }
}

impl ToU32 for i32 {
    #[allow(
        clippy::cast_sign_loss,
        reason = "intentionally lossy — callers ensure values are in range"
    )]
    fn to_u32(self) -> u32 { self as u32 }
}

impl ToU32 for u64 {
    #[allow(
        clippy::cast_possible_truncation,
        reason = "intentionally lossy — callers ensure values are in range"
    )]
    fn to_u32(self) -> u32 { self as u32 }
}

/// Conversion to `usize`.
///
/// `u32` → `usize` is lossless on 64-bit targets but the trait exists for
/// consistency. `f32` → `usize` may truncate and loses sign. The caller must
/// ensure the value is non-negative and in range.
pub trait ToUsize {
    /// Convert to `usize`.
    fn to_usize(self) -> usize;
}

impl ToUsize for u32 {
    fn to_usize(self) -> usize { self as usize }
}

impl ToUsize for f32 {
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "intentionally lossy — callers ensure values are in range"
    )]
    fn to_usize(self) -> usize { self as usize }
}

/// Narrowing conversion to `u8`.
///
/// Truncates integers above 255 and discards fractional/negative parts of
/// floats. The caller must ensure the value is in `[0, 255]`.
pub trait ToU8 {
    /// Convert to `u8`, potentially truncating or losing sign.
    fn to_u8(self) -> u8;
}

impl ToU8 for f32 {
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "intentionally lossy — callers ensure values are in range"
    )]
    fn to_u8(self) -> u8 { self as u8 }
}

impl ToU8 for u32 {
    #[allow(
        clippy::cast_possible_truncation,
        reason = "intentionally lossy — callers ensure values are in range"
    )]
    fn to_u8(self) -> u8 { self as u8 }
}

impl ToU8 for usize {
    #[allow(
        clippy::cast_possible_truncation,
        reason = "intentionally lossy — callers ensure values are in range"
    )]
    fn to_u8(self) -> u8 { self as u8 }
}

/// Narrowing conversion to `u16`.
///
/// Truncates integers above 65 535 and discards fractional/negative parts of
/// floats. The caller must ensure the value fits in `u16`'s range.
pub trait ToU16 {
    /// Convert to `u16`, potentially truncating or losing sign.
    fn to_u16(self) -> u16;
}

impl ToU16 for usize {
    #[allow(
        clippy::cast_possible_truncation,
        reason = "intentionally lossy — callers ensure values are in range"
    )]
    fn to_u16(self) -> u16 { self as u16 }
}

impl ToU16 for u32 {
    #[allow(
        clippy::cast_possible_truncation,
        reason = "intentionally lossy — callers ensure values are in range"
    )]
    fn to_u16(self) -> u16 { self as u16 }
}

impl ToU16 for f32 {
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "intentionally lossy — callers ensure values are in range"
    )]
    fn to_u16(self) -> u16 { self as u16 }
}

/// Widening conversion to `f64`.
///
/// All current impls are lossless or precision-losing only for very large
/// `usize` values (above 2^53). The caller should be aware that `usize`
/// values above `f64`'s exact-integer range will silently lose precision.
pub trait ToF64 {
    /// Convert to `f64`.
    fn to_f64(self) -> f64;
}

impl ToF64 for usize {
    #[allow(
        clippy::cast_precision_loss,
        reason = "intentionally lossy — callers ensure values are in range"
    )]
    fn to_f64(self) -> f64 { self as f64 }
}

impl ToF64 for u32 {
    fn to_f64(self) -> f64 { f64::from(self) }
}

impl ToF64 for i32 {
    fn to_f64(self) -> f64 { f64::from(self) }
}

impl ToF64 for u64 {
    #[allow(
        clippy::cast_precision_loss,
        reason = "intentionally lossy — callers ensure values are in range"
    )]
    fn to_f64(self) -> f64 { self as f64 }
}

impl ToF64 for f32 {
    fn to_f64(self) -> f64 { f64::from(self) }
}
