//! The **identifier cross-walk** — resolving one security to the identifier namespace
//! a quoting venue actually keys its panel by.
//!
//! # The asymmetry this exists to close
//!
//! A security has one identity but several identifiers. CelNet's aggregated-book panel
//! is keyed by whatever `instrument_id` an LP puts on the wire — for US Treasuries the
//! **CUSIP**, for a listed future its **contract code**, for a curated non-US govvie a
//! **slug**. The reference-data registry, meanwhile, may carry the same security under a
//! different canonical id with the CUSIP/ISIN only as cross-references. When the two
//! namespaces disagree, an exact `instrument_id` match misses, and a caller that needed
//! a named LP (an auto-hedge shedding risk) silently backstops to the synthetic
//! composite instead of filling on a real venue. The panel is healthy; the *lookup* is
//! keyed wrong.
//!
//! This module is that lookup, and nothing more. It is a pure index over identifiers —
//! it never contacts an LP, never widens what a venue quotes, and never invents a match.
//!
//! # Precedence (stable, documented, and deliberately narrow)
//!
//! [`IdentifierCrosswalk::resolve`] tries the query's identifiers in this fixed order
//! and stops at the first hit:
//!
//! 1. [`CrosswalkBasis::Exact`] — the query's `instrument_id` is itself a key in the
//!    indexed namespace. Matched **byte-for-byte**, so the resolved behaviour of an
//!    already-working lookup is unchanged.
//! 2. [`CrosswalkBasis::Cusip`] — the query's CUSIP matches an indexed row's CUSIP *or*
//!    an indexed row's `instrument_id`. CUSIP outranks ISIN because it is the narrower,
//!    issue-level identifier and is what the US wire actually carries.
//! 3. [`CrosswalkBasis::Isin`] — likewise on ISIN, the universal fallback that every
//!    curated row carries.
//!
//! Anything else is **unresolved**. There is no prefix matching, no fuzzy comparison, no
//! "closest maturity", and no substring rescue: an identifier that does not resolve
//! exactly on one of the three keys above returns `None`, and the caller keeps its
//! existing honest-miss behaviour. Matching a hedge onto an approximately-similar bond
//! is far worse than not filling it.
//!
//! # Ambiguity is a miss, not a coin-flip
//!
//! If two distinct indexed rows claim the same CUSIP or the same ISIN, that key is
//! **poisoned** and resolves to `None` (see [`Slot`]). Two rows sharing an issue-level
//! identifier is a reference-data defect; picking one of them would turn a visible data
//! problem into an invisible mis-booking. The `instrument_id` namespace cannot be
//! ambiguous — a later row with a duplicate id simply overwrites, mirroring the
//! last-writer-wins of the map it indexes.
//!
//! # Cost
//!
//! Build is O(rows) and happens once per published panel version; [`resolve`] is O(1)
//! (at most three hash probes). This sits on the auto-hedge fill path, so it must not
//! degrade into a linear scan of the quoted universe per fill.
//!
//! [`resolve`]: IdentifierCrosswalk::resolve

use std::collections::HashMap;
use std::collections::hash_map::Entry;

/// Which identifier a [`IdentifierCrosswalk::resolve`] hit was made on — recorded so a
/// caller can log/attribute *how* a security was resolved (an exact hit is the normal
/// case; a CUSIP or ISIN hit means the two namespaces disagree and is worth surfacing).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CrosswalkBasis {
    /// The `instrument_id` matched an indexed row's `instrument_id` byte-for-byte.
    Exact,
    /// The query's CUSIP matched an indexed row (on its CUSIP or its `instrument_id`).
    Cusip,
    /// The query's ISIN matched an indexed row (on its ISIN or its `instrument_id`).
    Isin,
}

impl CrosswalkBasis {
    /// A short stable label for logs / metrics.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Exact => "exact",
            Self::Cusip => "cusip",
            Self::Isin => "isin",
        }
    }
}

/// The identifiers known for ONE security, from whichever side of the cross-walk it
/// comes: an indexed row (a line the panel quotes) or a query (the id a caller holds).
///
/// `cusip` / `isin` are optional in practice — an empty string means "not known", which
/// is distinct from "known and empty" only in that an empty identifier is never indexed
/// and never resolves. Non-US curated govvies legitimately carry no CUSIP.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct IdentifierSet {
    /// The canonical id in this row's own namespace (a CUSIP, contract code, or slug).
    pub instrument_id: String,
    /// The CUSIP cross-reference, or empty when the security has none.
    pub cusip: String,
    /// The ISIN cross-reference, or empty when the security has none.
    pub isin: String,
}

