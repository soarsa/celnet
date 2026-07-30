//! Invariant validation — the §5.1 rules, one typed error per failure.

use crate::context::TransferContext;
use crate::error::TransferError;
use crate::transfer::{RiskTransfer, TransferKind, TransferPrice, TransferQuantity};

/// Validate a transfer against the current store/registry snapshot.
///
/// Checks, in order: a non-empty selection; enabled source/target books that
/// resolve to the legs' desks; that the two ends are distinct books; that every
/// selected position is present and currently in the source book;
/// kind ↔ desk/trader consistency; the partial-quantity bounds; and the
/// agreed-price-needs-reason rule. Returns the first failure.
///
/// Pure: reads only the passed [`RiskTransfer`] and [`TransferContext`].
pub fn check_transfer(transfer: &RiskTransfer, ctx: &TransferContext) -> Result<(), TransferError> {
    let source = &transfer.source;
    let target = &transfer.target;

    // 1. Non-empty selection.
    if source.position_ids.is_empty() {
        return Err(TransferError::EmptyPositionSet);
    }

    // 2. Source book: exists, enabled, owned by the leg's desk.
    let source_book = ctx
        .books
        .get(&source.risk_book_id)
        .ok_or_else(|| TransferError::SourceBookNotFound(source.risk_book_id.clone()))?;
    if !source_book.enabled {
        return Err(TransferError::SourceBookDisabled(
            source.risk_book_id.clone(),
        ));
    }
    if source_book.desk_id != source.desk_id {
        return Err(TransferError::SourceDeskMismatch {
            book: source.risk_book_id.clone(),
            book_desk: source_book.desk_id.clone(),
            leg_desk: source.desk_id.clone(),
        });
    }

    // 3. Target book: exists, enabled, owned by the leg's desk.
    let target_book = ctx
        .books
        .get(&target.risk_book_id)
        .ok_or_else(|| TransferError::TargetBookNotFound(target.risk_book_id.clone()))?;
    if !target_book.enabled {
        return Err(TransferError::TargetBookDisabled(
            target.risk_book_id.clone(),
        ));
    }
    if target_book.desk_id != target.desk_id {
        return Err(TransferError::TargetDeskMismatch {
            book: target.risk_book_id.clone(),
            book_desk: target_book.desk_id.clone(),
            leg_desk: target.desk_id.clone(),
        });
    }

    // 4. Distinct ends.
    if source.risk_book_id == target.risk_book_id {
        return Err(TransferError::SameSourceTargetBook(
            source.risk_book_id.clone(),
        ));
    }

    // 5. Every selected position is present and currently in the source book.
    let mut aggregate_notional = 0.0f64;
    for &pid in &source.position_ids {
        let pref = ctx
            .positions
            .get(&pid)
            .ok_or(TransferError::PositionNotFound(pid))?;
        if pref.risk_book_id != source.risk_book_id {
            return Err(TransferError::PositionNotInSourceBook {
                position_id: pid,
                actual_book: pref.risk_book_id.clone(),
                source_book: source.risk_book_id.clone(),
            });
        }
        aggregate_notional += pref.signed_notional;
    }

    // 6. Kind ↔ desk/trader consistency.
    match transfer.kind {
        TransferKind::ReAttribute => {
            if source.desk_id != target.desk_id {
                return Err(TransferError::ReAttributeCrossDesk {
                    source_desk: source.desk_id.clone(),
                    target_desk: target.desk_id.clone(),
                });
            }
        }
        TransferKind::DeskToDesk => {
            if source.desk_id == target.desk_id {
                return Err(TransferError::DeskToDeskSameDesk(source.desk_id.clone()));
            }
        }
        TransferKind::TraderToTrader => {
            if source.trader == target.trader {
                return Err(TransferError::TraderToTraderSameTrader(
                    source.trader.clone(),
                ));
            }
        }
    }

    // 7. Partial-quantity bounds: 0 < q ≤ |aggregate signed notional|.
    if let TransferQuantity::Partial(q) = transfer.quantity {
        if !(q.is_finite() && q > 0.0) {
            return Err(TransferError::NonPositivePartial(q));
        }
        let available = aggregate_notional.abs();
        if q > available {
            return Err(TransferError::PartialExceedsNotional {
                requested: q,
                available,
            });
        }
    }

    // 8. Agreed price requires a reason.
    if let TransferPrice::Agreed(_) = transfer.price
        && transfer.reason.trim().is_empty()
    {
        return Err(TransferError::AgreedPriceRequiresReason);
    }

    Ok(())
}
