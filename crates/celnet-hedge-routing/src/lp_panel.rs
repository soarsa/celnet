//! The **hedging LP panel** — the include/exclude liquidity-provider selection an
//! external exit action fans to, and the pure resolver that turns it into an
//! **effective LP set** against the live known-LP registry.
//!
//! Where an `RfqOut` leaf carries a *per-rule, include-only* LP list (kept working
//! for back-compat), a [`HedgeLpPanel`] is the **standing, scope-level** selection a
//! desk maintains once and every external action inherits: `SUBMIT_MARKET_ORDER`
//! (which order to route to), `RFQ_OUT` (which panel to fan to), and the external
//! leg of `SPLIT`. It generalises the include-only list with **exclude** semantics —
//! "hedge on all LPs *except* X" — the gap the hedging configuration guide flagged
//! (`docs/hedging/HEDGING-CONFIGURATION-GUIDE.md` §4;
//! `docs/hedging/AUTO-HEDGING-AND-INTERNALISATION-REQUIREMENTS.md` §4/§6.2).
//!
//! # Resolution ([`HedgeLpPanel::effective_lps`])
//!
//! ```text
//! base      = include.is_empty() ? all-known-LPs : include        (in include order)
//! effective = base − exclude                                       (dedup, order-preserving)
//! ```
//!
//! - **empty = all**: an empty include *and* empty exclude resolves to every known LP
//!   (the default panel — no restriction).
//! - **include-only**: the resolved set is exactly the named includes (minus any that
//!   also appear in exclude).
//! - **exclude-only**: start from all known LPs and subtract the excludes.
//! - **include + exclude**: the excludes win — an id in both is removed.
//! - **unknown-id rejected**: every id named in `include` or `exclude` must be a member
//!   of the known-LP set, else resolution fails, collecting **every** unknown id at once
//!   (the same validate-collects-all discipline as [`crate::HedgeGraph::validate`]).
//!
//! The resolver is pure: no I/O, deterministic order (known LPs iterate in the sorted
//! `BTreeSet` order; explicit includes keep the trader's order so an RFQ fan honours
//! their priority). The `known_lps` set is supplied by the caller from the live LP
//! registry (the aggregation hub / FIX LP sessions) — off the pinned pricing core.

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fmt;

/// A standing include/exclude liquidity-provider selection for hedging.
///
/// Both lists are empty by default — the unrestricted "all known LPs" panel. A desk
/// binds one of these to a scope (desk / book / instrument) server-side; the pure
/// resolver here is scope-agnostic.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HedgeLpPanel {
    /// LPs to start from. **Empty ⇒ start from every known LP.** A non-empty include
    /// restricts the base set to exactly these (in this order — the RFQ fan priority).
    #[serde(default)]
    pub include: Vec<String>,
    /// LPs to subtract from the base set (the exclude semantics). An id here is removed
    /// even if it also appears in `include`.
    #[serde(default)]
    pub exclude: Vec<String>,
}

/// A defect in an [`HedgeLpPanel`] resolved against a known-LP set.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LpPanelError {
    /// An `include` id is not a member of the known-LP registry.
    UnknownInclude {
        /// The unknown LP id.
        lp: String,
    },
    /// An `exclude` id is not a member of the known-LP registry.
    UnknownExclude {
        /// The unknown LP id.
        lp: String,
    },
    /// The panel resolves to **no** LPs (an include/exclude combination that removes
    /// everything, or an all-exclude against the full known set) — an external action
    /// could never route.
    EmptyEffectiveSet,
}

impl fmt::Display for LpPanelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LpPanelError::UnknownInclude { lp } => {
                write!(f, "hedge LP panel includes unknown LP {lp:?}")
            }
            LpPanelError::UnknownExclude { lp } => {
                write!(f, "hedge LP panel excludes unknown LP {lp:?}")
            }
            LpPanelError::EmptyEffectiveSet => {
                write!(f, "hedge LP panel resolves to no LPs (all excluded)")
            }
        }
    }
}

impl std::error::Error for LpPanelError {}