impl IdentifierSet {
    /// An identifier set carrying only a canonical id (no cross-references) — the
    /// degenerate query a caller makes when it knows nothing but the wire id.
    #[must_use]
    pub fn from_id(instrument_id: impl Into<String>) -> Self {
        Self {
            instrument_id: instrument_id.into(),
            cusip: String::new(),
            isin: String::new(),
        }
    }

    /// Whether this set carries any cross-reference beyond its canonical id — i.e.
    /// whether the cross-walk has anything to try after an exact miss.
    #[must_use]
    pub fn has_cross_refs(&self) -> bool {
        !normalize(&self.cusip).is_empty() || !normalize(&self.isin).is_empty()
    }
}

/// One entry of a cross-reference index: either a unique row, or **poisoned** because
/// two distinct rows claimed the same identifier (see the module docs — ambiguity
/// resolves to `None` rather than silently picking a row).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Slot {
    Unique(usize),
    Ambiguous,
}

/// A successful cross-walk: which indexed row matched, and on which identifier.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Resolution {
    /// The zero-based position of the matched row in the sequence the index was built
    /// from — the caller indexes back into its own slice with this.
    pub row: usize,
    /// Which identifier the match was made on.
    pub basis: CrosswalkBasis,
}

/// An O(1) index over a set of [`IdentifierSet`] rows, resolving a query's identifiers
/// onto a row by the fixed precedence documented at the module level.
///
/// Build it once per version of the indexed namespace (e.g. per published panel), then
/// resolve as often as needed.
#[derive(Clone, Debug, Default)]
pub struct IdentifierCrosswalk {
    /// `instrument_id -> row`. Byte-exact; last writer wins (mirrors the source map).
    exact: HashMap<String, usize>,
    /// Normalized CUSIP -> row, poisoned on collision. Also carries any row whose
    /// `instrument_id` *is* a CUSIP (the US-Treasury wire case).
    by_cusip: HashMap<String, Slot>,
    /// Normalized ISIN -> row, poisoned on collision. Also carries any row whose
    /// `instrument_id` *is* an ISIN.
    by_isin: HashMap<String, Slot>,
}

impl IdentifierCrosswalk {
    /// Index `rows` in order; the row position becomes [`Resolution::row`].
    ///
    /// Each row contributes its `instrument_id` to the exact namespace, and its CUSIP
    /// and ISIN (when non-empty) to the respective cross-reference namespaces. A row's
    /// `instrument_id` is *additionally* registered in both cross-reference namespaces,
    /// because a venue that keys its panel by CUSIP publishes the CUSIP as the id — so a
    /// query holding only a CUSIP must find it there.
    #[must_use]
    pub fn build<I>(rows: I) -> Self
    where
        I: IntoIterator<Item = IdentifierSet>,
    {
        let mut xw = Self::default();
        for (row, ids) in rows.into_iter().enumerate() {
            let id = ids.instrument_id.trim();
            if !id.is_empty() {
                xw.exact.insert(id.to_owned(), row);
                // The canonical id may itself BE the cross-reference (US Treasuries are
                // keyed by CUSIP on the wire), so register it in both cross-ref
                // namespaces. A collision there poisons the key exactly as any other.
                let normalized_id = normalize(id);
                insert_slot(&mut xw.by_cusip, normalized_id.clone(), row);
                insert_slot(&mut xw.by_isin, normalized_id, row);
            }
            let cusip = normalize(&ids.cusip);
            if !cusip.is_empty() {
                insert_slot(&mut xw.by_cusip, cusip, row);
            }
            let isin = normalize(&ids.isin);
            if !isin.is_empty() {
                insert_slot(&mut xw.by_isin, isin, row);
            }
        }
        xw
    }

    /// How many rows are indexed in the exact namespace.
    #[must_use]
    pub fn len(&self) -> usize {
        self.exact.len()
    }

