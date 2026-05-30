#!/usr/bin/env python3
"""Frozen golden-reference generator for Celnet.

Emits CSV reference tables of FX-option prices and Greeks computed by QuantLib
(the open-source golden oracle) using the Garman-Kohlhagen process and the
analytic European / barrier / binary engines. The CSVs are committed under
``crates/celnet-golden/data/`` and treated as *frozen* oracles: the Rust test
suite in ``celnet-golden`` loads them and asserts that the Celnet pricers match
QuantLib within documented tolerances.

Run via the pinned QuantLib venv so the emitted numbers are real, reproducible
data (see ``crates/celnet-golden/README.md`` for the regeneration procedure)::

    ~/.celnet-goldenv/bin/python tools/goldgen/generate.py

Convention mapping QuantLib -> Celnet (verified, identical units/signs):

* ``GarmanKohlagenProcess(spot, rfTS, rdTS, volTS)`` -- note QuantLib's signature
  takes the *foreign* (dividend) curve first, then the *domestic* (risk-free)
  curve; we pass r_for as the dividend yield and r_dom as the risk-free rate.
* ``NPV``            -> Celnet ``price`` (domestic premium per 1 base notional).
* ``delta``          -> Celnet ``delta_spot``   = e^{-r_f T} N(d1).
* ``gamma``          -> Celnet ``gamma``         = d2V/dS2.
* ``vega``           -> Celnet ``vega``          = dV/dsigma (per 1.0 abs vol).
* ``theta``          -> Celnet ``theta``         = dV/dt per year (= -dV/dT).
* ``rho``            -> Celnet ``rho_dom``       = dV/dr_dom.
* ``dividendRho``    -> Celnet ``rho_for``       = dV/dr_for.

Times are produced as exact Actual/365-Fixed year fractions by advancing the
evaluation date by ``round(T * 365)`` calendar days, so the year fraction QuantLib
sees equals the ``t`` written to the CSV (the Rust side consumes that same ``t``).
"""

from __future__ import annotations

import csv
from pathlib import Path

# Pyright may report "QuantLib could not be resolved": that is a false positive —
# Pyright uses the system interpreter, but this script is *run* under the pinned
# venv at ~/.celnet-goldenv (see crates/celnet-golden/README.md) where QuantLib is
# installed.
import QuantLib as ql

# --- Frozen evaluation context ------------------------------------------------

EVAL_DATE = ql.Date(15, 6, 2026)
DAY_COUNT = ql.Actual365Fixed()
CALENDAR = ql.NullCalendar()

OUT_DIR = Path(__file__).resolve().parents[2] / "crates" / "celnet-golden" / "data"


def _setup() -> None:
    ql.Settings.instance().evaluationDate = EVAL_DATE


def _expiry_for(t_years: float) -> tuple[ql.Date, float]:
    """Return (expiry_date, exact_year_fraction) for a target T in years.

    We snap T to an integral number of Act/365F days so the year fraction is
    exactly representable and identical on both the QuantLib and Celnet sides.
    """
    days = int(round(t_years * 365.0))
    expiry = EVAL_DATE + days
    t_exact = DAY_COUNT.yearFraction(EVAL_DATE, expiry)
    return expiry, t_exact


def _gk_process(
    spot: float, r_dom: float, r_for: float, vol: float
) -> ql.GarmanKohlagenProcess:
    spot_h = ql.QuoteHandle(ql.SimpleQuote(spot))
    r_dom_ts = ql.YieldTermStructureHandle(ql.FlatForward(EVAL_DATE, r_dom, DAY_COUNT))
    r_for_ts = ql.YieldTermStructureHandle(ql.FlatForward(EVAL_DATE, r_for, DAY_COUNT))
    vol_ts = ql.BlackVolTermStructureHandle(
        ql.BlackConstantVol(EVAL_DATE, CALENDAR, vol, DAY_COUNT)
    )
    # Signature: (spot, foreignTS, domesticTS, blackVolTS).
    return ql.GarmanKohlagenProcess(spot_h, r_for_ts, r_dom_ts, vol_ts)


