//! The cryptographically-unforgeable click-to-trade token core — the single
//! source of truth for the keyed-MAC last-look token used by **both** the
//! multiplexed RFS [`crate::services::stream`] session and the FIX acceptor edge
//! ([`crate::services::fix`]).
//!
//! A click-to-trade token must be impossible to forge without the server secret:
//! presenting a guessed / enumerated token would otherwise book a trade the maker
//! never quoted. The token is therefore a **keyed MAC** (a truncated `blake3` keyed
//! hash, RFC-grade keyed-hash construction) over the immutable line-binding tuple
//! `(subscription_id, sequence, side, premium_bits, valid_until_nanos)` plus a fresh
//! per-mint nonce. The 256-bit key is drawn **once at session start from the OS
//! CSPRNG** ([`getrandom`]); without it an attacker cannot produce a value the
//! server will accept, even by enumeration of the 64-bit token space.
//!
//! # Why a CSPRNG secret does not break pricing determinism
//!
//! This is a **runtime, control-plane ephemeral identity** — it authenticates a
//! click, it is *never* an input to any priced number. The platform's determinism
//! guardrail (`CLAUDE.md` rule 5) governs the **pricing** path (libm, no OS RNG) so
//! prices are bit-reproducible; a token's MAC tag is not a price and is not replayed
//! for pricing. Seeding the MAC key from a CSPRNG is exactly correct here:
//! unpredictability is the security property we need, and it leaves every priced
//! value untouched.
//!
//! # One token construction, two transports
//!
//! The RFS stream stamps tokens on a streamed line and books a click via
//! [`celnet_proto::Execute`]; the FIX acceptor stamps the *same* token on a
//! `Quote(S)` (carried in `QuoteID(117)`) and books a lift via
//! `NewOrderSingle(D)`. Both mint through the identical [`TokenMinter`] and both
//! validate through the identical last-look / replay / idempotency discipline of a
//! [`TokenLedger`], so a FIX lift is byte-for-byte the same execution math as a gRPC
//! click — no forked execution path (`CLAUDE.md` rule 9).

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

use celnet_proto::Side;

/// A live click-to-trade token stamped on a quoted line: the side and premium it
/// books and the deadline after which it is dead.
#[derive(Debug, Clone, Copy)]
pub(crate) struct LiveToken {
    /// BUY lifts the offer; SELL hits the bid.
    pub(crate) side: Side,
    /// The premium this token books (the offer for BUY, the bid for SELL).
    pub(crate) premium: f64,
    /// The token validity deadline (nanoseconds since the Unix epoch, UTC).
    pub(crate) valid_until_nanos: i64,
}

/// The immutable binding a click-to-trade token authenticates: a token is valid only
/// for *exactly* the line it was stamped on (its subscription/quote, sequence, side,
/// premium, and validity deadline). The MAC is computed over this tuple, so a token
/// cannot be lifted onto a different line, side, or premium.
#[derive(Debug, Clone, Copy)]
pub(crate) struct TokenBinding {
    /// The opaque line identity (RFS subscription id, or the FIX quote ordinal).
    pub(crate) subscription_id: u64,
    /// The monotonic sequence the token was stamped at.
    pub(crate) sequence: u64,
    /// The side this token books.
    pub(crate) side: Side,
    /// The premium this token books.
    pub(crate) premium: f64,
    /// The validity deadline (nanoseconds, UTC).
    pub(crate) valid_until_nanos: i64,
}

/// A cryptographically-unforgeable click-to-trade token minter (keyed `blake3` MAC).
#[derive(Debug)]
pub(crate) struct TokenMinter {
    /// The session-unique MAC key, drawn once from the OS CSPRNG at session start.
    key: [u8; 32],
    /// A fresh per-mint nonce so two tokens for the *same* binding tuple still differ
    /// (and a token reveals nothing about the key).
    nonce: AtomicU64,
}