impl HedgeLpPanel {
    /// An unrestricted panel (empty include + exclude ⇒ all known LPs).
    #[must_use]
    pub fn all() -> Self {
        Self::default()
    }

    /// An include-only panel over `lps` (the back-compat equivalent of a per-rule
    /// `RfqOut` include list, promoted to a standing panel).
    #[must_use]
    pub fn include_only(lps: impl IntoIterator<Item = String>) -> Self {
        Self {
            include: lps.into_iter().collect(),
            exclude: Vec::new(),
        }
    }

    /// Whether this panel names no restriction at all (empty include *and* exclude) —
    /// it resolves to every known LP.
    #[must_use]
    pub fn is_unrestricted(&self) -> bool {
        self.include.is_empty() && self.exclude.is_empty()
    }

    /// Validate every named id against `known_lps`, collecting **all** unknown-id
    /// defects at once (never stopping at the first). Does **not** check emptiness of
    /// the resolved set — that is [`Self::effective_lps`]'s concern.
    ///
    /// # Errors
    /// Every `include` / `exclude` id absent from `known_lps`, as a `Vec` of
    /// [`LpPanelError::UnknownInclude`] / [`LpPanelError::UnknownExclude`].
    pub fn validate(&self, known_lps: &BTreeSet<String>) -> Result<(), Vec<LpPanelError>> {
        let mut errs = Vec::new();
        for lp in &self.include {
            if !known_lps.contains(lp) {
                errs.push(LpPanelError::UnknownInclude { lp: lp.clone() });
            }
        }
        for lp in &self.exclude {
            if !known_lps.contains(lp) {
                errs.push(LpPanelError::UnknownExclude { lp: lp.clone() });
            }
        }
        if errs.is_empty() { Ok(()) } else { Err(errs) }
    }