# --- Parameter grid -----------------------------------------------------------

SPOTS = [0.80, 1.00, 1.2345, 100.0]
# Strikes expressed as a moneyness multiple of spot so the grid spans
# deep-ITM / ATM / deep-OTM consistently across very different spot scales.
MONEYNESS = [0.80, 0.90, 1.00, 1.10, 1.25]
VOLS = [0.05, 0.10, 0.20, 0.35, 0.60]
MATURITIES = [0.0274, 0.25, 1.0, 2.0]  # ~10d, 3m, 1y, 2y
RATE_PAIRS = [
    (0.05, 0.00),
    (0.02, 0.01),
    (0.01, 0.03),
    (-0.005, 0.02),
]
OPTION_TYPES = [("CALL", ql.Option.Call), ("PUT", ql.Option.Put)]


def generate_vanilla() -> int:
    """Write the frozen vanilla price+Greek grid; return the row count."""
    _setup()
    rows = []
    for spot in SPOTS:
        for m in MONEYNESS:
            strike = spot * m
            for vol in VOLS:
                for t_target in MATURITIES:
                    expiry, t = _expiry_for(t_target)
                    for r_dom, r_for in RATE_PAIRS:
                        proc = _gk_process(spot, r_dom, r_for, vol)
                        engine = ql.AnalyticEuropeanEngine(proc)
                        for type_name, opt_type in OPTION_TYPES:
                            payoff = ql.PlainVanillaPayoff(opt_type, strike)
                            option = ql.VanillaOption(
                                payoff, ql.EuropeanExercise(expiry)
                            )
                            option.setPricingEngine(engine)
                            rows.append(
                                {
                                    "option_type": type_name,
                                    "spot": spot,
                                    "strike": strike,
                                    "vol": vol,
                                    "t": t,
                                    "r_dom": r_dom,
                                    "r_for": r_for,
                                    "price": option.NPV(),
                                    "delta_spot": option.delta(),
                                    "gamma": option.gamma(),
                                    "vega": option.vega(),
                                    "theta": option.theta(),
                                    "rho_dom": option.rho(),
                                    "rho_for": option.dividendRho(),
                                }
                            )

    out = OUT_DIR / "vanilla_gk.csv"
    fields = [
        "option_type",
        "spot",
        "strike",
        "vol",
        "t",
        "r_dom",
        "r_for",
        "price",
        "delta_spot",
        "gamma",
        "vega",
        "theta",
        "rho_dom",
        "rho_for",
    ]
    _write_csv(out, fields, rows)
    return len(rows)


# --- Barrier references (analytic, continuous monitoring) ---------------------

BARRIER_KINDS = [
    ("DOWN_OUT", ql.Barrier.DownOut),
    ("DOWN_IN", ql.Barrier.DownIn),
    ("UP_OUT", ql.Barrier.UpOut),
    ("UP_IN", ql.Barrier.UpIn),
]


def generate_barrier() -> int:
    """Write analytic single-barrier references; return the row count."""
    _setup()
    rows = []
    spot = 100.0
    for vol in [0.10, 0.20, 0.35]:
        for t_target in [0.25, 1.0]:
            expiry, t = _expiry_for(t_target)
            for r_dom, r_for in [(0.05, 0.01), (0.02, 0.03)]:
                proc = _gk_process(spot, r_dom, r_for, vol)
                engine = ql.AnalyticBarrierEngine(proc)
                for type_name, opt_type in OPTION_TYPES:
                    for strike in [90.0, 100.0, 110.0]:
                        for kind_name, kind in BARRIER_KINDS:
                            # Place the barrier on the relevant side of spot.
                            barrier = (
                                85.0 if kind_name.startswith("DOWN") else 115.0
                            )
                            payoff = ql.PlainVanillaPayoff(opt_type, strike)
                            option = ql.BarrierOption(
                                kind, barrier, 0.0, payoff,
                                ql.EuropeanExercise(expiry),
                            )
                            option.setPricingEngine(engine)
                            rows.append(
                                {
                                    "barrier_type": kind_name,
                                    "option_type": type_name,
                                    "spot": spot,
                                    "strike": strike,
                                    "barrier": barrier,
                                    "rebate": 0.0,
                                    "vol": vol,
                                    "t": t,
                                    "r_dom": r_dom,
                                    "r_for": r_for,
                                    "price": option.NPV(),
                                }
                            )

    out = OUT_DIR / "barrier_gk.csv"
    fields = [
        "barrier_type",
        "option_type",
        "spot",
        "strike",
        "barrier",
        "rebate",
        "vol",
        "t",
        "r_dom",
        "r_for",
        "price",
    ]
    _write_csv(out, fields, rows)
    return len(rows)


