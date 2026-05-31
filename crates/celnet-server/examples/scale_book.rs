//! Shared fixture for the out-of-process scale harness (`scale_harness`,
//! `scale_backend`).
//!
//! This module is **not** a standalone example — it is `#[path]`-included by both
//! the driver harness and the backend binary so the *one* master book, its HRW
//! partition, the canonical pricing inputs, and the reporting numeraire are defined
//! in exactly one place (guardrail #2: no duplicated fixture forks). Cargo still
//! treats it as an example target, so it gets a trivial `main` that documents its
//! role and exits; the harness/backend never run it directly.
//!
//! The master book is a small investment-bank-shaped vanilla portfolio across
//! several currency pairs and two legal entities, mixing long/short so the firm
//! VaR genuinely diversifies (a naive sum of per-shard VaRs would be wrong — the
//! federation re-gathers and re-derives once). Risk facts partition by their
//! `(entity, pair)` HRW natural owner; the pricing/surface/quote unary path
//! forwards by pair. The driver and every backend compute the **same** partition
//! from the same replica list, so the seeded union is always exactly the whole
//! book and the federated firm answer reconciles to a single-node oracle.

// This file is both a Cargo example target (with a trivial `main`) AND a `#[path]`-
// included module of `scale_harness` / `scale_backend`. In the standalone-example
// compilation its `pub` fixture items are unreachable (`unreachable_pub`) and unused
// (`dead_code`); both are expected for a shared fixture, exactly as `tests/common`
// does. Allow them here so the crate's `-D warnings` gate stays green.
#![allow(dead_code, unreachable_pub)]

use celnet_risk_cube::{
    BookId as CubeBookId, DeskId, EntityId, FactKey, FactMeasure, LocationId, PositionId, RiskFact,
    TraderId,
};
use celnet_risk_fleet::natural_owner_of;
use celnet_risk_normalize::{PositionRisk, canonicalize};
use celnet_router::{PartitionMap, Replica, ReplicaId, ReplicaSet};
use celnet_server::services::risk::store::{BookedPosition, PositionStore};
use celnet_types::{Ccy, CcyPair, DeltaConvention, OptionType, PremiumStyle, VanillaInputs};

/// The four currency pairs the fleet partitions over (a disjoint pair universe so
/// the book fans across more than one backend).
#[must_use]
pub fn pairs() -> [CcyPair; 4] {
    [
        CcyPair::new(Ccy::EUR, Ccy::USD),
        CcyPair::new(Ccy::GBP, Ccy::USD),
        CcyPair::new(Ccy::AUD, Ccy::USD),
        CcyPair::new(Ccy::USD, Ccy::JPY),
    ]
}

/// One master position: an id, a `(entity, book, pair)` placement, and its
/// economics. The `(entity, pair)` drives the HRW partition key.
#[derive(Clone, Copy)]
pub struct Master {
    /// The position id (one current fact per id; fits a `u32` cube handle).
    pub id: u64,
    /// The legal entity handle (part of the `(entity, pair)` HRW partition key).
    pub entity: u32,
    /// The book handle this leg sits in.
    pub book: u32,
    /// The currency pair this leg trades.
    pub pair: CcyPair,
    /// Call or put on the base currency.
    pub option: OptionType,
    /// Signed base-currency notional (positive = long the option).
    pub notional: f64,
}

/// The shared master book: 14 vanilla legs across 4 pairs × 2 legal entities × a
/// few books, mixing long/short.
#[must_use]
pub fn master_book() -> Vec<Master> {
    let [eurusd, gbpusd, audusd, usdjpy] = pairs();
    // (id, entity, book, pair, option, notional)
    let rows: &[(u64, u32, u32, CcyPair, OptionType, f64)] = &[
        (1, 1, 11, eurusd, OptionType::Call, 10_000_000.0),
        (2, 1, 11, eurusd, OptionType::Put, -4_000_000.0),
        (3, 1, 12, gbpusd, OptionType::Call, 7_000_000.0),
        (4, 1, 12, gbpusd, OptionType::Call, 3_000_000.0),
        (5, 1, 11, audusd, OptionType::Put, 5_000_000.0),
        (6, 2, 21, eurusd, OptionType::Call, 6_000_000.0),
        (7, 2, 21, eurusd, OptionType::Call, -2_000_000.0),
        (8, 2, 22, gbpusd, OptionType::Put, 8_000_000.0),
        (9, 2, 22, audusd, OptionType::Call, 9_000_000.0),
        (10, 2, 21, audusd, OptionType::Call, -3_000_000.0),
        (11, 1, 12, eurusd, OptionType::Call, 2_500_000.0),
        (12, 2, 22, gbpusd, OptionType::Call, 1_500_000.0),
        (13, 1, 13, usdjpy, OptionType::Call, 4_000_000.0),
        (14, 2, 23, usdjpy, OptionType::Put, -1_750_000.0),
    ];
    rows.iter()
        .map(|&(id, entity, book, pair, option, notional)| Master {
            id,
            entity,
            book,
            pair,
            option,
            notional,
        })
        .collect()
}

