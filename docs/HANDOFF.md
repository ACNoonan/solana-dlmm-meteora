# HANDOFF — v0.1 feature-complete, awaiting publish

For the next agent (or future-Adam) picking this up cold. Read this
first; then `CHANGELOG.md` for the running record of what landed; then
`DESIGN.md` for original framing (see its dated amendment blocks — the
2026-07-21 block records how the three open decisions were resolved and
how the surface evolved past the original sketch).

## Status

- **All v0.1 milestones (1–7) from `DESIGN.md` are done.** The full
  swap path is ported: bin math, per-bin primitives, dynamic-fee FSM,
  and both orchestrators (`compute_swap_full` exact-in,
  `compute_swap_full_exact_out`), including limit orders and
  collect-fee-mode — upstream features newer than DESIGN.md.
- **39 tests passing**, fmt + clippy (lib + tests) clean:
  - `tests/units.rs` — bin math + per-bin primitives (11)
  - `tests/dynamic_fee.rs` — FSM value pins (9)
  - `tests/swap_full.rs` — orchestrator value pins incl. limit orders,
    fee modes, round trip (12)
  - `tests/props.rs` — proptest invariants (3)
  - `tests/mainnet_replay.rs` — captured SOL-USDC mainnet state,
    oracle-pinned quotes (4)
- One runtime dep: `ruint`. One dev-dep: `proptest`.
- **Not yet on crates.io. Adam handles publishing** — `Cargo.toml`
  metadata is ready; `cargo publish --dry-run` is the remaining step.

## The shape that shipped (decisions resolved 2026-07-21)

| Decision | Resolution |
|---|---|
| Where's the FSM upstream? | All of it in `commons/src/extensions/lb_pair.rs`; ported to `src/dynamic_fee.rs` as free functions |
| `LbPair` shape | `PoolView` projection: `active_id`, `bin_step`, `has_rewards`, verbatim `StaticParameters`/`VariableParameters` PODs (IDL layouts minus padding). Status/activation gating = caller's job |
| Mutation API | FSM takes `&mut PoolView` (upstream-faithful); orchestrators copy internally (upstream's own `let mut lb_pair = *lb_pair`) and return `pool_after` in the result |
| Limit orders / collect-fee-mode | Ported (full current-upstream parity). `BinView` carries `price` + the three limit-order fields; MM-only pools degenerate to identical results |

Two deliberate, documented divergences from upstream `quote.rs`:

1. **Bin walking**: flat sorted `&[BinView]` instead of bin arrays +
   bitmaps. Absent id = empty bin; walking past the supplied bins ⇒
   `ErrorCode::PoolOutOfLiquidity` (upstream's bitmap-exhausted error).
2. **`pool_after.last_update_timestamp = current_timestamp`**: the
   throwaway upstream quote never writes it; the on-chain swap does.
   Needed for chained simulation to decay correctly.

Replicated upstream quirk worth knowing: the exact-**out** loop resolves
an uninitialized (`price == 0`) bin via `get_or_store_bin_price`; the
exact-**in** loop reads `bin.price` as-is. Callers should pass decoded
prices (or 0 only where upstream tolerates it).

## Conventions that must hold (unchanged)

- **Verbatim from upstream.** Every ported function carries a comment
  pointing at the upstream file. Allowed adjustments: `anyhow::Result` →
  `crate::error::Result`, `.context(...)` → `.ok_or(ErrorCode::...)`,
  method → free-function rehosting, explicit `use` paths.
- **Value-pinned tests via independent oracle** — expectations come from
  `scripts/oracle_fsm.py` / `scripts/oracle_swap.py` (independent Python
  re-implementations), never from the Rust under test.
- `#[allow(clippy::all)]` on the lib root; tests linted with `-D warnings`.
- Update `CHANGELOG.md` + the README Status line every milestone.

## How to validate any change

```sh
cargo build && cargo test
cargo fmt --all -- --check
cargo clippy --tests -- -D warnings && cargo clippy --lib
```

## scripts/

- `oracle_fsm.py`, `oracle_swap.py` — the independent oracles. Run
  directly to reprint every pinned value in the test suite.
- `capture_fixture.py` — re-captures live SOL-USDC pool state from
  public RPC (stdlib-only: JSON-RPC, anchor-IDL account decode, ed25519
  PDA derivation) and regenerates `tests/mainnet_replay.rs`. Re-running
  it changes the pinned numbers (live state moved) — commit the
  regenerated file and its new expectations together.

## v0.2+ roadmap (deferrals locked in)

| Item | Notes |
|---|---|
| `get_id_from_price` | Upstream is float-based (`rust_decimal` log10). Re-derive integer-only, value-pin against the float impl across the bin range |
| Token-2022 transfer-fee math | Lift sibling's `transfer_fee` module verbatim; wraps amounts around `compute_swap_full` |
| `commons` differential proptest | Add `commons` as dev-dep (heavy anchor graph — that's why it waited), fuzz ours vs `quote_exact_in`/`_out` |
| `litesvm` differential | v0.3, mirroring sibling. Load `lb_clmm.so`, drive swap ixs, byte-exact compare. Also upgrades mainnet replay from oracle-pinned to tx-exact |
| `safe_*_cast` re-genericization | Only if a future port needs non-u64 targets |

## Don'ts

- Don't `git push` or `cargo publish` without explicit instruction
  (Adam handles both). Local commits on `main` are fine.
- Don't add `MeteoraAg/dlmm-sdk` as a dev-dep before the v0.2
  differential work starts.
- Don't edit `tests/mainnet_replay.rs` numbers by hand — regenerate via
  `scripts/capture_fixture.py`.

## End-of-session pointers (2026-07-21)

- Branch: `main`, four local commits ahead of the 2026-04-27 push
  (milestones 4, 5, 6, 7). Not pushed.
- All 39 tests passing, fmt + clippy clean.
- Next action: Adam reviews, pushes, runs `cargo publish --dry-run`,
  then `cargo publish` 0.1.0 and tags `v0.1.0`.
