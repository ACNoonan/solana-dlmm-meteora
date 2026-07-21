//! The multi-bin swap orchestrator — `compute_swap_full` and the per-bin
//! quote steps it composes.
//!
//! Extracted from MeteoraAg/dlmm-sdk `commons/src/quote.rs`
//! (`quote_exact_in` / `quote_exact_out` and their per-bin helpers) and
//! `commons/src/typedefs.rs` (`BinQuoteResult`). The arithmetic is
//! byte-for-byte upstream; what this crate strips is the account plumbing:
//!
//! - Bin-array + bitmap walking (`get_bin_array_pubkeys_for_swap`,
//!   `shift_active_bin_if_empty_gap`) is replaced by walking a flat,
//!   caller-prepared `&[BinView]` sorted ascending by `bin_id`. A bin id
//!   absent from the slice is an empty bin: the active id advances through
//!   it, exactly as upstream advances through empty bins inside an array.
//!   When the active id walks past the last supplied bin in the swap
//!   direction with amount remaining, the loop errors with
//!   [`ErrorCode::PoolOutOfLiquidity`] — upstream's "Pool out of
//!   liquidity" when the bitmap runs out of arrays.
//! - `Clock` becomes a `current_timestamp` parameter.
//! - Token-2022 transfer-fee inclusion/exclusion
//!   (`calculate_transfer_fee_*_amount`) is deferred to v0.2: amounts here
//!   are what reaches the pool, not what leaves the user's wallet.
//! - `validate_swap_activation` (pool status / activation gating) is
//!   account validation and stays with the caller.
//!
//! Upstream quotes on a throwaway copy of the pool and never writes
//! `last_update_timestamp` (the on-chain swap instruction does). Because
//! these orchestrators return the post-swap [`PoolView`] for chained
//! simulation, they set `last_update_timestamp = current_timestamp` on the
//! returned state — the one deliberate divergence from the quote path,
//! matching what the on-chain program persists after a swap.

use crate::bin_math::BASIS_POINT_MAX;
use crate::dynamic_fee::{
    compute_fee, compute_fee_from_amount, fee_on_input, is_support_limit_order, update_references,
    update_volatility_accumulator, PoolView, LIMIT_ORDER_FEE_SHARE,
};
use crate::error::{ErrorCode, Result};
use crate::swap_math::{
    get_amount_in, get_amount_out, get_limit_order_amounts_by_direction,
    get_max_amount_out_with_limit_orders, get_or_store_bin_price, BinView,
};
use crate::u128x128_math::Rounding;

/// Result of an exact-in swap across bins. Field names mirror upstream
/// `SwapExactInQuote`; `pool_after` is this crate's addition (the post-swap
/// pool state, for chained simulation).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SwapExactInQuote {
    pub amount_out: u64,
    pub fee: u64,
    pub protocol_fee: u64,
    /// Post-swap pool state (active bin + volatility FSM).
    pub pool_after: PoolView,
}

/// Result of an exact-out swap across bins. Field names mirror upstream
/// `SwapExactOutQuote`; `amount_in` is fee-included. `pool_after` is this
/// crate's addition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SwapExactOutQuote {
    pub amount_in: u64,
    pub fee: u64,
    pub protocol_fee: u64,
    /// Post-swap pool state (active bin + volatility FSM).
    pub pool_after: PoolView,
}

/// Result of a per-bin quote calculation. Mirrors upstream
/// `typedefs.rs::BinQuoteResult`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BinQuoteResult {
    /// Amount of input consumed (includes trading fee when fee_on_input)
    pub amount_in: u64,
    /// Amount of output produced (excludes trading fee when fee_on_output)
    pub amount_out: u64,
    /// Total trading fee
    pub fee: u64,
    /// Protocol portion of the trading fee
    pub protocol_fee: u64,
}

/// Internal fill result for a single liquidity layer within a bin.
struct FillResult {
    amount_in: u64,
    amount_left: u64,
    out_amount: u64,
}

