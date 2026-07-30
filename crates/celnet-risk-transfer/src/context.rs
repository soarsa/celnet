//! The validation context — the small, server-populated input that keeps this
//! crate server-free.
//!
//! The server builds these maps from its position store (which position is in
//! which book, with what signed notional) and its identity registry (which
//! books are enabled and which desk owns each). [`crate::check_transfer`] reads
//! only these maps, so the crate never links the server.

use std::collections::HashMap;

/// A position as the store currently holds it.
#[derive(Debug, Clone, PartialEq)]
pub struct PositionRef {
    /// The book the position is currently stamped into.
    pub risk_book_id: String,
    /// The position's signed base-currency notional (+ long, − short).
    pub signed_notional: f64,
}

/// A book as the identity registry currently holds it.
#[derive(Debug, Clone, PartialEq)]
pub struct BookRef {
    /// The desk that owns the book.
    pub desk_id: String,
    /// Whether the book is enabled (only enabled books are valid transfer
    /// ends).
    pub enabled: bool,
}

/// The full read snapshot [`crate::check_transfer`] validates against.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TransferContext {
    /// Every selectable position keyed by id.
    pub positions: HashMap<u64, PositionRef>,
    /// Every book keyed by id.
    pub books: HashMap<String, BookRef>,
}

impl TransferContext {
    /// A convenience builder from the two maps.
    #[must_use]
    pub fn new(positions: HashMap<u64, PositionRef>, books: HashMap<String, BookRef>) -> Self {
        Self { positions, books }
    }
}
