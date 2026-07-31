//! The normalized corporate-action event record — the CAEV/CAMV vocabulary and lifecycle.
//!
//! Celnet adopts the open ISO 15022 (MT 564/565/566) and ISO 20022 (`seev.031/033/036/037/039`)
//! **code sets and message semantics** as its internal contract (guardrail 8: adopt the open
//! vocabulary, embed no vendor product). This record is the vendor-neutral normalization target
//! every ingest adapter maps onto; the pure effect math in [`crate::effect`] reads its
//! [`CaEvent::terms`] and effective date, nothing else.

use serde::{Deserialize, Serialize};

use crate::date::CivilDate;

/// CAEV — the corporate-action **event type**. The govvie-deterministic set is derivable in-house
/// from open issuance data; the corporate set ([`Caev::Tend`]/[`Caev::Exof`]/[`Caev::Conv`]) is
/// modelled here as an effect shape but sourced only through the pluggable vendor-feed adapter
/// (§2–§3 of the sourcing doc). Serialized as the ISO 4-letter code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Caev {
    /// Final redemption at maturity (mandatory). Position realised, schedule ends.
    #[serde(rename = "REDM")]
    Redm,
    /// Interest / coupon payment (mandatory). Cash income on the holding; future schedule intact.
    #[serde(rename = "INTR")]
    Intr,
    /// Full call — mandatory early redemption of the whole issue at the call price.
    #[serde(rename = "MCAL")]
    Mcal,
    /// Partial call — a pro-rata / lottery call of part of the issue at the call price.
    #[serde(rename = "PCAL")]
    Pcal,
    /// Partial redemption — pro-rata reduction of outstanding nominal (no lottery).
    #[serde(rename = "PRED")]
    Pred,
    /// Sinking-fund drawing / lottery redemption of a scheduled sinking amount.
    #[serde(rename = "DRAW")]
    Draw,
    /// Put — holder exercises a put, redeeming (all or part) at the put price.
    #[serde(rename = "BPUT")]
    Bput,
    /// Tender offer (voluntary) — holder optionally sells back at the tender price. Corporate.
    #[serde(rename = "TEND")]
    Tend,
    /// Exchange offer — the security is exchanged into a target instrument. Corporate.
    #[serde(rename = "EXOF")]
    Exof,
    /// Conversion — the security converts into a target instrument at a ratio. Corporate.
    #[serde(rename = "CONV")]
    Conv,
}

impl Caev {
    /// The 4-letter ISO code (e.g. `"REDM"`).
    #[must_use]
    pub fn code(self) -> &'static str {
        match self {
            Self::Redm => "REDM",
            Self::Intr => "INTR",
            Self::Mcal => "MCAL",
            Self::Pcal => "PCAL",
            Self::Pred => "PRED",
            Self::Draw => "DRAW",
            Self::Bput => "BPUT",
            Self::Tend => "TEND",
            Self::Exof => "EXOF",
            Self::Conv => "CONV",
        }
    }

    /// Whether this event's *data* is deterministically derivable from open govvie issuance terms
    /// (`true`) or arrives only through the pluggable commercial corporate-CA adapter (`false`).
    /// The math for both is implemented in-house; this flags the honest OSS coverage boundary (§3).
    #[must_use]
    pub fn is_govvie_derivable(self) -> bool {
        matches!(
            self,
            Self::Redm
                | Self::Intr
                | Self::Mcal
                | Self::Pcal
                | Self::Pred
                | Self::Draw
                | Self::Bput
        )
    }

    /// Whether the event reduces the whole position to zero (a full realisation) as opposed to a
    /// scaling or income event — before considering a partial [`CaTerms::redeemed_fraction`].
    #[must_use]
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Redm | Self::Exof | Self::Conv)
    }
}