    /// Resolve this panel against `known_lps` into the **effective LP set** an external
    /// action fans to (`base − exclude`, `base` = the includes or, when empty, every
    /// known LP). Deterministic order: explicit includes keep the trader's order;
    /// the all-known base iterates the sorted `BTreeSet`. Duplicates are removed.
    ///
    /// # Errors
    /// [`LpPanelError::UnknownInclude`] / [`LpPanelError::UnknownExclude`] for any
    /// named id absent from `known_lps` (all collected at once), or
    /// [`LpPanelError::EmptyEffectiveSet`] when the include/exclude combination leaves
    /// no LP to route to.
    pub fn effective_lps(
        &self,
        known_lps: &BTreeSet<String>,
    ) -> Result<Vec<String>, Vec<LpPanelError>> {
        self.validate(known_lps)?;

        let excluded: BTreeSet<&String> = self.exclude.iter().collect();
        let base: Vec<&String> = if self.include.is_empty() {
            known_lps.iter().collect()
        } else {
            self.include.iter().collect()
        };

        let mut seen = BTreeSet::new();
        let mut out = Vec::with_capacity(base.len());
        for lp in base {
            if excluded.contains(lp) {
                continue;
            }
            if seen.insert(lp.as_str()) {
                out.push(lp.clone());
            }
        }

        if out.is_empty() {
            return Err(vec![LpPanelError::EmptyEffectiveSet]);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn known(ids: &[&str]) -> BTreeSet<String> {
        ids.iter().map(|s| (*s).to_string()).collect()
    }

    // ---- oracle cases (a hand-written truth table) --------------------------

    #[test]
    fn empty_panel_resolves_to_all_known_sorted() {
        let k = known(&["LP-C", "LP-A", "LP-B"]);
        let eff = HedgeLpPanel::all().effective_lps(&k).unwrap();
        // BTreeSet iteration is sorted → deterministic all-known order.
        assert_eq!(eff, vec!["LP-A", "LP-B", "LP-C"]);
    }

    #[test]
    fn include_only_is_exactly_the_includes_in_order() {
        let k = known(&["LP-A", "LP-B", "LP-C", "LP-D"]);
        let panel = HedgeLpPanel {
            include: vec!["LP-C".into(), "LP-A".into()],
            exclude: vec![],
        };
        // Include order preserved (the trader's RFQ fan priority).
        assert_eq!(panel.effective_lps(&k).unwrap(), vec!["LP-C", "LP-A"]);
    }

    #[test]
    fn exclude_only_subtracts_from_all_known() {
        let k = known(&["LP-A", "LP-B", "LP-C"]);
        let panel = HedgeLpPanel {
            include: vec![],
            exclude: vec!["LP-B".into()],
        };
        assert_eq!(panel.effective_lps(&k).unwrap(), vec!["LP-A", "LP-C"]);
    }

    #[test]
    fn include_and_exclude_interaction_excludes_win() {
        let k = known(&["LP-A", "LP-B", "LP-C", "LP-D"]);
        let panel = HedgeLpPanel {
            include: vec!["LP-A".into(), "LP-B".into(), "LP-C".into()],
            exclude: vec!["LP-B".into()],
        };
        // B is in both → removed.
        assert_eq!(panel.effective_lps(&k).unwrap(), vec!["LP-A", "LP-C"]);
    }

    #[test]
    fn duplicate_includes_are_deduped_first_occurrence_kept() {
        let k = known(&["LP-A", "LP-B"]);
        let panel = HedgeLpPanel {
            include: vec!["LP-A".into(), "LP-A".into(), "LP-B".into()],
            exclude: vec![],
        };
        assert_eq!(panel.effective_lps(&k).unwrap(), vec!["LP-A", "LP-B"]);
    }

    #[test]
    fn unknown_include_id_rejected() {
        let k = known(&["LP-A", "LP-B"]);
        let panel = HedgeLpPanel {
            include: vec!["LP-A".into(), "GHOST".into()],
            exclude: vec![],
        };
        let errs = panel.effective_lps(&k).unwrap_err();
        assert!(errs.contains(&LpPanelError::UnknownInclude { lp: "GHOST".into() }));
    }

    #[test]
    fn unknown_exclude_id_rejected() {
        let k = known(&["LP-A", "LP-B"]);
        let panel = HedgeLpPanel {
            include: vec![],
            exclude: vec!["PHANTOM".into()],
        };
        let errs = panel.effective_lps(&k).unwrap_err();
        assert!(errs.contains(&LpPanelError::UnknownExclude {
            lp: "PHANTOM".into()
        }));
    }

    #[test]
    fn all_unknown_ids_collected_at_once() {
        let k = known(&["LP-A"]);
        let panel = HedgeLpPanel {
            include: vec!["X".into()],
            exclude: vec!["Y".into()],
        };
        let errs = panel.effective_lps(&k).unwrap_err();
        assert_eq!(errs.len(), 2, "both unknowns reported at once: {errs:?}");
    }

    #[test]
    fn excluding_everything_is_an_empty_set_error() {
        let k = known(&["LP-A", "LP-B"]);
        let panel = HedgeLpPanel {
            include: vec![],
            exclude: vec!["LP-A".into(), "LP-B".into()],
        };
        assert_eq!(
            panel.effective_lps(&k).unwrap_err(),
            vec![LpPanelError::EmptyEffectiveSet]
        );
    }

    #[test]
    fn is_unrestricted_flags_the_default_panel() {
        assert!(HedgeLpPanel::all().is_unrestricted());
        assert!(!HedgeLpPanel::include_only(["LP-A".to_string()]).is_unrestricted());
        assert!(
            !HedgeLpPanel {
                include: vec![],
                exclude: vec!["LP-A".into()]
            }
            .is_unrestricted()
        );
    }

    #[test]
    fn validate_passes_for_known_ids_without_resolving() {
        let k = known(&["LP-A", "LP-B"]);
        let panel = HedgeLpPanel {
            include: vec!["LP-A".into()],
            exclude: vec!["LP-B".into()],
        };
        // validate does not flag the (here non-empty) resolved set.
        assert!(panel.validate(&k).is_ok());
    }

    #[test]
    fn error_display_is_nonempty() {
        assert!(
            !LpPanelError::UnknownInclude { lp: "X".into() }
                .to_string()
                .is_empty()
        );
        assert!(!LpPanelError::EmptyEffectiveSet.to_string().is_empty());
    }
}
