//! Shared who's-trading attribution: the maker's auto-pricer identity and the
//! resolution of the [`celnet_proto::AttributionRecord`] carried through the
//! quote / stream / trade lifecycle.
//!
//! Every line the edge prices is quoted by an **automated pricer** — the engine —
//! so the flow is never anonymous on the who's-trading dimension
//! (`docs/RISK-HIERARCHY.md`: book is a cube dimension; `docs/TRADING-UNIVERSE-SCALE.md`
//! §12: the blotter / Book stop being anonymous). This module is the single source
//! of that identity and of how it composes with a client-supplied requesting seat,
//! so the RFQ ([`crate::services::quote`]) and RFS ([`crate::services::stream`])
//! paths attribute flow identically (API-first parity).
//!
//! # Model
//!
//! * **`quoted_by`** — always the maker's auto-pricer book ([`maker_auto_pricer`]):
//!   the engine priced and showed the market. A client cannot claim to have quoted
//!   a line the maker priced, so this is stamped maker-side, overriding any
//!   client-supplied `quoted_by`.
//! * **`held_by`** — the requesting seat, when the client supplied one (its
//!   `quoted_by` on the request is the seat the quote is requested *on behalf of*,
//!   i.e. who will hold the resulting position). Absent ⇒ the maker holds it (a
//!   warehoused auto-quoted line), represented by omitting `held_by` (held by the
//!   `quoted_by` auto-pricer, per the field's presence contract).
//! * **`won` / `lp_count`** — carried through verbatim from the client request
//!   when present (the competition context is the client's to assert), else absent.

use celnet_proto::{AttributionRecord, BookId, Owner, owner};

/// The maker's auto-pricer book identifier (the engine-quoted book). A stable,
/// vendor-neutral identity for the automated pricer that shows every edge line.
pub(crate) const MAKER_AUTO_PRICER_ID: &str = "celnet-auto-pricer";

/// The maker's auto-pricer book key the auto-quoted flow is attributed to.
pub(crate) const MAKER_BOOK: &str = "AUTO-MM";

/// The maker's auto-pricer [`BookId`]: the `quoted_by` of every edge-quoted line.
pub(crate) fn maker_auto_pricer() -> BookId {
    BookId {
        book: MAKER_BOOK.to_owned(),
        owner: Some(Owner {
            seat: Some(owner::Seat::AutoPricer(MAKER_AUTO_PRICER_ID.to_owned())),
        }),
    }
}

/// Resolve the attribution chain to stamp on a quote / streamed line / fill from an
/// optional client-supplied request attribution.
///
/// Always stamps the maker auto-pricer as `quoted_by`. When the client supplied a
/// requesting seat (its request `quoted_by`), that seat becomes `held_by` (it will
/// hold the booked position); the client's `won` / `lp_count` competition context
/// is carried through verbatim. With no client attribution the line is a plain
/// auto-quoted maker line: `quoted_by` only, `held_by` omitted (the auto-pricer
/// holds it).
pub(crate) fn resolve(request_attribution: Option<&AttributionRecord>) -> AttributionRecord {
    let quoted_by = Some(maker_auto_pricer());
    match request_attribution {
        Some(req) => AttributionRecord {
            quoted_by,
            // The client's requesting seat (its `quoted_by`) holds the position.
            held_by: req.quoted_by.clone(),
            won: req.won,
            lp_count: req.lp_count,
        },
        None => AttributionRecord {
            quoted_by,
            held_by: None,
            won: None,
            lp_count: None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn trader_seat(book: &str, trader: &str) -> BookId {
        BookId {
            book: book.to_owned(),
            owner: Some(Owner {
                seat: Some(owner::Seat::Trader(trader.to_owned())),
            }),
        }
    }

    /// A plain auto-quoted line (no client attribution) is stamped with the maker
    /// auto-pricer as `quoted_by`, so the flow is never anonymous.
    #[test]
    fn unattributed_request_gets_maker_auto_pricer() {
        let a = resolve(None);
        let quoted = a.quoted_by.expect("quoted_by always set");
        assert_eq!(quoted.book, MAKER_BOOK);
        assert!(matches!(
            quoted.owner.unwrap().seat,
            Some(owner::Seat::AutoPricer(id)) if id == MAKER_AUTO_PRICER_ID
        ));
        assert!(
            a.held_by.is_none(),
            "auto-pricer holds an unattributed line"
        );
    }

    /// A client-supplied requesting seat becomes the `held_by`, the maker stays the
    /// `quoted_by`, and the competition context carries through.
    #[test]
    fn client_seat_becomes_holder_maker_stays_quoter() {
        let req = AttributionRecord {
            quoted_by: Some(trader_seat("EM-VOL-1", "jdoe")),
            held_by: None,
            won: Some(true),
            lp_count: Some(3),
        };
        let a = resolve(Some(&req));
        // Maker quotes.
        assert_eq!(a.quoted_by.as_ref().unwrap().book, MAKER_BOOK);
        // Client seat holds.
        let held = a.held_by.expect("client seat becomes holder");
        assert_eq!(held.book, "EM-VOL-1");
        assert!(matches!(
            held.owner.unwrap().seat,
            Some(owner::Seat::Trader(t)) if t == "jdoe"
        ));
        assert_eq!(a.won, Some(true));
        assert_eq!(a.lp_count, Some(3));
    }
}
