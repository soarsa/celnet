/**
 * transferModel — the pure, client-side model behind the FI Risk Transfer ticket
 * (docs/RISK-TRANSFER-REQUIREMENTS.md §9.1). No React, no transport — so the ticket
 * and its tests share ONE source of truth for:
 *
 *   • SYNTHESISED position lines. The mock (and, on a real backend, the FI seam) has
 *     no per-position store for FI risk — risk is per-portfolio synthetic + routed /
 *     transfer overlays. So a portfolio's selectable "positions" are DERIVED here,
 *     deterministically, by splitting its live {@link RiskBookRisk} row (net notional
 *     + risk vector) into N labelled lots with stable bigint ids. Those ids ride into
 *     `TransferLeg.positionIds`. This is ILLUSTRATIVE: on a real backend the position
 *     lines are server-authoritative; the moved economics are driven by
 *     `quantityFull` / `partialNotional` (the mock's risk math uses the notional, not
 *     the per-line vectors), so the split is faithful in aggregate.
 *   • The `effectiveDeskId` of a portfolio (walk the parent chain — a sub-book with a
 *     null desk inherits its ancestor's desk, e.g. `fx-emea-vanilla` → `emea`).
 *   • KIND inference from the target selection (same desk ⇒ re-attribution; other
 *     desk ⇒ desk-to-desk; a named trader ⇒ trader-to-trader).
 *   • The before/after PREVIEW math (source risk ↓, target risk ↑, realised P&L in the
 *     source at the transfer price) — computed for DISPLAY, server-authoritative on
 *     submit.
 */

import type { RiskBook, RiskBookRisk, RiskVector, TransferKind } from "../../data/contract";

/**
 * The illustrative par mark the mid / mark-to-market basis prices a transfer at. On a
 * real backend this is the live composite mark; the offline mock crosses at par
 * (`MOCK_TRANSFER_MARK = 100`), so a MID/MARK transfer realises zero P&L in the source
 * and an AGREED off-mark cross realises `(price − 100) · notional/100` — control-visible.
 */
export const PAR_MARK = 100;

/** The maximum number of synthetic lots a portfolio's risk row is split into. */
const MAX_LOTS = 4;

/** A synthesised, selectable position line derived from a portfolio's live risk row. */
export interface PositionLine {
  /** Stable synthetic id (deterministic from the book id + lot index) → `positionIds`. */
  id: bigint;
  /** Human label, e.g. "EMEA Rates — lot 2/3". */
  label: string;
  /** The signed base-currency notional this lot carries (+ long / − short). */
  notionalBase: number;
  /** This lot's slice of the portfolio's pass-through risk vector. */
  risk: RiskVector;
}

/** A small, deterministic 63-bit hash of a string → a stable synthetic position id. */
function hashToBigInt(s: string): bigint {
  let h = 1469598103934665603n; // FNV-1a 64-bit offset basis
  for (let i = 0; i < s.length; i += 1) {
    h ^= BigInt(s.charCodeAt(i));
    h = (h * 1099511628211n) & 0x7fffffffffffffffn; // FNV prime, kept in the i64 domain
  }
  // Never 0 — a 0 id reads as "unset" downstream.
  return h === 0n ? 1n : h;
}

/** A deterministic weight in [1, 2) for lot `i` of `bookId` (stable across renders). */
function lotWeight(bookId: string, i: number): number {
  const h = Number(hashToBigInt(`${bookId}#${i}`) % 1000n);
  return 1 + h / 1000;
}

/**
 * Split a portfolio's live risk row into deterministic, labelled position lots whose
 * notionals + risk vectors SUM to the row's totals (so selecting every lot reproduces
 * the book). The lot count tracks the row's `positionCount`, capped at {@link MAX_LOTS}.
 */
export function synthesizePositions(row: RiskBookRisk): PositionLine[] {
  const n = Math.min(MAX_LOTS, Math.max(1, row.positionCount));
  const rowDv01 = row.dv01 ?? 0;
  const weights: number[] = [];
  let sumW = 0;
  for (let i = 0; i < n; i += 1) {
    const w = lotWeight(row.bookId, i);
    weights.push(w);
    sumW += w;
  }
  const lines: PositionLine[] = [];
  for (let i = 0; i < n; i += 1) {
    const frac = weights[i]! / sumW;
    lines.push({
      id: hashToBigInt(`${row.bookId}#lot#${i}`),
      label: `${row.name} — lot ${i + 1}/${n}`,
      notionalBase: row.netNotional * frac,
      risk: {
        dv01: rowDv01 * frac,
        delta: row.delta * frac,
        gamma: row.gamma * frac,
        vega: row.vega * frac,
        theta: row.theta * frac,
      },
    });
  }
  return lines;
}

/**
 * The desk that effectively owns a portfolio: its own `deskId`, or — when null — the
 * nearest ancestor's desk (a sub-book inherits its parent's desk). `""` when no
 * ancestor carries a desk. Guards against a cyclic parent chain by bounding the walk.
 */
export function effectiveDeskId(bookId: string, books: readonly RiskBook[]): string {
  const byId = new Map(books.map((b) => [b.id, b]));
  let cur = byId.get(bookId);
  let guard = 0;
  while (cur && guard < books.length + 1) {
    if (cur.deskId !== null && cur.deskId.length > 0) return cur.deskId;
    cur = cur.parentId !== null ? byId.get(cur.parentId) : undefined;
    guard += 1;
  }
  return "";
}