impl TokenMinter {
    /// Mint a fresh-keyed minter for one session / acceptor.
    pub(crate) fn new() -> Self {
        let mut key = [0u8; 32];
        // OS CSPRNG. `getrandom` cannot fail on a supported platform; a failure here
        // means the OS entropy source is unavailable, which is unrecoverable for a
        // security-bearing token, so we refuse to serve forgeable tokens.
        getrandom::getrandom(&mut key)
            .expect("OS CSPRNG unavailable: cannot mint unforgeable tokens");
        Self {
            key,
            nonce: AtomicU64::new(0),
        }
    }

    /// Serialize a binding + nonce into the MAC message: a fixed-width, unambiguous
    /// big-endian encoding so distinct tuples never collide on the same message.
    fn message(binding: &TokenBinding, nonce: u64) -> [u8; 41] {
        let mut msg = [0u8; 41];
        msg[0..8].copy_from_slice(&binding.subscription_id.to_be_bytes());
        msg[8..16].copy_from_slice(&binding.sequence.to_be_bytes());
        msg[16] = binding.side as u8;
        msg[17..25].copy_from_slice(&binding.premium.to_bits().to_be_bytes());
        msg[25..33].copy_from_slice(&binding.valid_until_nanos.to_be_bytes());
        msg[33..41].copy_from_slice(&nonce.to_be_bytes());
        msg
    }

    /// The 64-bit MAC tag for a binding under a given nonce (first 8 bytes of the
    /// keyed `blake3` hash, big-endian).
    pub(crate) fn tag(&self, binding: &TokenBinding, nonce: u64) -> u64 {
        let mac = blake3::keyed_hash(&self.key, &Self::message(binding, nonce));
        u64::from_be_bytes(mac.as_bytes()[0..8].try_into().expect("32-byte hash"))
    }

    /// Mint an unforgeable token for a line binding (always `>= 1`). The fresh nonce
    /// makes two mints of the same binding distinct; a zero tag is re-minted so the
    /// token is always a non-zero sentinel-safe value.
    pub(crate) fn mint(&self, binding: &TokenBinding) -> u64 {
        loop {
            let nonce = self.nonce.fetch_add(1, Ordering::Relaxed);
            let t = self.tag(binding, nonce);
            if t != 0 {
                return t;
            }
        }
    }
}

/// The outcome of presenting a token to a [`TokenLedger`] for a last-look booking.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum BookOutcome {
    /// The token authenticated, was live and unconsumed within its window: book at
    /// the carried side + premium.
    Booked {
        /// The side the token books.
        side: Side,
        /// The premium the token books.
        premium: f64,
    },
    /// The token was already consumed (a replay / double-click within its window).
    AlreadyConsumed,
    /// The token is not a live stamped token (forged / enumerated / retired).
    UnknownToken,
    /// The token was presented after its validity deadline (last-look).
    Expired,
}

/// The per-line ledger of live + consumed click-to-trade tokens: the last-look,
/// replay-protection, and bounded-consumed-set discipline shared by the RFS stream
/// and the FIX acceptor. Distinct from the MAC [`TokenMinter`] (which mints), this
/// tracks *which* minted tokens are currently liftable and which have been spent.
#[derive(Debug, Default)]
pub(crate) struct TokenLedger {
    /// The currently-live tokens (keyed by opaque token value). A new sequence mints
    /// fresh tokens and clears the prior ones, so a token is live only for the
    /// sequence it was stamped on.
    live: HashMap<u64, LiveToken>,
    /// Tokens already consumed by an accepted lift, mapped to their validity
    /// deadline, so a replayed lift (or a second click) is rejected as
    /// already-consumed rather than re-booked. **Bounded**: a consumed token only
    /// needs to block replay *within its own validity window* (after expiry the
    /// token is rejected as `Expired` regardless), so entries past their
    /// `valid_until_nanos` are evicted — the set can never grow without bound over a
    /// long session (the scale guardrail, `CLAUDE.md` rule 6).
    consumed: HashMap<u64, i64>,
}

