# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

**v0.1 feature-complete** — milestones 1–7 of 7 from `DESIGN.md`
landed. Remaining before the 0.1.0 release: `cargo publish` (Adam).

### Added
- **Docs for release** — milestone 7: crate-root docs updated to the
  final v0.1 surface with a quick-start; README gained a Usage section
  and the license line went from "will be" to "is"; `DESIGN.md` gained
  a dated amendment block recording how the three HANDOFF decisions
  were resolved and how the surface tracked upstream's limit-order /
  collect-fee-mode evolution; `docs/HANDOFF.md` rewritten for the
  post-v0.1 state (v0.2+ roadmap, capture-script workflow, publish
  checklist).
- **Property invariants under proptest** (`tests/props.rs`) — milestone
  6, DESIGN.md test layer 5: price strictly monotonic in bin id (fuzzed
  over ±2_000 ids × bin_steps 1–100), swap output bounded by supplied
  reserves / fee bounded by input / protocol fee bounded by fee /
  active bin landing inside the supplied range (fuzzed pools, bins, and
  amounts, accepting `PoolOutOfLiquidity` as a legitimate outcome), and
  the volatility-accumulator cap.
- **Mainnet-replay fixture** (`tests/mainnet_replay.rs`) — test layer
  3, adapted: real SOL-USDC pool state (the pair upstream's own
  integration tests use) captured 2026-07-21 via public RPC — LbPair +
  the three bin arrays around the active bin, 51 live bins — with four
  quotes (1 and 5 SOL exact-in, 100 USDC exact-in reversed, 100 USDC
  exact-out) pinned by the independent Python oracle over the captured
  state. Byte-exact *transaction* replay (capturing a historical swap's
  pre-state needs an archive node) is deliberately deferred to the
  litesvm differential in v0.3, mirroring the sibling's pacing.
- **`scripts/`** — the independent Python oracles (`oracle_fsm.py`,
  `oracle_swap.py`) that generated every pinned expectation in the test
  suite, plus `capture_fixture.py`, which re-captures live pool state
  (pure-stdlib RPC + anchor-IDL account decoding + ed25519 PDA
  derivation) and regenerates `tests/mainnet_replay.rs`.
- **Multi-bin orchestrator** (`swap_full` module) — milestone 5. Verbatim
  port of the `commons/src/quote.rs` math core: the per-bin quote steps
  `swap_exact_in_quote_at_bin` / `swap_exact_out_quote_at_bin` (the DLMM
  analog of a CLMM `compute_swap_step`), their fill helpers
  (MM → processed limit orders → open limit orders), `split_fee`
  (limit-order fee share vs protocol share), and the exact-in /
  exact-out orchestration loops as `compute_swap_full` /
  `compute_swap_full_exact_out`. Limit orders and collect-fee-mode
  (decision 3: full current-upstream parity) are included.
- **Account plumbing stripped, math kept** — bin-array + bitmap walking
  is replaced by walking the caller's flat `&[BinView]` (sorted
  ascending by `bin_id`; an absent id is an empty bin; walking past the
  supplied bins errors `PoolOutOfLiquidity`); `Clock` becomes a
  `current_timestamp` parameter; Token-2022 transfer-fee wrapping stays
  v0.2; `validate_swap_activation` stays with the caller.
- **`SwapExactInQuote` / `SwapExactOutQuote`** mirror the upstream
  result shapes plus a `pool_after: PoolView` field (decision 2: the
  orchestrator copies the pool internally, exactly like upstream's
  `let mut lb_pair = *lb_pair`, and returns the post-swap state).
  One deliberate divergence from the quote path, documented in the
  module docs: `pool_after.last_update_timestamp` is set to
  `current_timestamp`, matching what the on-chain swap persists
  (the throwaway upstream quote never writes it) so chained
  simulation decays correctly.
- **`BinView` extended** with the upstream `Bin` fields the quote path
  reads: `price` (stored price, 0 = uninitialized; the exact-out loop
  resolves it via the new `get_or_store_bin_price`, the exact-in loop
  reads it as-is — both replicated from upstream), plus
  `limit_order_ask_side` / `open_order_amount` /
  `processed_order_remaining_amount`. New `swap_math` functions:
  `get_or_store_bin_price`, `get_limit_order_amounts_by_direction`,
  `get_max_amount_out_with_limit_orders`.
- **`get_amount_in` / `get_amount_out` regained the upstream `Rounding`
  parameter** (milestone 3 had specialized them to hardcoded rounding;
  the quote path passes rounding explicitly, so the verbatim signature
  wins). Breaking vs the unreleased milestone-3 surface.
