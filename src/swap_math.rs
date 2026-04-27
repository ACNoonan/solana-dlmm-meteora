//! Structural per-bin swap primitives.
//!
//! These are the deterministic, fee-independent halves of upstream's
//! `Bin::swap` (`MeteoraAg/dlmm-sdk` `commons/src/extensions/bin.rs`). The
//! pool-level dynamic-fee FSM ports in milestone 4; the orchestrator that
//! composes the FSM with these primitives into `compute_swap_step` /
//! `compute_swap_full` ports in milestone 5.
//!
//! Splitting the per-bin math from the per-pool fee state mirrors upstream's
//! own architecture — `Bin::swap` calls into `&LbPair` for every fee
//! computation, so a self-contained pure `compute_swap_step` is a v0.2+
//! synthesis, not a verbatim extraction.

use crate::bin_math::SCALE_OFFSET;
use crate::error::Result;
use crate::full_math::{safe_mul_shr_u64, safe_shl_div_u64};
use crate::u128x128_math::Rounding;

/// Caller-flattened projection of upstream `Bin`. The caller decodes their
/// `BinArray`s into a sorted slice of these and hands it to the orchestrator.
///
/// Only the fields the structural swap-step math reads are exposed — the
/// liquidity-supply / per-bin fee-growth fields stay in the caller's
/// decoded `Bin` until later milestones need them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BinView {
    pub bin_id: i32,
    /// Reserve of token X in this bin, in base units (lamports for SOL,
    /// `10^decimals` units for an SPL token).
    pub amount_x: u64,
    /// Reserve of token Y in this bin.
    pub amount_y: u64,
}

/// Maximum output the bin can produce in the swap direction.
///
/// Mirrors `Bin::get_max_amount_out`. `swap_for_y == true` means the user
/// is putting X in and pulling Y out; the bin's Y reserve is the cap.
#[inline]
pub fn get_max_amount_out(bin: &BinView, swap_for_y: bool) -> u64 {
    if swap_for_y {
        bin.amount_y
    } else {
        bin.amount_x
    }
}

/// Maximum input (pre-fee) that fully drains the bin's output reserve at
/// the given Q64.64 price. Rounds up — upstream's exact-out semantics.
///
/// Mirrors `Bin::get_max_amount_in`.
pub fn get_max_amount_in(bin: &BinView, price: u128, swap_for_y: bool) -> Result<u64> {
    if swap_for_y {
        // user puts X in, drains Y. amount_in_x = amount_y / price.
        safe_shl_div_u64(bin.amount_y.into(), price, SCALE_OFFSET, Rounding::Up)
    } else {
        // user puts Y in, drains X. amount_in_y = amount_x * price.
        safe_mul_shr_u64(bin.amount_x.into(), price, SCALE_OFFSET, Rounding::Up)
    }
}

/// Output amount produced by a given input amount (post-fee) at the bin's
/// price. Rounds down — upstream's exact-in semantics.
///
/// Mirrors `Bin::get_amount_out`.
pub fn get_amount_out(amount_in: u64, price: u128, swap_for_y: bool) -> Result<u64> {
    if swap_for_y {
        // X in, Y out. amount_out_y = amount_in_x * price.
        safe_mul_shr_u64(price, amount_in.into(), SCALE_OFFSET, Rounding::Down)
    } else {
        // Y in, X out. amount_out_x = amount_in_y / price.
        safe_shl_div_u64(amount_in.into(), price, SCALE_OFFSET, Rounding::Down)
    }
}

/// Input amount (pre-fee) required to obtain a given output amount at the
/// bin's price. Rounds up — upstream's exact-out semantics.
///
/// Mirrors `Bin::get_amount_in`. Note that this is the pre-fee input; the
/// caller adds the swap fee separately (the fee is computed at the pool
/// level, not the bin level).
pub fn get_amount_in(amount_out: u64, price: u128, swap_for_y: bool) -> Result<u64> {
    if swap_for_y {
        safe_shl_div_u64(amount_out.into(), price, SCALE_OFFSET, Rounding::Up)
    } else {
        safe_mul_shr_u64(amount_out.into(), price, SCALE_OFFSET, Rounding::Up)
    }
}