/// CAMV — the mandatory / voluntary indicator that drives the election lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Camv {
    /// Mandatory — applies automatically, no holder election (REDM/INTR/PRED/DRAW).
    #[serde(rename = "MAND")]
    Mand,
    /// Voluntary — holder-optional; opens an election (TEND, a discretionary buy-back).
    #[serde(rename = "VOLU")]
    Volu,
    /// Mandatory with options / choice — must respond, choosing among outcomes (an EXOF election).
    #[serde(rename = "CHOS")]
    Chos,
}

impl Camv {
    /// Whether the event requires a desk election before it can be confirmed/applied
    /// (`true` for VOLU/CHOS; `false` for MAND which auto-applies at the entitlement dates).
    #[must_use]
    pub fn requires_election(self) -> bool {
        matches!(self, Self::Volu | Self::Chos)
    }
}

/// The lifecycle status of a corporate-action event.
///
/// Every transition is an **append** in the golden-source store (§7.3) — `Reversed`/`Cancelled`
/// supersede a prior state rather than mutating it, so a wrongly-applied event can be un-applied
/// and history stays auditable (the seev.037/seev.039 semantics).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CaStatus {
    /// Announced (MT 564 / seev.031); not yet applied.
    #[serde(rename = "announced")]
    Announced,
    /// Desk election resolved for a VOLU/CHOS event (MT 565 / seev.033).
    #[serde(rename = "elected")]
    Elected,
    /// Movement confirmed (MT 566 / seev.036); ready to apply.
    #[serde(rename = "confirmed")]
    Confirmed,
    /// Position effect applied to the book / inventory.
    #[serde(rename = "applied")]
    Applied,
    /// A confirmed movement reversed (seev.037) — the effect is un-applied.
    #[serde(rename = "reversed")]
    Reversed,
    /// An announced event cancelled before application (seev.039).
    #[serde(rename = "cancelled")]
    Cancelled,
}

impl CaStatus {
    /// Whether `next` is a legal successor of `self` in the announce→elect→confirm→apply machine
    /// (§7.5), including the reversal/cancellation escapes. Pure — the store enforces this before
    /// appending a superseding record.
    #[must_use]
    pub fn can_transition_to(self, next: Self) -> bool {
        use CaStatus::{Announced, Applied, Cancelled, Confirmed, Elected, Reversed};
        matches!(
            (self, next),
            (Announced, Elected)
                | (Announced, Confirmed)
                | (Announced, Cancelled)
                | (Elected, Confirmed)
                | (Elected, Cancelled)
                | (Confirmed, Applied)
                | (Confirmed, Cancelled)
                | (Applied, Reversed)
        )
    }
}

/// The lifecycle dates of a corporate-action event (record date → ex → deadline → payment).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaDates {
    /// When the event was announced (MT 564 / seev.031).
    pub announcement: CivilDate,
    /// The record date — the balance snapshot that fixes entitlement.
    pub record: CivilDate,
    /// The ex date — from which the security trades without the entitlement.
    pub ex: CivilDate,
    /// The response / market deadline for a VOLU/CHOS election (`None` for MAND).
    pub response_deadline: Option<CivilDate>,
    /// The payment / effective date — when the movement settles and the effect applies.
    pub payment: CivilDate,
}

/// The economic terms of a corporate-action event — the amounts/rates the effect functions read.
///
/// Amounts are expressed **per 100 units of face**, matching the schedule's per-100 convention, so
/// the effect math is a direct arithmetic combination with no unit conversion.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CaTerms {
    /// The cash price per 100 face of a redemption / call / put / tender (par = `100.0`); for
    /// [`Caev::Intr`] the coupon cash per 100 face (or `0.0` to let the schedule's own coupon
    /// stand). Unused for a pure exchange/conversion.
    pub cash_per_100: f64,
    /// The fraction of currently-outstanding nominal redeemed by a partial event
    /// (PRED / PCAL / DRAW / partial TEND / partial BPUT), in `[0, 1]`. `1.0` (or a terminal
    /// CAEV) means a full realisation.
    pub redeemed_fraction: f64,
    /// The exchange / conversion target instrument id (EXOF / CONV); empty otherwise.
    pub target_instrument: String,
    /// Units of the target created per 100 face of the source (EXOF / CONV); `0.0` otherwise.
    pub target_units_per_100: f64,
}

