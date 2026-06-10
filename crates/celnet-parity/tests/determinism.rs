//! Parity row 15: **determinism** — identical inputs produce **bit-identical**
//! outputs, on every run and every thread. The platform's libm-only math
//! (`celnet-core::math`) makes every priced quantity reproducible to the last
//! bit; this underpins the auditable/replayable posture
//! (`docs/CAPABILITIES-VS-COMPETITION.md` §5 "Verification posture", and the
//! zero-cost lossless audit sink) that closed terminals cannot offer.
//!
//! ## Why this uses exact bit comparison (`to_bits()`)
//!
//! The binding rule "never compare floats with `==`/`!=`" guards against
//! tolerance-free *correctness* comparisons. It does **not** apply here: the
//! property under test *is* bit-for-bit reproducibility, so an exact integer
//! comparison of the IEEE-754 bit pattern (`f64::to_bits`) is the *correct*
//! instrument, not a numerical-agreement check. We route through the integer
//! bit pattern precisely to avoid float `==` semantics (NaN, ±0).
//!
//! ## What makes these checks non-vacuous
//!
//! Calling a pure function twice back-to-back only proves referential
//! transparency, not reproducibility. To genuinely prove reproducibility we:
//!
//!  1. **Reconstruct inputs from scratch** — serialize each input to a stable
//!     decimal/string snapshot, parse it back into a *freshly-built* value with
//!     no shared provenance, and require the recomputed output to be
//!     bit-identical. This catches a computation that secretly depends on input
//!     *identity* / address / allocation rather than value.
//!  2. **Cross-thread** — run the same computation on `N` spawned threads and
//!     require all of them to agree to the bit. This catches hidden global or
//!     thread-local state (lazy caches, `thread_local!` scratch, atomics).
//!  3. **Golden-bits baseline** — compare against a committed table of expected
//!     bit patterns ([`GOLDEN_VANILLA`]). This proves cross-*run* and
//!     cross-*build* stability: a change in the math, the compiler's float
//!     contraction, or the platform that perturbs a single ULP fails the test.

use std::thread;

use celnet_conventions::resolve;
use celnet_core::Smile;
use celnet_exotics::{
    BarrierKind, BarrierStyle, DigitalKind, SingleBarrier, digital_price, single_barrier_price,
};
use celnet_surface::{MarketContext, MarketQuotes, build_smile};
use celnet_types::{CcyPair, OptionType, Tenor, VanillaInputs};
use celnet_vanilla::{greeks, price};

/// Assert two f64s are bit-identical (the determinism property, not a numeric
/// comparison — see module docs for why `to_bits()` exact equality is correct
/// here and the float-`==` rule does not apply).
#[track_caller]
fn assert_bit_identical(a: f64, b: f64, what: &str) {
    assert_eq!(
        a.to_bits(),
        b.to_bits(),
        "{what} not bit-identical: {a} ({:#018x}) vs {b} ({:#018x})",
        a.to_bits(),
        b.to_bits()
    );
}

/// Number of worker threads for the cross-thread reproducibility check.
const THREADS: usize = 8;

/// A stable, lossless snapshot of a [`VanillaInputs`]: the six scalar fields
/// rendered to a round-trippable string. Serializing and re-parsing builds a
/// *fresh* value with no shared provenance with the original — so a recomputation
/// from the snapshot proves value-determinism, not identity-determinism.
fn snapshot(i: &VanillaInputs) -> String {
    // `{:?}` on f64 is the shortest round-trippable decimal in Rust; parsing it
    // back yields the bit-identical f64.
    format!(
        "{:?}|{:?}|{:?}|{:?}|{:?}|{:?}",
        i.spot, i.strike, i.vol, i.t, i.r_dom, i.r_for
    )
}

/// Reconstruct a [`VanillaInputs`] from a [`snapshot`] string. The parsed
/// scalars are bit-identical to the originals (shortest-round-trip decimal), but
/// the resulting struct is freshly allocated from text with no link to the
/// source value.
fn from_snapshot(s: &str) -> VanillaInputs {
    let mut it = s.split('|').map(|f| f.parse::<f64>().expect("round-trips"));
    let mut next = || it.next().expect("six fields");
    VanillaInputs::new(next(), next(), next(), next(), next(), next())
}