/**
 * Infer the transfer KIND from the target selection (the taxonomy of §4): a named
 * target trader ⇒ a hand-off (`TRADER_TO_TRADER`); otherwise same desk as source ⇒ a
 * light `RE_ATTRIBUTE` (books immediately, no counterparty), a different desk ⇒ an
 * economic `DESK_TO_DESK` cross (lands Pending for four-eyes acceptance).
 */
export function inferKind(
  sourceDesk: string,
  targetDesk: string,
  targetTrader: string,
): TransferKind {
  if (targetTrader.trim().length > 0) return "TRADER_TO_TRADER";
  return sourceDesk === targetDesk ? "RE_ATTRIBUTE" : "DESK_TO_DESK";
}

/** One-line plain-English gloss of what a kind does (shown under the inferred badge). */
export function kindMeaning(kind: TransferKind): string {
  switch (kind) {
    case "RE_ATTRIBUTE":
      return "Re-attribution within one desk — economics unchanged, books immediately (no counterparty).";
    case "DESK_TO_DESK":
      return "Economic cross to another desk — realises P&L in the source, opens in the target; lands Pending for four-eyes acceptance.";
    case "TRADER_TO_TRADER":
      return "Hand-off to another trader — lands Pending; the recipient must accept before it books.";
  }
}

/** The zero risk vector (a dense numeric vector — 0, never null, when irrelevant). */
export const ZERO_RISK: RiskVector = { dv01: 0, delta: 0, gamma: 0, vega: 0, theta: 0 };

/** Sum a set of position lines into one aggregate notional + risk vector. */
export function aggregateLines(lines: readonly PositionLine[]): {
  notionalBase: number;
  risk: RiskVector;
} {
  let notionalBase = 0;
  const risk: RiskVector = { ...ZERO_RISK };
  for (const l of lines) {
    notionalBase += l.notionalBase;
    risk.dv01 += l.risk.dv01;
    risk.delta += l.risk.delta;
    risk.gamma += l.risk.gamma;
    risk.vega += l.risk.vega;
    risk.theta += l.risk.theta;
  }
  return { notionalBase, risk };
}

/**
 * The signed notional a transfer moves out of a source book whose current net is
 * `bookNet` — Full moves the whole net, Partial the requested magnitude in the net's
 * direction, bounded to it. Mirrors the mock's `sliceSourceRisk`, so the inbox preview
 * matches what the accept actually books. Illustrative on a real backend.
 */
export function movedNotionalOf(
  bookNet: number,
  quantityFull: boolean,
  partialNotional: number | null,
): number {
  const dir = bookNet < 0 ? -1 : 1;
  return quantityFull ? bookNet : dir * Math.min(Math.abs(partialNotional ?? 0), Math.abs(bookNet));
}

/** The before/after economics a transfer previews (client-side; server-authoritative on submit). */
export interface TransferPreview {
  /** The signed base-currency notional that moves out of source and into target. */
  movedNotional: number;
  /** The fraction-scaled risk vector that moves. */
  movedRisk: RiskVector;
  /** P&L crystallised in the source at the transfer price vs the par mark. */
  realizedPnlSource: number;
  /** The numeric transfer price the legs book at. */
  transferPrice: number;
  /** The source portfolio's net notional AFTER the move. */
  sourceNetAfter: number;
  /** The target portfolio's net notional AFTER the move. */
  targetNetAfter: number;
}

/**
 * Compute the before/after preview from the SELECTED lines + the ticket inputs. Full
 * moves the whole selection; Partial moves the requested magnitude in the selection's
 * net direction, bounded to it. The price is the par mark for MID / MARK-to-market, the
 * agreed override for AGREED. Realised P&L is zero at the mark, non-zero only off-mark.
 */
export function computePreview(args: {
  selected: readonly PositionLine[];
  quantityFull: boolean;
  partialNotional: number | null;
  agreedPrice: number | null;
  isAgreed: boolean;
  sourceNet: number;
  targetNet: number;
}): TransferPreview {
  const { notionalBase: selTotal, risk: selRisk } = aggregateLines(args.selected);
  const dir = selTotal < 0 ? -1 : 1;
  const movedNotional = args.quantityFull
    ? selTotal
    : dir * Math.min(Math.abs(args.partialNotional ?? 0), Math.abs(selTotal));
  const frac = selTotal !== 0 ? movedNotional / selTotal : 0;
  const movedRisk: RiskVector = {
    dv01: selRisk.dv01 * frac,
    delta: selRisk.delta * frac,
    gamma: selRisk.gamma * frac,
    vega: selRisk.vega * frac,
    theta: selRisk.theta * frac,
  };
  const transferPrice = args.isAgreed ? (args.agreedPrice ?? PAR_MARK) : PAR_MARK;
  const realizedPnlSource = ((transferPrice - PAR_MARK) * movedNotional) / 100;
  return {
    movedNotional,
    movedRisk,
    realizedPnlSource,
    transferPrice,
    sourceNetAfter: args.sourceNet - movedNotional,
    targetNetAfter: args.targetNet + movedNotional,
  };
}