/// Internal result for filling across all liquidity layers (MM + limit orders) in a bin.
struct ExactInFillResult {
    amount_in: u64,
    amount_left: u64,
    out_amount: u64,
    mm_amount_in: u64,
}

/// Calculate how much of `amount` can be filled against `max_amount_out` of liquidity at `price`.
fn calculate_exact_in_fill_amount(
    bin: &BinView,
    amount: u64,
    max_amount_out: u64,
    swap_for_y: bool,
) -> Result<FillResult> {
    if max_amount_out == 0 {
        return Ok(FillResult {
            amount_in: 0,
            amount_left: amount,
            out_amount: 0,
        });
    }
    let max_amount_in = get_amount_in(max_amount_out, bin.price, swap_for_y, Rounding::Up)?;
    if amount >= max_amount_in {
        Ok(FillResult {
            amount_in: max_amount_in,
            amount_left: amount
                .checked_sub(max_amount_in)
                .ok_or(ErrorCode::MathOverflow)?,
            out_amount: max_amount_out,
        })
    } else {
        let out_amount = get_amount_out(amount, bin.price, swap_for_y, Rounding::Down)?;
        Ok(FillResult {
            amount_in: amount,
            amount_left: 0,
            out_amount,
        })
    }
}

/// Fill a bin's liquidity layers: MM first, then processed limit orders, then open limit orders.
fn get_exact_in_fill_amount_result(
    bin: &BinView,
    amount_in: u64,
    swap_for_y: bool,
    support_limit_order: bool,
) -> Result<ExactInFillResult> {
    let mm_amount = if swap_for_y {
        bin.amount_y
    } else {
        bin.amount_x
    };
    let mm_fill = calculate_exact_in_fill_amount(bin, amount_in, mm_amount, swap_for_y)?;

    if !support_limit_order {
        return Ok(ExactInFillResult {
            amount_in: mm_fill.amount_in,
            amount_left: mm_fill.amount_left,
            out_amount: mm_fill.out_amount,
            mm_amount_in: mm_fill.amount_in,
        });
    }

    let mut total_amount_in = mm_fill.amount_in;
    let mut total_amount_out = mm_fill.out_amount;
    let amount_left_after_mm = mm_fill.amount_left;

    if amount_left_after_mm > 0 {
        let (open_order_amount, processed_order_remaining) =
            get_limit_order_amounts_by_direction(bin, swap_for_y);

        // Fill processed orders first
        let processed_fill = calculate_exact_in_fill_amount(
            bin,
            amount_left_after_mm,
            processed_order_remaining,
            swap_for_y,
        )?;
        total_amount_in = total_amount_in
            .checked_add(processed_fill.amount_in)
            .ok_or(ErrorCode::MathOverflow)?;
        total_amount_out = total_amount_out
            .checked_add(processed_fill.out_amount)
            .ok_or(ErrorCode::MathOverflow)?;

        // Fill open orders next
        if processed_fill.amount_left > 0 {
            let open_fill = calculate_exact_in_fill_amount(
                bin,
                processed_fill.amount_left,
                open_order_amount,
                swap_for_y,
            )?;
            total_amount_in = total_amount_in
                .checked_add(open_fill.amount_in)
                .ok_or(ErrorCode::MathOverflow)?;
            total_amount_out = total_amount_out
                .checked_add(open_fill.out_amount)
                .ok_or(ErrorCode::MathOverflow)?;
        }
    }

    Ok(ExactInFillResult {
        amount_in: total_amount_in,
        amount_left: amount_in
            .checked_sub(total_amount_in)
            .ok_or(ErrorCode::MathOverflow)?,
        out_amount: total_amount_out,
        mm_amount_in: mm_fill.amount_in,
    })
}

