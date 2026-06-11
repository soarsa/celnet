//! Frozen-bits FX calibration regression — the byte-identity gate for any
//! refactor that touches `celnet-surface` internals (CRYPTO-SURFACE-LEAF-SPEC
//! §5.1).
//!
//! Pins the **complete** FX calibration pipeline output bits: for each of the
//! five [`SmileModel`]s × two fixed broker quote sets, the calibrated smile's
//! implied vol at five strike ratios is asserted with `to_bits` equality — 50
//! `u64` pins. Any change to the FX fit path (including "pure" code motion of
//! the solver kit, ADR-0008 byte-identity discipline) must leave every pin
//! untouched; epsilon agreement is **not** accepted.
//!
//! The pins are platform-portable because every fit transcendental routes
//! through libm (`celnet_core::math`), the crate's existing bit-reproducibility
//! contract (see `calibrate.rs` "Determinism").
//!
//! # Sanctioned regeneration (only when the fixtures are intentionally changed)
//!
//! The assertion test prints the literal pin table before asserting; run
//!
//! ```text
//! cargo test -p celnet-surface --test fx_fit_pin -- --nocapture
//! ```
//!
//! and paste the printed `PINS` initializer over the constant below. The print
//! is part of the test (no `#[ignore]` escape hatch): regeneration is always a
//! deliberate, reviewed edit of this file, never a hidden runtime mode.

use celnet_conventions::resolve;
use celnet_core::Smile;
use celnet_surface::{MarketContext, MarketQuotes, SmileModel, build_model_smile};
use celnet_types::{Carry, CcyPair, Tenor};

/// The five calibrated smile models, in pin order.
const MODELS: [SmileModel; 5] = [
    SmileModel::MarketHedge,
    SmileModel::StochasticVol,
    SmileModel::Parametric,
    SmileModel::ParametricSurface,
    SmileModel::ExtendedSurface,
];

/// Strike ratios `x` (strike = `F·x`) at which each calibrated smile is pinned.
const STRIKE_RATIOS: [f64; 5] = [0.85, 0.95, 1.00, 1.05, 1.15];

/// The fixed EURUSD-1Y market context: `spot = 1.10`,
/// `Carry::FxRates { r_dom: 0.02, r_for: 0.01 }`, `t = 1.0`.
fn fixed_ctx() -> MarketContext {
    let conv = resolve(CcyPair::parse("EURUSD").unwrap(), Tenor::Years(1)).record;
    MarketContext::new(
        1.10,
        Carry::FxRates {
            r_dom: 0.02,
            r_for: 0.01,
        },
        1.0,
        conv,
    )
}

/// The two fixed broker quote sets: A (pronounced skew, five-point) and
/// B (mild, three-point).
fn quote_sets() -> [(&'static str, MarketQuotes); 2] {
    [
        (
            "A/five_point",
            MarketQuotes::five_point(0.11, -0.02, 0.006, -0.035, 0.012),
        ),
        (
            "B/three_point",
            MarketQuotes::three_point(0.10, -0.005, 0.0025),
        ),
    ]
}

/// Compute the full 2×5×5 pin table from the live calibration path.
fn computed_pins() -> [[[u64; 5]; 5]; 2] {
    let ctx = fixed_ctx();
    let f = ctx.forward();
    let mut out = [[[0_u64; 5]; 5]; 2];
    for (qi, (_, quotes)) in quote_sets().iter().enumerate() {
        for (mi, model) in MODELS.iter().enumerate() {
            let smile = build_model_smile(*model, &ctx, quotes)
                .expect("fixed FX pin fixtures must calibrate under every model");
            for (xi, x) in STRIKE_RATIOS.iter().enumerate() {
                out[qi][mi][xi] = smile.implied_vol(f * x, f, ctx.t).0.to_bits();
            }
        }
    }
    out
}

/// The frozen pins, captured from the unmodified pre-refactor FX path
/// (lane/crypto-surface D0) — `[quote_set][model][strike_ratio]`.
#[rustfmt::skip]
const PINS: [[[u64; 5]; 5]; 2] = [
    [
        [4594461895233561268, 4593197352666371746, 4592536085343199480, 4592328085970881790, 4593033457812773163],
        [4594148999501261629, 4593274779156893152, 4592760167673690250, 4592399764732221098, 4592203984890568024],
        [4594151549888816100, 4593287093974688223, 4592590756007337001, 4592278130493854160, 4592278697391504715],
        [4594152823373753863, 4593159822549565840, 4592590756007337001, 4592244790712812590, 4592257406330464839],
        [4594152823412800733, 4593159822562747135, 4592590756007337001, 4592244790731997245, 4592257406452680210],
    ],
    [
        [4593124213766799527, 4592099902509873903, 4591856347654164922, 4591834754771180859, 4592316939797406773],
        [4593110890259649823, 4592110101055637901, 4591870180066957722, 4591839764326833264, 4592264473604289996],
        [4592952501256984629, 4592117573182614752, 4591870180066957722, 4591835959633388872, 4592253156351396739],
        [4593052680350665058, 4592111749633230321, 4591870180066957722, 4591839960268512584, 4592237940386525962],
        [4593052680350665058, 4592111749633230321, 4591870180066957722, 4591839960268512584, 4592237940386525963],
    ],
];

/// Every FX calibration output bit is frozen: 5 models × 2 quote sets × 5
/// strikes, `to_bits` equality against the captured pins.
#[test]
fn fx_calibration_bits_are_frozen() {
    let got = computed_pins();

    // Sanctioned-regeneration aid: print the literal table (visible with
    // `--nocapture`) BEFORE asserting, so an intentional fixture change can be
    // re-pinned by paste, and an unintentional drift shows its full extent.
    println!("const PINS: [[[u64; 5]; 5]; 2] = [");
    for set in &got {
        println!("    [");
        for row in set {
            println!(
                "        [{}, {}, {}, {}, {}],",
                row[0], row[1], row[2], row[3], row[4]
            );
        }
        println!("    ],");
    }
    println!("];");

    let sets = quote_sets();
    for (qi, set) in got.iter().enumerate() {
        for (mi, row) in set.iter().enumerate() {
            for (xi, bits) in row.iter().enumerate() {
                assert_eq!(
                    *bits,
                    PINS[qi][mi][xi],
                    "FX fit bits drifted: quotes {} model {:?} strike ratio {} \
                     (got {:e}, pinned {:e})",
                    sets[qi].0,
                    MODELS[mi],
                    STRIKE_RATIOS[xi],
                    f64::from_bits(*bits),
                    f64::from_bits(PINS[qi][mi][xi]),
                );
            }
        }
    }
}
