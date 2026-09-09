//! ISDA Common Domain Model (CDM) 2026 alignment.
//!
//! Provides the canonical digital trade lifecycle event model, contract terms,
//! and regulatory event lineage conforming to the ISDA CDM 2026 specification.
//!
//! Supports seamless interoperability with institutional post-trade networks,
//! central counterparties (CCPs), regulatory reporting repositories, and
//! digital workflow automation.

#![forbid(unsafe_code)]

use crate::{BrokenDate, Ccy, OptionType, Underlying};
use serde::{Deserialize, Serialize};

/// Role played by a party in an ISDA CDM trade lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CdmPartyRole {
    /// The executing financial entity (dealer / market maker).
    ExecutingEntity,
    /// The direct counterparty (client / price taker).
    Counterparty,
    /// Clearing member broker.
    ClearingBroker,
    /// Central Counterparty (CCP) clearinghouse.
    ClearingHouse,
    /// Calculation agent for fixing and settlement determinations.
    CalculationAgent,
    /// Custodian holding underlying collateral or assets.
    Custodian,
    /// Prime broker sponsoring the trade.
    PrimeBroker,
}

/// An entity participating in an ISDA CDM transaction.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CdmParty {
    /// Legal Entity Identifier (LEI) or internal account id.
    pub party_id: String,
    /// Human-readable entity name.
    pub name: Option<String>,
    /// Capacity / role of the party in the transaction.
    pub role: CdmPartyRole,
}

impl CdmParty {
    /// Convenience constructor for an identified party.
    #[must_use]
    pub fn new(party_id: impl Into<String>, role: CdmPartyRole) -> Self {
        Self {
            party_id: party_id.into(),
            name: None,
            role,
        }
    }

    /// Attach a human-readable legal entity name.
    #[must_use]
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }
}

/// Globally unique trade identification following regulatory UPI/UTI standards.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CdmTradeIdentifier {
    /// LEI of the entity issuing the identifier.
    pub issuer_lei: String,
    /// Unique trade identifier issued by the venue or maker.
    pub assigned_trade_id: String,
    /// Regulatory Unique Trade Identifier (UTI / USI) where assigned.
    pub uti: Option<String>,
}

impl CdmTradeIdentifier {
    /// Create a new trade identifier.
    #[must_use]
    pub fn new(issuer_lei: impl Into<String>, trade_id: impl Into<String>) -> Self {
        Self {
            issuer_lei: issuer_lei.into(),
            assigned_trade_id: trade_id.into(),
            uti: None,
        }
    }

    /// Attach a regulatory UTI.
    #[must_use]
    pub fn with_uti(mut self, uti: impl Into<String>) -> Self {
        self.uti = Some(uti.into());
        self
    }
}

/// Option exercise style.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CdmExerciseStyle {
    /// Exercise only at maturity horizon.
    European,
    /// Exercise at any business day up to maturity.
    American,
    /// Exercise on predefined discrete exercise windows.
    Bermudan,
}

/// Settlement delivery mechanism.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CdmSettlementType {
    /// Physical delivery of the underlying asset.
    Physical,
    /// Cash settlement in the settlement currency.
    Cash,
}

/// Option payout specification in ISDA CDM format.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CdmOptionPayout {
    /// Call or Put.
    pub option_type: OptionType,
    /// Strike price level.
    pub strike: f64,
    /// Exercise style.
    pub exercise_style: CdmExerciseStyle,
    /// Cash or physical settlement.
    pub settlement_type: CdmSettlementType,
    /// Premium amount paid by the buyer.
    pub premium: f64,
    /// Currency of the premium payment.
    pub premium_ccy: Ccy,
}

/// Interest rate payout leg specification.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CdmInterestRatePayout {
    /// Whether the leg is fixed rate or floating index.
    pub is_fixed: bool,
    /// Fixed rate (e.g. 0.042 = 4.2%) or spread over floating index.
    pub rate_or_spread: f64,
    /// Day count convention identifier (e.g. "ACT/360", "30/360").
    pub day_count: String,
    /// Payment frequency in months.
    pub payment_frequency_months: u32,
    /// Floating rate index name (e.g. "USD-SOFR-OISCompound") if floating.
    pub floating_index: Option<String>,
}

/// Forward payout leg specification.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CdmForwardPayout {
    /// Agreed outright forward exchange rate or price.
    pub forward_price: f64,
    /// Fixing date for non-deliverable forwards (NDFs).
    pub fixing_date: Option<BrokenDate>,
    /// Settlement delivery date.
    pub settlement_date: BrokenDate,
}

/// Economic payout mechanism for an ISDA CDM product.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum CdmPayout {
    /// Vanilla or exotic option payout.
    Option(CdmOptionPayout),
    /// Single or multi-leg interest rate swap payout.
    InterestRate(CdmInterestRatePayout),
    /// Forward or NDF outright payout.
    Forward(CdmForwardPayout),
}

/// Full product definition under ISDA CDM 2026.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CdmProduct {
    /// The underlying asset (cross-asset: FX, metals, equities, commodities, crypto).
    pub underlying: Underlying,
    /// Total contract notional amount.
    pub notional: f64,
    /// Currency of the principal notional.
    pub notional_ccy: Ccy,
    /// Effective inception date of the trade.
    pub effective_date: BrokenDate,
    /// Expiration / maturity termination date.
    pub termination_date: BrokenDate,
    /// Economic payout terms.
    pub payout: CdmPayout,
}

