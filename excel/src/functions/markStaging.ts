/**
 * Two-phase, idempotent mark staging for CELNET.MARK (docs §3.3 / §4).
 *
 * A worksheet function that mutates server state on every recalculation is a
 * footgun. `CELNET.MARK` therefore STAGES a contribution keyed by a deterministic
 * idempotency key and renders PENDING; the actual `mark_surface` write commits
 * only when the trader confirms in the task-pane Contribute panel. A recalc with
 * unchanged args re-stages under the SAME key — the staging table dedupes, so a
 * thousand recalcs produce one pending contribution, and the server's own dedupe
 * makes the eventual commit at-most-once (determinism + idempotency).
 *
 * The staging table is process-global (module state) so the task pane and the
 * functions runtime see the same pending set. The key is a stable hash of
 * (pair, tenor, pillar, vol, sessionEpoch); the sessionEpoch ties a key to this
 * workbook session so a re-open re-stages cleanly rather than colliding.
 */

import type { Connection } from "../transport/connection";
import {
  parsePair,
  parseSmileModel,
  parseStrikeOrDelta,
  parseTenor,
  ShapingError,
  type MarkStatus,
} from "./shaping";
import type { SmileModel } from "../contract/contract";

/** A staged (not-yet-committed) contribution awaiting task-pane confirmation. */
export interface StagedMark {
  readonly stagingId: string;
  readonly pair: string;
  readonly tenor: string;
  readonly pillar: string;
  readonly vol: number;
  /** The smile-calibration model the eventual commit selects (default VV). */
  readonly model: SmileModel;
  readonly comment: string;
  status: "PENDING" | "COMMITTED" | "REJECTED";
  surfaceVersionAfter?: bigint;
  detail: string;
}

/** The arguments a CELNET.MARK cell stages. */
export interface MarkArgs {
  readonly pair: string;
  readonly tenor: string;
  readonly pillar: string;
  readonly vol: number;
  readonly comment: string;
  /** Optional smile-calibration model selector (VV/SABR/SVI/SSVI); default VV. */
  readonly model?: string | undefined;
}

/** This workbook session's epoch, mixed into every idempotency key. */
const SESSION_EPOCH = Date.now().toString(36);

/** The process-global staging table, keyed by deterministic idempotency key. */
const staged = new Map<string, StagedMark>();

/**
 * A stable, collision-resistant key over the contribution identity + session.
 * A 53-bit FNV-1a fold is sufficient to dedupe a workbook's contributions; the
 * server applies its own authoritative dedupe on commit.
 */
export function idempotencyKey(args: MarkArgs): string {
  const canonical = [
    args.pair.trim().toUpperCase(),
    args.tenor.trim().toUpperCase(),
    args.pillar.trim().toUpperCase(),
    args.vol,
    parseSmileModel(args.model),
    SESSION_EPOCH,
  ].join("|");
  let h = 0xcbf29ce484222325n;
  const prime = 0x100000001b3n;
  const mask = (1n << 64n) - 1n;
  for (let i = 0; i < canonical.length; i++) {
    h = (h ^ BigInt(canonical.charCodeAt(i))) & mask;
    h = (h * prime) & mask;
  }
  return `mark-${h.toString(36)}`;
}

/**
 * Stage a contribution (validating the arguments shape against the contract).
 * Re-staging identical args returns the SAME pending entry (idempotent). NEVER
 * writes to the server here — the commit is the task-pane confirmation step.
 */
export async function stageMark(_conn: Connection, args: MarkArgs): Promise<MarkStatus> {
  // Validate the arguments parse against the contract vocabulary up front, so a
  // malformed mark fails fast in the cell rather than at commit.
  parsePair(args.pair);
  parseTenor(args.tenor);
  parseStrikeOrDelta(args.pillar);
  const model = parseSmileModel(args.model);
  if (!Number.isFinite(args.vol) || args.vol <= 0 || args.vol >= 5) {
    throw new ShapingError(`invalid vol \`${args.vol}\` (absolute, e.g. 0.102)`);
  }
  const key = idempotencyKey(args);
  const existing = staged.get(key);
  if (existing) {
    return {
      status: existing.status,
      stagingId: existing.stagingId,
      surfaceVersionAfter: existing.surfaceVersionAfter,
      detail: existing.detail,
    };
  }
  const entry: StagedMark = {
    stagingId: key,
    pair: args.pair,
    tenor: args.tenor,
    pillar: args.pillar,
    vol: args.vol,
    model,
    comment: args.comment,
    status: "PENDING",
    detail: "staged — confirm in the task pane to contribute",
  };
  staged.set(key, entry);
  return {
    status: "PENDING",
    stagingId: key,
    surfaceVersionAfter: undefined,
    detail: entry.detail,
  };
}

/** The currently-staged contributions (for the task-pane Contribute panel). */
export function pendingMarks(): readonly StagedMark[] {
  return [...staged.values()].filter((m) => m.status === "PENDING");
}

/** Look up a staged contribution by its staging id. */
export function getStaged(stagingId: string): StagedMark | undefined {
  return staged.get(stagingId);
}

/**
 * Mark a staged contribution committed (called by the task pane after the server
 * `mark_surface` confirms), recording the resulting surface_version + audit id.
 */
export function markCommitted(stagingId: string, surfaceVersionAfter: bigint, detail: string): void {
  const entry = staged.get(stagingId);
  if (!entry) return;
  entry.status = "COMMITTED";
  entry.surfaceVersionAfter = surfaceVersionAfter;
  entry.detail = detail;
}

/** Mark a staged contribution rejected (e.g. convention mismatch on normalize). */
export function markRejected(stagingId: string, detail: string): void {
  const entry = staged.get(stagingId);
  if (!entry) return;
  entry.status = "REJECTED";
  entry.detail = detail;
}

/** Clear all staging state (test isolation / workbook teardown). */
export function clearStaging(): void {
  staged.clear();
}
