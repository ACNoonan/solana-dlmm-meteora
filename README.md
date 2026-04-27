# solana-dlmm-meteora

Pure-Rust, no-RPC swap math for the [Meteora DLMM](https://github.com/MeteoraAg/dlmm-sdk)
(Dynamic Liquidity Market Maker) on Solana. Sibling to
[`solana-clmm-raydium`](https://github.com/ACNoonan/solana-clmm-raydium).

## Status

**Pre-implementation.** This repo currently contains only [`DESIGN.md`](DESIGN.md),
which captures the v0.1 plan, math differences from CLMM, module layout,
and test strategy. Code drops once the litesvm differential test is
finished in `solana-clmm-raydium` (it teaches the harness shape we'll
reuse here).

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

Will be dual-licensed Apache-2.0 OR MIT (matching peer crates).
