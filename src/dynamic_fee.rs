//! The dynamic-fee FSM — pool-level fee state and its per-swap updates.
//!
//! Extracted from MeteoraAg/dlmm-sdk:
//! - `commons/src/extensions/lb_pair.rs` (`update_references`,
//!   `update_volatility_accumulator`, `advance_active_bin`, `compute_fee`,
//!   `compute_fee_from_amount`, `compute_protocol_fee`, `get_base_fee`,
//!   `get_variable_fee`, `compute_variable_fee`, `get_total_fee`,
//!   `is_support_limit_order`, `fee_on_input`)
//! - `commons/src/conversions/{function_type,collect_fee_mode}.rs`
//! - `commons/src/constants.rs` (`MAX_FEE_RATE`, `FEE_PRECISION`,
//!   `LIMIT_ORDER_FEE_SHARE`)
//! - the `StaticParameters` / `VariableParameters` layouts from the anchor
//!   IDL (`idls/dlmm.json`), which `commons` obtains via `declare_program!`.
//!
//! Upstream hosts the FSM as `LbPairExtension` methods on the full ~900-byte
//! anchor `LbPair` account. This crate rehosts them as free functions over
//! [`PoolView`] — a flat, caller-prepared projection carrying only the state
//! the swap path reads (same posture as `BinView`). The arithmetic inside
//! each function is byte-for-byte upstream; the only adjustments are
//! `anyhow::Result` → `crate::error::Result` and method → free-function
//! rehosting.
//!
//! The fee is `base_fee + variable_fee(volatility)`. The volatility FSM:
//! [`update_references`] runs once per swap (freezes/decays the reference
//! against `filter_period` / `decay_period`), then
//! [`update_volatility_accumulator`] runs per bin crossed (accumulates
//! `|index_reference - active_id|`, capped at `max_volatility_accumulator`).

use crate::bin_math::{BASIS_POINT_MAX, MAX_BIN_ID, MIN_BIN_ID};
use crate::error::{ErrorCode, Result};

// --- commons/src/constants.rs ---

/// Maximum fee rate. 10%
pub const MAX_FEE_RATE: u64 = 100_000_000;

pub const FEE_PRECISION: u64 = 1_000_000_000;

/// Limit order fee share (BPS). Portion of limit order trading fee that goes to the order placer.
pub const LIMIT_ORDER_FEE_SHARE: u16 = 5000;

// --- idls/dlmm.json `StaticParameters` (via commons `declare_program!`) ---

/// Parameters set by the protocol. Field-for-field the upstream
/// `StaticParameters` POD minus its explicit `_padding` arrays (callers
/// construct this by hand from a decoded `LbPair`, not by pod-casting).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct StaticParameters {
    /// Used for base fee calculation. base_fee_rate = base_factor * bin_step * 10 * 10^base_fee_power_factor
    pub base_factor: u16,
    /// Filter period determine high frequency trading time window.
    pub filter_period: u16,
    /// Decay period determine when the volatile fee start decay / decrease.
    pub decay_period: u16,
    /// Reduction factor controls the volatile fee rate decrement rate.
    pub reduction_factor: u16,
    /// Used to scale the variable fee component depending on the dynamic of the market
    pub variable_fee_control: u32,
    /// Maximum number of bin crossed can be accumulated. Used to cap volatile fee rate.
    pub max_volatility_accumulator: u32,
    /// Min bin id supported by the pool based on the configured bin step.
    pub min_bin_id: i32,
    /// Max bin id supported by the pool based on the configured bin step.
    pub max_bin_id: i32,
    /// Portion of swap fees retained by the protocol by controlling protocol_share parameter. protocol_swap_fee = protocol_share * total_swap_fee
    pub protocol_share: u16,
    /// Base fee power factor
    pub base_fee_power_factor: u8,
    /// function type
    pub function_type: u8,
    /// Collect fee mode
    pub collect_fee_mode: u8,
}

// --- idls/dlmm.json `VariableParameters` (via commons `declare_program!`) ---

