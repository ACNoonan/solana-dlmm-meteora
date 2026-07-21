"""Capture real mainnet DLMM pool state (SOL-USDC pair) via public RPC,
decode LbPair + bin arrays from the anchor IDL layout, quote swaps with the
independent Python oracle, and emit a Rust fixture test.
"""
import json
import hashlib
import struct
import urllib.request
import base64
import sys

sys.path.insert(0, __file__.rsplit('/', 1)[0])
import oracle_swap as oracle  # noqa: E402

RPC = "https://api.mainnet-beta.solana.com"
PAIR = "HTvjzsfX3yU6BUodCjZ5vZkUrAxMDTrBs3CJaq43ashR"
DLMM_PROGRAM = "LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo"

# ---- base58 ----
B58 = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz"

def b58decode(s):
    n = 0
    for c in s:
        n = n * 58 + B58.index(c)
    raw = n.to_bytes((n.bit_length() + 7) // 8, 'big')
    pad = len(s) - len(s.lstrip('1'))
    return b'\x00' * pad + raw

def b58encode(b):
    n = int.from_bytes(b, 'big')
    out = ""
    while n:
        n, r = divmod(n, 58)
        out = B58[r] + out
    pad = len(b) - len(b.lstrip(b'\x00'))
    return '1' * pad + out

# ---- ed25519 on-curve check (RFC 8032 decompression) ----
P = 2**255 - 19
D = (-121665 * pow(121666, P - 2, P)) % P

def is_on_curve(b):
    y = int.from_bytes(b, 'little') & ((1 << 255) - 1)
    sign = b[31] >> 7
    if y >= P:
        return False
    x2 = (y * y - 1) * pow(D * y * y + 1, P - 2, P) % P
    x = pow(x2, (P + 3) // 8, P)
    if (x * x - x2) % P != 0:
        x = x * pow(2, (P - 1) // 4, P) % P
    if (x * x - x2) % P != 0:
        return False
    if x == 0 and sign == 1:
        return False
    return True

def find_program_address(seeds, program_id):
    for bump in range(255, -1, -1):
        buf = b''.join(seeds) + bytes([bump]) + program_id + b"ProgramDerivedAddress"
        h = hashlib.sha256(buf).digest()
        if not is_on_curve(h):
            return h, bump
    raise RuntimeError("no bump found")

# ---- RPC ----
def rpc(method, params):
    req = urllib.request.Request(
        RPC,
        data=json.dumps({"jsonrpc": "2.0", "id": 1, "method": method,
                         "params": params}).encode(),
        headers={"Content-Type": "application/json"})
    with urllib.request.urlopen(req, timeout=30) as r:
        resp = json.load(r)
    if "error" in resp:
        raise RuntimeError(resp["error"])
    return resp["result"]

def get_account(pubkey_b58):
    r = rpc("getAccountInfo", [pubkey_b58, {"encoding": "base64"}])
    v = r["value"]
    if v is None:
        return None
    return base64.b64decode(v["data"][0])

# ---- IDL layout decoding ----
IDL_PATH = __file__.rsplit('/', 1)[0] + '/dlmm_idl.json'
try:
    idl = json.load(open(IDL_PATH))
except FileNotFoundError:
    urllib.request.urlretrieve(
        'https://raw.githubusercontent.com/MeteoraAg/dlmm-sdk/main/idls/dlmm.json', IDL_PATH)
    idl = json.load(open(IDL_PATH))
TYPES = {t['name']: t for t in idl['types']}
PRIM = {"u8": 1, "u16": 2, "u32": 4, "i32": 4, "u64": 8, "i64": 8,
        "u128": 16, "i128": 16, "pubkey": 32, "bool": 1}

def type_size(t):
    if isinstance(t, str):
        return PRIM[t]
    if "array" in t:
        elem, n = t["array"]
        return type_size(elem) * n
    if "defined" in t:
        fields = TYPES[t["defined"]["name"]]['type']['fields']
        return sum(type_size(f['type']) for f in fields)
    raise ValueError(t)

def decode(t, buf, off):
    if isinstance(t, str):
        size = PRIM[t]
        raw = buf[off:off + size]
        if t == "pubkey":
            return b58encode(raw), off + size
        signed = t.startswith("i")
        return int.from_bytes(raw, 'little', signed=signed), off + size
    if "array" in t:
        elem, n = t["array"]
        out = []
        for _ in range(n):
            v, off = decode(elem, buf, off)
            out.append(v)
        return out, off
    if "defined" in t:
        fields = TYPES[t["defined"]["name"]]['type']['fields']
        d = {}
        for f in fields:
            d[f['name']], off = decode(f['type'], buf, off)
        return d, off
    raise ValueError(t)

def decode_account(type_name, data):
    obj, off = decode({"defined": {"name": type_name}}, data, 8)
    return obj

# ---- capture ----
print("fetching LbPair...", file=sys.stderr)
pair_data = get_account(PAIR)
lb = decode_account("LbPair", pair_data)
sp, vp = lb['parameters'], lb['v_parameters']
active_id = lb['active_id']
print(f"active_id={active_id} bin_step={lb['bin_step']} status={lb['status']}", file=sys.stderr)
print(f"static: {sp}", file=sys.stderr)
print(f"variable: {vp}", file=sys.stderr)

has_rewards = any(r['mint'] != '11111111111111111111111111111111'
                  for r in lb['reward_infos'])

def bin_array_index(bin_id):
    q, r = divmod(bin_id, 70)  # python divmod is floor-based: matches upstream
    return q

pair_bytes = b58decode(PAIR)
prog_bytes = b58decode(DLMM_PROGRAM)
center = bin_array_index(active_id)
bins = []
for idx in (center - 1, center, center + 1):
    key_bytes, _ = find_program_address(
        [b"bin_array", pair_bytes, struct.pack("<q", idx)], prog_bytes)
    key = b58encode(key_bytes)
    print(f"bin array idx={idx}: {key}", file=sys.stderr)
    data = get_account(key)
    if data is None:
        print(f"  (missing — no liquidity)", file=sys.stderr)
        continue
    arr = decode_account("BinArray", data)
    assert arr['index'] == idx, (arr['index'], idx)
    lower = idx * 70
    for i, b in enumerate(arr['bins']):
        bins.append(oracle.Bin(
            lower + i, b['amount_x'], b['amount_y'], b['price'],
            b['limit_order_ask_side'], b['open_order_amount'],
            b['processed_order_remaining_amount']))

bins.sort(key=lambda b: b.bin_id)
# Trim to a window around the active bin: enough depth for the pinned quotes.
WINDOW = 25
bins = [b for b in bins if abs(b.bin_id - active_id) <= WINDOW]
nonzero = sum(1 for b in bins if b.amount_x or b.amount_y)
print(f"kept {len(bins)} bins (±{WINDOW}), {nonzero} with liquidity", file=sys.stderr)

# ---- oracle quotes over the captured state ----
def mkpool():
    return oracle.Pool(
        active_id=active_id, bin_step=lb['bin_step'],
        base_factor=sp['base_factor'], filter_period=sp['filter_period'],
        decay_period=sp['decay_period'], reduction_factor=sp['reduction_factor'],
        variable_fee_control=sp['variable_fee_control'],
        max_volatility_accumulator=sp['max_volatility_accumulator'],
        protocol_share=sp['protocol_share'],
        base_fee_power_factor=sp['base_fee_power_factor'],
        function_type=sp['function_type'], collect_fee_mode=sp['collect_fee_mode'],
        has_rewards=has_rewards,
        volatility_accumulator=vp['volatility_accumulator'],
        volatility_reference=vp['volatility_reference'],
        index_reference=vp['index_reference'],
        last_update_timestamp=vp['last_update_timestamp'])

NOW = vp['last_update_timestamp'] + 120  # 2 min after the pool's last update

quotes = []
for amount, sfy, label in [
    (1_000_000_000, True, "1 SOL -> USDC"),        # X = SOL (9 dp)
    (5_000_000_000, True, "5 SOL -> USDC"),
    (100_000_000, False, "100 USDC -> SOL"),        # Y = USDC (6 dp)
]:
    p = mkpool()
    r = oracle.compute_swap_full(p, bins, amount, sfy, NOW)
    quotes.append((amount, sfy, label, r, p))
    print(f"{label}: {r}  active->{p.active_id} acc->{p.volatility_accumulator}", file=sys.stderr)

p = mkpool()
xo = oracle.compute_swap_full_exact_out(p, bins, 100_000_000, True, NOW)
print(f"exact-out 100 USDC: {xo} active->{p.active_id}", file=sys.stderr)

# ---- emit Rust fixture test ----
def fmt_bins():
    lines = []
    for b in bins:
        lines.append(f"    BinView {{ bin_id: {b.bin_id}, amount_x: {b.amount_x}, "
                     f"amount_y: {b.amount_y}, price: {b.price}, "
                     f"limit_order_ask_side: {b.ask_side}, open_order_amount: {b.open_order}, "
                     f"processed_order_remaining_amount: {b.processed_remaining} }},")
    return "\n".join(lines)

exact_in_asserts = []
for amount, sfy, label, r, p in quotes:
    exact_in_asserts.append(f"""
#[test]
fn replay_{'x_to_y' if sfy else 'y_to_x'}_{amount}() {{
    // {label}
    let quote = compute_swap_full(&pool(), &bins(), {amount}, {'true' if sfy else 'false'}, NOW).unwrap();
    assert_eq!(quote.amount_out, {r['amount_out']});
    assert_eq!(quote.fee, {r['fee']});
    assert_eq!(quote.protocol_fee, {r['protocol_fee']});
    assert_eq!(quote.pool_after.active_id, {p.active_id});
    assert_eq!(quote.pool_after.v_parameters.volatility_accumulator, {p.volatility_accumulator});
}}""")

rust = f"""//! Mainnet-replay fixture (milestone 6, DESIGN.md test layer 3).
//!
//! Real pool state captured from mainnet via public RPC
//! (`api.mainnet-beta.solana.com`) on 2026-07-21:
//! the Meteora DLMM SOL-USDC pair `{PAIR}`
//! (bin_step {lb['bin_step']}, the pair upstream's own integration tests use), LbPair
//! account + the three bin arrays around the active bin, trimmed to
//! ±{WINDOW} bins. Expected outputs pinned by the independent Python oracle
//! over the captured state — NOT by replaying an on-chain transaction
//! (byte-exact tx replay arrives with the litesvm differential in v0.3).
//!
//! Generated by `capture_fixture.py`; do not edit the numbers by hand.

use solana_dlmm_meteora::{{
    compute_swap_full, compute_swap_full_exact_out, BinView, PoolView, StaticParameters,
    VariableParameters,
}};

/// 120s after the captured `last_update_timestamp` — a realistic quote gap
/// (outside the filter period, inside nothing in particular).
const NOW: i64 = {NOW};

fn pool() -> PoolView {{
    PoolView {{
        active_id: {active_id},
        bin_step: {lb['bin_step']},
        has_rewards: {'true' if has_rewards else 'false'},
        parameters: StaticParameters {{
            base_factor: {sp['base_factor']},
            filter_period: {sp['filter_period']},
            decay_period: {sp['decay_period']},
            reduction_factor: {sp['reduction_factor']},
            variable_fee_control: {sp['variable_fee_control']},
            max_volatility_accumulator: {sp['max_volatility_accumulator']},
            min_bin_id: {sp['min_bin_id']},
            max_bin_id: {sp['max_bin_id']},
            protocol_share: {sp['protocol_share']},
            base_fee_power_factor: {sp['base_fee_power_factor']},
            function_type: {sp['function_type']},
            collect_fee_mode: {sp['collect_fee_mode']},
        }},
        v_parameters: VariableParameters {{
            volatility_accumulator: {vp['volatility_accumulator']},
            volatility_reference: {vp['volatility_reference']},
            index_reference: {vp['index_reference']},
            last_update_timestamp: {vp['last_update_timestamp']},
        }},
    }}
}}

#[rustfmt::skip]
fn bins() -> Vec<BinView> {{
    vec![
{fmt_bins()}
    ]
}}
{"".join(exact_in_asserts)}

#[test]
fn replay_exact_out_100_usdc() {{
    let quote = compute_swap_full_exact_out(&pool(), &bins(), 100_000_000, true, NOW).unwrap();
    assert_eq!(quote.amount_in, {xo['amount_in']});
    assert_eq!(quote.fee, {xo['fee']});
    assert_eq!(quote.protocol_fee, {xo['protocol_fee']});
}}
"""

out = __file__.rsplit("/", 2)[0] + "/tests/mainnet_replay.rs"
open(out, "w").write(rust)
print(f"wrote {out}", file=sys.stderr)
