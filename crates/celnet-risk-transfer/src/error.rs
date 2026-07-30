//! The typed validation error — one variant per §5.1 invariant failure.

use thiserror::Error;

/// A reason a [`crate::RiskTransfer`] failed [`crate::check_transfer`].
///
/// One variant per invariant so the server can map each precisely to a wire
/// status and a trader-facing message.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum TransferError {
    /// The source leg selected no positions.
    #[error("transfer selects no positions")]
    EmptyPositionSet,

    /// A selected position id is unknown to the context.
    #[error("position {0} not found")]
    PositionNotFound(u64),

    /// A selected position is not currently in the source book.
    #[error("position {position_id} is in book {actual_book}, not the source book {source_book}")]
    PositionNotInSourceBook {
        /// The offending position.
        position_id: u64,
        /// The book the position is actually in.
        actual_book: String,
        /// The source book the transfer claims it is in.
        source_book: String,
    },

    /// The source book does not exist in the registry.
    #[error("source book {0} not found")]
    SourceBookNotFound(String),

    /// The source book is disabled and cannot be transferred out of.
    #[error("source book {0} is disabled")]
    SourceBookDisabled(String),

    /// The source leg's desk does not match the source book's owning desk.
    #[error("source book {book} is owned by desk {book_desk}, not the leg's desk {leg_desk}")]
    SourceDeskMismatch {
        /// The source book.
        book: String,
        /// The desk that actually owns the book.
        book_desk: String,
        /// The desk the source leg claims.
        leg_desk: String,
    },

    /// The target book does not exist in the registry.
    #[error("target book {0} not found")]
    TargetBookNotFound(String),

    /// The target book is disabled and cannot receive a transfer.
    #[error("target book {0} is disabled")]
    TargetBookDisabled(String),

    /// The target leg's desk does not match the target book's owning desk.
    #[error("target book {book} is owned by desk {book_desk}, not the leg's desk {leg_desk}")]
    TargetDeskMismatch {
        /// The target book.
        book: String,
        /// The desk that actually owns the book.
        book_desk: String,
        /// The desk the target leg claims.
        leg_desk: String,
    },

    /// Source and target books are the same — nothing would move.
    #[error("source and target are the same book {0}")]
    SameSourceTargetBook(String),

    /// A [`crate::TransferKind::ReAttribute`] must stay within one desk.
    #[error(
        "re-attribution crosses desks (source {source_desk} → target {target_desk}); \
         use a desk-to-desk transfer"
    )]
    ReAttributeCrossDesk {
        /// The source desk.
        source_desk: String,
        /// The target desk.
        target_desk: String,
    },

    /// A [`crate::TransferKind::DeskToDesk`] must cross desks.
    #[error("desk-to-desk transfer stays within one desk {0}; use a re-attribution")]
    DeskToDeskSameDesk(String),

    /// A [`crate::TransferKind::TraderToTrader`] must hand off to a different
    /// trader.
    #[error("trader-to-trader transfer hands off to the same trader {0}")]
    TraderToTraderSameTrader(String),

    /// A `Partial` quantity was zero, negative, or non-finite.
    #[error("partial quantity {0} must be > 0 and finite")]
    NonPositivePartial(f64),

    /// A `Partial` quantity exceeds the available aggregate notional.
    #[error("partial quantity {requested} exceeds available notional {available}")]
    PartialExceedsNotional {
        /// The requested magnitude.
        requested: f64,
        /// The available (absolute aggregate) notional.
        available: f64,
    },

    /// An agreed transfer price was given without a rationale.
    #[error("an agreed transfer price requires a non-empty reason")]
    AgreedPriceRequiresReason,
}