/// Parameters that change based on the dynamic of the market. Field-for-field
/// the upstream `VariableParameters` POD minus its `_padding` arrays.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct VariableParameters {
    /// Volatility accumulator measure the number of bin crossed since reference bin ID. Normally (without filter period taken into consideration), reference bin ID is the active bin of last swap.
    /// It affects the variable fee rate
    pub volatility_accumulator: u32,
    /// Volatility reference is decayed volatility accumulator. It is always <= volatility_accumulator
    pub volatility_reference: u32,
    /// Active bin id of last swap.
    pub index_reference: i32,
    /// Last timestamp the variable parameters was updated
    pub last_update_timestamp: i64,
}

/// Caller-flattened projection of the upstream `LbPair` account. Only the
/// state the swap path reads — pubkeys, reward configs, oracle, and the
/// bin-array bitmap stay in the caller's decoded account (the bitmap is
/// unnecessary here because the caller hands the orchestrator a flat,
/// sorted `&[BinView]` instead of bin arrays).
///
/// Pool status / activation gating (`validate_swap_activation`) is account
/// validation, not swap math, and is likewise the caller's job.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PoolView {
    /// The pool's active bin id.
    pub active_id: i32,
    /// Bin step. Represent the price increment / decrement, in bps.
    pub bin_step: u16,
    /// True if any of the pool's `reward_infos[i].mint` is a non-default
    /// pubkey. Feeds [`is_support_limit_order`] — upstream checks the
    /// reward mints directly, which a pubkey-free projection cannot.
    pub has_rewards: bool,
    /// Static pool parameters.
    pub parameters: StaticParameters,
    /// Variable (volatility FSM) parameters.
    pub v_parameters: VariableParameters,
}

// --- commons/src/conversions/function_type.rs ---

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FunctionType {
    Undetermined,
    LiquidityMining,
    LimitOrder,
}

impl TryFrom<u8> for FunctionType {
    type Error = ErrorCode;

    fn try_from(value: u8) -> Result<Self> {
        match value {
            0 => Ok(FunctionType::Undetermined),
            1 => Ok(FunctionType::LiquidityMining),
            2 => Ok(FunctionType::LimitOrder),
            _ => Err(ErrorCode::InvalidParameter),
        }
    }
}

// --- commons/src/conversions/collect_fee_mode.rs ---

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CollectFeeMode {
    InputOnly,
    OnlyY,
}

impl TryFrom<u8> for CollectFeeMode {
    type Error = ErrorCode;

    fn try_from(value: u8) -> Result<Self> {
        match value {
            0 => Ok(CollectFeeMode::InputOnly),
            1 => Ok(CollectFeeMode::OnlyY),
            _ => Err(ErrorCode::InvalidParameter),
        }
    }
}

// --- commons/src/extensions/lb_pair.rs ---

/// Once-per-swap reference update. Mirrors `LbPair::update_references`.
///
/// If at least `filter_period` has elapsed since the last update, the
/// reference bin id is frozen at the current active bin, and the volatility
/// reference is either decayed by `reduction_factor` (inside the decay
/// window) or zeroed (outside it).
pub fn update_references(pool: &mut PoolView, current_timestamp: i64) -> Result<()> {
    let v_params = &mut pool.v_parameters;
    let s_params = &pool.parameters;

    let elapsed = current_timestamp
        .checked_sub(v_params.last_update_timestamp)
        .ok_or(ErrorCode::MathOverflow)?;

    // Not high frequency trade
    if elapsed >= s_params.filter_period as i64 {
        // Update active id of last transaction
        v_params.index_reference = pool.active_id;
        // filter period < t < decay_period. Decay time window.
        if elapsed < s_params.decay_period as i64 {
            let volatility_reference = v_params
                .volatility_accumulator
                .checked_mul(s_params.reduction_factor as u32)
                .ok_or(ErrorCode::MathOverflow)?
                .checked_div(BASIS_POINT_MAX as u32)
                .ok_or(ErrorCode::MathOverflow)?;

            v_params.volatility_reference = volatility_reference;
        }
        // Out of decay time window
        else {
            v_params.volatility_reference = 0;
        }
    }

    Ok(())
}