/// Row 15 (vanilla) — price and the full 13-Greek set are reproducible to the
/// bit: (a) recomputed from a from-scratch snapshot reconstruction, (b) across
/// `THREADS` independent threads, and (c) against a committed golden-bits table.
#[test]
fn price_and_greeks_are_bit_identical() {
    let markets = [
        VanillaInputs::new(100.0, 100.0, 0.20, 1.0, 0.05, 0.0),
        VanillaInputs::new(1.10, 1.25, 0.09, 0.5, 0.02, 0.01),
        VanillaInputs::new(110.0, 95.0, 0.30, 0.25, 0.01, 0.03),
        VanillaInputs::new(1.20, 1.05, 0.11, 5.0, 0.03, 0.01),
    ];
    let mut rows = 0usize;
    for i in &markets {
        for opt in [OptionType::Call, OptionType::Put] {
            // Reference values, computed once.
            let p_ref = price(opt, i);
            let g_ref = greeks(opt, i);

            // (a) Reconstruct the inputs from a serialized snapshot — a value
            // built from scratch out of text, sharing no provenance with `i` —
            // and require bit-identical outputs. Proves value-determinism.
            let snap = snapshot(i);
            let rebuilt = from_snapshot(&snap);
            assert_bit_identical(price(opt, &rebuilt), p_ref, "price (from snapshot)");
            let g_rebuilt = greeks(opt, &rebuilt);
            assert_bit_identical(g_rebuilt.price, g_ref.price, "greeks.price (from snapshot)");
            for (field, a, b) in greek_fields(&g_rebuilt, &g_ref) {
                assert_bit_identical(a, b, field);
            }

            // (b) Cross-thread: every worker rebuilds inputs from the snapshot
            // and recomputes; all must agree to the bit. Catches hidden
            // global/thread-local state.
            let snap_for_threads = snap.clone();
            let handles: Vec<_> = (0..THREADS)
                .map(|_| {
                    let s = snap_for_threads.clone();
                    thread::spawn(move || {
                        let inp = from_snapshot(&s);
                        let g = greeks(opt, &inp);
                        (price(opt, &inp).to_bits(), greeks_to_bits(&g))
                    })
                })
                .collect();
            for h in handles {
                let (pbits, gbits) = h.join().expect("worker thread");
                assert_eq!(pbits, p_ref.to_bits(), "price differs across threads");
                assert_eq!(
                    gbits,
                    greeks_to_bits(&g_ref),
                    "greeks differ across threads"
                );
            }
            rows += 1;
        }
    }
    assert!(rows >= 8, "determinism vanilla rows under-gated: {rows}");

    // (c) Golden-bits baseline — proves cross-RUN / cross-BUILD stability. Each
    // entry is (snapshot, OptionType discriminant, price_bits, greeks_bits).
    // A single-ULP change anywhere in the math or its lowering fails here.
    let mut golden_rows = 0usize;
    for &(snap, is_call, price_bits, greeks_bits) in GOLDEN_VANILLA {
        let inp = from_snapshot(snap);
        let opt = if is_call {
            OptionType::Call
        } else {
            OptionType::Put
        };
        assert_eq!(
            price(opt, &inp).to_bits(),
            price_bits,
            "golden price bits drifted for snapshot {snap} (opt call={is_call})"
        );
        assert_eq!(
            greeks_to_bits(&greeks(opt, &inp)),
            greeks_bits,
            "golden greeks bits drifted for snapshot {snap} (opt call={is_call})"
        );
        golden_rows += 1;
    }
    assert_eq!(
        golden_rows,
        GOLDEN_VANILLA.len(),
        "golden table under-gated"
    );
}

