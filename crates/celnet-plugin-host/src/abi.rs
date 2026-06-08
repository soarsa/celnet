//! The host-controlled core-module ABI for Tier-2 Wasm pricing models.
//!
//! The frozen [`celnet_plugin_api`] contract is `Copy`-POD in / `Copy`-POD out,
//! which lowers cleanly onto a **core Wasm module** (we do not need the
//! Component Model — see `docs/PLUGIN-HOST-ALT.md` §3.1/§7). The host owns the
//! calling convention end-to-end, and it carries the *generalized*, carry-tagged
//! vocabulary so a guest can declare and price a non-FX asset class:
//!
//! - [`CarryInputs`] is a fixed record of [`INPUT_FIELDS`] little-endian `f64`s
//!   (`spot, strike, vol, t, carry_field_0, carry_field_1`) followed by two
//!   little-endian `i32` discriminant words (`underlying_class, carry_kind`),
//!   laid out by [`input_to_bytes`]. The host serializes it into the guest's
//!   linear memory at a guest-provided scratch buffer and passes the `(ptr, len)`
//!   pair to the guest entry point.
//! - [`CarryGreeks`] is a fixed record of [`GREEKS_FIELDS`] little-endian `f64`s
//!   in the order documented in [`greeks_from_words`], followed by one
//!   little-endian `i32` `rate_kind` discriminant. Two of the `f64` words are the
//!   carry-tagged rate sensitivities, read as FX (`rho_dom, rho_for`) or carry
//!   (`discount_rho, carry_rho`) according to `rate_kind`. The guest writes them
//!   into an output scratch buffer; the host reads `(ptr, len)` back.
//! - A scalar `f64` price is returned directly in a Wasm register.
//! - The option type is marshalled as an `i32` discriminant ([`opt_to_abi`] /
//!   [`opt_from_abi`]): `0 = Call`, `1 = Put`.
//!
//! # Carry encoding (vendor-neutral, asset-class-tagged)
//!
//! The two numeric carry fields hold either the FX two rates (`r_dom, r_for`) or
//! the cost-of-carry pair (`r, b`), distinguished by the `carry_kind` word
//! ([`CARRY_KIND_FX_RATES`] / [`CARRY_KIND_COST_OF_CARRY`]). The numeric block
//! `spot, strike, vol, t, carry_0, carry_1` keeps the FX two-rate layout at the
//! same byte offsets as before, so an FX guest reading `r_dom`/`r_for` at offsets
//! 32/40 needs no change; only the trailing discriminant words and the rate-kind
//! tag are new. The currency-pair identity of an FX underlying does not enter the
//! carry/forward math and is not marshalled — the asset class is a single
//! [`UNDERLYING_CLASS_FX`] discriminant.
//!
//! # Determinism boundary obligations
//!
//! Every `f64` crossing the boundary is **NaN-canonicalized** ([`canonicalize`])
//! so a guest cannot smuggle a non-canonical NaN bit pattern into a replay (Wasm
//! permits multiple NaN payloads; we collapse them to one canonical quiet NaN).
//! Combined with the contract's `libm`-only transcendentals and "no `==`/`NaN`
//! compares" rule, this makes Tier-2 output bit-identical across runs and
//! platforms — the invariant the replay harness asserts.

use celnet_core::{CarryGreeks, CarryInputs};
use celnet_types::{Carry, OptionType, RateSensitivities, Underlying};

/// Number of `f64` fields in the marshalled [`CarryInputs`] numeric block
/// (`spot, strike, vol, t, carry_field_0, carry_field_1`).
pub const INPUT_FIELDS: usize = 6;
/// Number of `i32` discriminant words trailing the [`CarryInputs`] numeric block
/// (`underlying_class, carry_kind`).
pub const INPUT_TAGS: usize = 2;
/// Byte length of a marshalled [`CarryInputs`] record: the `f64` numeric block
/// plus the trailing `i32` discriminant words.
pub const INPUT_BYTES: usize =
    INPUT_FIELDS * core::mem::size_of::<f64>() + INPUT_TAGS * core::mem::size_of::<i32>();