    /// Whether nothing is indexed.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.exact.is_empty()
    }

    /// Resolve `query` onto an indexed row by the documented precedence
    /// (exact `instrument_id` → CUSIP → ISIN), or `None` when no identifier the query
    /// carries resolves exactly — in which case the caller must keep its existing
    /// behaviour rather than approximate a match.
    #[must_use]
    pub fn resolve(&self, query: &IdentifierSet) -> Option<Resolution> {
        // 1. Exact — byte-for-byte, so a lookup that already worked is untouched.
        let id = query.instrument_id.trim();
        if !id.is_empty()
            && let Some(&row) = self.exact.get(id)
        {
            return Some(Resolution {
                row,
                basis: CrosswalkBasis::Exact,
            });
        }
        // 2. CUSIP — the narrower issue-level identifier, and what the US wire carries.
        let cusip = normalize(&query.cusip);
        if !cusip.is_empty()
            && let Some(Slot::Unique(row)) = self.by_cusip.get(&cusip).copied()
        {
            return Some(Resolution {
                row,
                basis: CrosswalkBasis::Cusip,
            });
        }
        // 3. ISIN — the universal fallback every curated row carries.
        let isin = normalize(&query.isin);
        if !isin.is_empty()
            && let Some(Slot::Unique(row)) = self.by_isin.get(&isin).copied()
        {
            return Some(Resolution {
                row,
                basis: CrosswalkBasis::Isin,
            });
        }
        None
    }

    /// Resolve on the canonical id alone — the degenerate query for a caller that holds
    /// no cross-references. Equivalent to `resolve(&IdentifierSet::from_id(id))`, but it
    /// also tries the id as a CUSIP/ISIN, so a caller holding a bare ISIN still finds a
    /// CUSIP-keyed row.
    #[must_use]
    pub fn resolve_id(&self, instrument_id: &str) -> Option<Resolution> {
        self.resolve(&IdentifierSet {
            instrument_id: instrument_id.to_owned(),
            cusip: instrument_id.to_owned(),
            isin: instrument_id.to_owned(),
        })
    }
}

/// Insert `row` at `key`, poisoning the slot when a DIFFERENT row already claimed it.
/// Re-inserting the same row (a spec whose `instrument_id` equals its own CUSIP) is not
/// a collision.
fn insert_slot(map: &mut HashMap<String, Slot>, key: String, row: usize) {
    match map.entry(key) {
        Entry::Vacant(v) => {
            v.insert(Slot::Unique(row));
        }
        Entry::Occupied(mut o) => {
            if o.get() != &Slot::Unique(row) {
                o.insert(Slot::Ambiguous);
            }
        }
    }
}

/// Normalize a cross-reference identifier for keying: trim surrounding whitespace and
/// upper-case ASCII. CUSIPs and ISINs are defined over `[0-9A-Z]`, so case is not
/// meaningful and a feed that lower-cases one must still resolve. Nothing else is
/// altered — no punctuation stripping, no check-digit repair, no length coercion.
fn normalize(id: &str) -> String {
    id.trim().to_ascii_uppercase()
}