/// Pair every sensitivity field of two [`celnet_types::Greeks`] for field-wise
/// bit comparison (the 13 sensitivities plus `price`).
fn greek_fields<'a>(
    a: &'a celnet_types::Greeks,
    b: &'a celnet_types::Greeks,
) -> [(&'static str, f64, f64); 14] {
    [
        ("greeks.price", a.price, b.price),
        ("delta_spot", a.delta_spot, b.delta_spot),
        ("delta_forward", a.delta_forward, b.delta_forward),
        ("gamma", a.gamma, b.gamma),
        ("vega", a.vega, b.vega),
        ("theta", a.theta, b.theta),
        ("rho_dom", a.rho_dom, b.rho_dom),
        ("rho_for", a.rho_for, b.rho_for),
        ("vanna", a.vanna, b.vanna),
        ("volga", a.volga, b.volga),
        ("charm", a.charm, b.charm),
        ("speed", a.speed, b.speed),
        ("zomma", a.zomma, b.zomma),
        ("color", a.color, b.color),
    ]
}

/// Fold a full Greek set into a stable bit tuple for cross-thread / golden
/// equality (order matches [`greek_fields`]).
fn greeks_to_bits(g: &celnet_types::Greeks) -> [u64; 14] {
    [
        g.price.to_bits(),
        g.delta_spot.to_bits(),
        g.delta_forward.to_bits(),
        g.gamma.to_bits(),
        g.vega.to_bits(),
        g.theta.to_bits(),
        g.rho_dom.to_bits(),
        g.rho_for.to_bits(),
        g.vanna.to_bits(),
        g.volga.to_bits(),
        g.charm.to_bits(),
        g.speed.to_bits(),
        g.zomma.to_bits(),
        g.color.to_bits(),
    ]
}

/// Committed golden-bits baseline: `(snapshot, is_call, price_bits, greeks_bits)`.
///
/// These are the exact IEEE-754 bit patterns the libm-only pricer produces for
/// the textbook regime; they are checked into the test so any future change that
/// perturbs a single ULP (a math tweak, a compiler float-contraction change, a
/// platform difference) is caught as a *cross-run* reproducibility regression,
/// not silently absorbed. Regenerate intentionally (and review the diff) only
/// when a numerical change is deliberate.
type GoldenRow = (&'static str, bool, u64, [u64; 14]);
static GOLDEN_VANILLA: &[GoldenRow] = &[
    (
        "100.0|100.0|0.2|1.0|0.05|0.0",
        true,
        GOLDEN_CALL_PRICE_BITS,
        GOLDEN_CALL_GREEKS_BITS,
    ),
    (
        "100.0|100.0|0.2|1.0|0.05|0.0",
        false,
        GOLDEN_PUT_PRICE_BITS,
        GOLDEN_PUT_GREEKS_BITS,
    ),
];

// The textbook S=K=100, σ=20%, T=1, r_d=5%, r_f=0 regime. Price 10.450583572…
// (call) is the long-standing Black-Scholes reference value gated elsewhere in
// the suite, so the bit pattern below is anchored to a value validated against
// QuantLib / the closed form — not an arbitrary capture.
const GOLDEN_CALL_PRICE_BITS: u64 = 0x4024_e6b2_e3d5_4dc0;
const GOLDEN_PUT_PRICE_BITS: u64 = 0x4016_4b4a_67d3_fea0;
const GOLDEN_CALL_GREEKS_BITS: [u64; 14] = [
    GOLDEN_CALL_PRICE_BITS,
    0x3fe4_60ea_ac7c_7829,
    0x3fe4_60ea_ac7c_7829,
    0x3f93_3659_aba1_2cb7,
    0x4042_c313_919b_65ab,
    0xc019_a7f6_d64e_6176,
    0x404a_9dc1_f48d_2850,
    0xc04f_d76e_ad82_7bc0,
    0xbfd2_02f4_10e7_19ec,
    0x4023_b33a_f27c_c45b,
    0x3fb0_cf8e_762d_0720,
    0xbf40_e825_f331_ac78,
    0xbfb6_c12b_cdac_7dc3,
    0xbf85_90d9_2292_ffa2,
];
const GOLDEN_PUT_GREEKS_BITS: [u64; 14] = [
    GOLDEN_PUT_PRICE_BITS,
    0xbfd7_3e2a_a707_0fae,
    0xbfd7_3e2a_a707_0fae,
    0x3f93_3659_aba1_2cb7,
    0x4042_c313_919b_65ab,
    0xbffa_86ad_9f97_a536,
    0xc044_f1fa_9f78_0414,
    0x4042_2891_527d_8440,
    0xbfd2_02f4_10e7_19ec,
    0x4023_b33a_f27c_c45b,
    0x3fb0_cf8e_762d_0720,
    0xbf40_e825_f331_ac78,
    0xbfb6_c12b_cdac_7dc3,
    0xbf85_90d9_2292_ffa2,
];

/// Row 15 (surface + exotics) — the broker→smile construction and the analytic
/// exotic pricers are bit-identical across (a) a from-scratch quote
/// reconstruction and (b) `THREADS` independent threads. A smile rebuilt from
/// freshly-parsed quotes yields bit-identical implied vols at the wings, and the
/// digital/barrier pricers reproduce exactly across threads.
#[test]
fn smile_and_exotics_are_bit_identical() {
    // Stable scalar snapshot of the market state + quotes (built from scratch in
    // each evaluation so no provenance is shared). Each rebuild re-resolves the
    // conventions from the pair/tenor as well, so a worker thread shares no state
    // at all with the reference.
    let state = "1.10|0.02|0.01|1.0|0.105|0.012|0.0035|0.020|0.009";
    fn rebuild(s: &str) -> (MarketContext, MarketQuotes) {
        let conv = resolve(CcyPair::parse("EURUSD").unwrap(), Tenor::Years(1)).record;
        let mut it = s.split('|').map(|f| f.parse::<f64>().expect("round-trips"));
        let mut n = || it.next().expect("field");
        let ctx = MarketContext::new(
            n(),
            celnet_types::Carry::FxRates {
                r_dom: n(),
                r_for: n(),
            },
            n(),
            conv,
        );
        let quotes = MarketQuotes::five_point(n(), n(), n(), n(), n());
        (ctx, quotes)
    }

    let (ctx_ref, quotes_ref) = rebuild(state);
    let f = ctx_ref.forward();
    let smile_ref = build_smile(&ctx_ref, &quotes_ref).expect("smile builds");
    let strikes = [0.9 * f, f, 1.1 * f, 1.25 * f];
    let vol_ref: Vec<u64> = strikes
        .iter()
        .map(|&k| smile_ref.implied_vol(k, f, ctx_ref.t).0.to_bits())
        .collect();

    let mut rows = 0usize;

    // (a) Rebuild context+quotes from scratch and require bit-identical vols.
    let (ctx2, quotes2) = rebuild(state);
    let smile2 = build_smile(&ctx2, &quotes2).expect("smile rebuilds");
    for (j, &k) in strikes.iter().enumerate() {
        let v = smile2.implied_vol(k, f, ctx2.t).0;
        assert_eq!(
            v.to_bits(),
            vol_ref[j],
            "smile implied vol drifted under from-scratch rebuild at K={k}"
        );
        rows += 1;
    }

    // (b) Cross-thread: each worker rebuilds the smile from the snapshot string
    // and reprices the wings; all must agree to the bit.
    let handles: Vec<_> = (0..THREADS)
        .map(|_| {
            let s = state.to_string();
            thread::spawn(move || {
                let (c, q) = rebuild(&s);
                let smile = build_smile(&c, &q).expect("worker smile builds");
                let fwd = c.forward();
                [0.9 * fwd, fwd, 1.1 * fwd, 1.25 * fwd]
                    .iter()
                    .map(|&k| smile.implied_vol(k, fwd, c.t).0.to_bits())
                    .collect::<Vec<_>>()
            })
        })
        .collect();
    for h in handles {
        let v = h.join().expect("worker thread");
        assert_eq!(v, vol_ref, "smile implied vol differs across threads");
    }

    // Exotics: digital + barrier reproduce across threads from rebuilt inputs.
    let i = VanillaInputs::new(100.0, 100.0, 0.2, 1.0, 0.03, 0.01);
    let isnap = snapshot(&i);
    let d_ref = digital_price(DigitalKind::cash(OptionType::Call), &(&i).into()).to_bits();
    let spec = SingleBarrier {
        kind: BarrierKind {
            up: false,
            style: BarrierStyle::KnockOut,
            option: OptionType::Call,
        },
        strike: 100.0,
        barrier: 85.0,
        rebate: 0.0,
    };
    let b_ref = single_barrier_price(&(&i).into(), spec).to_bits();
    let ex_handles: Vec<_> = (0..THREADS)
        .map(|_| {
            let s = isnap.clone();
            thread::spawn(move || {
                let inp = from_snapshot(&s);
                let d =
                    digital_price(DigitalKind::cash(OptionType::Call), &(&inp).into()).to_bits();
                let b = single_barrier_price(&(&inp).into(), spec).to_bits();
                (d, b)
            })
        })
        .collect();
    for h in ex_handles {
        let (d, b) = h.join().expect("worker thread");
        assert_eq!(d, d_ref, "digital price differs across threads");
        assert_eq!(b, b_ref, "barrier price differs across threads");
    }
    rows += 2;

    assert!(
        rows >= 6,
        "determinism surface/exotics rows under-gated: {rows}"
    );
}
