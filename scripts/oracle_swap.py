"""Independent Python oracle for the DLMM quote path (commons/src/quote.rs).

Re-implements the upstream algorithms (per-bin fills, fee split, exact-in /
exact-out orchestration) with arbitrary-precision ints + explicit range
asserts. Prints pinned expected values for tests/swap_full.rs.
"""

BASIS_POINT_MAX = 10_000
MAX_FEE_RATE = 100_000_000
FEE_PRECISION = 1_000_000_000
LIMIT_ORDER_FEE_SHARE = 5_000
SCALE_OFFSET = 64
ONE = 1 << 64
U64 = 2**64
U128 = 2**128
MIN_BIN_ID, MAX_BIN_ID = -443_636, 443_636


def ck64(x):
    assert 0 <= x < U64, f"u64 overflow: {x}"
    return x


# ---- u64x64 pow / price (upstream u64x64_math.rs, price_math.rs) ----

def pow_q64(base, exp):
    invert = exp < 0
    if exp == 0:
        return ONE
    e = abs(exp)
    if e >= 0x80000:
        return None
    squared = base
    result = ONE
    if squared >= result:
        squared = (U128 - 1) // squared
        invert = not invert
    for bit in range(19):
        if e & (1 << bit):
            result = (result * squared) >> SCALE_OFFSET
            if result >= U128:
                return None
        squared = (squared * squared) >> SCALE_OFFSET
        if squared >= U128:
            return None
    if result == 0:
        return None
    if invert:
        result = (U128 - 1) // result
    return result


def get_price_from_id(active_id, bin_step):
    bps = (bin_step << SCALE_OFFSET) // BASIS_POINT_MAX
    p = pow_q64(ONE + bps, active_id)
    assert p is not None
    return p


# ---- u128x128 math with rounding (upstream u128x128_math.rs) ----

def mul_shr(x, y, offset, up):
    prod = x * y
    result = prod >> offset
    if up and (prod & ((1 << offset) - 1)) != 0:
        result += 1
    assert result < U128
    return result


def shl_div(x, y, offset, up):
    assert y != 0
    num = x << offset
    if up:
        result = (num + y - 1) // y
    else:
        result = num // y
    assert result < U128
    return result


def get_amount_out(amount_in, price, swap_for_y, up=False):
    if swap_for_y:
        return ck64(mul_shr(price, amount_in, SCALE_OFFSET, up))
    return ck64(shl_div(amount_in, price, SCALE_OFFSET, up))


def get_amount_in(amount_out, price, swap_for_y, up=True):
    if swap_for_y:
        return ck64(shl_div(amount_out, price, SCALE_OFFSET, up))
    return ck64(mul_shr(amount_out, price, SCALE_OFFSET, up))


# ---- pool + FSM (upstream lb_pair.rs) ----

class Pool:
    def __init__(self, active_id, bin_step, base_factor, filter_period,
                 decay_period, reduction_factor, variable_fee_control,
                 max_volatility_accumulator, protocol_share,
                 base_fee_power_factor=0, function_type=0, collect_fee_mode=0,
                 has_rewards=False,
                 volatility_accumulator=0, volatility_reference=0,
                 index_reference=0, last_update_timestamp=0):
        self.__dict__.update(locals())
        del self.__dict__['self']


def update_references(p, now):
    elapsed = now - p.last_update_timestamp
    if elapsed >= p.filter_period:
        p.index_reference = p.active_id
        if elapsed < p.decay_period:
            p.volatility_reference = (p.volatility_accumulator * p.reduction_factor) // BASIS_POINT_MAX
        else:
            p.volatility_reference = 0


def update_volatility_accumulator(p):
    delta = abs(p.index_reference - p.active_id)
    p.volatility_accumulator = min(p.volatility_reference + delta * BASIS_POINT_MAX,
                                   p.max_volatility_accumulator)


def get_total_fee(p):
    base = p.base_factor * p.bin_step * 10 * 10**p.base_fee_power_factor
    var = 0
    if p.variable_fee_control > 0:
        sq = (p.volatility_accumulator * p.bin_step) ** 2
        var = (p.variable_fee_control * sq + 99_999_999_999) // 100_000_000_000
    return min(base + var, MAX_FEE_RATE)