/// The curated-universe identifiers for `instrument_id`, resolved through
/// [`crate::spec_by_id`] / [`crate::future_by_id`] — the aliases a caller can attach to
/// a query when it holds nothing but a wire id and the registry has no entry.
///
/// Returns `None` for an id outside the curated universes, so the caller learns
/// "unknown security" rather than receiving an empty alias set that silently resolves
/// nothing.
#[must_use]
pub fn curated_identifiers(instrument_id: &str) -> Option<IdentifierSet> {
    if let Some(spec) = crate::spec_by_id(instrument_id) {
        return Some(IdentifierSet {
            instrument_id: spec.instrument_id.clone(),
            cusip: spec.cusip.clone().unwrap_or_default(),
            isin: spec.isin.clone(),
        });
    }
    // A listed future's contract code is its own market identifier; it carries no
    // CUSIP/ISIN, so the set is exact-only by construction rather than by omission.
    crate::future_by_id(instrument_id).map(|f| IdentifierSet::from_id(f.instrument_id.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A three-row panel shaped like the real one: two US Treasuries keyed by CUSIP
    /// (ISIN as a cross-ref) and one curated gilt keyed by a slug (ISIN only, no CUSIP).
    fn panel() -> Vec<IdentifierSet> {
        vec![
            IdentifierSet {
                instrument_id: "912797UU9".to_owned(),
                cusip: "912797UU9".to_owned(),
                isin: "US912797UU95".to_owned(),
            },
            IdentifierSet {
                instrument_id: "912828XX1".to_owned(),
                cusip: "912828XX1".to_owned(),
                isin: "US912828XX10".to_owned(),
            },
            IdentifierSet {
                instrument_id: "uk-gilt-5y-2031".to_owned(),
                cusip: String::new(),
                isin: "GB00GILT5312".to_owned(),
            },
        ]
    }

    #[test]
    fn exact_instrument_id_resolves_first() {
        let xw = IdentifierCrosswalk::build(panel());
        let r = xw
            .resolve(&IdentifierSet::from_id("912828XX1"))
            .expect("exact id resolves");
        assert_eq!(r.row, 1);
        assert_eq!(r.basis, CrosswalkBasis::Exact);
    }

    #[test]
    fn exact_outranks_cusip_and_isin_when_they_disagree() {
        // A pathological query whose id names row 0 but whose cross-refs name row 1:
        // precedence must pick the exact id, deterministically.
        let xw = IdentifierCrosswalk::build(panel());
        let r = xw
            .resolve(&IdentifierSet {
                instrument_id: "912797UU9".to_owned(),
                cusip: "912828XX1".to_owned(),
                isin: "US912828XX10".to_owned(),
            })
            .expect("resolves");
        assert_eq!(r.row, 0, "the exact instrument_id wins");
        assert_eq!(r.basis, CrosswalkBasis::Exact);
    }

    #[test]
    fn registry_slug_resolves_onto_a_cusip_keyed_panel_by_cusip() {
        // THE DEFECT: the registry knows this security as a slug; the panel keys it by
        // CUSIP. The exact match misses and the CUSIP cross-ref rescues it.
        let xw = IdentifierCrosswalk::build(panel());
        let query = IdentifierSet {
            instrument_id: "ust-10y-benchmark".to_owned(),
            cusip: "912828XX1".to_owned(),
            isin: "US912828XX10".to_owned(),
        };
        let r = xw.resolve(&query).expect("cross-walks by CUSIP");
        assert_eq!(r.row, 1);
        assert_eq!(
            r.basis,
            CrosswalkBasis::Cusip,
            "CUSIP outranks ISIN when both would hit"
        );
    }

    #[test]
    fn isin_resolves_when_no_cusip_exists() {
        // A non-US govvie carries no CUSIP at all, so ISIN is the only cross-ref.
        let xw = IdentifierCrosswalk::build(panel());
        let query = IdentifierSet {
            instrument_id: "gilt-5y-on-the-run".to_owned(),
            cusip: String::new(),
            isin: "GB00GILT5312".to_owned(),
        };
        let r = xw.resolve(&query).expect("cross-walks by ISIN");
        assert_eq!(r.row, 2);
        assert_eq!(r.basis, CrosswalkBasis::Isin);
    }

    #[test]
    fn cross_ref_matching_is_case_insensitive_but_never_fuzzy() {
        let xw = IdentifierCrosswalk::build(panel());
        let lowered = IdentifierSet {
            instrument_id: "whatever".to_owned(),
            cusip: "  912828xx1 ".to_owned(),
            isin: String::new(),
        };
        assert_eq!(
            xw.resolve(&lowered).expect("case/space insensitive").row,
            1
        );
        // A prefix of a real CUSIP is NOT a match.
        let prefix = IdentifierSet {
            instrument_id: "whatever".to_owned(),
            cusip: "912828XX".to_owned(),
            isin: String::new(),
        };
        assert!(xw.resolve(&prefix).is_none(), "no prefix matching");
    }

    #[test]
    fn unresolvable_id_stays_unresolved() {
        // The retired demo-seed shape: an id in nobody's namespace, whose ISIN exists
        // in no quoted row. It must resolve to NOTHING so the caller falls back
        // honestly rather than filling on an approximately-similar bond.
        let xw = IdentifierCrosswalk::build(panel());
        let query = IdentifierSet {
            instrument_id: "ust-2y-note".to_owned(),
            cusip: String::new(),
            isin: "US91282CKM23".to_owned(),
        };
        assert!(xw.resolve(&query).is_none());
        assert!(xw.resolve(&IdentifierSet::from_id("acme-5y-corp")).is_none());
        assert!(xw.resolve(&IdentifierSet::default()).is_none());
    }

    #[test]
    fn a_duplicated_cross_ref_poisons_the_key_instead_of_guessing() {
        // Two distinct rows claiming one ISIN is a data defect. Resolving it to either
        // row would hide the defect behind a plausible fill.
        let rows = vec![
            IdentifierSet {
                instrument_id: "AAA".to_owned(),
                cusip: String::new(),
                isin: "US0000000AA1".to_owned(),
            },
            IdentifierSet {
                instrument_id: "BBB".to_owned(),
                cusip: String::new(),
                isin: "US0000000AA1".to_owned(),
            },
        ];
        let xw = IdentifierCrosswalk::build(rows);
        let ambiguous = IdentifierSet {
            instrument_id: "query".to_owned(),
            cusip: String::new(),
            isin: "US0000000AA1".to_owned(),
        };
        assert!(
            xw.resolve(&ambiguous).is_none(),
            "an ambiguous cross-ref must not resolve"
        );
        // …but each row is still reachable by its own unambiguous canonical id.
        assert_eq!(xw.resolve_id("AAA").expect("exact still works").row, 0);
        assert_eq!(xw.resolve_id("BBB").expect("exact still works").row, 1);
    }

    #[test]
    fn a_row_whose_id_equals_its_own_cusip_is_not_a_collision() {
        // Every US Treasury row is this shape; it must not poison its own key.
        let xw = IdentifierCrosswalk::build(panel());
        let by_cusip_only = IdentifierSet {
            instrument_id: "unknown-slug".to_owned(),
            cusip: "912797UU9".to_owned(),
            isin: String::new(),
        };
        assert_eq!(xw.resolve(&by_cusip_only).expect("resolves").row, 0);
    }

    #[test]
    fn resolve_id_finds_a_cusip_keyed_row_from_a_bare_isin() {
        let xw = IdentifierCrosswalk::build(panel());
        let r = xw.resolve_id("US912797UU95").expect("bare ISIN resolves");
        assert_eq!(r.row, 0);
        assert_eq!(r.basis, CrosswalkBasis::Isin);
    }

    #[test]
    fn curated_identifiers_expose_the_quoted_universe_cross_refs() {
        let spec = crate::government_universe()
            .into_iter()
            .find(|s| s.cusip.is_some())
            .expect("the curated universe has US rows");
        let ids = curated_identifiers(&spec.instrument_id).expect("curated id resolves");
        assert_eq!(ids.instrument_id, spec.instrument_id);
        assert_eq!(ids.isin, spec.isin);
        assert_eq!(ids.cusip, spec.cusip.clone().unwrap_or_default());
        // …and its ISIN cross-walks back onto the same row.
        let xw = IdentifierCrosswalk::build(
            crate::government_universe()
                .into_iter()
                .map(|s| IdentifierSet {
                    instrument_id: s.instrument_id,
                    cusip: s.cusip.unwrap_or_default(),
                    isin: s.isin,
                }),
        );
        let by_isin = xw
            .resolve(&IdentifierSet {
                instrument_id: "not-the-wire-id".to_owned(),
                cusip: String::new(),
                isin: spec.isin.clone(),
            })
            .expect("ISIN cross-walks into the quoted universe");
        assert_eq!(by_isin.basis, CrosswalkBasis::Isin);
        assert!(curated_identifiers("definitely-not-a-security").is_none());
    }

    #[test]
    fn the_whole_curated_universe_is_unambiguous_under_the_crosswalk() {
        // If the shipped universe poisoned its own keys, the cross-walk would silently
        // stop resolving real bonds — assert the data supports the mechanism.
        let universe = crate::government_universe();
        let xw = IdentifierCrosswalk::build(universe.iter().map(|s| IdentifierSet {
            instrument_id: s.instrument_id.clone(),
            cusip: s.cusip.clone().unwrap_or_default(),
            isin: s.isin.clone(),
        }));
        for (row, spec) in universe.iter().enumerate() {
            let by_isin = xw
                .resolve(&IdentifierSet {
                    instrument_id: format!("alias-of-{row}"),
                    cusip: String::new(),
                    isin: spec.isin.clone(),
                })
                .unwrap_or_else(|| panic!("{} must resolve by ISIN", spec.instrument_id));
            assert_eq!(by_isin.row, row, "{} resolved to the wrong row", spec.isin);
        }
    }
}