/// Split trading fee between user (LP) fee and protocol fee, accounting for limit order fee share.
fn split_fee(
    trading_fee: u64,
    protocol_share: u16,
    mm_amount_in: u64,
    total_amount_in: u64,
) -> Result<(u64, u64)> {
    if total_amount_in == 0 || trading_fee == 0 {
        return Ok((0, 0));
    }

    // mm_fee = ceil(trading_fee * mm_amount_in / total_amount_in)
    let mm_fee: u64 = u128::from(trading_fee)
        .checked_mul(mm_amount_in.into())
        .ok_or(ErrorCode::MathOverflow)?
        .checked_add(
            u128::from(total_amount_in)
                .checked_sub(1)
                .ok_or(ErrorCode::MathOverflow)?,
        )
        .ok_or(ErrorCode::MathOverflow)?
        .checked_div(total_amount_in.into())
        .ok_or(ErrorCode::MathOverflow)?
        .try_into()
        .map_err(|_| ErrorCode::MathOverflow)?;

    let total_lo_fee = trading_fee
        .checked_sub(mm_fee)
        .ok_or(ErrorCode::MathOverflow)?;

    // LO fee: portion that goes to order placer
    let lo_fee: u64 = u128::from(total_lo_fee)
        .checked_mul(LIMIT_ORDER_FEE_SHARE.into())
        .ok_or(ErrorCode::MathOverflow)?
        .checked_div(BASIS_POINT_MAX as u128)
        .ok_or(ErrorCode::MathOverflow)?
        .try_into()
        .map_err(|_| ErrorCode::MathOverflow)?;

    let lo_protocol_fee = total_lo_fee
        .checked_sub(lo_fee)
        .ok_or(ErrorCode::MathOverflow)?;

    // MM protocol fee
    let mm_protocol_fee: u64 = u128::from(mm_fee)
        .checked_mul(protocol_share.into())
        .ok_or(ErrorCode::MathOverflow)?
        .checked_div(BASIS_POINT_MAX as u128)
        .ok_or(ErrorCode::MathOverflow)?
        .try_into()
        .map_err(|_| ErrorCode::MathOverflow)?;

    let total_protocol_fee = lo_protocol_fee
        .checked_add(mm_protocol_fee)
        .ok_or(ErrorCode::MathOverflow)?;
    let total_user_fee = trading_fee
        .checked_sub(total_protocol_fee)
        .ok_or(ErrorCode::MathOverflow)?;

    Ok((total_user_fee, total_protocol_fee))
}

/// Per-bin exact-in quote with limit order and fee mode support.
///
/// Mirrors upstream `swap_exact_in_quote_at_bin` — the per-bin swap step
/// (the DLMM analog of a CLMM `compute_swap_step`).
pub fn swap_exact_in_quote_at_bin(
    bin: &BinView,
    pool: &PoolView,
    in_amount: u64,
    swap_for_y: bool,
    support_limit_order: bool,
    fee_on_input: bool,
) -> Result<BinQuoteResult> {
    let mut trading_fee: u64 = 0;
    let mut excluded_fee_amount_in = in_amount;

    if fee_on_input {
        let fee = compute_fee_from_amount(pool, in_amount)?;
        trading_fee = fee;
        excluded_fee_amount_in = in_amount.checked_sub(fee).ok_or(ErrorCode::MathOverflow)?;
    }

    let fill_result = get_exact_in_fill_amount_result(
        bin,
        excluded_fee_amount_in,
        swap_for_y,
        support_limit_order,
    )?;

    let amount_left = fill_result.amount_left;
    let out_amount = fill_result.out_amount;
    let mut included_fee_amount_in = in_amount;

    if amount_left > 0 {
        excluded_fee_amount_in = excluded_fee_amount_in
            .checked_sub(amount_left)
            .ok_or(ErrorCode::MathOverflow)?;

        if fee_on_input {
            let fee = compute_fee(pool, excluded_fee_amount_in)?;
            trading_fee = fee;
            included_fee_amount_in = excluded_fee_amount_in
                .checked_add(fee)
                .ok_or(ErrorCode::MathOverflow)?;
        } else {
            included_fee_amount_in = excluded_fee_amount_in;
        }
    }

    let mut excluded_fee_amount_out = out_amount;

    if !fee_on_input {
        let fee = compute_fee_from_amount(pool, out_amount)?;
        trading_fee = fee;
        excluded_fee_amount_out = out_amount.checked_sub(fee).ok_or(ErrorCode::MathOverflow)?;
    }

    let (_user_fee, protocol_fee) = split_fee(
        trading_fee,
        pool.parameters.protocol_share,
        fill_result.mm_amount_in,
        fill_result.amount_in,
    )?;

    Ok(BinQuoteResult {
        amount_in: included_fee_amount_in,
        amount_out: excluded_fee_amount_out,
        fee: trading_fee,
        protocol_fee,
    })
}