/// The canonical 1Y 10-vol mark every leg is booked under (the convention-free
/// canonical leaf is re-derived from these inputs server-side).
#[must_use]
pub fn inputs() -> VanillaInputs {
    VanillaInputs::new(1.10, 1.12, 0.10, 1.0, 0.04, 0.02)
}

impl Master {
    /// The `BookedPosition` for this master leg.
    #[must_use]
    pub fn booked(self) -> BookedPosition {
        BookedPosition {
            position_id: self.id,
            pair: self.pair,
            option: self.option,
            notional_base: self.notional,
            inputs: inputs(),
            quoted_delta: DeltaConvention::SpotUnadjusted,
            premium_style: PremiumStyle::DomesticPips,
            surface_version: 1,
        }
    }

    /// The firm-consistent `FactKey` for this master leg. Org handles are stable
    /// integers on every store (each store interns the same dictionary in the same
    /// order, then we address by raw handle); `desk`/`entity` are explicit so
    /// partition + grouping are unambiguous without parent-pointer resolution.
    #[must_use]
    pub fn key(self) -> FactKey {
        FactKey {
            trader: TraderId(self.entity * 100 + self.book),
            book: CubeBookId(self.book),
            desk: DeskId(self.entity * 10),
            ccy_pair: self.pair,
            location: LocationId(self.entity),
            entity: EntityId(self.entity),
        }
    }

    /// The `RiskFact` (for HRW routing at partition time).
    #[must_use]
    pub fn fact(self) -> RiskFact {
        let position = PositionRisk::new(
            self.pair,
            self.option,
            self.notional,
            inputs(),
            DeltaConvention::SpotUnadjusted,
            PremiumStyle::DomesticPips,
        );
        RiskFact {
            position_id: PositionId(u32::try_from(self.id).unwrap()),
            key: self.key(),
            measure: FactMeasure {
                leaf: canonicalize(&position),
                position,
            },
            surface_version: 1,
        }
    }
}

/// The replica set `1..=n`, all `Up` — the membership the front edge builds from
/// its backend list (replica ids are assigned in backend-list order).
#[must_use]
pub fn replica_set(n: u64) -> ReplicaSet {
    ReplicaSet::new((1..=n).map(|i| Replica::up(ReplicaId(i))).collect())
        .expect("non-empty replica set")
}

/// The HRW natural owner replica id (`1..=n`) of `m` under an `n`-replica fleet.
#[must_use]
pub fn owner_replica_of(m: Master, n: u64) -> u64 {
    let set = replica_set(n);
    let map = PartitionMap::new(&set);
    natural_owner_of(&m.fact(), &map)
        .expect("non-empty set has an owner")
        .0
}

/// Seed `store` with exactly the master legs whose HRW natural owner (under an
/// `n`-replica fleet) is `replica` — this backend's disjoint slice.
pub fn seed_slice(store: &PositionStore, replica: u64, n: u64) -> usize {
    let mut seeded = 0usize;
    for m in master_book() {
        if owner_replica_of(m, n) == replica {
            store
                .upsert(m.booked(), m.key(), None)
                .expect("upsert master leg");
            seeded += 1;
        }
    }
    seeded
}

/// The reporting numeraire (USD) with the spot rates the EUR/GBP/AUD/JPY base legs
/// need. JPY is quoted USD/JPY so a JPY leg's USD value uses `1/spot`; the cube's
/// per-ccy collapse takes the rate as units-of-numeraire per unit-ccy, so we list
/// the direct USD-per-CCY rate for each.
#[must_use]
pub fn usd_rates() -> Vec<(&'static str, f64)> {
    vec![
        ("EUR", 1.10),
        ("GBP", 1.27),
        ("AUD", 0.66),
        ("JPY", 1.0 / 150.0),
    ]
}

/// A trivial entry point so Cargo's example target compiles. The harness and the
/// backend `#[path]`-include this module; nobody runs it as a binary.
fn main() {
    eprintln!(
        "scale_book is a shared fixture module for the scale harness; \
         run `--example scale_harness` instead."
    );
}
