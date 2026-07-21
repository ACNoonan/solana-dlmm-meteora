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

use crate::bin_math::{get_price_from_id, SCALE_OFFSET};
use crate::error::Result;
use crate::full_math::{safe_mul_shr_u64, safe_shl_div_u64};
use crate::u128x128_math::Rounding;

/// Caller-flattened projection of upstream `Bin`. The caller decodes their
/// `BinArray`s into a slice of these, sorted ascending by `bin_id`, and
/// hands it to the orchestrator.
///
/// Only the fields the swap path reads are exposed — the liquidity-supply /
/// per-bin fee-growth fields stay in the caller's decoded `Bin`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BinView {
    pub bin_id: i32,
    /// Reserve of token X in this bin, in base units (lamports for SOL,
    /// `10^decimals` units for an SPL token).
    pub amount_x: u64,
    /// Reserve of token Y in this bin.
    pub amount_y: u64,
    /// The stored Q64.64 bin price from the decoded `Bin`, or 0 if the
    /// on-chain bin has never had its price initialized. The exact-out
    /// path resolves 0 via [`get_or_store_bin_price`]; the exact-in path
    /// reads it as-is — both mirror upstream's quote loop behavior.
    pub price: u128,
    /// Upstream `Bin::limit_order_ask_side` flag byte (`!= 0` means the
    /// bin's limit orders sit on the ask side).
    pub limit_order_ask_side: u8,
    /// Open (unfilled) limit-order amount resting in this bin.
    pub open_order_amount: u64,
    /// Remaining amount of processed (partially filled) limit orders.
    pub processed_order_remaining_amount: u64,
}

/// Resolve the bin's Q64.64 price, computing and storing it from
/// `bin_id` + `bin_step` when the decoded price is 0.
///
/// Mirrors `Bin::get_or_store_bin_price`.
pub fn get_or_store_bin_price(bin: &mut BinView, bin_step: u16) -> Result<u128> {
    if bin.price == 0 {
        bin.price = get_price_from_id(bin.bin_id, bin_step)?;
    }

    Ok(bin.price)
}

/// Returns (open_order_amount, processed_order_remaining_amount) for the matching limit order
/// side based on swap direction. Returns (0, 0) if limit orders don't match the swap direction.
///
/// Mirrors `Bin::get_limit_order_amounts_by_direction`.
pub fn get_limit_order_amounts_by_direction(bin: &BinView, swap_for_y: bool) -> (u64, u64) {
    let is_ask_side = bin.limit_order_ask_side != 0;
    // swap_for_y (selling X for Y) can fill bid side orders (!is_ask_side)
    // !swap_for_y (selling Y for X) can fill ask side orders (is_ask_side)
    if (swap_for_y && !is_ask_side) || (!swap_for_y && is_ask_side) {
        (bin.open_order_amount, bin.processed_order_remaining_amount)
    } else {
        (0, 0)
    }
}

/// Returns the maximum amount out including both MM liquidity and limit order amounts.
///
/// Mirrors `Bin::get_max_amount_out_with_limit_orders`.
pub fn get_max_amount_out_with_limit_orders(
    bin: &BinView,
    swap_for_y: bool,
    support_limit_order: bool,
) -> u64 {
    let mm_amount = get_max_amount_out(bin, swap_for_y);
    if !support_limit_order {
        return mm_amount;
    }
    let (open_order, processed_remaining) = get_limit_order_amounts_by_direction(bin, swap_for_y);
    mm_amount
        .saturating_add(open_order)
        .saturating_add(processed_remaining)
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
/// price. Every upstream call site passes `Rounding::Down` (exact-in
/// semantics).
///
/// Mirrors `Bin::get_amount_out`.
pub fn get_amount_out(
    amount_in: u64,
    price: u128,
    swap_for_y: bool,
    rounding: Rounding,
) -> Result<u64> {
    if swap_for_y {
        // X in, Y out. amount_out_y = amount_in_x * price.
        safe_mul_shr_u64(price, amount_in.into(), SCALE_OFFSET, rounding)
    } else {
        // Y in, X out. amount_out_x = amount_in_y / price.
        safe_shl_div_u64(amount_in.into(), price, SCALE_OFFSET, rounding)
    }
}

/// Input amount (pre-fee) required to obtain a given output amount at the
/// bin's price. Every upstream call site passes `Rounding::Up` (exact-out
/// semantics).
///
/// Mirrors `Bin::get_amount_in`. Note that this is the pre-fee input; the
/// caller adds the swap fee separately (the fee is computed at the pool
/// level, not the bin level).
pub fn get_amount_in(
    amount_out: u64,
    price: u128,
    swap_for_y: bool,
    rounding: Rounding,
) -> Result<u64> {
    if swap_for_y {
        safe_shl_div_u64(amount_out.into(), price, SCALE_OFFSET, rounding)
    } else {
        safe_mul_shr_u64(amount_out.into(), price, SCALE_OFFSET, rounding)
    }
}