fn get_excluded_fee_amount_in(
    bin: &BinView,
    swap_for_y: bool,
    included_fee_amount_out: u64,
) -> Result<u64> {
    let mm_amount = if swap_for_y {
        bin.amount_y
    } else {
        bin.amount_x
    };

    let (open_order_amount, processed_order_remaining_amount) =
        get_limit_order_amounts_by_direction(bin, swap_for_y);

    let mut remaining_amount_out = included_fee_amount_out;
    let mut total_amount_in: u64 = 0;

    let exact_out_amount = remaining_amount_out.min(mm_amount);
    let amount_in = get_amount_in(exact_out_amount, bin.price, swap_for_y, Rounding::Up)?;
    remaining_amount_out = remaining_amount_out
        .checked_sub(exact_out_amount)
        .ok_or(ErrorCode::MathOverflow)?;
    total_amount_in = total_amount_in
        .checked_add(amount_in)
        .ok_or(ErrorCode::MathOverflow)?;

    if remaining_amount_out > 0 {
        let exact_out_amount = remaining_amount_out.min(processed_order_remaining_amount);
        let amount_in = get_amount_in(exact_out_amount, bin.price, swap_for_y, Rounding::Up)?;
        remaining_amount_out = remaining_amount_out
            .checked_sub(exact_out_amount)
            .ok_or(ErrorCode::MathOverflow)?;
        total_amount_in = total_amount_in
            .checked_add(amount_in)
            .ok_or(ErrorCode::MathOverflow)?;

        if remaining_amount_out > 0 {
            let exact_out_amount = remaining_amount_out.min(open_order_amount);
            let amount_in = get_amount_in(exact_out_amount, bin.price, swap_for_y, Rounding::Up)?;
            total_amount_in = total_amount_in
                .checked_add(amount_in)
                .ok_or(ErrorCode::MathOverflow)?;
        }
    }

    Ok(total_amount_in)
}

/// Per-bin exact-out quote with limit order and fee mode support.
///
/// Mirrors upstream `swap_exact_out_quote_at_bin`.
pub fn swap_exact_out_quote_at_bin(
    bin: &BinView,
    pool: &PoolView,
    out_amount: u64,
    swap_for_y: bool,
    support_limit_order: bool,
    fee_on_input: bool,
) -> Result<BinQuoteResult> {
    let mut included_fee_amount_out = out_amount;

    if !fee_on_input {
        let fee = compute_fee(pool, out_amount)?;
        included_fee_amount_out = out_amount.checked_add(fee).ok_or(ErrorCode::MathOverflow)?;
    }

    let max_amount_out = get_max_amount_out_with_limit_orders(bin, swap_for_y, support_limit_order);

    if included_fee_amount_out >= max_amount_out {
        // Drain entire bin
        return swap_exact_in_quote_at_bin(
            bin,
            pool,
            u64::MAX,
            swap_for_y,
            support_limit_order,
            fee_on_input,
        );
    }

    // Calculate required input for exact output
    let excluded_fee_amount_in =
        get_excluded_fee_amount_in(bin, swap_for_y, included_fee_amount_out)?;

    let included_fee_amount_in = if fee_on_input {
        let fee = compute_fee(pool, excluded_fee_amount_in)?;
        excluded_fee_amount_in
            .checked_add(fee)
            .ok_or(ErrorCode::MathOverflow)?
    } else {
        excluded_fee_amount_in
    };

    let mut result = swap_exact_in_quote_at_bin(
        bin,
        pool,
        included_fee_amount_in,
        swap_for_y,
        support_limit_order,
        fee_on_input,
    )?;

    // Delta between quoted output and requested output goes to protocol (rounding)
    if result.amount_out > out_amount {
        let delta = result
            .amount_out
            .checked_sub(out_amount)
            .ok_or(ErrorCode::MathOverflow)?;
        if delta > 1 {
            result.protocol_fee = result
                .protocol_fee
                .checked_add(delta)
                .ok_or(ErrorCode::MathOverflow)?;
        }
    }

    result.amount_out = out_amount;

    Ok(result)
}

