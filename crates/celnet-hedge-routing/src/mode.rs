//! The **exit mode** — whether a resolved exit fires by itself or waits for a trader.
//!
//! The desk asked for this in plain words: *"we don't want traders to receive a popup —
//! we want to be able to have it manual, i.e. suggestions risk exists, and then have an
//! auto exit."* Two distinct things are being asked for, and this enum is the switch
//! between them:
//!
//! - [`HedgeExitMode::Auto`] — the historical behaviour. A breach resolves an action and
//!   the engine executes it, with no human in the loop.
//! - [`HedgeExitMode::Suggest`] — the engine does **all** the same work (measure, band,
//!   resolve the policy, size the vehicle) and then **stops**, publishing a standing
//!   suggestion onto the risk surface. Nothing trades until the trader fires it.
//!
//! `Suggest` is deliberately **not** a confirmation dialog. A modal interrupts whatever
//! the trader is doing and vanishes when dismissed; a standing suggestion sits on the
//! risk panel next to the risk it refers to, survives being ignored, and is still there
//! when the desk comes back to it. That is the difference the request turns on.
//!
//! [`Auto`](HedgeExitMode::Auto) is the `Default` so a firm that has configured no mode
//! anywhere keeps today's behaviour exactly.

use serde::{Deserialize, Serialize};
use std::fmt;

/// Whether a resolved exit action fires automatically or waits for a trader.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum HedgeExitMode {
    /// Fire on breach, no interaction. **The default** — today's behaviour.
    #[default]
    Auto,
    /// Publish a standing suggestion and trade nothing until a trader fires it. No popup;
    /// the suggestion lives on the risk surface.
    Suggest,
}

impl HedgeExitMode {
    /// A short, stable label for provenance / UI / logging.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            HedgeExitMode::Auto => "auto",
            HedgeExitMode::Suggest => "suggest",
        }
    }

    /// Whether the engine may execute without a trader. `false` under
    /// [`Suggest`](Self::Suggest) — the single predicate the booking path branches on.
    #[must_use]
    pub fn fires_automatically(self) -> bool {
        matches!(self, HedgeExitMode::Auto)
    }
}

impl fmt::Display for HedgeExitMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_is_the_default_so_an_unconfigured_firm_is_unchanged() {
        assert_eq!(HedgeExitMode::default(), HedgeExitMode::Auto);
        assert!(HedgeExitMode::default().fires_automatically());
    }

    #[test]
    fn suggest_never_fires_automatically() {
        assert!(!HedgeExitMode::Suggest.fires_automatically());
        assert_eq!(HedgeExitMode::Suggest.label(), "suggest");
        assert_eq!(HedgeExitMode::Suggest.to_string(), "suggest");
    }

    #[test]
    fn mode_round_trips_through_json() {
        for m in [HedgeExitMode::Auto, HedgeExitMode::Suggest] {
            let json = serde_json::to_string(&m).unwrap();
            assert_eq!(serde_json::from_str::<HedgeExitMode>(&json).unwrap(), m);
        }
    }

    /// A config persisted before exit modes existed carries no key and must reload as
    /// `Auto` — the no-silent-change contract.
    #[test]
    fn legacy_json_without_a_mode_reloads_as_auto() {
        #[derive(Deserialize)]
        struct Scope {
            #[serde(default)]
            mode: HedgeExitMode,
        }
        let s: Scope = serde_json::from_str("{}").unwrap();
        assert_eq!(s.mode, HedgeExitMode::Auto);
    }
}
