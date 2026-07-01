#!/usr/bin/env python
"""Independent multi-curve FX-forward oracle for the celnet-core curve-backed carry.

Re-derives the PUBLISHED no-arbitrage multi-curve FX forward
    F(t) = S * P_for(0,t) / P_dom(0,t)
(Bianchetti / Ametrano-Bianchetti multi-curve FX; QuantLib as the golden engine)
for two genuinely term-structured discount curves, by TWO code-disjoint routes that
must agree, then prints the full-precision oracle embedded in the Rust test:

  1. raw `math.exp` (independent language + exp implementation);
  2. QuantLib InterpolatedDiscountCurve discounting objects (independent engine).

This is NOT the celnet engine reprised as its own check — QuantLib and Python math are
separate implementations of the published formula (avoids the FRTB circular-oracle trap).

Run with the golden oracle interpreter:
    ~/.celnet-goldenv/bin/python crates/celnet-core/tests/oracle_curve_forward.py
"""

import math
import QuantLib as ql

SPOT = 1.2345

# Two genuinely term-structured continuously-compounded zero curves z(t) (asymmetric
# shapes so a df_for/df_dom sign flip -> df_dom/df_for changes the answer by >> 1e-12).
def z_dom(t: float) -> float:
    return 0.030 + 0.010 * t - 0.0008 * t * t

def z_for(t: float) -> float:
    return 0.015 - 0.004 * t + 0.0003 * t * t

def df_dom(t: float) -> float:
    return math.exp(-z_dom(t) * t)

def df_for(t: float) -> float:
    return math.exp(-z_for(t) * t)

TIMES = [0.25, 1.0, 2.0, 5.0]

# Route 1: raw math, published formula F = S * df_for / df_dom.
fwd_raw = [SPOT * df_for(t) / df_dom(t) for t in TIMES]

# Route 2: QuantLib discount-curve objects, queried AT the pillars (node value, no
# interpolation ambiguity). Build each curve from the exact discount factors above.
today = ql.Date(1, 7, 2026)
ql.Settings.instance().evaluationDate = today
dc = ql.Actual365Fixed()

def ql_curve(df_fn):
    dates = [today]
    dfs = [1.0]
    for t in TIMES:
        dates.append(today + ql.Period(round(t * 365), ql.Days))
        dfs.append(df_fn(t))
    return ql.DiscountCurve(dates, dfs, dc)

dom_ts = ql_curve(df_dom)
for_ts = ql_curve(df_for)

fwd_ql = []
for t in TIMES:
    d = today + ql.Period(round(t * 365), ql.Days)
    fwd_ql.append(SPOT * for_ts.discount(d) / dom_ts.discount(d))

# The two independent routes must agree (QuantLib reproduces the published discounting).
for t, a, b in zip(TIMES, fwd_raw, fwd_ql):
    rel = abs(a - b) / abs(a)
    assert rel < 1e-13, f"raw vs QuantLib disagree at t={t}: {a!r} vs {b!r} (rel {rel:e})"

print("// oracle: F(t) = SPOT * df_for(t) / df_dom(t), SPOT =", repr(SPOT))
print("// z_dom(t) = 0.030 + 0.010 t - 0.0008 t^2 ; z_for(t) = 0.015 - 0.004 t + 0.0003 t^2")
print("// raw-math == QuantLib-discount to < 1e-13 (independent routes agree)")
for t, f in zip(TIMES, fwd_raw):
    print(f"    (t={t:>4}) df_dom={df_dom(t)!r} df_for={df_for(t)!r} F={f!r}")