impl CaTerms {
    /// A full-redemption-at-par terms block (REDM / a full MCAL) — the common govvie default.
    #[must_use]
    pub fn full_at_par() -> Self {
        Self {
            cash_per_100: 100.0,
            redeemed_fraction: 1.0,
            target_instrument: String::new(),
            target_units_per_100: 0.0,
        }
    }

    /// A partial-redemption terms block: `fraction` of nominal returned at `price_per_100`.
    #[must_use]
    pub fn partial(fraction: f64, price_per_100: f64) -> Self {
        Self {
            cash_per_100: price_per_100,
            redeemed_fraction: fraction,
            target_instrument: String::new(),
            target_units_per_100: 0.0,
        }
    }

    /// A coupon-income terms block: `coupon_per_100` cash per 100 face (or `0.0` to take the
    /// schedule's own coupon).
    #[must_use]
    pub fn coupon(coupon_per_100: f64) -> Self {
        Self {
            cash_per_100: coupon_per_100,
            redeemed_fraction: 0.0,
            target_instrument: String::new(),
            target_units_per_100: 0.0,
        }
    }
}

/// A normalized corporate-action event, keyed by the instrument's ISIN.
///
/// This is the record the golden-source store journals and the lifecycle drives; the pure effect
/// functions in [`crate::effect`] consume `caev`, `terms` and the effective ([`CaDates::payment`])
/// date. Provenance (`source_ref`) carries the originating MT 564 / seev.031 message reference for
/// audit lineage (§4.3, §6.2).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CaEvent {
    /// The ISO 6166 ISIN of the affected instrument (the join key to the schedule / position).
    pub isin: String,
    /// The event type (§10.1 semantics).
    pub caev: Caev,
    /// The mandatory / voluntary indicator (drives the election lifecycle).
    pub camv: Camv,
    /// The lifecycle dates.
    pub dates: CaDates,
    /// The economic terms.
    pub terms: CaTerms,
    /// The current lifecycle status.
    pub status: CaStatus,
    /// The originating source-message reference (MT 564 / seev.031 id) for audit lineage; the
    /// ingest adapter that produced it is recorded alongside in the store's lineage metadata.
    pub source_ref: String,
}

impl CaEvent {
    /// The effective (payment / entitlement) date at which the effect applies.
    #[must_use]
    pub fn effective_date(&self) -> CivilDate {
        self.dates.payment
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn govvie_set_is_flagged_derivable_corporate_is_not() {
        assert!(Caev::Redm.is_govvie_derivable());
        assert!(Caev::Draw.is_govvie_derivable());
        assert!(!Caev::Tend.is_govvie_derivable());
        assert!(!Caev::Exof.is_govvie_derivable());
    }

    #[test]
    fn lifecycle_transitions_are_gated() {
        assert!(CaStatus::Announced.can_transition_to(CaStatus::Confirmed));
        assert!(CaStatus::Confirmed.can_transition_to(CaStatus::Applied));
        assert!(CaStatus::Applied.can_transition_to(CaStatus::Reversed));
        // Illegal jumps.
        assert!(!CaStatus::Announced.can_transition_to(CaStatus::Applied));
        assert!(!CaStatus::Applied.can_transition_to(CaStatus::Confirmed));
        assert!(!CaStatus::Cancelled.can_transition_to(CaStatus::Applied));
    }

    #[test]
    fn caev_and_camv_serialize_as_iso_codes() {
        assert_eq!(serde_json::to_string(&Caev::Mcal).unwrap(), "\"MCAL\"");
        assert_eq!(serde_json::to_string(&Camv::Volu).unwrap(), "\"VOLU\"");
    }
}
