// Math is extracted byte-for-byte from MeteoraAg/dlmm-sdk and we want to keep
// it diffable against upstream. We therefore turn off most clippy lints on the
// lib itself (the extracted code) — tests are linted normally.
#![allow(clippy::all)]

//! Pure-Rust, no-RPC swap math for the Meteora DLMM (Dynamic Liquidity Market
//! Maker) on Solana.
//!
//! This crate contains the deterministic integer arithmetic that the on-chain
//! Meteora DLMM program executes — extracted unchanged into a library that has
//! no dependency on `anchor-lang`, `solana-program`, the Solana runtime, or
//! the Anchor account model. Given pre-decoded pool state and bin-array data,
//! every function here is a pure function of its inputs.
//!
//! # Status
//!
//! Pre-v0.1. Milestones 1–2 of the v0.1 plan have landed (crate scaffold +
//! bin id ↔ price math). The remaining milestones — single-bin swap step,
//! dynamic-fee FSM, multi-bin orchestrator, and differential tests — are
//! tracked in `DESIGN.md`.
//!
//! # Provenance
//!
//! Math is extracted from
//! [`MeteoraAg/dlmm-sdk`](https://github.com/MeteoraAg/dlmm-sdk)
//! `commons/src/`. The arithmetic itself is byte-for-byte identical to the
//! upstream implementation; the only changes are dropping `anyhow::Result`
//! plumbing in favor of an internal [`ErrorCode`] and rehosting paths into
//! flat crate-root modules.

pub mod bin_math;
pub mod dynamic_fee;
pub mod error;
pub mod full_math;
pub mod swap_full;
pub mod swap_math;
pub mod u128x128_math;

// ---- curated public API ----

pub use error::ErrorCode;

pub use bin_math::{get_price_from_id, MAX_BIN_ID, MIN_BIN_ID};

pub use swap_math::{
    get_amount_in, get_amount_out, get_limit_order_amounts_by_direction, get_max_amount_in,
    get_max_amount_out, get_max_amount_out_with_limit_orders, get_or_store_bin_price, BinView,
};

pub use swap_full::{
    compute_swap_full, compute_swap_full_exact_out, swap_exact_in_quote_at_bin,
    swap_exact_out_quote_at_bin, BinQuoteResult, SwapExactInQuote, SwapExactOutQuote,
};

pub use dynamic_fee::{
    advance_active_bin, compute_fee, compute_fee_from_amount, compute_protocol_fee,
    compute_variable_fee, fee_on_input, get_base_fee, get_total_fee, get_variable_fee,
    is_support_limit_order, update_references, update_volatility_accumulator, CollectFeeMode,
    FunctionType, PoolView, StaticParameters, VariableParameters,
};

pub use u128x128_math::Rounding;