/// Per-bin-crossed accumulator update. Mirrors
/// `LbPair::update_volatility_accumulator`.
///
/// `volatility_accumulator = min(volatility_reference + |index_reference -
/// active_id| * BASIS_POINT_MAX, max_volatility_accumulator)`.
pub fn update_volatility_accumulator(pool: &mut PoolView) -> Result<()> {
    let v_params = &mut pool.v_parameters;
    let s_params = &pool.parameters;

    let delta_id = i64::from(v_params.index_reference)
        .checked_sub(pool.active_id.into())
        .ok_or(ErrorCode::MathOverflow)?
        .unsigned_abs();

    let volatility_accumulator = u64::from(v_params.volatility_reference)
        .checked_add(
            delta_id
                .checked_mul(BASIS_POINT_MAX as u64)
                .ok_or(ErrorCode::MathOverflow)?,
        )
        .ok_or(ErrorCode::MathOverflow)?;

    v_params.volatility_accumulator = core::cmp::min(
        volatility_accumulator,
        s_params.max_volatility_accumulator.into(),
    )
    .try_into()
    .map_err(|_| ErrorCode::MathOverflow)?;

    Ok(())
}

/// Base fee rate in `FEE_PRECISION` units. Mirrors `LbPair::get_base_fee`.
pub fn get_base_fee(pool: &PoolView) -> Result<u128> {
    Ok(u128::from(pool.parameters.base_factor)
        .checked_mul(pool.bin_step.into())
        .ok_or(ErrorCode::MathOverflow)?
        .checked_mul(10u128)
        .ok_or(ErrorCode::MathOverflow)?
        .checked_mul(10u128.pow(pool.parameters.base_fee_power_factor.into()))
        .ok_or(ErrorCode::MathOverflow)?)
}

/// Variable fee rate for the pool's current volatility accumulator.
/// Mirrors `LbPair::get_variable_fee`.
pub fn get_variable_fee(pool: &PoolView) -> Result<u128> {
    compute_variable_fee(pool, pool.v_parameters.volatility_accumulator)
}

/// Variable fee rate for an arbitrary volatility accumulator value.
/// Mirrors `LbPair::compute_variable_fee`.
///
/// `variable_fee = ceil(variable_fee_control * (volatility_accumulator *
/// bin_step)^2 / 1e11)`.
pub fn compute_variable_fee(pool: &PoolView, volatility_accumulator: u32) -> Result<u128> {
    if pool.parameters.variable_fee_control > 0 {
        let volatility_accumulator: u128 = volatility_accumulator.into();
        let bin_step: u128 = pool.bin_step.into();
        let variable_fee_control: u128 = pool.parameters.variable_fee_control.into();

        let square_vfa_bin = volatility_accumulator
            .checked_mul(bin_step)
            .ok_or(ErrorCode::MathOverflow)?
            .checked_pow(2)
            .ok_or(ErrorCode::MathOverflow)?;

        let v_fee = variable_fee_control
            .checked_mul(square_vfa_bin)
            .ok_or(ErrorCode::MathOverflow)?;

        let scaled_v_fee = v_fee
            .checked_add(99_999_999_999)
            .ok_or(ErrorCode::MathOverflow)?
            .checked_div(100_000_000_000)
            .ok_or(ErrorCode::MathOverflow)?;

        return Ok(scaled_v_fee);
    }

    Ok(0)
}

/// Total fee rate (base + variable), capped at [`MAX_FEE_RATE`].
/// Mirrors `LbPair::get_total_fee`.
pub fn get_total_fee(pool: &PoolView) -> Result<u128> {
    let total_fee_rate = get_base_fee(pool)?
        .checked_add(get_variable_fee(pool)?)
        .ok_or(ErrorCode::MathOverflow)?;
    let total_fee_rate_cap = core::cmp::min(total_fee_rate, MAX_FEE_RATE.into());
    Ok(total_fee_rate_cap)
}

