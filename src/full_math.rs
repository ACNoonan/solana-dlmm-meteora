//! Adapter wrappers around `u128x128_math` that cast u128 results to u64.
//!
//! Mirrors the shape of `MeteoraAg/dlmm-sdk` `commons/src/math/utils.rs`
//! `safe_mul_shr_cast` / `safe_shl_div_cast` / `safe_mul_div_cast`. Upstream's
//! signature is generic over `T: num_traits::FromPrimitive`; in practice
//! every call site in `commons/src/extensions/bin.rs` (the part we extract)
//! instantiates with `T = u64`. Specializing avoids pulling `num-traits`
//! into the runtime dep graph.

use crate::error::{ErrorCode, Result};
use crate::u128x128_math::{mul_div, mul_shr, shl_div, Rounding};

#[inline]
pub fn safe_mul_shr_u64(x: u128, y: u128, offset: u8, rounding: Rounding) -> Result<u64> {
    cast(mul_shr(x, y, offset, rounding))
}

#[inline]
pub fn safe_shl_div_u64(x: u128, y: u128, offset: u8, rounding: Rounding) -> Result<u64> {
    cast(shl_div(x, y, offset, rounding))
}

#[inline]
pub fn safe_mul_div_u64(x: u128, y: u128, denominator: u128, rounding: Rounding) -> Result<u64> {
    cast(mul_div(x, y, denominator, rounding))
}

#[inline]
fn cast(value: Option<u128>) -> Result<u64> {
    value
        .ok_or(ErrorCode::MathOverflow)?
        .try_into()
        .map_err(|_| ErrorCode::MathOverflow)
}