/// Number of `f64` fields in the marshalled [`CarryGreeks`] record (two of which
/// are the carry-tagged rate sensitivities, interpreted via `rate_kind`).
pub const GREEKS_FIELDS: usize = 14;
/// Byte length of a marshalled [`CarryGreeks`] record: the `f64` block plus the
/// trailing `i32` `rate_kind` discriminant word.
pub const GREEKS_BYTES: usize =
    GREEKS_FIELDS * core::mem::size_of::<f64>() + core::mem::size_of::<i32>();

/// Discriminant for the FX underlying asset class.
pub const UNDERLYING_CLASS_FX: i32 = 0;

/// Discriminant for [`Carry::FxRates`] (`carry_field_0 = r_dom`, `_1 = r_for`).
pub const CARRY_KIND_FX_RATES: i32 = 0;
/// Discriminant for [`Carry::CostOfCarry`] (`carry_field_0 = r`, `_1 = b`).
pub const CARRY_KIND_COST_OF_CARRY: i32 = 1;

/// Discriminant for [`RateSensitivities::Fx`] (`rate_field_0 = rho_dom`,
/// `_1 = rho_for`).
pub const RATE_KIND_FX: i32 = 0;
/// Discriminant for [`RateSensitivities::Carry`] (`rate_field_0 = discount_rho`,
/// `_1 = carry_rho`).
pub const RATE_KIND_CARRY: i32 = 1;

/// The canonical quiet-NaN bit pattern the host normalizes every boundary NaN to.
///
/// Wasm `f64.const`/arithmetic may produce any NaN payload; pinning one pattern
/// removes that degree of freedom so two runs are bit-identical. This is the
/// IEEE-754 canonical quiet NaN with a zero payload and clear sign bit.
pub const CANONICAL_NAN_BITS: u64 = 0x7ff8_0000_0000_0000;

/// Map an [`OptionType`] to its ABI discriminant.
#[must_use]
pub const fn opt_to_abi(opt: OptionType) -> i32 {
    match opt {
        OptionType::Call => 0,
        OptionType::Put => 1,
    }
}

/// Map an ABI discriminant back to an [`OptionType`].
///
/// Returns `None` for any value other than `0` (Call) or `1` (Put), so a guest
/// cannot coerce an out-of-range discriminant into a valid option type.
#[must_use]
pub const fn opt_from_abi(raw: i32) -> Option<OptionType> {
    match raw {
        0 => Some(OptionType::Call),
        1 => Some(OptionType::Put),
        _ => None,
    }
}

/// The asset-class discriminant of an [`Underlying`].
#[must_use]
pub const fn underlying_to_abi(u: Underlying) -> i32 {
    match u {
        Underlying::Fx(_) => UNDERLYING_CLASS_FX,
    }
}

/// Decompose a [`Carry`] into `(carry_kind, field_0, field_1)` for the wire.
#[must_use]
pub const fn carry_to_abi(carry: Carry) -> (i32, f64, f64) {
    match carry {
        Carry::FxRates { r_dom, r_for } => (CARRY_KIND_FX_RATES, r_dom, r_for),
        Carry::CostOfCarry { r, b } => (CARRY_KIND_COST_OF_CARRY, r, b),
    }
}

/// Collapse any NaN to the single [`CANONICAL_NAN_BITS`] pattern; pass every
/// other value (including signed zeros and infinities) through unchanged.
///
/// Applied to every `f64` crossing the host↔guest boundary in both directions.
#[must_use]
pub fn canonicalize(x: f64) -> f64 {
    if x.is_nan() {
        f64::from_bits(CANONICAL_NAN_BITS)
    } else {
        x
    }
}