impl TokenLedger {
    /// A fresh, empty ledger.
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Retire the prior sequence's live tokens (a new line always supersedes the old
    /// so a click books the *current* streamed/quoted premium, never a stale one).
    pub(crate) fn clear_live(&mut self) {
        self.live.clear();
    }

    /// Register a freshly-minted live token for `(side, premium)` under its deadline.
    pub(crate) fn register(
        &mut self,
        token: u64,
        side: Side,
        premium: f64,
        valid_until_nanos: i64,
    ) {
        self.live.insert(
            token,
            LiveToken {
                side,
                premium,
                valid_until_nanos,
            },
        );
    }

    /// The number of currently-live tokens (introspection / tests).
    #[cfg(test)]
    pub(crate) fn live_len(&self) -> usize {
        self.live.len()
    }

    /// The number of consumed entries currently retained (bounded-set tests).
    #[cfg(test)]
    pub(crate) fn consumed_len(&self) -> usize {
        self.consumed.len()
    }

    /// Iterate the live tokens (introspection / tests).
    #[cfg(test)]
    pub(crate) fn live_tokens(&self) -> impl Iterator<Item = (&u64, &LiveToken)> {
        self.live.iter()
    }

    /// Any one currently-live token value (introspection / tests).
    #[cfg(test)]
    pub(crate) fn any_live_token(&self) -> Option<u64> {
        self.live.keys().next().copied()
    }

    /// Whether `token` is currently live (introspection / tests).
    #[cfg(test)]
    pub(crate) fn is_live(&self, token: u64) -> bool {
        self.live.contains_key(&token)
    }

    /// Record `token` as consumed (so a replayed lift is rejected as
    /// already-consumed), keyed by its validity deadline, and **evict every
    /// already-expired consumed token** in the same pass.
    ///
    /// Replay protection only has to hold *within a token's validity window*: once a
    /// token is past its `valid_until_nanos` a lift presenting it is rejected as
    /// `Expired` before the consumed-set is ever consulted, so retaining expired
    /// entries buys no extra protection. Pruning them on every insert bounds the set
    /// to at most the tokens minted within one validity window, so it can never grow
    /// without bound over an arbitrarily long session.
    fn record_consumed(&mut self, token: u64, valid_until_nanos: i64, now_nanos: i64) {
        self.consumed
            .retain(|_, &mut deadline| deadline >= now_nanos);
        self.consumed.insert(token, valid_until_nanos);
    }

    /// Is `token` a still-valid consumed token (consumed within its window, blocking
    /// a replay)? Expired consumed entries are lazily pruned and never matched here:
    /// an expired token is the `Expired` path, not the `AlreadyConsumed` path.
    fn is_consumed(&self, token: u64, now_nanos: i64) -> bool {
        self.consumed
            .get(&token)
            .is_some_and(|&deadline| deadline >= now_nanos)
    }

    /// Present `token` for a last-look booking at `now_nanos`, applying the *exact*
    /// precedence the RFS click-to-trade path uses: already-consumed (replay) →
    /// unknown/forged → expired → book. On a successful book the token is marked
    /// consumed (bounded, expiry-evicting) and retired from the live set so a second
    /// lift of the same token rejects as `AlreadyConsumed`.
    pub(crate) fn try_book(&mut self, token: u64, now_nanos: i64) -> BookOutcome {
        // Already consumed (a replay / second click within the window) — checked
        // first so a replayed lift is `AlreadyConsumed`, not re-booked.
        if self.is_consumed(token, now_nanos) {
            return BookOutcome::AlreadyConsumed;
        }
        // Unknown / forged token: not a live stamped token for this line.
        let Some(live) = self.live.get(&token).copied() else {
            return BookOutcome::UnknownToken;
        };
        // Expired: presented after its validity deadline (last-look).
        if now_nanos > live.valid_until_nanos {
            return BookOutcome::Expired;
        }
        // Book it: mark consumed and retire the live token (idempotency: a second
        // lift now rejects as already-consumed).
        self.record_consumed(token, live.valid_until_nanos, now_nanos);
        self.live.remove(&token);
        BookOutcome::Booked {
            side: live.side,
            premium: live.premium,
        }
    }
}