# --- Digital (cash-or-nothing) references -------------------------------------


def _digital_payoff(
    style_name: str, opt_type: int, strike: float, payout: float
) -> object:
    """Construct the QuantLib binary payoff for the requested settlement style.

    * ``CASH`` -> ``CashOrNothingPayoff``  (pays ``payout`` units of domestic cash).
    * ``ASSET`` -> ``AssetOrNothingPayoff`` (pays one unit of the foreign asset,
      worth ``S_T``; QuantLib's asset-or-nothing payoff ignores the cash amount).
    """
    if style_name == "CASH":
        return ql.CashOrNothingPayoff(opt_type, strike, payout)
    if style_name == "ASSET":
        return ql.AssetOrNothingPayoff(opt_type, strike)
    raise ValueError(f"unknown digital style: {style_name}")


DIGITAL_STYLES = ["CASH", "ASSET"]


def generate_digital() -> int:
    """Write analytic cash- and asset-or-nothing digital references.

    Covers both settlement styles (cash-or-nothing and asset-or-nothing) across
    both directions (call/put), so the exotics digital pricer is independently
    gated on all four flavours. The ``payout`` column is the cash amount for the
    cash-or-nothing rows and a unit (one asset unit) for the asset-or-nothing
    rows; the Rust side prices per one payout unit and multiplies by ``payout``.
    Returns the row count.
    """
    _setup()
    rows = []
    payout = 1.0
    for spot in [1.00, 100.0]:
        for m in [0.95, 1.00, 1.05]:
            strike = spot * m
            for vol in [0.10, 0.25]:
                for t_target in [0.25, 1.0]:
                    expiry, t = _expiry_for(t_target)
                    for r_dom, r_for in [(0.05, 0.01), (0.01, 0.02)]:
                        proc = _gk_process(spot, r_dom, r_for, vol)
                        engine = ql.AnalyticEuropeanEngine(proc)
                        for style_name in DIGITAL_STYLES:
                            for type_name, opt_type in OPTION_TYPES:
                                payoff = _digital_payoff(
                                    style_name, opt_type, strike, payout
                                )
                                option = ql.VanillaOption(
                                    payoff, ql.EuropeanExercise(expiry)
                                )
                                option.setPricingEngine(engine)
                                rows.append(
                                    {
                                        "style": style_name,
                                        "option_type": type_name,
                                        "spot": spot,
                                        "strike": strike,
                                        "payout": payout,
                                        "vol": vol,
                                        "t": t,
                                        "r_dom": r_dom,
                                        "r_for": r_for,
                                        "price": option.NPV(),
                                    }
                                )

    out = OUT_DIR / "digital_gk.csv"
    fields = [
        "style",
        "option_type",
        "spot",
        "strike",
        "payout",
        "vol",
        "t",
        "r_dom",
        "r_for",
        "price",
    ]
    _write_csv(out, fields, rows)
    return len(rows)


