# solana-dlmm-meteora

Pure-Rust, no-RPC swap math for the [Meteora DLMM](https://github.com/MeteoraAg/dlmm-sdk)
(Dynamic Liquidity Market Maker) on Solana. Sibling to
[`solana-clmm-raydium`](https://github.com/ACNoonan/solana-clmm-raydium).

## Status

**Pre-v0.1 (milestones 1–6 of 7 landed).** The full swap path is ported
verbatim from `MeteoraAg/dlmm-sdk` `commons/`: bin id → Q64.64 price,
the 256-bit multiply-divide engine, the per-bin swap primitives, the
dynamic-fee FSM (volatility accumulator/reference updates plus the
base + variable fee computations) over a flat `PoolView` projection,
and the multi-bin orchestrators `compute_swap_full` /
`compute_swap_full_exact_out` (limit orders and collect-fee-mode
included). 39 tests: value-pinned expectations from an independent
Python oracle, proptest invariants, and a captured-mainnet-state
replay of the SOL-USDC pair. Remaining before 0.1.0: `cargo publish`.
See [`DESIGN.md`](DESIGN.md) for the design, math differences from
CLMM, module layout, and test strategy; deferred to v0.2+:
`get_id_from_price`, Token-2022 transfer-fee math, and the
`commons`/litesvm differential layers.
[`CHANGELOG.md`](CHANGELOG.md) tracks what's landed; [`docs/HANDOFF.md`](docs/HANDOFF.md)
is the working-state doc for picking implementation back up.

## Usage

Decode your `LbPair` + `BinArray` accounts however you like (RPC,
snapshot, backtest state), flatten them into a `PoolView` and a
`bin_id`-sorted `&[BinView]`, and quote:

```rust
use solana_dlmm_meteora::{compute_swap_full, BinView, PoolView};

let quote = compute_swap_full(&pool, &bins, amount_in, swap_for_y, now)?;
// quote.amount_out, quote.fee, quote.protocol_fee
// quote.pool_after — post-swap state (active bin + volatility FSM)
//                    for chained simulation
```

Every function is a pure function of its inputs: no RPC, no Anchor, no
`solana-program`, one tiny integer-math dep (`ruint`).

## Positioning

| | model | crate |
|---|---|---|
| Uniswap V3 (EVM) | tick-based CLMM | [`uniswap_v3_math`](https://crates.io/crates/uniswap_v3_math) |
| Raydium (Solana) | tick-based CLMM | [`solana-clmm-raydium`](https://crates.io/crates/solana-clmm-raydium) |
| Orca Whirlpools (Solana) | tick-based CLMM | [`orca_whirlpools_core`](https://crates.io/crates/orca_whirlpools_core) |
| **Meteora (Solana)** | **bin-based DLMM** | **this repo** |

Meteora's DLMM is the still-open math-crate gap in the Solana DEX
ecosystem — every other major liquidity model has an extracted
no-RPC math library, except this one.

## License

Dual-licensed Apache-2.0 OR MIT (matching peer crates).