/// One minted two-way click-to-trade token: its opaque value, the side+premium it
/// books, and its validity deadline. The transport (RFS `TradableToken`, or FIX
/// `QuoteID`) maps this to its own wire form.
#[derive(Debug, Clone, Copy)]
pub(crate) struct MintedToken {
    /// The opaque, unforgeable token value.
    pub(crate) token: u64,
    /// The side this token books (SELL hits the bid, BUY lifts the offer).
    pub(crate) side: Side,
    /// The premium this token books.
    pub(crate) premium: f64,
    /// The validity deadline (nanoseconds, UTC).
    pub(crate) valid_until_nanos: i64,
}

/// The inputs to a two-way click-to-trade mint: the opaque line identity the MAC
/// binds (the RFS subscription id, or the FIX quote ordinal), the line's sequence, and
/// the bid/offer premia.
#[derive(Debug, Clone, Copy)]
pub(crate) struct TwoWayLine {
    /// The opaque line identity the keyed MAC binds.
    pub(crate) line_id: u64,
    /// The line's sequence number.
    pub(crate) sequence: u64,
    /// The bid premium (a SELL hits this; non-positive ⇒ no SELL token).
    pub(crate) bid: f64,
    /// The offer premium (a BUY lifts this).
    pub(crate) offer: f64,
}