# --- Touch / no-touch / double-no-touch / double-touch references -------------
#
# These are the flagship FX "touch" products. QuantLib exposes them through the
# *double*-barrier binary engine (`AnalyticDoubleBarrierBinaryEngine`, European
# exercise, cash-or-nothing payout 1.0): a `DoubleBarrier.KnockOut` cash binary
# pays the discounted cash iff the spot never leaves the corridor `(L, U)` — i.e.
# it *is* the double-no-touch. Its `KnockIn` complement is the double-touch.
#
# Crucially, the *single*-barrier touch family is recovered as the wide-corridor
# limit of the very same independent QuantLib engine: pushing one wall far away
# (`L → 0` or `U → ∞`) leaves a single live wall, so the KnockOut cash binary
# collapses to the single **no-touch** (rebate paid at expiry) and its complement
# to the single **one-touch** (deferred / at-expiry). This makes QuantLib a fully
# independent oracle for the whole touch family — not the tautological
# `no_touch = df − one_touch` identity the in-crate tests use, but a separate
# implementation (QuantLib's reflection series) of the survival probability.
#
# The one-touch *at-hit* timing has no clean QuantLib analytic engine (QuantLib's
# `AnalyticBinaryBarrierEngine` prices a different knock-and-in-the-money binary),
# so the frozen grid validates the *at-expiry* timing — which shares all of the
# drift/exponent/sign machinery with the at-hit form — and the at-hit form is
# pinned separately in-crate by the `at_hit_dominates_deferred` ordering plus the
# Monte-Carlo cross-check. The CSV's `timing` column records `AT_EXPIRY`.

def _far_wall(spot: float, vol: float, t: float, up: bool) -> float:
    """A wall placed `n_std` standard deviations of `ln S_T` away from spot.

    `n_std = 12` puts the wall ≳ 12σ√T out, so the conditional probability of ever
    touching it is below `1e-30` — numerically a single live wall — while keeping
    the QuantLib reflection series well-conditioned (an effectively-infinite wall
    makes the series fail to converge). This recovers the *single* no-touch as the
    wide-corridor limit of the independent double-barrier-binary engine.
    """
    n_std = 12.0
    spread = n_std * vol * (t**0.5)
    return spot * (2.718_281_828_459_045**spread) if up else spot / (
        2.718_281_828_459_045**spread
    )


def _double_binary_no_touch(
    proc: object, expiry: ql.Date, lower: float, upper: float
) -> float:
    """QuantLib double-no-touch value: discounted cash (=1) iff spot stays in (L,U).

    Uses the European cash-or-nothing double-barrier binary KnockOut engine — an
    implementation fully independent of Celnet's image series.
    """
    payoff = ql.CashOrNothingPayoff(ql.Option.Call, 0.0, 1.0)
    option = ql.DoubleBarrierOption(
        ql.DoubleBarrier.KnockOut, lower, upper, 0.0, payoff,
        ql.EuropeanExercise(expiry),
    )
    option.setPricingEngine(ql.AnalyticDoubleBarrierBinaryEngine(proc))
    return option.NPV()


TOUCH_SPOT = 100.0
# Corridors span tight (≈5% wide) through wide, symmetric and skewed, so the
# image-series term count and the single-wall cap are exercised across regimes.
TOUCH_CORRIDORS = [
    (95.0, 106.0),
    (90.0, 112.0),
    (85.0, 120.0),
    (80.0, 130.0),
    (70.0, 145.0),
    (92.0, 130.0),
    (75.0, 108.0),
]
# Single barriers on each side of spot, near→far, for the single-touch limit.
TOUCH_SINGLE_UPPER = [105.0, 110.0, 115.0, 125.0, 140.0]
TOUCH_SINGLE_LOWER = [95.0, 90.0, 85.0, 75.0, 60.0]
TOUCH_VOLS = [0.08, 0.15, 0.25, 0.40]
TOUCH_MATS = [0.0833, 0.25, 1.0]  # ~1m, 3m, 1y
TOUCH_RATES = [(0.05, 0.01), (0.01, 0.03), (-0.005, 0.02)]


