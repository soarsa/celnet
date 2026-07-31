//! The effective-dated, append-only, journal-backed golden-source store (§7.3).
//!
//! Every write is an **append** carrying its [`Provenance`] `(source, valid_from, recorded_at,
//! quality)`, durable via `celnet-journal` (fsync'd, torn-tail-safe recovery). Reads resolve the
//! **effective** version for a `(valuation, known-as-of)` pair — bitemporal — so a value's state
//! "as known on X, effective on Y" is reconstructable. A corporate-action reversal (seev.037) or
//! cancellation (seev.039) appends a **superseding** version rather than deleting, so a wrongly
//! applied redemption can be un-applied and history stays auditable (§4.3, §10.2).

use celnet_corpactions::{CaEvent, CivilDate};
use celnet_journal::{Journal, RecordKind};
use serde::{Deserialize, Serialize};

use crate::master::{InstrumentMaster, Provenance};

/// A corporate-action event as stored, keyed by a stable `ca_id` (the supersession key) and carrying
/// mastering provenance. A lifecycle transition (announce → confirm → apply → reversed/cancelled)
/// appends a new version with the same `ca_id`; the latest version is the current state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StoredCorpAction {
    /// The stable id of the corporate action (shared across its lifecycle versions).
    pub ca_id: String,
    /// The normalized event (its `status` is the state of *this* version).
    pub event: CaEvent,
    /// The mastering metadata for this version.
    pub provenance: Provenance,
}

/// The append-only journal event: a new instrument-master version or a corporate-action version.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
enum StoreEvent {
    Instrument(InstrumentMaster),
    CorpAction(StoredCorpAction),
}

/// Failure modes of the golden-source store.
#[derive(Debug)]
pub enum StoreError {
    /// The durable journal failed (open / append / recovery).
    Journal(celnet_journal::JournalError),
    /// A journalled record could not be decoded (a corrupt or contract-drifted payload).
    Decode(serde_json::Error),
}

impl core::fmt::Display for StoreError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Journal(e) => write!(f, "golden-source journal error: {e}"),
            Self::Decode(e) => write!(f, "golden-source record decode error: {e}"),
        }
    }
}

impl core::error::Error for StoreError {}

impl From<celnet_journal::JournalError> for StoreError {
    fn from(e: celnet_journal::JournalError) -> Self {
        Self::Journal(e)
    }
}

impl From<serde_json::Error> for StoreError {
    fn from(e: serde_json::Error) -> Self {
        Self::Decode(e)
    }
}

/// The mutable evolution of `celnet-refdata`'s static `Vec` into a mastered golden source.
///
/// Holds the full append-order history of every instrument-master and corporate-action version in
/// memory (rebuilt from the journal on [`GoldenSourceStore::open`]) and mirrors each write to the
/// durable log. The projections are `Vec`s in append order, so append-order *is* recorded order and
/// bitemporal resolution is a filtered scan.
#[derive(Debug)]
pub struct GoldenSourceStore {
    journal: Journal,
    masters: Vec<InstrumentMaster>,
    corp_actions: Vec<StoredCorpAction>,
}

impl GoldenSourceStore {
    /// Open (creating if absent) the store backed by the journal at `path`, rebuilding the in-memory
    /// projections by replaying the durable log.
    ///
    /// # Errors
    /// [`StoreError::Journal`] on an IO / recovery failure; [`StoreError::Decode`] if a journalled
    /// payload is not a valid record.
    pub fn open(path: impl AsRef<std::path::Path>) -> Result<Self, StoreError> {
        let journal = Journal::open(path)?;
        let mut masters = Vec::new();
        let mut corp_actions = Vec::new();
        for record in journal.records()? {
            if record.kind != RecordKind::Data {
                continue; // a compaction snapshot carries no domain event here
            }
            match serde_json::from_slice::<StoreEvent>(&record.payload)? {
                StoreEvent::Instrument(m) => masters.push(m),
                StoreEvent::CorpAction(c) => corp_actions.push(c),
            }
        }
        Ok(Self {
            journal,
            masters,
            corp_actions,
        })
    }

    /// Append a new effective-dated instrument-master version, durably.
    ///
    /// # Errors
    /// [`StoreError::Journal`] on an IO failure.
    pub fn upsert_instrument(&mut self, master: InstrumentMaster) -> Result<(), StoreError> {
        let payload = serde_json::to_vec(&StoreEvent::Instrument(master.clone()))?;
        self.journal.append(&payload)?;
        self.masters.push(master);
        Ok(())
    }