- **Twelve value-pinned orchestrator tests** (`tests/swap_full.rs`)
  from an independent Python re-implementation of the full quote path:
  single-bin exact-in, 3-bin crossings both directions with climbing
  dynamic fees, fee-on-output mode, limit-order fills across all three
  liquidity layers, warm-pool reference decay, out-of-liquidity (and
  empty-slice) errors, gap-bin equivalence, exact-out with the
  drain-entire-bin path, lazy-price resolution, and an
  exact-in → exact-out round trip that reproduces the input exactly.
- **Dynamic-fee FSM** (`dynamic_fee` module) — milestone 4, the
  Meteora-specific piece with no CLMM analog. Verbatim port of the fee
  logic in `commons/src/extensions/lb_pair.rs`: `update_references`
  (once-per-swap reference freeze/decay against `filter_period` /
  `decay_period`), `update_volatility_accumulator` (per-bin-crossed
  accumulation capped at `max_volatility_accumulator`),
  `advance_active_bin`, and the fee computations `get_base_fee`,
  `compute_variable_fee`, `get_variable_fee`, `get_total_fee` (capped at
  `MAX_FEE_RATE`), `compute_fee` (gross-up, ceil),
  `compute_fee_from_amount` (contained fee, ceil), and
  `compute_protocol_fee` (floor). Plus `is_support_limit_order` /
  `fee_on_input` and their `FunctionType` / `CollectFeeMode` conversions
  (`commons/src/conversions/`) — upstream features newer than DESIGN.md
  that the current quote path depends on.
- **`PoolView` projection** (decision 2 of `docs/HANDOFF.md`, resolved
  as sketched): a flat caller-prepared projection of the upstream
  `LbPair` account carrying `active_id`, `bin_step`, `has_rewards`, and
  verbatim `StaticParameters` / `VariableParameters` PODs (layouts from
  the anchor IDL, minus explicit padding). FSM functions are free
  functions over `&mut PoolView` — same field paths as upstream methods,
  so the extracted bodies stay diffable. Pubkeys, rewards, oracle, and
  the bin-array bitmap stay in the caller's decoded account; pool
  status / activation gating is account validation and stays out.
- Three `ErrorCode` variants: `InsufficientLiquidity` (upstream
  `advance_active_bin` bound), `PoolOutOfLiquidity` (reserved for the
  milestone-5 orchestrator loop), `InvalidParameter` (enum-discriminant
  conversions).
- **Nine value-pinned FSM tests** (`tests/dynamic_fee.rs`) from an
  independent Python oracle, over a realistic bin_step=25 preset:
  quiet-pool fees, variable fee at volatility, `base_fee_power_factor`
  scaling and the `MAX_FEE_RATE` cap, all three `update_references`
  branches, accumulator accumulation + cap, a swap-shaped 3-bin FSM
  sequence with climbing fees, `advance_active_bin` bounds, and the
  limit-order-support / collect-fee-mode flag tables.

