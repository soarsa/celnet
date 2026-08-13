// ONE CONTRACT — the Excel-side decoder for the AuthService instrument
// reference-data roster (`list_instruments` / `get_instrument`), the exact
// field-for-field mirror of the server's `crates/celnet-server/src/ws/codec.rs`
// `instrument_def_to_json` (+ its `external_id_to_json` / `family_to_json`
// sub-encoders). Same single, current `celnet.wire` contract (CLAUDE.md rule 9),
// second encoding: snake_case proto field names; exactly ONE family sub-object,
// keyed by its family token (deposit / fra / stir_future / vanilla_irs / ois /
// bond), detected in the server's `family_from_json` order.
//
// Listing / get is open to any AUTHENTICATED user (curve-bootstrap + pricing
// resolve against the roster); create / update / delete are admin-only and are
// NOT part of the add-in's read-only reference surface — so there is only a
// DECODER here (no client-side `InstrumentDef` encoder: "no dead encoder",
// mirroring the read-only path). The family terms are carried VERBATIM (snake_case
// keys, unchanged values) — a lossless projection the spill renders without
// per-family boilerplate; the family token names which family a row is.

import type { WireObject } from "./wsCodec";

/** The instrument family discriminant (the wire family token). */
export type InstrumentFamily =
  | "deposit"
  | "fra"
  | "stir_future"
  | "vanilla_irs"
  | "ois"
  | "bond";

/** The family tokens in the server's `family_from_json` detection order. */
export const INSTRUMENT_FAMILIES: readonly InstrumentFamily[] = [
  "deposit",
  "fra",
  "stir_future",
  "vanilla_irs",
  "ois",
  "bond",
];

/** One external identifier on an instrument (`celnet.wire.ExternalId`). */
export interface ExternalId {
  readonly scheme: string;
  readonly value: string;
}

/**
 * An instrument definition (`celnet.wire.InstrumentDefDesc`): the base fields plus
 * the family discriminant and that family's terms. The `terms` bag preserves EVERY
 * field the server encodes for the family verbatim (snake_case keys, unchanged
 * values), so the reference-data spill renders any of the six families uniformly;
 * `family` names which one it is (`""` only for a malformed frame with no family).
 */
export interface InstrumentDef {
  readonly instrumentId: string;
  readonly name: string;
  readonly description: string;
  readonly currency: string;
  readonly externalIds: readonly ExternalId[];
  /**
   * The sub-asset-type taxonomy label (`government` / `corporate` /
   * `government_future` / `rate_future` / `swap` / `money_market`); `""` when the
   * definition carries no classification. A govvie and a corporate both arrive as
   * `family: "bond"` — this is what tells them apart.
   */
  readonly subAssetType: string;
  /** Region taxonomy label (`us` / `uk` / `de` / `fr` / `it` / `eu`); `""` when unclassified. */
  readonly region: string;
  readonly family: InstrumentFamily | "";
  readonly terms: Readonly<Record<string, unknown>>;
}

// scalar accessors (decode side) — defensive against a malformed frame.
function str(o: WireObject, key: string): string {
  const v = o[key];
  return typeof v === "string" ? v : "";
}

function array(o: WireObject, key: string): WireObject[] {
  const v = o[key];
  return Array.isArray(v) ? (v as WireObject[]) : [];
}

/** Decode one `ExternalId` `{ scheme, value }`. */
export function externalIdFromWire(o: WireObject): ExternalId {
  return { scheme: str(o, "scheme"), value: str(o, "value") };
}

/**
 * Decode an `InstrumentDefDesc` from its wire form, detecting the single family
 * sub-object exactly as the server's `family_from_json` does (deposit → fra →
 * stir_future → vanilla_irs → ois → bond, first match wins). The family's terms
 * are copied verbatim; a frame with no family (never emitted by the server)
 * decodes to `family: ""` with empty terms rather than throwing.
 */
export function instrumentDefFromWire(o: WireObject): InstrumentDef {
  let family: InstrumentFamily | "" = "";
  let terms: Record<string, unknown> = {};
  for (const f of INSTRUMENT_FAMILIES) {
    const sub = o[f];
    if (sub && typeof sub === "object" && !Array.isArray(sub)) {
      family = f;
      terms = { ...(sub as Record<string, unknown>) };
      break;
    }
  }
  return {
    instrumentId: str(o, "instrument_id"),
    name: str(o, "name"),
    description: str(o, "description"),
    currency: str(o, "currency"),
    externalIds: array(o, "external_ids").map(externalIdFromWire),
    subAssetType: str(o, "sub_asset_type"),
    region: str(o, "region"),
    family,
    terms,
  };
}

/** Decode a `list_instruments` response (`{ instruments: [...] }`, server-framed `type:"instruments"`). */
export function instrumentsResponseFromWire(o: WireObject): InstrumentDef[] {
  return array(o, "instruments").map(instrumentDefFromWire);
}

/** Decode a `get_instrument` response (`{ instrument: {...} | null }`, server-framed `type:"instrument"`). */
export function instrumentResponseFromWire(o: WireObject): InstrumentDef | null {
  const v = o["instrument"];
  if (!v || typeof v !== "object") return null;
  return instrumentDefFromWire(v as WireObject);
}