/// Fee to add on top of a fee-excluded amount (gross-up, ceil division).
/// Mirrors `LbPair::compute_fee`.
pub fn compute_fee(pool: &PoolView, amount: u64) -> Result<u64> {
    let total_fee_rate = get_total_fee(pool)?;
    let denominator = u128::from(FEE_PRECISION)
        .checked_sub(total_fee_rate)
        .ok_or(ErrorCode::MathOverflow)?;

    // Ceil division
    let fee = u128::from(amount)
        .checked_mul(total_fee_rate)
        .ok_or(ErrorCode::MathOverflow)?
        .checked_add(denominator)
        .ok_or(ErrorCode::MathOverflow)?
        .checked_sub(1)
        .ok_or(ErrorCode::MathOverflow)?;

    let scaled_down_fee = fee
        .checked_div(denominator)
        .ok_or(ErrorCode::MathOverflow)?;

    Ok(scaled_down_fee
        .try_into()
        .map_err(|_| ErrorCode::MathOverflow)?)
}

/// Fee contained inside a fee-included amount (ceil division).
/// Mirrors `LbPair::compute_fee_from_amount`.
pub fn compute_fee_from_amount(pool: &PoolView, amount_with_fees: u64) -> Result<u64> {
    let total_fee_rate = get_total_fee(pool)?;

    let fee_amount = u128::from(amount_with_fees)
        .checked_mul(total_fee_rate)
        .ok_or(ErrorCode::MathOverflow)?
        .checked_add((FEE_PRECISION - 1).into())
        .ok_or(ErrorCode::MathOverflow)?;

    let scaled_down_fee = fee_amount
        .checked_div(FEE_PRECISION.into())
        .ok_or(ErrorCode::MathOverflow)?;

    Ok(scaled_down_fee
        .try_into()
        .map_err(|_| ErrorCode::MathOverflow)?)
}

/// Protocol's share of a fee amount (floor division).
/// Mirrors `LbPair::compute_protocol_fee`.
pub fn compute_protocol_fee(pool: &PoolView, fee_amount: u64) -> Result<u64> {
    let protocol_fee = u128::from(fee_amount)
        .checked_mul(pool.parameters.protocol_share.into())
        .ok_or(ErrorCode::MathOverflow)?
        .checked_div(BASIS_POINT_MAX as u128)
        .ok_or(ErrorCode::MathOverflow)?;

    Ok(protocol_fee
        .try_into()
        .map_err(|_| ErrorCode::MathOverflow)?)
}

/// Move the active bin one step in the swap direction.
/// Mirrors `LbPair::advance_active_bin`.
pub fn advance_active_bin(pool: &mut PoolView, swap_for_y: bool) -> Result<()> {
    let next_active_bin_id = if swap_for_y {
        pool.active_id.checked_sub(1)
    } else {
        pool.active_id.checked_add(1)
    }
    .ok_or(ErrorCode::MathOverflow)?;

    crate::require!(
        next_active_bin_id >= MIN_BIN_ID && next_active_bin_id <= MAX_BIN_ID,
        ErrorCode::InsufficientLiquidity
    );

    pool.active_id = next_active_bin_id;

    Ok(())
}

/// Whether this pair supports limit orders based on its function_type and reward configuration.
/// Mirrors `LbPair::is_support_limit_order`; the reward-mint scan is replaced
/// by the caller-supplied [`PoolView::has_rewards`] flag.
pub fn is_support_limit_order(pool: &PoolView) -> bool {
    let Some(function_type) = FunctionType::try_from(pool.parameters.function_type).ok() else {
        return false;
    };
    match function_type {
        FunctionType::LimitOrder => true,
        FunctionType::LiquidityMining => false,
        FunctionType::Undetermined => !pool.has_rewards,
    }
}

/// Whether the fee is charged on the input token for the given swap direction.
/// Mirrors `LbPair::fee_on_input`.
pub fn fee_on_input(pool: &PoolView, swap_for_y: bool) -> bool {
    let Some(mode) = CollectFeeMode::try_from(pool.parameters.collect_fee_mode).ok() else {
        return true;
    };
    match mode {
        CollectFeeMode::InputOnly => true,
        CollectFeeMode::OnlyY => !swap_for_y,
    }
}
