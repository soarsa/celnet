//! The deterministic replicated state machine: a priced book over `u64 → f64`.
//!
//! Replication is only useful if every replica that applies the same committed
//! log prefix lands on **bit-identical** state. To make that property *testable*
//! by `f64::to_bits` equality, the replicated state here is a small, fully
//! deterministic priced book: a map from an opaque `u64` instrument key to an
//! `f64` price/Greek value, mutated by an ordered stream of [`BookUpdate`]s.
//!
//! # Determinism contract
//!
//! [`BookState::apply`] is a pure function of the prior state and the update.
//! It performs **no** floating-point operation whose result depends on iteration
//! order, hashing, or platform RNG: a `Set` overwrites, an `AccumulateBits`
//! folds via integer-exact `to_bits`/`from_bits` round-tripping so the stored
//! value is *literally the leader's bits*, and the canonical snapshot iterates a
//! `BTreeMap` (sorted by key) so the byte serialization is order-independent.
//! Two nodes that apply the same committed entries therefore agree to the last
//! bit — the property the replication tests assert with [`BookState::to_bits`].

use std::collections::BTreeMap;

/// A single deterministic mutation of the replicated priced book.
#[derive(Debug, Clone, PartialEq)]
pub enum BookUpdate {
    /// Set `key`'s value to exactly `value` (last-writer-wins at that index).
    Set {
        /// The instrument key.
        key: u64,
        /// The exact `f64` value to store (compared downstream by `to_bits`).
        value: f64,
    },
    /// Add `delta` to `key`'s current value (0.0 if absent), storing the exact
    /// IEEE-754 result. Replicated so every node performs the identical add in
    /// the identical order ⇒ identical resulting bits.
    Add {
        /// The instrument key.
        key: u64,
        /// The increment.
        delta: f64,
    },
    /// Remove `key` from the book (no-op if absent).
    Remove {
        /// The instrument key to drop.
        key: u64,
    },
}

impl BookUpdate {
    /// Encode the update to deterministic bytes (a 1-byte tag + fields).
    ///
    /// `f64` fields are stored by their raw IEEE-754 bits (`to_bits`), so the
    /// encoding round-trips the exact value — including signed zero and the NaN
    /// bit pattern — with no decimal rounding.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(1 + 8 + 8);
        match self {
            BookUpdate::Set { key, value } => {
                buf.push(0u8);
                buf.extend_from_slice(&key.to_le_bytes());
                buf.extend_from_slice(&value.to_bits().to_le_bytes());
            }
            BookUpdate::Add { key, delta } => {
                buf.push(1u8);
                buf.extend_from_slice(&key.to_le_bytes());
                buf.extend_from_slice(&delta.to_bits().to_le_bytes());
            }
            BookUpdate::Remove { key } => {
                buf.push(2u8);
                buf.extend_from_slice(&key.to_le_bytes());
            }
        }
        buf
    }

    /// Decode an update from its byte form.
    ///
    /// # Errors
    ///
    /// Returns [`UpdateError`] if the tag is unknown or the buffer is too short
    /// for the tagged variant's fields.
    pub fn decode(bytes: &[u8]) -> Result<Self, UpdateError> {
        let (&tag, rest) = bytes.split_first().ok_or(UpdateError::Truncated)?;
        match tag {
            0 | 1 => {
                if rest.len() < 16 {
                    return Err(UpdateError::Truncated);
                }
                let key = u64::from_le_bytes(rest[0..8].try_into().expect("8 bytes"));
                let bits = u64::from_le_bytes(rest[8..16].try_into().expect("8 bytes"));
                let val = f64::from_bits(bits);
                Ok(if tag == 0 {
                    BookUpdate::Set { key, value: val }
                } else {
                    BookUpdate::Add { key, delta: val }
                })
            }
            2 => {
                if rest.len() < 8 {
                    return Err(UpdateError::Truncated);
                }
                let key = u64::from_le_bytes(rest[0..8].try_into().expect("8 bytes"));
                Ok(BookUpdate::Remove { key })
            }
            other => Err(UpdateError::UnknownTag(other)),
        }
    }
}

/// A failure decoding a [`BookUpdate`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateError {
    /// The buffer is too short for the tagged variant.
    Truncated,
    /// The leading tag byte does not name a known variant.
    UnknownTag(u8),
}

impl std::fmt::Display for UpdateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UpdateError::Truncated => write!(f, "book update truncated"),
            UpdateError::UnknownTag(t) => write!(f, "book update unknown tag {t}"),
        }
    }
}

impl std::error::Error for UpdateError {}

/// The deterministic replicated priced book.
///
/// A sorted `u64 → f64` map; `BTreeMap` (not `HashMap`) so the canonical
/// snapshot and `to_bits` digest are independent of insertion order and of any
/// hash seed — a precondition for byte-identical cross-node comparison.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BookState {
    book: BTreeMap<u64, f64>,
}