impl CdmProduct {
    /// Validate structural invariants on product terms.
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.notional <= 0.0 {
            return Err("notional must be strictly positive");
        }
        let eff = (
            self.effective_date.year,
            self.effective_date.month,
            self.effective_date.day,
        );
        let term = (
            self.termination_date.year,
            self.termination_date.month,
            self.termination_date.day,
        );
        if eff > term {
            return Err("effective_date cannot be strictly after termination_date");
        }
        Ok(())
    }
}

/// Regulatory and operational lifecycle event types under ISDA CDM 2026.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CdmLifecycleEventType {
    /// Trade execution / trade capture on trading venue or bilateral RFQ.
    Execution,
    /// Post-trade allocation of block execution to specific funds/sub-accounts.
    Allocation,
    /// Bilateral affirmation of preliminary trade economics.
    Affirmation,
    /// Legally binding confirmation under master agreement.
    Confirmation,
    /// Novation to a Central Counterparty (CCP) clearinghouse.
    Clearing,
    /// Cash settlement or physical delivery execution.
    Settlement,
    /// Periodic index reset or fixing observation.
    RateReset,
    /// Option exercise notification and payoff triggering.
    Exercise,
    /// Legal novation (substitution of a contracting party).
    Novation,
    /// Early trade termination, cancellation, or full unwinding.
    Termination,
}

/// An immutable, verifiable ISDA CDM lifecycle event with audit lineage.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CdmLifecycleEvent {
    /// Unique event identifier.
    pub event_id: String,
    /// Type of lifecycle transition.
    pub event_type: CdmLifecycleEventType,
    /// Timestamp of event occurrence in epoch nanoseconds.
    pub timestamp_nanos: u64,
    /// Prior event identifier in the audit lineage DAG, if any.
    pub lineage_event_id: Option<String>,
    /// Trade identifier.
    pub trade_id: CdmTradeIdentifier,
    /// Associated product economics.
    pub product: CdmProduct,
    /// Participating legal parties and their roles.
    pub parties: Vec<CdmParty>,
}

impl CdmLifecycleEvent {
    /// Construct a new trade execution event.
    #[must_use]
    pub fn new_execution(
        event_id: impl Into<String>,
        timestamp_nanos: u64,
        trade_id: CdmTradeIdentifier,
        product: CdmProduct,
        parties: Vec<CdmParty>,
    ) -> Self {
        Self {
            event_id: event_id.into(),
            event_type: CdmLifecycleEventType::Execution,
            timestamp_nanos,
            lineage_event_id: None,
            trade_id,
            product,
            parties,
        }
    }

    /// Advance this event to a downstream lifecycle state (e.g. Execution -> Confirmation).
    #[must_use]
    pub fn transition_to(
        &self,
        new_event_id: impl Into<String>,
        new_event_type: CdmLifecycleEventType,
        timestamp_nanos: u64,
    ) -> Self {
        Self {
            event_id: new_event_id.into(),
            event_type: new_event_type,
            timestamp_nanos,
            lineage_event_id: Some(self.event_id.clone()),
            trade_id: self.trade_id.clone(),
            product: self.product.clone(),
            parties: self.parties.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CcyPair;

    #[test]
    fn cdm_event_lineage_preservation() {
        let trade_id = CdmTradeIdentifier::new("5493000XYZLEI12345", "TRD-2026-001")
            .with_uti("UTI-9876543210");

        let parties = vec![
            CdmParty::new("DEALER_LEI", CdmPartyRole::ExecutingEntity),
            CdmParty::new("CLIENT_LEI", CdmPartyRole::Counterparty),
        ];

        let product = CdmProduct {
            underlying: Underlying::Fx(CcyPair::parse("EURUSD").unwrap()),
            notional: 10_000_000.0,
            notional_ccy: Ccy::EUR,
            effective_date: BrokenDate::new(2026, 9, 7),
            termination_date: BrokenDate::new(2026, 12, 7),
            payout: CdmPayout::Option(CdmOptionPayout {
                option_type: OptionType::Call,
                strike: 1.1000,
                exercise_style: CdmExerciseStyle::European,
                settlement_type: CdmSettlementType::Cash,
                premium: 125_000.0,
                premium_ccy: Ccy::USD,
            }),
        };

        assert!(product.validate().is_ok());

        let exec = CdmLifecycleEvent::new_execution(
            "EVT-001",
            1_788_000_000_000_000_000,
            trade_id,
            product,
            parties,
        );

        assert_eq!(exec.event_type, CdmLifecycleEventType::Execution);
        assert!(exec.lineage_event_id.is_none());

        let conf = exec.transition_to("EVT-002", CdmLifecycleEventType::Confirmation, 1_788_000_000_500_000_000);
        assert_eq!(conf.event_type, CdmLifecycleEventType::Confirmation);
        assert_eq!(conf.lineage_event_id.as_deref(), Some("EVT-001"));
        assert_eq!(conf.product.notional, 10_000_000.0);
    }
}
