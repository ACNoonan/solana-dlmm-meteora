# Design — solana-dlmm-meteora v0.1

Captured 2026-04-27 to plant the flag while the litesvm differential is
finished in `solana-clmm-raydium`. Future-you (or future-Claude) reads
this cold to start implementation.

## Goal

A pure-Rust, no-RPC Rust crate that exposes the deterministic integer
arithmetic of Meteora's DLMM swap path — extracted byte-for-byte from
`MeteoraAg/dlmm-sdk` `programs/lb_clmm/src/`, with no Anchor /
solana-program / runtime dependencies. Same shape and same posture as
[`solana-clmm-raydium`](https://github.com/ACNoonan/solana-clmm-raydium).

Given pre-decoded pool state and bin-array data, every public function
is pure, deterministic, and round-trips byte-exact against the on-chain
program output.

## What's a DLMM

**DLMM = Dynamic Liquidity Market Maker.** Meteora's term, not industry-
standard. Two ways it differs fundamentally from a tick-based CLMM:

1. **Bin-based price discretization.** Instead of continuous sqrt-price
   ticks, price space is partitioned into fixed-width **bins**. A bin
   is identified by an `i32` `bin_id`, and the price within a bin is
   constant — `(1 + bin_step / 10_000) ^ bin_id`. Liquidity in a bin is
   a flat slab, not an integral over a range. Swap math reduces to:
   "consume the active bin's liquidity until it runs out, advance to the
   next bin, repeat."
2. **Dynamic fees.** A volatility-tracking FSM bumps the swap fee during
   volatile periods. The fee is `base_fee + variable_fee(volatility)`
   and the variable component depends on a "volatility accumulator" that
   updates per-swap. No analog in our extracted Raydium code.

Other distinctive bits:
- Fee/protocol-fee splits are computed in-bin, not at the pool level.
- Swaps that traverse many bins amortize the dynamic-fee FSM updates;
  the on-chain code uses a "reference" volatility that's frozen at the
  swap's first bin and decayed thereafter.

## Math primitives to extract

Mirroring upstream `MeteoraAg/dlmm-sdk` paths:

| Upstream | Our module | Notes |
|---|---|---|
| `programs/lb_clmm/src/math/u64x64_math.rs` | `bin_math.rs` | bin id ↔ price (Q64.64), `get_price_from_id` |
| `programs/lb_clmm/src/math/safe_math.rs` | `full_math.rs` | likely portable verbatim from `solana-clmm-raydium` |
| `programs/lb_clmm/src/math/utils_math.rs` | `bin_math.rs` (merge) | `safe_mul_div_cast` etc. |
| `programs/lb_clmm/src/state/dynamic_fee.rs` | `dynamic_fee.rs` | **The FSM. Net-new vs CLMM.** |
| `programs/lb_clmm/src/manager/bin_array_manager.rs` | `bin_array.rs` | bin lookup, walking |
| `programs/lb_clmm/src/manager/swap_manager.rs` | `swap_full.rs` | the orchestration loop |
| `programs/lb_clmm/src/state/lb_pair.rs` swap helpers | `swap_math.rs` | single-bin step |
| `commons/src/quote.rs` | (reference for diff testing) | NOT extracted — used as differential proptest oracle |

The dynamic-fee FSM is the highest-risk port. It's the part with no CLMM
analog and the part most likely to drift between upstream releases.
Extract carefully; ship verbatim from the released `programs/lb_clmm`
crate (not `commons` — `commons` may rewrite it).

## Public API surface (target)

Mirror `solana-clmm-raydium`'s curated re-exports as closely as makes
sense:

```rust
pub use bin_math::{
    get_price_from_id, get_id_from_price, MAX_BIN_ID, MIN_BIN_ID,
};

pub use swap_math::{compute_swap_step, SwapStep};

pub use swap_full::{compute_swap_full, BinPool, BinView, SwapResult};

pub use dynamic_fee::{
    DynamicFeeState, FeeParameters, update_volatility_accumulator,
};

pub use error::ErrorCode;
```

`compute_swap_full(&BinPool, &[BinView], amount, sqrt_price_limit, ...)`
should be the headline "one call to swap" function, same role as in
`solana-clmm-raydium`. `BinView` is the flat (bin_id, liquidity_x,
liquidity_y) view that the caller flattens from their decoded
`BinArray`s.

## Test strategy

Five layers, in priority order:

1. **Verbatim port of upstream test vectors** from `programs/lb_clmm`
   inline `#[test]`s and `commons/tests/`. Same approach as porting
   spl-token-2022's transfer-fee tests in `solana-clmm-raydium`.
2. **Differential proptest** against `MeteoraAg/dlmm-sdk` `commons`
   crate as a dev-dep — directly compare our `compute_swap_full` to
   `commons::quote::quote_exact_in`/`_out` over fuzzed inputs.
   Locks parity automatically as upstream evolves.
3. **Mainnet replay** of 10–20 captured swaps via the same Helius
   pattern this org's CLMM crate uses. DLMM pools have higher activity
   than Token-2022-on-CLMM, so capture is easier.
4. **Litesvm differential** — load the on-chain `lb_clmm` ELF, drive
   swap instructions, compare to our math byte-for-byte. The harness
   pattern carries directly from
   `solana-clmm-raydium/tests/litesvm_diff.rs` (once finished there).
   Reference: `commons/tests/integration/test_swap.rs`.
5. **Property invariants** under proptest: monotonicity of price in
   bin id, swap-step bounds, fee accumulation conservation.

## Reuse from `solana-clmm-raydium`

Direct lifts (zero changes):
- `error.rs` / `Result<T>` / `require!` / `require_gt!` / `require_gte!`
  macros.
- `big_num.rs` / `full_math.rs` / `unsafe_math.rs` (U256/U128/MulDiv
  machinery).
- `Cargo.toml` shape, `[lib] doctest = false`, `package.exclude` for
  fixtures, no_std posture (`#![allow(clippy::all)]` on extracted code).
- CI workflow (`.github/workflows/ci.yml`).
- README badge layout, CHANGELOG (Keep-a-Changelog) structure.
- `scripts/fetch_fixtures.py` adapted to DLMM program ID + account
  discriminators.
- Audit-doc structure in `docs/audits/`.

Roughly half of `solana-clmm-raydium`'s code is reusable infrastructure.

## v0.1 milestones (rough order)

1. **Repo scaffold** — `cargo new --lib`, copy infrastructure files
   from `solana-clmm-raydium`. (~1 hr)
2. **Bin math** — `get_price_from_id`, `get_id_from_price`, constants,
   bounds. Verbatim port + value-pinned tests. (~half day)
3. **Single-bin swap step** — analog of `compute_swap_step`.
   (~half day)
4. **Dynamic-fee FSM** — port `state/dynamic_fee.rs`. Carefully —
   this is the one Meteora-specific piece. (~1 day)
5. **Multi-bin orchestrator** — `compute_swap_full` analog with bin-
   array walking. (~half day)
6. **Test layer** — verbatim port of upstream tests + differential
   proptest + mainnet replay (5–10 fixtures). (~1 day)
7. **README + CHANGELOG + docs.rs prep** — copy
   `solana-clmm-raydium`'s pattern. (~2 hrs)
8. **`cargo publish` 0.1.0**.

Total estimate: **5–7 working days**.

## Decisions deferred to implementation time

- Whether to ship `commons`-style high-level quote helpers
  (`quote_exact_in` / `quote_exact_out` wrappers) at v0.1 or v0.2.
- Whether to expose the `BinArray` decoder (probably not — same
  rationale as `solana-clmm-raydium` not exposing `PoolState` decoders;
  caller's job).
- Whether to feature-gate the differential dev-deps. Likely no — same
  rationale as `solana-clmm-raydium`'s spl-token-2022-interface dev-dep.

## References

- Upstream: <https://github.com/MeteoraAg/dlmm-sdk>
- Litesvm test pattern: `MeteoraAg/dlmm-sdk` `commons/tests/integration/test_swap.rs`
- Sibling crate: <https://github.com/ACNoonan/solana-clmm-raydium>
- Audit framing for the ecosystem gap: that crate's
  `docs/audits/v0.1.0-external-review.md` §1.x.