    /// Append a new corporate-action version (announce / status transition / reversal), durably.
    ///
    /// # Errors
    /// [`StoreError::Journal`] on an IO failure.
    pub fn record_corp_action(&mut self, ca: StoredCorpAction) -> Result<(), StoreError> {
        let payload = serde_json::to_vec(&StoreEvent::CorpAction(ca.clone()))?;
        self.journal.append(&payload)?;
        self.corp_actions.push(ca);
        Ok(())
    }

    /// The version of `instrument_id` **effective** for a `valuation` date and **known** as of
    /// `known_as_of` (bitemporal read). Among versions with `valid_from <= valuation` and
    /// `recorded_at <= known_as_of`, the latest-effective-then-latest-recorded wins.
    #[must_use]
    pub fn effective_instrument(
        &self,
        instrument_id: &str,
        valuation: CivilDate,
        known_as_of: CivilDate,
    ) -> Option<&InstrumentMaster> {
        self.masters
            .iter()
            .enumerate()
            .filter(|(_, m)| {
                m.instrument_id == instrument_id
                    && m.provenance.valid_from <= valuation
                    && m.provenance.recorded_at <= known_as_of
            })
            .max_by(|(ia, a), (ib, b)| {
                a.provenance
                    .valid_from
                    .cmp(&b.provenance.valid_from)
                    .then(a.provenance.recorded_at.cmp(&b.provenance.recorded_at))
                    .then(ia.cmp(ib))
            })
            .map(|(_, m)| m)
    }

    /// The most recently appended version of `instrument_id` (current known state).
    #[must_use]
    pub fn latest_instrument(&self, instrument_id: &str) -> Option<&InstrumentMaster> {
        self.masters
            .iter()
            .rev()
            .find(|m| m.instrument_id == instrument_id)
    }

    /// The most recently appended version whose carried ISIN is `isin` (the CA join: events are
    /// keyed by ISIN, the store by `instrument_id`).
    #[must_use]
    pub fn latest_instrument_by_isin(&self, isin: &str) -> Option<&InstrumentMaster> {
        self.masters.iter().rev().find(|m| m.isin() == Some(isin))
    }

    /// Every appended version of `instrument_id`, in append (recorded) order — the audit history a
    /// reversal walks to restore the pre-event schedule.
    #[must_use]
    pub fn instrument_versions(&self, instrument_id: &str) -> Vec<&InstrumentMaster> {
        self.masters
            .iter()
            .filter(|m| m.instrument_id == instrument_id)
            .collect()
    }

    /// Every distinct instrument id in the store, in first-seen order.
    #[must_use]
    pub fn instrument_ids(&self) -> Vec<String> {
        let mut seen: Vec<String> = Vec::new();
        for m in &self.masters {
            if !seen.contains(&m.instrument_id) {
                seen.push(m.instrument_id.clone());
            }
        }
        seen
    }

    /// The full append-order lifecycle history of one corporate action.
    #[must_use]
    pub fn corp_action_history(&self, ca_id: &str) -> Vec<&StoredCorpAction> {
        self.corp_actions
            .iter()
            .filter(|c| c.ca_id == ca_id)
            .collect()
    }

    /// The current (latest-appended) version of one corporate action.
    #[must_use]
    pub fn latest_corp_action(&self, ca_id: &str) -> Option<&StoredCorpAction> {
        self.corp_actions.iter().rev().find(|c| c.ca_id == ca_id)
    }

    /// The current version of every corporate action whose event targets `isin`, one per `ca_id`.
    #[must_use]
    pub fn corp_actions_for_isin(&self, isin: &str) -> Vec<&StoredCorpAction> {
        let mut ids: Vec<&str> = Vec::new();
        for c in &self.corp_actions {
            if c.event.isin == isin && !ids.contains(&c.ca_id.as_str()) {
                ids.push(c.ca_id.as_str());
            }
        }
        ids.into_iter()
            .filter_map(|id| self.latest_corp_action(id))
            .collect()
    }

    /// The current (latest-appended) version of **every** corporate action, one per
    /// distinct `ca_id`, in first-seen order — the CA-inbox read across the whole store.
    #[must_use]
    pub fn current_corp_actions(&self) -> Vec<&StoredCorpAction> {
        let mut ids: Vec<&str> = Vec::new();
        for c in &self.corp_actions {
            if !ids.contains(&c.ca_id.as_str()) {
                ids.push(c.ca_id.as_str());
            }
        }
        ids.into_iter()
            .filter_map(|id| self.latest_corp_action(id))
            .collect()
    }

    /// The durable sequence of the most recent append (for checkpoint watermarking / diagnostics).
    #[must_use]
    pub fn last_sequence(&self) -> Option<u64> {
        self.journal.last_sequence()
    }
}
