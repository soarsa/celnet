/**
 * assetClass — the per-row asset-class discriminator that drives the Book
 * workspace's hard asset-vertical separation (the fix that stopped Fixed-Income
 * OIS rows leaking into the FX Options domain's Book lenses).
 *
 * These are pure-function unit tests: the domain→asset map, the instrument-family
 * discriminator, and the three row classifiers (`DeskRequest` / `Deal` /
 * `RatesPosition`). The reliable discriminator is the INSTRUMENT FAMILY — an OIS
 * fixed-leg tenor (`tenorYears`) is fixed_income; anything else is fx_options — so
 * a mixed set classifies each row independently (not by desk / counterparty name).
 */
import { describe, expect, it } from "vitest";

import {
  capabilityAssetForDomain,
  dealAsset,
  deskRequestAsset,
  instrumentAsset,
  ratesPositionAsset,
} from "../src/data/assetClass";
import { DEFAULT_USD_SOFR_CURVE } from "../src/data/ratesPricing";
import type { Deal, DeskRequest, OisInstrument, RatesPosition } from "../src/data/contract";

/** A genuine OIS instrument (a numeric fixed-leg tenor ⇒ fixed_income). */
function ois(tenorYears = 5): OisInstrument {
  return { tenorYears, fixedRate: 0.04, notional: 50_000_000, direction: "PAY_FIXED" };
}

/**
 * A crafted NON-OIS instrument — no numeric `tenorYears` fixed leg (the shape a
 * would-be FX-option desk row carries). The classifier keys on the OIS fixed-leg
 * tenor, so a row lacking it is fx_options. Cast through `unknown` because the
 * desk/rates streams are structurally typed OIS-only on this contract.
 */
const nonOis = { fixedRate: 0.04, notional: 1_000_000 } as unknown as OisInstrument;

function deskRequest(instrument: OisInstrument, counterparty = "Acme"): DeskRequest {
  return {
    requestId: `req-${counterparty}`,
    kind: "RFQ",
    counterparty,
    desk: "g10-rates",
    instrument,
    curveSet: DEFAULT_USD_SOFR_CURVE,
    side: "BUY",
    notional: 10_000_000,
    receivedAtNanos: 1n,
    expiresAtNanos: 2n,
    state: "QUOTED",
    quote: { price: 0.04, notional: 10_000_000, validForMs: 30_000, trader: "T" },
  };
}

function deal(instrument: OisInstrument, counterparty = "Acme"): Deal {
  return {
    dealId: `deal-${counterparty}`,
    requestId: `req-${counterparty}`,
    kind: "RFQ",
    counterparty,
    desk: "g10-rates",
    productKind: "OIS",
    instrument,
    curveSet: DEFAULT_USD_SOFR_CURVE,
    side: "BUY",
    notional: 10_000_000,
    price: 0.04,
    executedAtNanos: 1n,
    trader: "T",
  };
}

function ratesPosition(instrument: OisInstrument, positionId = 1n): RatesPosition {
  return { positionId, entity: 1, book: 10, instrument };
}

describe("capabilityAssetForDomain — the active-domain → asset-class map", () => {
  it("maps the fixed_income domain to the fixed_income asset class", () => {
    expect(capabilityAssetForDomain("fixed_income")).toBe("fixed_income");
  });

  it("maps the fx_options domain to the fx_options asset class", () => {
    expect(capabilityAssetForDomain("fx_options")).toBe("fx_options");
  });

  it("maps the admin pseudo-domain to fx_options (the Book's default lens)", () => {
    expect(capabilityAssetForDomain("admin")).toBe("fx_options");
  });
});

describe("instrumentAsset — the reliable instrument-family discriminator", () => {
  it("classifies an OIS instrument (numeric tenorYears) as fixed_income", () => {
    expect(instrumentAsset(ois(10))).toBe("fixed_income");
    expect(instrumentAsset({ tenorYears: 2 })).toBe("fixed_income");
  });

  it("classifies an instrument with no tenorYears as fx_options", () => {
    expect(instrumentAsset({})).toBe("fx_options");
    expect(instrumentAsset(nonOis)).toBe("fx_options");
  });
});

describe("row classifiers split a mixed set by instrument family", () => {
  it("deskRequestAsset: OIS ⇒ fixed_income, non-OIS ⇒ fx_options", () => {
    expect(deskRequestAsset(deskRequest(ois(5), "Meridian"))).toBe("fixed_income");
    expect(deskRequestAsset(deskRequest(nonOis, "FxDesk"))).toBe("fx_options");

    const mixed = [deskRequest(ois(5), "Meridian"), deskRequest(nonOis, "FxDesk")];
    expect(mixed.map(deskRequestAsset)).toEqual(["fixed_income", "fx_options"]);
    // The filter each Book lens applies keeps only the active domain's asset.
    expect(mixed.filter((r) => deskRequestAsset(r) === "fixed_income")).toHaveLength(1);
    expect(mixed.filter((r) => deskRequestAsset(r) === "fx_options")).toHaveLength(1);
  });

  it("dealAsset: OIS ⇒ fixed_income, non-OIS ⇒ fx_options", () => {
    expect(dealAsset(deal(ois(10), "Northwind"))).toBe("fixed_income");
    expect(dealAsset(deal(nonOis, "FxDesk"))).toBe("fx_options");

    const mixed = [deal(ois(10), "Northwind"), deal(nonOis, "FxDesk")];
    expect(mixed.map(dealAsset)).toEqual(["fixed_income", "fx_options"]);
  });

  it("ratesPositionAsset: OIS ⇒ fixed_income, non-OIS ⇒ fx_options", () => {
    expect(ratesPositionAsset(ratesPosition(ois(2), 1n))).toBe("fixed_income");
    expect(ratesPositionAsset(ratesPosition(nonOis, 2n))).toBe("fx_options");

    const mixed = [ratesPosition(ois(2), 1n), ratesPosition(nonOis, 2n)];
    expect(mixed.map(ratesPositionAsset)).toEqual(["fixed_income", "fx_options"]);
  });
});