def compute_fee(p, amount):
    rate = get_total_fee(p)
    denom = FEE_PRECISION - rate
    return ck64((amount * rate + denom - 1) // denom)


def compute_fee_from_amount(p, amount_with_fees):
    rate = get_total_fee(p)
    return ck64((amount_with_fees * rate + FEE_PRECISION - 1) // FEE_PRECISION)


def is_support_limit_order(p):
    if p.function_type == 2:
        return True
    if p.function_type == 1:
        return False
    if p.function_type == 0:
        return not p.has_rewards
    return False


def fee_on_input(p, swap_for_y):
    if p.collect_fee_mode == 1:
        return not swap_for_y
    return True  # InputOnly or invalid


# ---- bins ----

class Bin:
    def __init__(self, bin_id, amount_x=0, amount_y=0, price=0,
                 ask_side=0, open_order=0, processed_remaining=0):
        self.bin_id = bin_id
        self.amount_x = amount_x
        self.amount_y = amount_y
        self.price = price
        self.ask_side = ask_side
        self.open_order = open_order
        self.processed_remaining = processed_remaining


def lo_amounts(b, swap_for_y):
    is_ask = b.ask_side != 0
    if (swap_for_y and not is_ask) or (not swap_for_y and is_ask):
        return (b.open_order, b.processed_remaining)
    return (0, 0)


def max_out_with_lo(b, swap_for_y, support):
    mm = b.amount_y if swap_for_y else b.amount_x
    if not support:
        return mm
    o, pr = lo_amounts(b, swap_for_y)
    return min(mm + o + pr, U64 - 1)  # saturating


# ---- per-bin quote (upstream quote.rs) ----

def calc_exact_in_fill(b, amount, max_amount_out, swap_for_y):
    if max_amount_out == 0:
        return (0, amount, 0)
    max_amount_in = get_amount_in(max_amount_out, b.price, swap_for_y, up=True)
    if amount >= max_amount_in:
        return (max_amount_in, amount - max_amount_in, max_amount_out)
    out = get_amount_out(amount, b.price, swap_for_y, up=False)
    return (amount, 0, out)


def exact_in_fill_result(b, amount_in, swap_for_y, support):
    mm = b.amount_y if swap_for_y else b.amount_x
    a_in, a_left, out = calc_exact_in_fill(b, amount_in, mm, swap_for_y)
    if not support:
        return (a_in, a_left, out, a_in)
    total_in, total_out = a_in, out
    mm_in = a_in
    if a_left > 0:
        open_o, proc = lo_amounts(b, swap_for_y)
        p_in, p_left, p_out = calc_exact_in_fill(b, a_left, proc, swap_for_y)
        total_in += p_in
        total_out += p_out
        if p_left > 0:
            o_in, _o_left, o_out = calc_exact_in_fill(b, p_left, open_o, swap_for_y)
            total_in += o_in
            total_out += o_out
    return (total_in, amount_in - total_in, total_out, mm_in)


def split_fee(trading_fee, protocol_share, mm_in, total_in):
    if total_in == 0 or trading_fee == 0:
        return (0, 0)
    mm_fee = ck64((trading_fee * mm_in + (total_in - 1)) // total_in)
    total_lo_fee = trading_fee - mm_fee
    lo_fee = (total_lo_fee * LIMIT_ORDER_FEE_SHARE) // BASIS_POINT_MAX
    lo_protocol = total_lo_fee - lo_fee
    mm_protocol = (mm_fee * protocol_share) // BASIS_POINT_MAX
    total_protocol = lo_protocol + mm_protocol
    return (trading_fee - total_protocol, total_protocol)


def swap_exact_in_at_bin(b, p, in_amount, swap_for_y, support, f_on_in):
    trading_fee = 0
    excl_in = in_amount
    if f_on_in:
        fee = compute_fee_from_amount(p, in_amount)
        trading_fee = fee
        excl_in = in_amount - fee
    t_in, a_left, out, mm_in = exact_in_fill_result(b, excl_in, swap_for_y, support)
    incl_in = in_amount
    if a_left > 0:
        excl_in = excl_in - a_left
        if f_on_in:
            fee = compute_fee(p, excl_in)
            trading_fee = fee
            incl_in = excl_in + fee
        else:
            incl_in = excl_in
    excl_out = out
    if not f_on_in:
        fee = compute_fee_from_amount(p, out)
        trading_fee = fee
        excl_out = out - fee
    _user, protocol = split_fee(trading_fee, p.protocol_share, mm_in, t_in)
    return dict(amount_in=incl_in, amount_out=excl_out, fee=trading_fee,
                protocol_fee=protocol)


def excluded_fee_amount_in(b, swap_for_y, incl_out):
    mm = b.amount_y if swap_for_y else b.amount_x
    open_o, proc = lo_amounts(b, swap_for_y)
    remaining = incl_out
    total_in = 0
    take = min(remaining, mm)
    total_in += get_amount_in(take, b.price, swap_for_y, up=True)
    remaining -= take
    if remaining > 0:
        take = min(remaining, proc)
        total_in += get_amount_in(take, b.price, swap_for_y, up=True)
        remaining -= take
        if remaining > 0:
            take = min(remaining, open_o)
            total_in += get_amount_in(take, b.price, swap_for_y, up=True)
    return ck64(total_in)


def swap_exact_out_at_bin(b, p, out_amount, swap_for_y, support, f_on_in):
    incl_out = out_amount
    if not f_on_in:
        incl_out = out_amount + compute_fee(p, out_amount)
    max_out = max_out_with_lo(b, swap_for_y, support)
    if incl_out >= max_out:
        return swap_exact_in_at_bin(b, p, U64 - 1, swap_for_y, support, f_on_in)
    excl_in = excluded_fee_amount_in(b, swap_for_y, incl_out)
    incl_in = excl_in + compute_fee(p, excl_in) if f_on_in else excl_in
    result = swap_exact_in_at_bin(b, p, incl_in, swap_for_y, support, f_on_in)
    if result['amount_out'] > out_amount:
        delta = result['amount_out'] - out_amount
        if delta > 1:
            result['protocol_fee'] += delta
    result['amount_out'] = out_amount
    return result


# ---- orchestrators ----

def find_bin(bins, bin_id):
    for b in bins:
        if b.bin_id == bin_id:
            return b
    return None


def bounds(bins):
    ids = [b.bin_id for b in bins]
    return (min(ids), max(ids)) if ids else (None, None)


def compute_swap_full(p, bins, amount_in, swap_for_y, now):
    update_references(p, now)
    support = is_support_limit_order(p)
    f_on_in = fee_on_input(p, swap_for_y)
    tot_out = tot_fee = tot_prot = 0
    left = amount_in
    lo, hi = bounds(bins)
    while left > 0:
        b = find_bin(bins, p.active_id)
        if b is not None:
            if max_out_with_lo(b, swap_for_y, support) > 0:
                update_volatility_accumulator(p)
                r = swap_exact_in_at_bin(b, p, left, swap_for_y, support, f_on_in)
                if r['amount_in'] > 0:
                    left -= r['amount_in']
                    tot_out += r['amount_out']
                    tot_fee += r['fee']
                    tot_prot += r['protocol_fee']
        if left > 0:
            if lo is None or (swap_for_y and p.active_id <= lo) or (not swap_for_y and p.active_id >= hi):
                return "PoolOutOfLiquidity"
            p.active_id += -1 if swap_for_y else 1
            assert MIN_BIN_ID <= p.active_id <= MAX_BIN_ID
    p.last_update_timestamp = now
    return dict(amount_out=ck64(tot_out), fee=ck64(tot_fee), protocol_fee=ck64(tot_prot))


def compute_swap_full_exact_out(p, bins, amount_out, swap_for_y, now):
    update_references(p, now)
    support = is_support_limit_order(p)
    f_on_in = fee_on_input(p, swap_for_y)
    tot_in = tot_fee = tot_prot = 0
    left = amount_out
    lo, hi = bounds(bins)
    while left > 0:
        b = find_bin(bins, p.active_id)
        if b is not None:
            bb = Bin(b.bin_id, b.amount_x, b.amount_y,
                     b.price if b.price else get_price_from_id(b.bin_id, p.bin_step),
                     b.ask_side, b.open_order, b.processed_remaining)
            if max_out_with_lo(bb, swap_for_y, support) > 0:
                update_volatility_accumulator(p)
                r = swap_exact_out_at_bin(bb, p, left, swap_for_y, support, f_on_in)
                if r['amount_out'] > 0:
                    left -= r['amount_out']
                    tot_in += r['amount_in']
                    tot_fee += r['fee']
                    tot_prot += r['protocol_fee']
        if left > 0:
            if lo is None or (swap_for_y and p.active_id <= lo) or (not swap_for_y and p.active_id >= hi):
                return "PoolOutOfLiquidity"
            p.active_id += -1 if swap_for_y else 1
    p.last_update_timestamp = now
    return dict(amount_in=ck64(tot_in), fee=ck64(tot_fee), protocol_fee=ck64(tot_prot))


if __name__ == "__main__":
    # ---- scenarios ----

    def preset(**kw):
        d = dict(active_id=100, bin_step=100, base_factor=10_000, filter_period=30,
                 decay_period=600, reduction_factor=5_000, variable_fee_control=40_000,
                 max_volatility_accumulator=350_000, protocol_share=500,
                 function_type=1)  # LiquidityMining: no limit orders unless overridden
        d.update(kw)
        return Pool(**d)


    def mkbins(*specs):
        """spec: (bin_id, amount_x, amount_y[, ask, open, proc]); price auto from id/step 100."""
        out = []
        for s in specs:
            bin_id, ax, ay = s[0], s[1], s[2]
            ask, opn, prc = (s[3], s[4], s[5]) if len(s) > 3 else (0, 0, 0)
            out.append(Bin(bin_id, ax, ay, get_price_from_id(bin_id, 100), ask, opn, prc))
        return out


    def dump(tag, p, r):
        print(f"-- {tag} --")
        print("  result:", r)
        if isinstance(r, dict):
            print(f"  pool_after: active_id={p.active_id} acc={p.volatility_accumulator} "
                  f"vol_ref={p.volatility_reference} idx_ref={p.index_reference} "
                  f"ts={p.last_update_timestamp}")


    B = 1_000_000  # bin reserve unit

    # 1. single-bin exact-in partial fill (fee on input, no LO)
    p = preset()
    bins = mkbins((100, 5 * B, 5 * B))
    r = compute_swap_full(p, bins, 1_000_000, True, 1_000)
    dump("single-bin exact-in, swap_for_y", p, r)

    # 2. multi-bin crossing exact-in (3 bins down)
    p = preset()
    bins = mkbins((98, 3 * B, 3 * B), (99, 3 * B, 3 * B), (100, 3 * B, 3 * B))
    r = compute_swap_full(p, bins, 3_000_000, True, 1_000)
    dump("3-bin crossing exact-in, swap_for_y", p, r)

    # 3. same but swapping up (y in, x out)
    p = preset()
    bins = mkbins((100, 2 * B, 2 * B), (101, 2 * B, 2 * B), (102, 2 * B, 2 * B))
    r = compute_swap_full(p, bins, 10_000_000, False, 1_000)
    dump("3-bin crossing exact-in, !swap_for_y", p, r)

    # 4. fee on output (collect_fee_mode=1, swap_for_y → fee on Y=output side)
    p = preset(collect_fee_mode=1)
    bins = mkbins((100, 5 * B, 5 * B))
    r = compute_swap_full(p, bins, 1_000_000, True, 1_000)
    dump("fee-on-output single bin", p, r)

    # 5. limit orders: bid-side orders filled by swap_for_y after MM drains
    p = preset(function_type=2)
    bins = mkbins((100, 0, 2 * B, 0, 1 * B, 500_000))
    r = compute_swap_full(p, bins, 1_000_000, True, 1_000)
    dump("limit-order fill, swap_for_y", p, r)

    # 6. warm pool: prior volatility + timestamps (decay branch) multi-bin
    p = preset(volatility_accumulator=80_000, volatility_reference=0,
               index_reference=103, last_update_timestamp=500)
    bins = mkbins((98, 3 * B, 3 * B), (99, 3 * B, 3 * B), (100, 3 * B, 3 * B))
    r = compute_swap_full(p, bins, 3_000_000, True, 600)  # elapsed 100 → decay
    dump("warm pool decay multi-bin", p, r)

    # 7. out of liquidity
    p = preset()
    bins = mkbins((99, 1 * B, 1 * B), (100, 1 * B, 1 * B))
    r = compute_swap_full(p, bins, 10_000_000_000, True, 1_000)
    dump("out of liquidity", p, r)

    # 8. exact-out single bin
    p = preset()
    bins = mkbins((100, 5 * B, 5 * B))
    r = compute_swap_full_exact_out(p, bins, 1_000_000, True, 1_000)
    dump("exact-out single bin", p, r)

    # 9. exact-out crossing with drain path
    p = preset()
    bins = mkbins((98, 3 * B, 3 * B), (99, 3 * B, 3 * B), (100, 3 * B, 3 * B))
    r = compute_swap_full_exact_out(p, bins, 5_000_000, True, 1_000)
    dump("exact-out 2-bin drain", p, r)

    # 10. exact-out, price stored as 0 in BinView (get_or_store path)
    p = preset()
    bins = mkbins((100, 5 * B, 5 * B))
    bins[0].price = 0
    r = compute_swap_full_exact_out(p, bins, 1_000_000, True, 1_000)
    dump("exact-out with lazy price", p, r)

    # 11. round-trip check: exact-in result fed to exact-out
    p = preset()
    bins = mkbins((98, 3 * B, 3 * B), (99, 3 * B, 3 * B), (100, 3 * B, 3 * B))
    r1 = compute_swap_full(p, bins, 3_000_000, True, 1_000)
    p2 = preset()
    r2 = compute_swap_full_exact_out(p2, bins, r1['amount_out'], True, 1_000)
    print("-- round trip --")
    print("  exact_in:", r1)
    print("  exact_out(amount_out):", r2)
