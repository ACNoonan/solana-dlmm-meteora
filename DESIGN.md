# Design — solana-dlmm-meteora v0.1

Captured 2026-04-27 to plant the flag while the litesvm differential is
finished in `solana-clmm-raydium`. Future-you (or future-Claude) reads
this cold to start implementation.

> **Amendments (2026-04-27, milestones 1+2 cut):** the upstream paths
> below originally pointed at `programs/lb_clmm/src/...`; that directory
> doesn't exist on `MeteoraAg/dlmm-sdk` `main`. The Meteora math now
> lives in the `commons` crate. The "Math primitives to extract" table
> and "Public API surface" sections below have been corrected to match
> reality. Three deferrals out of the original v0.1 surface are now in
> the table for the same reason — they don't have an integer-deterministic
> upstream to extract from. See [`CHANGELOG.md`](CHANGELOG.md) for the
> running record.
>
> **Amendments (2026-07-21, milestones 4–7 cut — v0.1 feature-complete):**
> the three open decisions from `docs/HANDOFF.md` were resolved:
> (1) the FSM lives entirely in `commons/src/extensions/lb_pair.rs`,
> confirmed; (2) pool state ships as a flat `PoolView` projection
> (`active_id`, `bin_step`, `has_rewards`, verbatim `StaticParameters` /
> `VariableParameters` PODs) rather than a full `LbPair` mirror;
> (3) FSM functions mutate `&mut PoolView` in place (upstream-faithful),
> while the orchestrators copy internally — exactly upstream's
> `let mut lb_pair = *lb_pair` — and return the post-swap state in the
> result (`pool_after`). Additionally, upstream had grown **limit
> orders** and **collect-fee-mode** since this document was written; the
> port follows current upstream `quote.rs` (full parity) rather than the
> MM-only surface sketched below, so `compute_swap_full` /
> `compute_swap_full_exact_out` port the quote path per-bin functions
> (`swap_exact_in_quote_at_bin` / `swap_exact_out_quote_at_bin`) instead
> of `Bin::swap`, and the result types mirror `SwapExactInQuote` /
> `SwapExactOutQuote` (plus `pool_after`) instead of
> `typedefs.rs::SwapResult`. The API sketch below is kept for the
> historical record; `src/lib.rs` is the source of truth.

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

Mirroring upstream `MeteoraAg/dlmm-sdk` paths (corrected 2026-04-27 —
upstream consolidated all of this into the `commons` crate; the
`programs/lb_clmm/src/...` paths in earlier drafts no longer exist):

| Upstream | Our module | Notes |
|---|---|---|
| `commons/src/math/u64x64_math.rs` (`pow`) | `bin_math.rs` | Q64.64 binary-exponential engine |
| `commons/src/math/price_math.rs` | `bin_math.rs` | `get_price_from_id` |
| `commons/src/constants.rs` | `bin_math.rs` (constants) | `MIN_BIN_ID`, `MAX_BIN_ID`, `BASIS_POINT_MAX` |
| `commons/src/math/u128x128_math.rs`, `utils.rs` | `full_math.rs` (TBD) | `mul_div`, `mul_shr`, `safe_*_cast` — port lazily as swap math demands them |
| `commons/src/extensions/bin.rs` (per-bin swap) | `swap_math.rs` | single-bin step |
| `commons/src/extensions/lb_pair.rs` (FSM glue) + dynamic-fee logic in the on-chain program | `dynamic_fee.rs` | **The FSM. Net-new vs CLMM.** Highest-risk port. |
| `commons/src/extensions/bin_array.rs`, `bin_array_bitmap.rs` | `bin_array.rs` | bin lookup, walking |
| `commons/src/quote.rs` | `swap_full.rs` | the orchestration loop (and a v0.2+ differential proptest oracle) |

Three primitives originally listed in the v0.1 surface but **deferred
out**, because upstream's only implementation isn't an integer-
deterministic primitive we can extract:

| Primitive | Why deferred | Likely fate |
|---|---|---|
| `get_id_from_price` | Upstream lives in `cli/src/math.rs` and uses `rust_decimal::Decimal::log10` — taking the dep would break the no-RPC integer-only posture; re-deriving an integer Q64.64 log is non-verbatim and non-trivial to value-pin | Re-derive integer-only in v0.2, value-pinned against the upstream float impl across the bin range |
| Token-2022 transfer-fee math | Symmetric with sibling — sibling shipped this in v0.2 (after v0.1 + audit) | v0.2 here as well; lift the sibling's `transfer_fee` module verbatim, re-pin against the same `spl_token_2022_interface` differential |
| `quote_exact_in` / `quote_exact_out` high-level wrappers | DESIGN.md called these out as "decisions deferred to implementation time"; sibling stopped at `compute_swap_full` for v0.1 | v0.2; trivial wrappers around `compute_swap_full` once the orchestrator exists |