def generate_touch() -> int:
    """Write the frozen single/double touch + no-touch + double-touch grid.

    Single rows carry `kind ∈ {ONE_TOUCH, NO_TOUCH}` with `barrier` set and the
    unused corridor wall left blank; double rows carry
    `kind ∈ {DNT, DOUBLE_TOUCH}` with `lower`/`upper` set. Every price is the
    QuantLib double-barrier-binary value (single rows via the wide-corridor
    limit), so the Rust gate is an *independent* oracle for the whole family.
    Returns the row count.
    """
    _setup()
    rows = []
    for vol in TOUCH_VOLS:
        for t_target in TOUCH_MATS:
            expiry, t = _expiry_for(t_target)
            for r_dom, r_for in TOUCH_RATES:
                proc = _gk_process(TOUCH_SPOT, r_dom, r_for, vol)
                df = ql.FlatForward(EVAL_DATE, r_dom, DAY_COUNT).discount(expiry)

                # --- single upper / lower no-touch & one-touch (wide-corridor) --
                far_lo = _far_wall(TOUCH_SPOT, vol, t, up=False)
                far_hi = _far_wall(TOUCH_SPOT, vol, t, up=True)
                for barrier in TOUCH_SINGLE_UPPER:
                    nt = _double_binary_no_touch(proc, expiry, far_lo, barrier)
                    _push_touch_single(rows, "NO_TOUCH", barrier, vol, t,
                                       r_dom, r_for, nt)
                    _push_touch_single(rows, "ONE_TOUCH", barrier, vol, t,
                                       r_dom, r_for, df - nt)
                for barrier in TOUCH_SINGLE_LOWER:
                    nt = _double_binary_no_touch(proc, expiry, barrier, far_hi)
                    _push_touch_single(rows, "NO_TOUCH", barrier, vol, t,
                                       r_dom, r_for, nt)
                    _push_touch_single(rows, "ONE_TOUCH", barrier, vol, t,
                                       r_dom, r_for, df - nt)

                # --- double-no-touch & double-touch -----------------------------
                for lower, upper in TOUCH_CORRIDORS:
                    dnt = _double_binary_no_touch(proc, expiry, lower, upper)
                    _push_touch_double(rows, "DNT", lower, upper, vol, t,
                                      r_dom, r_for, dnt)
                    _push_touch_double(rows, "DOUBLE_TOUCH", lower, upper, vol,
                                      t, r_dom, r_for, df - dnt)

    out = OUT_DIR / "touch_gk.csv"
    fields = [
        "kind",
        "timing",
        "spot",
        "barrier",
        "lower",
        "upper",
        "rebate",
        "vol",
        "t",
        "r_dom",
        "r_for",
        "price",
    ]
    _write_csv(out, fields, rows)
    return len(rows)


def _push_touch_single(
    rows: list,
    kind: str,
    barrier: float,
    vol: float,
    t: float,
    r_dom: float,
    r_for: float,
    price: float,
) -> None:
    rows.append(
        {
            "kind": kind,
            "timing": "AT_EXPIRY",
            "spot": TOUCH_SPOT,
            "barrier": barrier,
            "lower": "",
            "upper": "",
            "rebate": 1.0,
            "vol": vol,
            "t": t,
            "r_dom": r_dom,
            "r_for": r_for,
            "price": price,
        }
    )


def _push_touch_double(
    rows: list,
    kind: str,
    lower: float,
    upper: float,
    vol: float,
    t: float,
    r_dom: float,
    r_for: float,
    price: float,
) -> None:
    rows.append(
        {
            "kind": kind,
            "timing": "AT_EXPIRY",
            "spot": TOUCH_SPOT,
            "barrier": "",
            "lower": lower,
            "upper": upper,
            "rebate": 1.0,
            "vol": vol,
            "t": t,
            "r_dom": r_dom,
            "r_for": r_for,
            "price": price,
        }
    )


# --- Double knock-out / knock-in (vanilla payoff) references ------------------