/// Serialize [`CarryInputs`] to the fixed little-endian wire layout: the six
/// `f64` numeric words (`spot, strike, vol, t, carry_0, carry_1`, each
/// NaN-canonicalized) followed by the `underlying_class` and `carry_kind` `i32`
/// discriminant words.
#[must_use]
pub fn input_to_bytes(inputs: &CarryInputs) -> [u8; INPUT_BYTES] {
    let (carry_kind, carry_0, carry_1) = carry_to_abi(inputs.carry);
    let fields = [
        inputs.spot,
        inputs.strike,
        inputs.vol,
        inputs.t,
        carry_0,
        carry_1,
    ];
    let mut out = [0u8; INPUT_BYTES];
    for (i, f) in fields.iter().enumerate() {
        let b = canonicalize(*f).to_bits().to_le_bytes();
        out[i * 8..i * 8 + 8].copy_from_slice(&b);
    }
    // Trailing i32 discriminant words, packed after the numeric block.
    let base = INPUT_FIELDS * 8;
    out[base..base + 4].copy_from_slice(&underlying_to_abi(inputs.underlying).to_le_bytes());
    out[base + 4..base + 8].copy_from_slice(&carry_kind.to_le_bytes());
    out
}

/// Read a little-endian `f64` from `words[i]`, canonicalizing NaN.
fn word(words: &[f64], i: usize) -> f64 {
    canonicalize(words[i])
}

/// Reconstruct [`CarryGreeks`] from the [`GREEKS_FIELDS`] canonicalized words a
/// guest wrote plus the trailing `rate_kind` discriminant, in the host-defined
/// field order.
///
/// Order: `price, delta_spot, delta_forward, gamma, vega, theta, rate_field_0,
/// rate_field_1, vanna, volga, charm, speed, zomma, color` — matching the
/// [`celnet_core::CarryGreeks`] declaration so Tier-0 and Tier-2 agree
/// field-for-field. `rate_field_0`/`rate_field_1` are interpreted as FX
/// (`rho_dom, rho_for`) when `rate_kind == `[`RATE_KIND_FX`], else as carry
/// (`discount_rho, carry_rho`). Any unrecognized `rate_kind` is treated as the
/// generalized carry tagging (the guest's own verdict on its asset class).
#[must_use]
pub fn greeks_from_words(words: &[f64; GREEKS_FIELDS], rate_kind: i32) -> CarryGreeks {
    let rate_0 = word(words, 6);
    let rate_1 = word(words, 7);
    let rates = if rate_kind == RATE_KIND_FX {
        RateSensitivities::Fx {
            rho_dom: rate_0,
            rho_for: rate_1,
        }
    } else {
        RateSensitivities::Carry {
            discount_rho: rate_0,
            carry_rho: rate_1,
        }
    };
    CarryGreeks {
        price: word(words, 0),
        delta_spot: word(words, 1),
        delta_forward: word(words, 2),
        gamma: word(words, 3),
        vega: word(words, 4),
        theta: word(words, 5),
        rates,
        vanna: word(words, 8),
        volga: word(words, 9),
        charm: word(words, 10),
        speed: word(words, 11),
        zomma: word(words, 12),
        color: word(words, 13),
    }
}

/// Decode [`GREEKS_BYTES`] of little-endian guest memory into the
/// [`CarryGreeks`] record, canonicalizing every `f64` field and reading the
/// trailing `rate_kind` discriminant.
///
/// Returns `None` if `bytes` is not exactly [`GREEKS_BYTES`] long.
#[must_use]
pub fn greeks_from_bytes(bytes: &[u8]) -> Option<CarryGreeks> {
    if bytes.len() != GREEKS_BYTES {
        return None;
    }
    let mut words = [0.0_f64; GREEKS_FIELDS];
    for (i, w) in words.iter_mut().enumerate() {
        let mut le = [0u8; 8];
        le.copy_from_slice(&bytes[i * 8..i * 8 + 8]);
        *w = f64::from_bits(u64::from_le_bytes(le));
    }
    let base = GREEKS_FIELDS * 8;
    let mut kind = [0u8; 4];
    kind.copy_from_slice(&bytes[base..base + 4]);
    Some(greeks_from_words(&words, i32::from_le_bytes(kind)))
}
