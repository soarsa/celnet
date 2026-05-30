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
    print(f"QuantLib {ql.__version__}")
    print(f"vanilla_gk.csv  : {n_v} rows")
    print(f"barrier_gk.csv  : {n_b} rows")
    print(f"digital_gk.csv  : {n_d} rows")
    print(f"written to {OUT_DIR}")


if __name__ == "__main__":
    main()
