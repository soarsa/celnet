/**
 * assetClass — the ONE reliable per-row asset-class discriminator that drives the
 * Book workspace's hard asset-vertical separation (matching the fe-fi separation
 * already applied to Market Data / Risk / Quotes). Every Book sub-lens
 * (Positions & Booking / Aggregate Risk / Quotes / Deals) must show ONLY the
 * ACTIVE domain's asset class; these helpers map (a) the active {@link Domain} tab
 * to its {@link CapabilityAsset}, and (b) a data ROW to the asset class it belongs
 * to, so a lens can drop rows that do not belong to the current domain.
 *
 * The reliable discriminator is the INSTRUMENT FAMILY — not the desk / counterparty
 * name (which is cosmetic and unreliable). A row that carries an OIS instrument
 * (i.e. a `tenorYears` fixed leg) is `fixed_income`; anything else is `fx_options`.
 *
 * The desk-quoting and rates-book streams (`DeskRequest`, `Deal`, `RatesPosition`)
 * are structurally OIS-only under the one contract (the P0 rates arm), so every row
 * they carry classifies as `fixed_income`. That is WHY, under the FX Options domain,
 * the Quotes / Deals / Positions lenses are correctly EMPTY (there is no FX-option
 * desk-quoting or FX positions stream on this contract), while the Aggregate Risk
 * lens shows the FX options book. Under Fixed Income those three lenses show their
 * rates rows and the Aggregate Risk lens shows the rates netted-risk panel.
 */

import type { CapabilityAsset, Deal, DeskRequest, RatesPosition } from "./contract";
import type { Domain } from "../lib/commands";

/**
 * The asset class an active {@link Domain} tab scopes the Book to. The `admin`
 * pseudo-domain carries no asset class, so it falls back to `fx_options` (the Book's
 * default lens is the FX options book) — an admin viewing the Book sees the FX side.
 */
export function capabilityAssetForDomain(domain: Domain): CapabilityAsset {
  return domain === "fixed_income" ? "fixed_income" : "fx_options";
}

/**
 * Classify an instrument by FAMILY: an OIS (rates) instrument carries a numeric
 * `tenorYears` fixed leg and is `fixed_income`; anything else is `fx_options`. The
 * structural check (not a name match) is the reliable per-row discriminator.
 */
export function instrumentAsset(instrument: { tenorYears?: number }): CapabilityAsset {
  return typeof instrument.tenorYears === "number" ? "fixed_income" : "fx_options";
}

/** The asset class of a shown-quote / desk request (by its priced instrument). */
export function deskRequestAsset(request: DeskRequest): CapabilityAsset {
  return instrumentAsset(request.instrument);
}

/** The asset class of a booked deal (by its dealt instrument). */
export function dealAsset(deal: Deal): CapabilityAsset {
  return instrumentAsset(deal.instrument);
}

/** The asset class of a booked position (by its instrument). */
export function ratesPositionAsset(position: RatesPosition): CapabilityAsset {
  return instrumentAsset(position.instrument);
}