def generate_double_barrier() -> int:
    """Write analytic double-barrier knock-out and knock-in references.

    `AnalyticDoubleBarrierEngine` (KnockOut) prices the corridor knock-out vanilla
    directly — an oracle fully independent of Celnet's Ikeda-Kunitomo image
    series. The KnockIn row is the *independent* knock-in value (QuantLib's own
    KIKO/KOKI machinery is sensitive to which wall arms; we instead record the
    knock-in as `vanilla − KO_quantlib` computed from QuantLib's own vanilla and
    KO — still QuantLib-sourced on both legs, so a Celnet block error surfaces).
    Returns the row count.
    """
    _setup()
    rows = []
    spot = 100.0
    for vol in [0.10, 0.20, 0.35]:
        for t_target in [0.25, 1.0]:
            expiry, t = _expiry_for(t_target)
            for r_dom, r_for in [(0.05, 0.01), (0.01, 0.03)]:
                proc = _gk_process(spot, r_dom, r_for, vol)
                for lower, upper in [(85.0, 120.0), (90.0, 112.0), (80.0, 130.0)]:
                    for type_name, opt_type in OPTION_TYPES:
                        for strike in [90.0, 100.0, 110.0]:
                            payoff = ql.PlainVanillaPayoff(opt_type, strike)
                            ko = ql.DoubleBarrierOption(
                                ql.DoubleBarrier.KnockOut, lower, upper, 0.0,
                                payoff, ql.EuropeanExercise(expiry),
                            )
                            ko.setPricingEngine(ql.AnalyticDoubleBarrierEngine(proc))
                            van = ql.VanillaOption(
                                payoff, ql.EuropeanExercise(expiry)
                            )
                            van.setPricingEngine(ql.AnalyticEuropeanEngine(proc))
                            ko_npv = ko.NPV()
                            ki_npv = van.NPV() - ko_npv
                            for dko_kind, price in [("KO", ko_npv), ("KI", ki_npv)]:
                                rows.append(
                                    {
                                        "kind": dko_kind,
                                        "option_type": type_name,
                                        "spot": spot,
                                        "strike": strike,
                                        "lower": lower,
                                        "upper": upper,
                                        "vol": vol,
                                        "t": t,
                                        "r_dom": r_dom,
                                        "r_for": r_for,
                                        "price": price,
                                    }
                                )

    out = OUT_DIR / "double_barrier_gk.csv"
    fields = [
        "kind",
        "option_type",
        "spot",
        "strike",
        "lower",
        "upper",
        "vol",
        "t",
        "r_dom",
        "r_for",
        "price",
    ]
    _write_csv(out, fields, rows)
    return len(rows)


def _write_csv(path: Path, fields: list[str], rows: list[dict]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("w", newline="") as fh:
        fh.write(f"# Celnet frozen golden reference -- QuantLib {ql.__version__}\n")
        fh.write(
            "# Garman-Kohlhagen analytic engine; "
            "eval date 2026-06-15; Act/365F; NullCalendar.\n"
        )
        fh.write(
            "# Generated by tools/goldgen/generate.py -- DO NOT EDIT BY HAND.\n"
        )
        writer = csv.DictWriter(fh, fieldnames=fields)
        writer.writeheader()
        for row in rows:
            writer.writerow(row)


def main() -> None:
    n_v = generate_vanilla()
    n_b = generate_barrier()
    n_d = generate_digital()
    n_t = generate_touch()
    n_db = generate_double_barrier()
    print(f"QuantLib {ql.__version__}")
    print(f"vanilla_gk.csv         : {n_v} rows")
    print(f"barrier_gk.csv         : {n_b} rows")
    print(f"digital_gk.csv         : {n_d} rows")
    print(f"touch_gk.csv           : {n_t} rows")
    print(f"double_barrier_gk.csv  : {n_db} rows")
    print(f"written to {OUT_DIR}")


if __name__ == "__main__":
    main()
