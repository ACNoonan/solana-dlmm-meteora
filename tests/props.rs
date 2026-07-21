//! Property invariants under proptest (milestone 6, DESIGN.md test layer 5).
//!
//! These don't pin values — they assert structural contracts over fuzzed
//! inputs: price monotonicity in bin id, swap output bounded by supplied
//! reserves, fee bounded by input, the volatility-accumulator cap, and the
//! active bin landing inside the supplied range. Outcomes other than
//! success or `PoolOutOfLiquidity` (which fuzzed liquidity legitimately
//! produces) fail the property.

use proptest::prelude::*;
use solana_dlmm_meteora::{
    compute_swap_full, get_price_from_id, update_volatility_accumulator, BinView, ErrorCode,
    PoolView, StaticParameters, VariableParameters,
};

fn pool(active_id: i32, bin_step: u16) -> PoolView {
    PoolView {
        active_id,
        bin_step,
        has_rewards: false,
        parameters: StaticParameters {
            base_factor: 10_000,
            filter_period: 30,
            decay_period: 600,
            reduction_factor: 5_000,
            variable_fee_control: 40_000,
            max_volatility_accumulator: 350_000,
            min_bin_id: -443_636,
            max_bin_id: 443_636,
            protocol_share: 500,
            base_fee_power_factor: 0,
            function_type: 1,
            collect_fee_mode: 0,
        },
        v_parameters: VariableParameters {
            volatility_accumulator: 0,
            volatility_reference: 0,
            index_reference: active_id,
            last_update_timestamp: 0,
        },
    }
}

proptest! {
    #[test]
    fn price_monotonic_in_bin_id(id in -2_000i32..2_000, step in 1u16..=100) {
        let lo = get_price_from_id(id, step).unwrap();
        let hi = get_price_from_id(id + 1, step).unwrap();
        prop_assert!(lo < hi, "price not strictly increasing at id={id}, step={step}");
    }

    #[test]
    fn swap_bounded_by_reserves_and_input(
        reserves in proptest::collection::vec((0u64..10_000_000, 0u64..10_000_000), 1..6),
        amount_in in 1u64..30_000_000,
        swap_for_y: bool,
    ) {
        // Contiguous bins downward-from/upward-to the active bin so both
        // directions have something to walk.
        let active_id = 100i32;
        let n = reserves.len() as i32;
        let bins: Vec<BinView> = reserves
            .iter()
            .enumerate()
            .map(|(i, &(x, y))| BinView {
                bin_id: active_id - (n - 1) + i as i32,
                amount_x: x,
                amount_y: y,
                price: get_price_from_id(active_id - (n - 1) + i as i32, 100).unwrap(),
                ..BinView::default()
            })
            .collect();

        let total_out_reserve: u64 = bins
            .iter()
            .map(|b| if swap_for_y { b.amount_y } else { b.amount_x })
            .sum();

        let p = pool(active_id, 100);
        match compute_swap_full(&p, &bins, amount_in, swap_for_y, 1_000) {
            Ok(quote) => {
                prop_assert!(quote.amount_out <= total_out_reserve,
                    "amount_out {} exceeds supplied reserve {}", quote.amount_out, total_out_reserve);
                prop_assert!(quote.fee <= amount_in,
                    "fee {} exceeds amount_in {}", quote.fee, amount_in);
                prop_assert!(quote.protocol_fee <= quote.fee,
                    "protocol_fee {} exceeds fee {}", quote.protocol_fee, quote.fee);
                let after = quote.pool_after;
                prop_assert!(
                    after.v_parameters.volatility_accumulator
                        <= after.parameters.max_volatility_accumulator,
                    "volatility accumulator above cap",
                );
                // The active bin ends within the supplied range (success
                // means the loop never walked past the ends).
                let (min_id, max_id) = (bins[0].bin_id, bins[bins.len() - 1].bin_id);
                prop_assert!(after.active_id >= min_id.min(active_id));
                prop_assert!(after.active_id <= max_id.max(active_id));
            }
            Err(ErrorCode::PoolOutOfLiquidity) => {} // legitimate under fuzzed liquidity
            Err(e) => prop_assert!(false, "unexpected error: {e:?}"),
        }
    }

    #[test]
    fn volatility_accumulator_never_exceeds_cap(
        vol_ref in 0u32..2_000_000,
        idx_ref in -10_000i32..10_000,
        active in -10_000i32..10_000,
        cap in 1u32..2_000_000,
    ) {
        let mut p = pool(active, 100);
        p.parameters.max_volatility_accumulator = cap;
        p.v_parameters.volatility_reference = vol_ref;
        p.v_parameters.index_reference = idx_ref;
        update_volatility_accumulator(&mut p).unwrap();
        prop_assert!(p.v_parameters.volatility_accumulator <= cap);
    }
}
