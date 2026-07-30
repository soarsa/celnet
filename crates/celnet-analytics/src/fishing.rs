//! Quote-fishing / data-harvesting detection (§11.1a).
//!
//! A *fisher* fires many quotes/RFQs (or soaks the ESP stream) but rarely
//! deals, using our prices as free curve/mark data. The signal that identifies
//! one is: **high quote-to-trade ratio** (equivalently **low hit-rate**)
//! *combined with* **~zero net $/mm** — they consume pricing but leave us no
//! franchise value. A client who wins everything (high hit-rate) is not
//! fishing; a client we make real money on (positive net $/mm) is not fishing
//! however selective they are.
//!
//! [`fishing_score`] folds those signals into a single value bounded in
//! `[0, 1]` and **monotone** in each signal, so it can drive a read-only
//! tiering actuator (never an auto-block — §11.1a).

/// Net-$/mm scale (USD per USD 1mm) at which a client is considered *clearly
/// franchise-positive* and therefore not a fisher regardless of hit-rate.
///
/// This is a calibration constant, not a magic number: §11.1a frames the desk's
/// own $/mm distribution ("the client we make USD 120/mm on vs the one we make
/// USD 8/mm on"), so a net of ~USD 50/mm sits in the "clearly valuable" band
/// between those poles. A client at or above this net earns a zero fishing
/// penalty; the penalty ramps linearly to full as net $/mm falls to zero.
/// Tunable per desk without changing the formula's shape.
pub const DPM_VALUE_SCALE: f64 = 50.0;

/// The bounded, monotone quote-fishing score in `[0, 1]`.
///
/// Inputs are the already-computed rollup quantities:
/// - `hit_rate` = trades / quotes (`None` iff the client issued no quotes),
/// - `dpm_net` = net dollar-per-million (`None` iff the client did no trades),
/// - `quote_count` = number of quotes issued.
///
/// # Formula
///
/// ```text
/// fishing_score = fished_fraction · profit_penalty
///   fished_fraction = clamp(1 − hit_rate, 0, 1)              // low hit-rate ⇒ ↑
///   profit_penalty  = 1 − clamp(max(dpm_net, 0) / SCALE, 0, 1) // low net $/mm ⇒ ↑
/// ```
///
/// with two boundary conventions:
/// - **no quotes** (`quote_count == 0`) ⇒ score `0.0`: a client that consumed
///   no pricing cannot be fishing.
/// - **no trades** (`dpm_net == None`) ⇒ treat net $/mm as `0`, giving a full
///   profit penalty — the archetypal fisher (many quotes, zero fills) then
///   scores exactly `1.0` (its `hit_rate` is `0`).
///
/// # Why this shape
///
/// `fished_fraction` *is* the fraction of quotes that produced no trade — the
/// exact quantity a venue's order-to-trade-ratio policy polices (MiFID II
/// RTS 9), and monotone-increasing in the quote-to-trade ratio (`= 1/hit_rate`).
/// `profit_penalty` gates it: the product is high only when **both** the flow is
/// unconverted **and** it earns us ~nothing, so a selective-but-profitable
/// client and a low-margin-but-fully-converting client both score low. The
/// product of two `[0, 1]` factors stays in `[0, 1]`; each factor is monotone in
/// its signal, so the score is monotone in each signal with the others held
/// fixed.
#[must_use]
pub fn fishing_score(hit_rate: Option<f64>, dpm_net: Option<f64>, quote_count: u64) -> f64 {
    if quote_count == 0 {
        return 0.0;
    }
    // `quote_count > 0` guarantees `hit_rate` is `Some`; fall back to 0 (max
    // fishing) defensively rather than panic.
    let hr = hit_rate.unwrap_or(0.0);
    let fished_fraction = (1.0 - hr).clamp(0.0, 1.0);

    // No resolved trades ⇒ no franchise value ⇒ full penalty (net treated as 0).
    let net = dpm_net.unwrap_or(0.0);
    let profit_penalty = 1.0 - (net.max(0.0) / DPM_VALUE_SCALE).clamp(0.0, 1.0);

    fished_fraction * profit_penalty
}
