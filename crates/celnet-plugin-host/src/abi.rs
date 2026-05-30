//! The host-controlled core-module ABI for Tier-2 Wasm pricing models.
//!
//! The frozen [`celnet_plugin_api`] contract is `Copy`-POD in / `Copy`-POD out,
//! which lowers cleanly onto a **core Wasm module** (we do not need the
//! Component Model — see `docs/PLUGIN-HOST-ALT.md` §3.1/§7). The host owns the
//! calling convention end-to-end:
//!
//! - [`VanillaInputs`] is a fixed record of [`INPUT_FIELDS`] little-endian `f64`s
//!   (`spot, strike, vol, t, r_dom, r_for`). The host serializes it into the
//!   guest's linear memory at a guest-provided scratch buffer and passes the
//!   `(ptr, len)` pair to the guest entry point.
//! - [`Greeks`] is a fixed record of [`GREEKS_FIELDS`] little-endian `f64`s in
//!   the field order documented in [`greeks_from_words`]. The guest writes them
//!   into an output scratch buffer; the host reads `(ptr, len)` back.
//! - A scalar `f64` price is returned directly in a Wasm register.
//! - The option type is marshalled as an `i32` discriminant ([`opt_to_abi`] /
//!   [`opt_from_abi`]): `0 = Call`, `1 = Put`.
//!
//! # Determinism boundary obligations
//!
//! Every `f64` crossing the boundary is **NaN-canonicalized** ([`canonicalize`])
//! so a guest cannot smuggle a non-canonical NaN bit pattern into a replay (Wasm
//! permits multiple NaN payloads; we collapse them to one canonical quiet NaN).
//! Combined with the contract's `libm`-only transcendentals and "no `==`/`NaN`
//! compares" rule, this makes Tier-2 output bit-identical across runs and
//! platforms — the invariant the replay harness asserts.

use celnet_types::{Greeks, OptionType, VanillaInputs};

/// Number of `f64` fields in the marshalled [`VanillaInputs`] record.
pub const INPUT_FIELDS: usize = 6;
/// Byte length of a marshalled [`VanillaInputs`] record.
pub const INPUT_BYTES: usize = INPUT_FIELDS * core::mem::size_of::<f64>();

/// Number of `f64` fields in the marshalled [`Greeks`] record.
pub const GREEKS_FIELDS: usize = 14;
/// Byte length of a marshalled [`Greeks`] record.
pub const GREEKS_BYTES: usize = GREEKS_FIELDS * core::mem::size_of::<f64>();

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

/// Serialize [`VanillaInputs`] to the fixed little-endian wire layout, with each
/// field NaN-canonicalized on the way in.
#[must_use]
pub fn input_to_bytes(inputs: &VanillaInputs) -> [u8; INPUT_BYTES] {
    let fields = [
        inputs.spot,
        inputs.strike,
        inputs.vol,
        inputs.t,
        inputs.r_dom,
        inputs.r_for,
    ];
    let mut out = [0u8; INPUT_BYTES];
    for (i, f) in fields.iter().enumerate() {
        let b = canonicalize(*f).to_bits().to_le_bytes();
        out[i * 8..i * 8 + 8].copy_from_slice(&b);
    }
    out
}

/// Read a little-endian `f64` from `words[i]`, canonicalizing NaN.
fn word(words: &[f64], i: usize) -> f64 {
    canonicalize(words[i])
}

/// Reconstruct [`Greeks`] from the [`GREEKS_FIELDS`] canonicalized words a guest
/// wrote, in the host-defined field order.
///
/// Order: `price, delta_spot, delta_forward, gamma, vega, theta, rho_dom,
/// rho_for, vanna, volga, charm, speed, zomma, color` — matching the
/// [`celnet_types::Greeks`] declaration so Tier-0 and Tier-2 agree field-for-field.
#[must_use]
pub fn greeks_from_words(words: &[f64; GREEKS_FIELDS]) -> Greeks {
    Greeks {
        price: word(words, 0),
        delta_spot: word(words, 1),
        delta_forward: word(words, 2),
        gamma: word(words, 3),
        vega: word(words, 4),
        theta: word(words, 5),
        rho_dom: word(words, 6),
        rho_for: word(words, 7),
        vanna: word(words, 8),
        volga: word(words, 9),
        charm: word(words, 10),
        speed: word(words, 11),
        zomma: word(words, 12),
        color: word(words, 13),
    }
}

/// Decode [`GREEKS_BYTES`] of little-endian guest memory into the [`Greeks`]
/// record, canonicalizing every field.
///
/// Returns `None` if `bytes` is not exactly [`GREEKS_BYTES`] long.
#[must_use]
pub fn greeks_from_bytes(bytes: &[u8]) -> Option<Greeks> {
    if bytes.len() != GREEKS_BYTES {
        return None;
    }
    let mut words = [0.0_f64; GREEKS_FIELDS];
    for (i, w) in words.iter_mut().enumerate() {
        let mut le = [0u8; 8];
        le.copy_from_slice(&bytes[i * 8..i * 8 + 8]);
        *w = f64::from_bits(u64::from_le_bytes(le));
    }
    Some(greeks_from_words(&words))
}