The dynamic-fee FSM is the highest-risk port. It's the part with no CLMM
analog and the part most likely to drift between upstream releases.
Extract carefully and value-pin against captured mainnet swap fixtures
plus the future litesvm differential.

## Public API surface (target)

Mirror `solana-clmm-raydium`'s curated re-exports as closely as makes
sense:

```rust
pub use bin_math::{get_price_from_id, MAX_BIN_ID, MIN_BIN_ID};
//                                ^^^^ get_id_from_price deferred to v0.2

pub use swap_math::{compute_swap_step, SwapStep};

pub use swap_full::{compute_swap_full, BinPool, BinView, SwapResult};

pub use dynamic_fee::{
    DynamicFeeState, FeeParameters, update_volatility_accumulator,
};

pub use error::ErrorCode;
```

`compute_swap_full(&BinPool, &[BinView], amount, sqrt_price_limit, ...)`
should be the headline "one call to swap" function, same role as in
`solana-clmm-raydium`. `BinView` is a **flat** caller-prepared
projection of the upstream `Bin` (`bin_id`, `amount_x`, `amount_y`,
`liquidity_supply` — whatever the swap step needs and nothing more);
the caller flattens decoded `BinArray`s into `&[BinView]` themselves.
This keeps the v0.1 surface narrow and stable across upstream
`Bin`/`BinArray` changes.

`SwapResult` mirrors upstream `commons/src/typedefs.rs::SwapResult`
field names (`amount_in_with_fees`, `amount_out`, `fee`,
`protocol_fee_after_host_fee`, `host_fee`, `is_exact_out_amount`)
rather than re-shaping into the sibling's flatter form. This shape
is what the future differential proptest will compare against, so
making the diff trivial is worth the slightly chunkier surface.

## Test strategy

Five layers, in priority order:

1. **Verbatim port of upstream test vectors** from `programs/lb_clmm`
   inline `#[test]`s and `commons/tests/`. Same approach as porting
   spl-token-2022's transfer-fee tests in `solana-clmm-raydium`.
2. **Differential proptest** against `MeteoraAg/dlmm-sdk` `commons`
   crate as a dev-dep — directly compare our `compute_swap_full` to
   `commons::quote::quote_exact_in`/`_out` over fuzzed inputs.
   Locks parity automatically as upstream evolves. **Deferred to v0.2:**
   `commons` pulls anchor-lang + the full anchor codegen + solana-program;
   sibling went without a heavy differential dev-dep at v0.1 and added
   only `spl-token-2022-interface` for `transfer_fee` parity, then
   `litesvm` at v0.3. Same pacing here.
3. **Mainnet replay** of 10–20 captured swaps. Use public mainnet RPC
   (`api.mainnet-beta.solana.com`) for capture rather than Helius — it
   works for one-shot fetches without a `.env` requirement, and DLMM
   pools have enough activity that we don't need Helius's higher rate
   limit. Deviation from sibling, intentional.
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

- ~~Whether to ship `commons`-style high-level quote helpers
  (`quote_exact_in` / `quote_exact_out` wrappers) at v0.1 or v0.2.~~
  **Resolved 2026-04-27: v0.2.** `compute_swap_full` is the v0.1 ceiling.
- Whether to expose the `BinArray` decoder. **Resolved 2026-04-27: no,
  caller's job.** `BinView` is the flat caller-prepared projection;
  decoding stays out of the published crate, mirroring sibling's
  `PoolState` posture.
- ~~Whether to feature-gate the differential dev-deps.~~
  **Resolved 2026-04-27: drop the `commons` differential entirely from
  v0.1.** v0.1 leans on mainnet replay + property invariants; v0.2 adds
  `commons` (or litesvm) once we know what shape the orchestrator
  settles on.

## References

- Upstream: <https://github.com/MeteoraAg/dlmm-sdk>
- Litesvm test pattern: `MeteoraAg/dlmm-sdk` `commons/tests/integration/test_swap.rs`
- Sibling crate: <https://github.com/ACNoonan/solana-clmm-raydium>
- Audit framing for the ecosystem gap: that crate's
  `docs/audits/v0.1.0-external-review.md` §1.x.