### Added (milestones 1–3)
- **Crate scaffold** lifted from sibling
  [`solana-clmm-raydium`](https://github.com/ACNoonan/solana-clmm-raydium):
  `Cargo.toml` (MSRV 1.81, dual MIT/Apache-2.0, no runtime deps,
  `package.exclude` shaped for future fixtures), CI workflow, internal
  `ErrorCode` (`MathOverflow`, `BinIdOutOfBounds`) with
  `core::fmt::Display` + `core::error::Error`, and `require!` /
  `require_gt!` / `require_gte!` macros.
- The big-integer infrastructure (`big_num` / `full_math` /
  `unsafe_math` / `fixed_point_64`) was lifted from sibling and then
  deleted in the same milestone window once it became clear upstream
  Meteora DLMM math uses raw `u128` + `ruint::U256` and never touches
  the U128/U512/U1024 plumbing the sibling carries. Dropping it kept
  the v0.1 dep graph at zero runtime crates.
- **Bin id → Q64.64 price** (`bin_math` module). Verbatim port of
  upstream `commons/src/math/u64x64_math.rs::pow` (the 19-bit binary-
  exponential `(1 + bin_step / 10_000)^active_id` engine) and
  `commons/src/math/price_math.rs::get_price_from_id`. Public surface:
  `get_price_from_id`, `MAX_BIN_ID`, `MIN_BIN_ID`, plus the unhidden
  helpers `pow`, `ONE`, `SCALE_OFFSET`, `BASIS_POINT_MAX`, `PRECISION`
  on the `bin_math` module.
- **Five value-pinned tests** in `tests/units.rs`. Expectations were
  generated by an independent Python re-implementation of upstream's
  algorithm — the pinned integers catch porting mistakes the way a
  round-trip-through-the-function-under-test cannot. Coverage:
  `exp == 0` short-circuit (returns `2^64` regardless of `bin_step`),
  hand-pinned mid-range positive/negative IDs, the `MAX_BIN_ID` /
  `MIN_BIN_ID` saturation boundaries (price → `u128::MAX` / `1`),
  upstream constants match (`±443_636`), and a small monotonicity
  sanity check around `id = 0`.

- **Structural per-bin swap primitives** (`swap_math` module). Verbatim
  port of the four fee-independent halves of upstream's `Bin::swap`
  (`commons/src/extensions/bin.rs`): `get_amount_out`, `get_amount_in`,
  `get_max_amount_out`, `get_max_amount_in`, plus the `BinView` flat
  caller-prepared projection (`bin_id`, `amount_x`, `amount_y`).
  Rounding follows upstream — output paths round down, input paths round
  up, matching exact-in / exact-out semantics.
- **256-bit integer math** (`u128x128_math` and `full_math` modules).
  Verbatim ports of `commons/src/math/u128x128_math.rs` (`mul_div`,
  `mul_shr`, `shl_div`, `Rounding`) and the `safe_*_cast` helpers from
  `commons/src/math/utils.rs`. The `safe_*_cast` family is specialized
  to `u64` returns (`safe_mul_shr_u64`, `safe_shl_div_u64`,
  `safe_mul_div_u64`) since every call site in `commons/src/extensions/bin.rs`
  instantiates with `T = u64`. Avoids pulling `num-traits` into the dep
  graph; can be re-genericized in v0.2 if the dynamic-fee FSM needs it.
- **`ruint` runtime dep** (single, light). Matches upstream — `commons`
  uses `ruint::aliases::U256` for the same 256-bit multiply path. No
  anchor / solana-program coupling pulled in.
- **Six new value-pinned tests** in `tests/units.rs`. Independent
  Python oracle of the upstream `mul_div` / `mul_shr` / `shl_div`
  algorithm produced expected values; tests cover the unit-price
  identity, price = 2.0, ceil/floor rounding divergence at price = ONE+1,
  a realistic Q64.64 price (active_id=100, bin_step=100) plus the
  exact-in/exact-out asymmetry, max-amount caps, and the u128 → u64
  cast surfacing as `MathOverflow` instead of silently wrapping.

### Architecture note
- DLMM's per-bin swap step is **not** a self-contained pure function
  upstream — `Bin::swap` calls into `&LbPair` for every fee
  computation, since the fee is dynamic. Milestone 3 ships only the
  fee-independent (structural) half here. The dynamic-fee FSM ports
  in milestone 4; the orchestrator that composes them into
  `compute_swap_step` / `compute_swap_full` ports in milestone 5. This
  splitting mirrors upstream's per-bin / per-pool architecture rather
  than synthesizing a non-verbatim self-contained step.

### Notes on the extraction
- Upstream module paths in `DESIGN.md` (`programs/lb_clmm/src/math/...`)
  no longer exist on `MeteoraAg/dlmm-sdk` `main`. The math now lives in
  the `commons` crate at `commons/src/math/` and `commons/src/constants.rs`;
  the arithmetic is byte-for-byte the same as DESIGN.md described, just
  in a different location. We extract from `commons/` directly. The
  v0.2 differential proptest (milestone 6) can therefore use `commons`
  directly as a dev-dep without an extra hop.
- Only adjustments versus upstream are: `anyhow::Result<T>` →
  `crate::error::Result<T>`, and `.context("overflow")` →
  `.ok_or(ErrorCode::MathOverflow)?`. The integer arithmetic itself
  (every shift, mul, div, branch in `pow`) is unchanged.

### Deferred
- **`get_id_from_price` deferred to a later milestone.** Upstream's only
  implementation lives in `cli/src/math.rs` and uses `rust_decimal`
  floats (`price.log10().checked_div(base.log10())`). Shipping it now
  would either (a) introduce a `rust_decimal` runtime dependency,
  breaking the no-RPC, deterministic-integer-only posture, or (b)
  require a from-scratch Q64.64 integer-log not present in upstream
  (so non-verbatim, non-trivial to round-trip-pin). DESIGN.md will be
  amended at v0.1 cut to reflect the split.