/// Find the bin with `bin_id` in a slice sorted ascending by `bin_id`.
/// An absent id is an empty bin.
fn find_bin(bins: &[BinView], bin_id: i32) -> Option<&BinView> {
    bins.binary_search_by_key(&bin_id, |b| b.bin_id)
        .ok()
        .map(|i| &bins[i])
}

/// True if no supplied bin remains at or beyond `active_id` in the swap
/// direction — upstream's bitmap-exhausted "Pool out of liquidity" case.
fn out_of_supplied_bins(bins: &[BinView], active_id: i32, swap_for_y: bool) -> bool {
    if swap_for_y {
        bins.first().map_or(true, |b| active_id <= b.bin_id)
    } else {
        bins.last().map_or(true, |b| active_id >= b.bin_id)
    }
}

/// Exact-in swap across bins: the "one call to swap" orchestrator.
///
/// Mirrors upstream `quote_exact_in` with the account plumbing stripped
/// (see the module docs). `bins` must be sorted ascending by `bin_id` and
/// cover every bin the swap may touch; running past them errors with
/// [`ErrorCode::PoolOutOfLiquidity`].
pub fn compute_swap_full(
    pool: &PoolView,
    bins: &[BinView],
    amount_in: u64,
    swap_for_y: bool,
    current_timestamp: i64,
) -> Result<SwapExactInQuote> {
    debug_assert!(
        bins.windows(2).all(|w| w[0].bin_id < w[1].bin_id),
        "bins must be sorted ascending by bin_id with no duplicates",
    );

    let mut pool = *pool;
    update_references(&mut pool, current_timestamp)?;

    let support_limit_order = is_support_limit_order(&pool);
    let fee_on_input = fee_on_input(&pool, swap_for_y);

    let mut total_amount_out: u64 = 0;
    let mut total_fee: u64 = 0;
    let mut total_protocol_fee: u64 = 0;

    let mut amount_left = amount_in;

    while amount_left > 0 {
        if let Some(active_bin) = find_bin(bins, pool.active_id) {
            let max_out =
                get_max_amount_out_with_limit_orders(active_bin, swap_for_y, support_limit_order);

            if max_out > 0 {
                update_volatility_accumulator(&mut pool)?;

                let result = swap_exact_in_quote_at_bin(
                    active_bin,
                    &pool,
                    amount_left,
                    swap_for_y,
                    support_limit_order,
                    fee_on_input,
                )?;

                if result.amount_in > 0 {
                    amount_left = amount_left
                        .checked_sub(result.amount_in)
                        .ok_or(ErrorCode::MathOverflow)?;
                    total_amount_out = total_amount_out
                        .checked_add(result.amount_out)
                        .ok_or(ErrorCode::MathOverflow)?;
                    total_fee = total_fee
                        .checked_add(result.fee)
                        .ok_or(ErrorCode::MathOverflow)?;
                    total_protocol_fee = total_protocol_fee
                        .checked_add(result.protocol_fee)
                        .ok_or(ErrorCode::MathOverflow)?;
                }
            }
        }

        if amount_left > 0 {
            if out_of_supplied_bins(bins, pool.active_id, swap_for_y) {
                return Err(ErrorCode::PoolOutOfLiquidity);
            }
            crate::dynamic_fee::advance_active_bin(&mut pool, swap_for_y)?;
        }
    }

    // The quote path never persists the timestamp; the on-chain swap does.
    // We return the post-swap state for chained simulation, so persist it.
    pool.v_parameters.last_update_timestamp = current_timestamp;

    Ok(SwapExactInQuote {
        amount_out: total_amount_out,
        fee: total_fee,
        protocol_fee: total_protocol_fee,
        pool_after: pool,
    })
}

