"""Independent Python oracle for the DLMM dynamic-fee FSM.

Re-implements the arithmetic of MeteoraAg/dlmm-sdk
commons/src/extensions/lb_pair.rs from the algorithm description (not by
transcribing the Rust port under test). Prints pinned expected values for
tests/dynamic_fee.rs.
"""

BASIS_POINT_MAX = 10_000
MAX_FEE_RATE = 100_000_000
FEE_PRECISION = 1_000_000_000

U32 = 2**32
U64 = 2**64
U128 = 2**128


class Pool:
    def __init__(self, active_id, bin_step, base_factor, filter_period,
                 decay_period, reduction_factor, variable_fee_control,
                 max_volatility_accumulator, protocol_share,
                 base_fee_power_factor=0,
                 volatility_accumulator=0, volatility_reference=0,
                 index_reference=0, last_update_timestamp=0):
        self.active_id = active_id
        self.bin_step = bin_step
        self.base_factor = base_factor
        self.filter_period = filter_period
        self.decay_period = decay_period
        self.reduction_factor = reduction_factor
        self.variable_fee_control = variable_fee_control
        self.max_volatility_accumulator = max_volatility_accumulator
        self.protocol_share = protocol_share
        self.base_fee_power_factor = base_fee_power_factor
        self.volatility_accumulator = volatility_accumulator
        self.volatility_reference = volatility_reference
        self.index_reference = index_reference
        self.last_update_timestamp = last_update_timestamp


def update_references(p: Pool, now: int):
    elapsed = now - p.last_update_timestamp
    if elapsed >= p.filter_period:
        p.index_reference = p.active_id
        if elapsed < p.decay_period:
            # u32 math, floor division
            p.volatility_reference = (p.volatility_accumulator * p.reduction_factor) // BASIS_POINT_MAX
            assert p.volatility_reference < U32
        else:
            p.volatility_reference = 0


def update_volatility_accumulator(p: Pool):
    delta = abs(p.index_reference - p.active_id)
    acc = p.volatility_reference + delta * BASIS_POINT_MAX
    p.volatility_accumulator = min(acc, p.max_volatility_accumulator)
    assert p.volatility_accumulator < U32


def get_base_fee(p: Pool) -> int:
    return p.base_factor * p.bin_step * 10 * 10**p.base_fee_power_factor


def compute_variable_fee(p: Pool, acc: int) -> int:
    if p.variable_fee_control > 0:
        square = (acc * p.bin_step) ** 2
        v = p.variable_fee_control * square
        return (v + 99_999_999_999) // 100_000_000_000
    return 0


def get_total_fee(p: Pool) -> int:
    return min(get_base_fee(p) + compute_variable_fee(p, p.volatility_accumulator),
               MAX_FEE_RATE)


def compute_fee(p: Pool, amount: int) -> int:
    """Gross-up fee on a fee-excluded amount, ceil."""
    rate = get_total_fee(p)
    denom = FEE_PRECISION - rate
    fee = (amount * rate + denom - 1) // denom
    assert fee < U64
    return fee


def compute_fee_from_amount(p: Pool, amount_with_fees: int) -> int:
    rate = get_total_fee(p)
    fee = (amount_with_fees * rate + FEE_PRECISION - 1) // FEE_PRECISION
    assert fee < U64
    return fee


def compute_protocol_fee(p: Pool, fee_amount: int) -> int:
    return (fee_amount * p.protocol_share) // BASIS_POINT_MAX


# ---- pinned scenarios ----

# A realistic bin_step=25 preset (SOL/USDC-style pool).
def preset():
    return Pool(active_id=-1000, bin_step=25, base_factor=10_000,
                filter_period=30, decay_period=600, reduction_factor=5_000,
                variable_fee_control=40_000, max_volatility_accumulator=350_000,
                protocol_share=500)


print("== fee rates on quiet pool (acc=0) ==")
p = preset()
print("base_fee", get_base_fee(p))
print("variable_fee(acc=0)", compute_variable_fee(p, 0))
print("total_fee", get_total_fee(p))
print("compute_fee(1_000_000)", compute_fee(p, 1_000_000))
print("compute_fee_from_amount(1_000_000)", compute_fee_from_amount(p, 1_000_000))
print("compute_protocol_fee(2503)", compute_protocol_fee(p, 2503))

print("\n== variable fee at volatility ==")
p = preset()
p.volatility_accumulator = 50_000  # 5 bins crossed
print("variable_fee(50_000)", compute_variable_fee(p, 50_000))
print("total_fee", get_total_fee(p))
print("compute_fee(1_000_000)", compute_fee(p, 1_000_000))
print("compute_fee_from_amount(1_000_000)", compute_fee_from_amount(p, 1_000_000))

print("\n== base_fee_power_factor ==")
p = preset()
p.base_fee_power_factor = 2
print("base_fee", get_base_fee(p))
print("total_fee (capped)", get_total_fee(p))

print("\n== MAX_FEE_RATE cap via volatility ==")
p = preset()
p.volatility_accumulator = 350_000  # at the cap
print("variable_fee(350_000)", compute_variable_fee(p, 350_000))
print("total_fee (capped at 10%)", get_total_fee(p))
print("compute_fee(1_000_000) at cap", compute_fee(p, 1_000_000))

print("\n== update_references branches ==")
# high-frequency: elapsed < filter_period -> nothing changes
p = preset()
p.volatility_accumulator = 100_000
p.volatility_reference = 7
p.index_reference = -990
p.last_update_timestamp = 1_000
update_references(p, 1_000 + 29)
print("hf: index_reference", p.index_reference, "vol_ref", p.volatility_reference)

# decay window: filter <= elapsed < decay
p = preset()
p.volatility_accumulator = 100_000
p.volatility_reference = 7
p.index_reference = -990
p.last_update_timestamp = 1_000
update_references(p, 1_000 + 30)
print("decay: index_reference", p.index_reference, "vol_ref", p.volatility_reference)

# outside decay window
p = preset()
p.volatility_accumulator = 100_000
p.volatility_reference = 7
p.index_reference = -990
p.last_update_timestamp = 1_000
update_references(p, 1_000 + 600)
print("cold: index_reference", p.index_reference, "vol_ref", p.volatility_reference)

print("\n== update_volatility_accumulator ==")
p = preset()
p.volatility_reference = 25_000
p.index_reference = -1000
p.active_id = -1003  # crossed 3 bins
update_volatility_accumulator(p)
print("acc after 3 bins from ref 25_000:", p.volatility_accumulator)

p = preset()
p.volatility_reference = 340_000
p.index_reference = -1000
p.active_id = -1002
update_volatility_accumulator(p)
print("acc capped:", p.volatility_accumulator)

print("\n== full swap-shaped sequence ==")
# references updated once, then accumulator per bin crossed while active_id
# advances — mirrors the orchestrator's call pattern for a 3-bin crossing.
p = preset()
p.volatility_accumulator = 60_000
p.volatility_reference = 0
p.index_reference = -1000
p.last_update_timestamp = 10_000
update_references(p, 10_100)  # elapsed 100: decay window
print("after refs: vol_ref", p.volatility_reference, "idx_ref", p.index_reference)
fees = []
for _ in range(3):
    update_volatility_accumulator(p)
    fees.append(get_total_fee(p))
    p.active_id -= 1  # advance_active_bin(swap_for_y=true)
print("acc per bin:", p.volatility_accumulator, "fees:", fees)
print("final active_id:", p.active_id)