/// Mint the two click-to-trade tokens for a two-way line (SELL@bid, BUY@offer),
/// registering them as the ledger's live tokens (retiring any prior ones), and
/// returning the minted tokens. A non-positive bid mints no SELL token (there is
/// nothing to hit), so a degenerate line is BUY-only — identical to the RFS rule.
///
/// `now_nanos`/`validity_nanos` set the last-look window. This is the SINGLE mint path
/// both transports share.
pub(crate) fn mint_two_way(
    ledger: &mut TokenLedger,
    minter: &TokenMinter,
    line: TwoWayLine,
    now_nanos: i64,
    validity_nanos: i64,
) -> Vec<MintedToken> {
    let TwoWayLine {
        line_id,
        sequence: seq,
        bid,
        offer,
    } = line;
    // A new sequence retires the prior sequence's tokens: a click always books the
    // current quoted premium, never a stale one.
    ledger.clear_live();
    let valid_until = now_nanos.saturating_add(validity_nanos);
    let mut out = Vec::with_capacity(2);
    // SELL hits the bid (only when the bid is a real, positive price).
    if bid > 0.0 {
        let token = minter.mint(&TokenBinding {
            subscription_id: line_id,
            sequence: seq,
            side: Side::Sell,
            premium: bid,
            valid_until_nanos: valid_until,
        });
        ledger.register(token, Side::Sell, bid, valid_until);
        out.push(MintedToken {
            token,
            side: Side::Sell,
            premium: bid,
            valid_until_nanos: valid_until,
        });
    }
    // BUY lifts the offer.
    let token = minter.mint(&TokenBinding {
        subscription_id: line_id,
        sequence: seq,
        side: Side::Buy,
        premium: offer,
        valid_until_nanos: valid_until,
    });
    ledger.register(token, Side::Buy, offer, valid_until);
    out.push(MintedToken {
        token,
        side: Side::Buy,
        premium: offer,
        valid_until_nanos: valid_until,
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn binding(seq: u64, side: Side, premium: f64) -> TokenBinding {
        TokenBinding {
            subscription_id: 1,
            sequence: seq,
            side,
            premium,
            valid_until_nanos: 2_000_000_000,
        }
    }

    #[test]
    fn minted_tokens_are_unguessable_and_distinct() {
        let minter = TokenMinter::new();
        let b = binding(1, Side::Buy, 0.012);
        let t1 = minter.mint(&b);
        let t2 = minter.mint(&b);
        assert!(t1 >= 1 && t2 >= 1);
        assert_ne!(t1, t2, "fresh nonce ⇒ distinct tokens for one binding");
        assert_ne!(t1.abs_diff(t2), 1, "tokens must not be a dense sequence");
    }

    #[test]
    fn mac_is_key_bound_and_field_bound() {
        let minter = TokenMinter::new();
        let other = TokenMinter::new();
        let b = binding(5, Side::Buy, 0.012);
        assert_ne!(minter.tag(&b, 0), other.tag(&b, 0), "key-bound");
        assert_ne!(
            minter.tag(&b, 0),
            minter.tag(&binding(6, Side::Buy, 0.012), 0)
        );
        assert_ne!(
            minter.tag(&b, 0),
            minter.tag(&binding(5, Side::Sell, 0.012), 0)
        );
        assert_ne!(
            minter.tag(&b, 0),
            minter.tag(&binding(5, Side::Buy, 0.013), 0)
        );
        let mut shift = binding(5, Side::Buy, 0.012);
        shift.valid_until_nanos += 1;
        assert_ne!(minter.tag(&b, 0), minter.tag(&shift, 0));
    }

    #[test]
    fn ledger_books_then_rejects_replay_forged_expired() {
        let minter = TokenMinter::new();
        let mut ledger = TokenLedger::new();
        let now = 1_000_000_000;
        let valid_until = now + 1_000_000_000;
        let b = TokenBinding {
            subscription_id: 7,
            sequence: 1,
            side: Side::Buy,
            premium: 0.0123,
            valid_until_nanos: valid_until,
        };
        let token = minter.mint(&b);
        ledger.register(token, Side::Buy, 0.0123, valid_until);

        // Forged token → UnknownToken.
        assert_eq!(ledger.try_book(0x0BAD_F00D, now), BookOutcome::UnknownToken);

        // The live token books at its bound side+premium.
        assert_eq!(
            ledger.try_book(token, now),
            BookOutcome::Booked {
                side: Side::Buy,
                premium: 0.0123,
            }
        );
        // A replay within the window → AlreadyConsumed.
        assert_eq!(ledger.try_book(token, now), BookOutcome::AlreadyConsumed);
    }

    #[test]
    fn ledger_rejects_token_past_its_deadline() {
        let minter = TokenMinter::new();
        let mut ledger = TokenLedger::new();
        let valid_until = 1_000_000_000;
        let b = TokenBinding {
            subscription_id: 1,
            sequence: 1,
            side: Side::Sell,
            premium: 0.01,
            valid_until_nanos: valid_until,
        };
        let token = minter.mint(&b);
        ledger.register(token, Side::Sell, 0.01, valid_until);
        // Now is strictly past the deadline ⇒ Expired (last-look).
        assert_eq!(
            ledger.try_book(token, valid_until + 1),
            BookOutcome::Expired
        );
    }

    #[test]
    fn consumed_set_stays_bounded_as_tokens_expire() {
        let mut ledger = TokenLedger::new();
        // Book many tokens whose windows are already long past as `now` advances:
        // each insert evicts the prior expired entries, so the set stays tiny.
        for i in 0..1000u64 {
            let now = (i as i64) * 1_000_000_000;
            let valid_until = now + 10;
            ledger.register(i.wrapping_add(1), Side::Buy, 0.01, valid_until);
            let _ = ledger.try_book(i.wrapping_add(1), now);
        }
        assert!(
            ledger.consumed_len() <= 2,
            "consumed set must stay bounded as tokens expire; held {}",
            ledger.consumed_len()
        );
    }
}