/// Exact-out swap across bins.
///
/// Mirrors upstream `quote_exact_out` with the account plumbing stripped
/// (see the module docs). The returned `amount_in` is fee-included.
pub fn compute_swap_full_exact_out(
    pool: &PoolView,
    bins: &[BinView],
    amount_out: u64,
    swap_for_y: bool,
    current_timestamp: i64,
) -> Result<SwapExactOutQuote> {
    debug_assert!(
        bins.windows(2).all(|w| w[0].bin_id < w[1].bin_id),
        "bins must be sorted ascending by bin_id with no duplicates",
    );

    let mut pool = *pool;
    update_references(&mut pool, current_timestamp)?;

    let support_limit_order = is_support_limit_order(&pool);
    let fee_on_input = fee_on_input(&pool, swap_for_y);

    let mut total_amount_in: u64 = 0;
    let mut total_fee: u64 = 0;
    let mut total_protocol_fee: u64 = 0;

    let mut amount_out_left = amount_out;

    while amount_out_left > 0 {
        if let Some(active_bin) = find_bin(bins, pool.active_id) {
            // Upstream's exact-out loop resolves an uninitialized bin price
            // before quoting (the exact-in loop does not — replicated).
            let mut active_bin = *active_bin;
            let _price = get_or_store_bin_price(&mut active_bin, pool.bin_step)?;

            let max_out =
                get_max_amount_out_with_limit_orders(&active_bin, swap_for_y, support_limit_order);

            if max_out > 0 {
                update_volatility_accumulator(&mut pool)?;

                let result = swap_exact_out_quote_at_bin(
                    &active_bin,
                    &pool,
                    amount_out_left,
                    swap_for_y,
                    support_limit_order,
                    fee_on_input,
                )?;

                if result.amount_out > 0 {
                    amount_out_left = amount_out_left
                        .checked_sub(result.amount_out)
                        .ok_or(ErrorCode::MathOverflow)?;
                    total_amount_in = total_amount_in
                        .checked_add(result.amount_in)
                        .ok_or(ErrorCode::MathOverflow)?;
                    total_fee = total_fee
                        .checked_add(result.fee)
                        .ok_or(ErrorCode::MathOverflow)?;
                    total_protocol_fee = total_protocol_fee
                        .checked_add(result.protocol_fee)
                        .ok_or(ErrorCode::MathOverflow)?;
                }
            }
        }

        if amount_out_left > 0 {
            if out_of_supplied_bins(bins, pool.active_id, swap_for_y) {
                return Err(ErrorCode::PoolOutOfLiquidity);
            }
            crate::dynamic_fee::advance_active_bin(&mut pool, swap_for_y)?;
        }
    }

    // See compute_swap_full: persist the timestamp like the on-chain swap.
    pool.v_parameters.last_update_timestamp = current_timestamp;

    Ok(SwapExactOutQuote {
        amount_in: total_amount_in,
        fee: total_fee,
        protocol_fee: total_protocol_fee,
        pool_after: pool,
    })
}