impl BookState {
    /// A fresh, empty book.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Apply one update, mutating the book deterministically.
    pub fn apply(&mut self, update: &BookUpdate) {
        match *update {
            BookUpdate::Set { key, value } => {
                self.book.insert(key, value);
            }
            BookUpdate::Add { key, delta } => {
                let entry = self.book.entry(key).or_insert(0.0);
                *entry += delta;
            }
            BookUpdate::Remove { key } => {
                self.book.remove(&key);
            }
        }
    }

    /// The current value for `key`, if present.
    #[must_use]
    pub fn get(&self, key: u64) -> Option<f64> {
        self.book.get(&key).copied()
    }

    /// The number of instruments in the book.
    #[must_use]
    pub fn len(&self) -> usize {
        self.book.len()
    }

    /// Whether the book is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.book.is_empty()
    }

    /// A canonical, order-independent `(key, f64-bits)` digest of the whole book.
    ///
    /// This is the **bit-identity oracle**: two `BookState`s are bit-identical
    /// iff their `to_bits` vectors are equal. Each value is captured as its raw
    /// `f64::to_bits`, so equality is exact IEEE-754 bit equality (not tolerance
    /// based), and the `BTreeMap` ordering makes the vector independent of the
    /// order in which updates arrived.
    #[must_use]
    pub fn to_bits(&self) -> Vec<(u64, u64)> {
        self.book.iter().map(|(&k, &v)| (k, v.to_bits())).collect()
    }

    /// Serialize the whole book to a canonical, deterministic byte form for a
    /// durable snapshot (the captured state machine inside a snapshot file).
    ///
    /// Layout (all little-endian): a `u64` entry count, then each `(key u64,
    /// value-bits u64)` pair **in `BTreeMap` key order**. Values are stored by
    /// their raw IEEE-754 bits (`f64::to_bits`), so the round-trip is *exact* —
    /// signed zero, subnormals and NaN bit patterns are preserved, never decimal-
    /// rounded — and the byte stream is independent of the order updates arrived
    /// (the sorted iteration is the same canonicalization [`BookState::to_bits`]
    /// uses). Encoding is a pure function of the state, so two nodes with the same
    /// applied state encode to byte-identical snapshot payloads.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(8 + self.book.len() * 16);
        buf.extend_from_slice(&(self.book.len() as u64).to_le_bytes());
        for (&k, &v) in &self.book {
            buf.extend_from_slice(&k.to_le_bytes());
            buf.extend_from_slice(&v.to_bits().to_le_bytes());
        }
        buf
    }

    /// Reconstruct a book from its canonical [`BookState::encode`] bytes.
    ///
    /// The result is `to_bits`-identical to the state that was encoded.
    ///
    /// # Errors
    ///
    /// Returns [`UpdateError::Truncated`] if the buffer is shorter than the
    /// declared entry count requires (a corrupt or partial snapshot payload).
    pub fn decode(bytes: &[u8]) -> Result<Self, UpdateError> {
        if bytes.len() < 8 {
            return Err(UpdateError::Truncated);
        }
        let count = u64::from_le_bytes(bytes[0..8].try_into().expect("8 bytes")) as usize;
        let body = &bytes[8..];
        let need = count.checked_mul(16).ok_or(UpdateError::Truncated)?;
        if body.len() < need {
            return Err(UpdateError::Truncated);
        }
        let mut book = BTreeMap::new();
        for i in 0..count {
            let off = i * 16;
            let key = u64::from_le_bytes(body[off..off + 8].try_into().expect("8 bytes"));
            let bits = u64::from_le_bytes(body[off + 8..off + 16].try_into().expect("8 bytes"));
            book.insert(key, f64::from_bits(bits));
        }
        Ok(Self { book })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apply_is_deterministic_and_order_independent_for_sets() {
        let mut a = BookState::new();
        let mut b = BookState::new();
        let u1 = BookUpdate::Set {
            key: 10,
            value: 1.25,
        };
        let u2 = BookUpdate::Set {
            key: 20,
            value: -3.5,
        };
        a.apply(&u1);
        a.apply(&u2);
        // Reverse insertion order; BTreeMap canonicalizes.
        b.apply(&u2);
        b.apply(&u1);
        assert_eq!(a.to_bits(), b.to_bits());
    }

    #[test]
    fn add_stores_exact_bits() {
        let mut s = BookState::new();
        s.apply(&BookUpdate::Add { key: 1, delta: 0.1 });
        s.apply(&BookUpdate::Add { key: 1, delta: 0.2 });
        // The replicated add reproduces the same (non-exact-decimal) sum bits a
        // single node would compute — that is the point: identical ops, identical bits.
        assert_eq!(s.get(1).unwrap().to_bits(), (0.1f64 + 0.2f64).to_bits());
    }

    #[test]
    fn snapshot_codec_round_trips_bit_identically() {
        let mut s = BookState::new();
        s.apply(&BookUpdate::Add { key: 1, delta: 0.1 });
        s.apply(&BookUpdate::Add { key: 1, delta: 0.2 });
        s.apply(&BookUpdate::Set {
            key: 7,
            value: f64::from_bits(0x3ff0_0000_0000_0001), // 1.0 + 1 ULP
        });
        s.apply(&BookUpdate::Set {
            key: 3,
            value: -0.0,
        });
        s.apply(&BookUpdate::Set {
            key: 9,
            value: f64::from_bits(0x7ff8_0000_0000_0001), // a NaN bit pattern
        });
        let back = BookState::decode(&s.encode()).expect("decodes");
        // Bit-exact equality of the whole book (NaN / -0.0 / sub-ULP safe).
        assert_eq!(s.to_bits(), back.to_bits());
        // Encoding is a pure function of the state.
        assert_eq!(s.encode(), back.encode());
    }

    #[test]
    fn snapshot_decode_rejects_truncated_payload() {
        let mut s = BookState::new();
        s.apply(&BookUpdate::Set { key: 1, value: 1.0 });
        let bytes = s.encode();
        // Lopping any byte off the tail must surface Truncated, never panic.
        for cut in 0..bytes.len() {
            assert_eq!(
                BookState::decode(&bytes[..cut]),
                Err(UpdateError::Truncated)
            );
        }
    }

    #[test]
    fn empty_book_snapshot_round_trips() {
        let s = BookState::new();
        let back = BookState::decode(&s.encode()).expect("decodes empty");
        assert!(back.is_empty());
        assert_eq!(s.to_bits(), back.to_bits());
    }

    #[test]
    fn update_error_display_is_exact() {
        // Pins the Display strings (kills `UpdateError::fmt -> Ok(Default::default())`,
        // which would emit an empty string).
        assert_eq!(UpdateError::Truncated.to_string(), "book update truncated");
        assert_eq!(
            UpdateError::UnknownTag(7).to_string(),
            "book update unknown tag 7"
        );
    }

    #[test]
    fn len_is_empty_and_to_bits_reflect_contents() {
        let mut s = BookState::new();
        // Empty: len 0, is_empty true, to_bits empty. Kills `len -> 0/1`,
        // `is_empty -> true` (already true here, also checked non-empty below),
        // and `to_bits -> vec![(_,_)]` (must be EMPTY for an empty book).
        assert_eq!(s.len(), 0);
        assert!(s.is_empty());
        assert_eq!(s.to_bits(), Vec::<(u64, u64)>::new());
        // Two distinct keys with exact, recognizable bits.
        s.apply(&BookUpdate::Set {
            key: 11,
            value: 2.0,
        });
        s.apply(&BookUpdate::Set {
            key: 22,
            value: 4.0,
        });
        // len == 2 (kills `len -> 0/1`).
        assert_eq!(s.len(), 2);
        // is_empty false for a populated book (kills `is_empty -> true`).
        assert!(!s.is_empty());
        // to_bits is the exact, key-sorted (key, value-bits) pairs — kills every
        // `to_bits -> vec![(c, d)]` constant: neither key (11/22) nor the value bits
        // match any (0,0)/(0,1)/(1,0)/(1,1) constant, and the length is 2 not 1.
        assert_eq!(
            s.to_bits(),
            vec![(11u64, 2.0f64.to_bits()), (22u64, 4.0f64.to_bits())]
        );
    }

    #[test]
    fn decode_rejects_short_remove_and_short_set() {
        // A Remove tag (2) with a < 8-byte body must be Truncated (kills the
        // `rest.len() < 8` guard direction `< with >` in decode at line 99 — under
        // `>`, a 7-byte body would be (wrongly) accepted or index out of range).
        let mut short_remove = vec![2u8];
        short_remove.extend_from_slice(&[0u8; 7]); // 7 < 8
        assert_eq!(
            BookUpdate::decode(&short_remove),
            Err(UpdateError::Truncated)
        );
        // Exactly 8 bytes decodes (so the boundary is `< 8`, not `<= 8`).
        let mut exact_remove = vec![2u8];
        exact_remove.extend_from_slice(&5u64.to_le_bytes());
        assert_eq!(
            BookUpdate::decode(&exact_remove),
            Ok(BookUpdate::Remove { key: 5 })
        );
    }

    #[test]
    fn update_codec_round_trips_all_variants() {
        for u in [
            BookUpdate::Set {
                key: 7,
                value: f64::from_bits(0x7ff8_0000_0000_0001),
            },
            BookUpdate::Add {
                key: 8,
                delta: -0.0,
            },
            BookUpdate::Remove { key: 9 },
        ] {
            let back = BookUpdate::decode(&u.encode()).expect("decodes");
            // Compare by bits for the float-carrying variants (NaN/-0.0 safe).
            match (&u, &back) {
                (
                    BookUpdate::Set { key: ka, value: va },
                    BookUpdate::Set { key: kb, value: vb },
                ) => {
                    assert_eq!(ka, kb);
                    assert_eq!(va.to_bits(), vb.to_bits());
                }
                (
                    BookUpdate::Add { key: ka, delta: da },
                    BookUpdate::Add { key: kb, delta: db },
                ) => {
                    assert_eq!(ka, kb);
                    assert_eq!(da.to_bits(), db.to_bits());
                }
                (BookUpdate::Remove { key: ka }, BookUpdate::Remove { key: kb }) => {
                    assert_eq!(ka, kb)
                }
                _ => panic!("variant changed across codec"),
            }
        }
    }
}
