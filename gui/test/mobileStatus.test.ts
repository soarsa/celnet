/**
 * mobileStatus + useMobileLayout — the PURE logic behind the mobile read-only status
 * board (`app/mobile/MobileStatusApp`): which asset tabs an identity may see, how the
 * firm-wide risk/deal/hedge rows split by asset class, the per-portfolio RAG folds and
 * worst-first ordering, the glance summary strip, and the viewport/pointer detection
 * rule that decides mobile-vs-desktop.
 *
 * These are DOM-free unit tests over the real modules. Two invariants matter most and
 * are covered explicitly:
 *   • a `null` DV01 is NEVER folded into a fabricated 0 — it stays `null` (rendered as
 *     an em dash) unless at least one portfolio genuinely reports one;
 *   • a BREACHED cap reports `fraction === +Infinity`; the "worst utilisation %" strip
 *     must stay FINITE (the breach is carried by the band/breach count instead).
 */
import { describe, expect, it } from "vitest";

import {
  bandSeverity,
  bookAssetIndex,
  dealsForAsset,
  entitledAssets,
  hedgesForAsset,
  MOBILE_ASSET_TABS,
  MOBILE_SUB_VIEWS,
  ragBandOfFraction,
  riskCards,
  riskRowsForAsset,
  summariseRisk,
  worstBandOf,
  worstFractionOf,
  type CanPredicate,
} from "../src/lib/mobileStatus";
import {
  computeIsMobile,
  COARSE_POINTER_MAX_WIDTH,
  MOBILE_MAX_WIDTH,
} from "../src/hooks/useMobileLayout";
import { DEFAULT_USD_SOFR_CURVE } from "../src/data/ratesPricing";
import type {
  CapabilityAsset,
  Deal,
  HedgeProvenance,
  OisInstrument,
  RagBand,
  RiskBookRisk,
  RiskLimitUtilization,
} from "../src/data/contract";

/** A genuine OIS instrument (a numeric fixed-leg tenor ⇒ fixed_income). */
function ois(tenorYears = 5): OisInstrument {
  return { tenorYears, fixedRate: 0.04, notional: 50_000_000, direction: "PAY_FIXED" };
}

/**
 * A crafted NON-OIS instrument — no numeric `tenorYears` fixed leg, the shape the
 * asset discriminator classifies as fx_options. Cast through `unknown` because the
 * desk/rates streams are structurally typed OIS-only on this contract.
 */
const nonOis = { fixedRate: 0.04, notional: 1_000_000 } as unknown as OisInstrument;

function deal(instrument: OisInstrument, dealId: string, riskBookId?: string): Deal {
  return {
    dealId,
    requestId: `req-${dealId}`,
    kind: "RFQ",
    counterparty: "Acme",
    desk: "g10-rates",
    productKind: "OIS",
    instrument,
    curveSet: DEFAULT_USD_SOFR_CURVE,
    side: "BUY",
    notional: 10_000_000,
    price: 0.04,
    executedAtNanos: 1n,
    trader: "T",
    ...(riskBookId === undefined ? {} : { riskBookId }),
  };
}

function util(
  metric: string,
  used: number,
  limit: number,
  fraction: number,
  band: RagBand,
): RiskLimitUtilization {
  return { metric, used, limit, fraction, band };
}

function riskRow(bookId: string, over: Partial<RiskBookRisk> = {}): RiskBookRisk {
  return {
    bookId,
    name: bookId.toUpperCase(),
    netNotional: 0,
    grossNotional: 0,
    positionCount: 0,
    delta: 0,
    gamma: 0,
    vega: 0,
    theta: 0,
    dv01: null,
    pnl: null,
    limits: [],
    ...over,
  };
}

function hedge(hedgeId: string): HedgeProvenance {
  return {
    hedgeId,
    book: "rates-core",
    instrument: "OIS 10Y",
    firedAt: 1_700_000_000_000,
    metric: "net_dv01" as HedgeProvenance["metric"],
    threshold: 100,
    netRisk: -140,
    utilization: 1.4,
    band: "red",
    policyPath: [0, 2],
    action: null,
    internalCrossed: 40,
    externalHedged: 60,
    residual: 40,
    hedgePrice: 0.0412,
  } as HedgeProvenance;
}

/** A `can` predicate that grants `view` on exactly the listed assets. */
function canView(...assets: CapabilityAsset[]): CanPredicate {
  return (action, asset) => action === "view" && assets.includes(asset);
}

describe("entitledAssets — the asset tabs this identity may see", () => {
  it("returns both tabs in display order (Fixed Income first) when both are granted", () => {
    expect(entitledAssets(canView("fixed_income", "fx_options"))).toEqual([
      "fixed_income",
      "fx_options",
    ]);
  });

  it("returns only the granted asset when the identity is scoped to one", () => {
    expect(entitledAssets(canView("fixed_income"))).toEqual(["fixed_income"]);
    expect(entitledAssets(canView("fx_options"))).toEqual(["fx_options"]);
  });

  it("returns no tabs when the identity may view neither asset", () => {
    expect(entitledAssets(canView())).toEqual([]);
  });

  it("gates on the VIEW action specifically (a non-view grant is not a tab)", () => {
    const canOnlyManage: CanPredicate = (action) => action !== "view";
    expect(entitledAssets(canOnlyManage)).toEqual([]);
  });

  it("exposes exactly the two asset tabs and three sub-views the board renders", () => {
    expect(MOBILE_ASSET_TABS.map((t) => t.id)).toEqual(["fixed_income", "fx_options"]);
    expect(MOBILE_SUB_VIEWS.map((v) => v.id)).toEqual(["risk", "hedge", "client"]);
  });
});

describe("bookAssetIndex — portfolio id → the asset classes routed into it", () => {
  it("tags a portfolio with the asset class of every deal routed into it", () => {
    const index = bookAssetIndex([deal(ois(10), "d1", "rates-core"), deal(nonOis, "d2", "fx-core")]);
    expect(index.get("rates-core")).toEqual(new Set(["fixed_income"]));
    expect(index.get("fx-core")).toEqual(new Set(["fx_options"]));
  });

  it("accumulates BOTH assets when a portfolio took mixed routed flow", () => {
    const index = bookAssetIndex([deal(ois(5), "d1", "mixed"), deal(nonOis, "d2", "mixed")]);
    expect(index.get("mixed")).toEqual(new Set(["fixed_income", "fx_options"]));
  });

  it("skips deals that never routed (absent or empty riskBookId)", () => {
    const index = bookAssetIndex([deal(ois(5), "d1"), deal(ois(5), "d2", "")]);
    expect(index.size).toBe(0);
  });
});

describe("riskRowsForAsset — which portfolios show under an asset tab", () => {
  const rows = [riskRow("rates-core"), riskRow("fx-core"), riskRow("fresh")];
  const index = bookAssetIndex([deal(ois(10), "d1", "rates-core"), deal(nonOis, "d2", "fx-core")]);

  it("shows a tagged portfolio under exactly the assets it took flow in", () => {
    expect(riskRowsForAsset(rows, index, "fixed_income").map((r) => r.bookId)).toEqual([
      "rates-core",
      "fresh",
    ]);
    expect(riskRowsForAsset(rows, index, "fx_options").map((r) => r.bookId)).toEqual(["fx-core"]);
  });

  it("shows an UNCLASSIFIED portfolio under fixed_income only — never a fabricated FX exposure", () => {
    const fresh = [riskRow("fresh")];
    expect(riskRowsForAsset(fresh, new Map(), "fixed_income")).toHaveLength(1);
    expect(riskRowsForAsset(fresh, new Map(), "fx_options")).toHaveLength(0);
  });

  it("shows a mixed-flow portfolio under BOTH tabs", () => {
    const mixedIndex = bookAssetIndex([deal(ois(5), "d1", "mixed"), deal(nonOis, "d2", "mixed")]);
    const mixed = [riskRow("mixed")];
    expect(riskRowsForAsset(mixed, mixedIndex, "fixed_income")).toHaveLength(1);
    expect(riskRowsForAsset(mixed, mixedIndex, "fx_options")).toHaveLength(1);
  });
});

describe("dealsForAsset / hedgesForAsset — the blotter splits", () => {
  it("splits client deals by dealt instrument family", () => {
    const deals = [deal(ois(10), "d1"), deal(nonOis, "d2")];
    expect(dealsForAsset(deals, "fixed_income").map((d) => d.dealId)).toEqual(["d1"]);
    expect(dealsForAsset(deals, "fx_options").map((d) => d.dealId)).toEqual(["d2"]);
  });

  it("shows the fired-hedge audit trail under fixed_income and honestly EMPTY under FX", () => {
    const hedges = [hedge("h1"), hedge("h2")];
    expect(hedgesForAsset(hedges, "fixed_income").map((h) => h.hedgeId)).toEqual(["h1", "h2"]);
    expect(hedgesForAsset(hedges, "fx_options")).toEqual([]);
  });

  it("does not alias the caller's hedge array (the FI split is a copy)", () => {
    const hedges = [hedge("h1")];
    const split = hedgesForAsset(hedges, "fixed_income");
    expect(split).not.toBe(hedges);
    expect(split).toEqual(hedges);
  });
});

describe("RAG folds — band from fraction, worst band, worst FINITE fraction", () => {
  it("mirrors the server band thresholds (>=1 red, >=0.8 amber, else green)", () => {
    expect(ragBandOfFraction(0)).toBe("green");
    expect(ragBandOfFraction(0.799)).toBe("green");
    expect(ragBandOfFraction(0.8)).toBe("amber");
    expect(ragBandOfFraction(0.999)).toBe("amber");
    expect(ragBandOfFraction(1)).toBe("red");
    expect(ragBandOfFraction(Number.POSITIVE_INFINITY)).toBe("red");
  });

  it("orders severity red > amber > green", () => {
    expect(bandSeverity("red")).toBeGreaterThan(bandSeverity("amber"));
    expect(bandSeverity("amber")).toBeGreaterThan(bandSeverity("green"));
  });

  it("takes the WORST band across a portfolio's caps", () => {
    expect(worstBandOf([])).toBe("green");
    expect(worstBandOf([util("net", 1, 10, 0.1, "green")])).toBe("green");
    expect(worstBandOf([util("net", 9, 10, 0.9, "amber"), util("gross", 1, 10, 0.1, "green")])).toBe(
      "amber",
    );
    expect(worstBandOf([util("net", 9, 10, 0.9, "amber"), util("gross", 11, 10, 1.1, "red")])).toBe(
      "red",
    );
  });

  it("keeps the worst utilisation FINITE when a cap is breached (+Infinity fraction)", () => {
    const breached = [
      util("net_notional", 5_000_000, 0, Number.POSITIVE_INFINITY, "red"),
      util("gross_notional", 9, 10, 0.9, "amber"),
    ];
    expect(worstFractionOf(breached)).toBe(0.9);
    expect(Number.isFinite(worstFractionOf(breached))).toBe(true);
    // The breach is still carried — by the BAND, not by an infinite percentage.
    expect(worstBandOf(breached)).toBe("red");
  });

  it("reports 0 when there are no caps at all", () => {
    expect(worstFractionOf([])).toBe(0);
  });
});

describe("riskCards — worst-first ordering", () => {
  it("sorts breaching portfolios first, then by utilisation, then by gross notional", () => {
    const rows = [
      riskRow("calm", { grossNotional: 10, limits: [util("net", 1, 10, 0.1, "green")] }),
      riskRow("breach", { grossNotional: 1, limits: [util("net", 11, 10, 1.1, "red")] }),
      riskRow("watch", { grossNotional: 5, limits: [util("net", 9, 10, 0.9, "amber")] }),
    ];
    expect(riskCards(rows).map((c) => c.row.bookId)).toEqual(["breach", "watch", "calm"]);
  });

  it("breaks a band+utilisation tie by the larger gross notional", () => {
    const rows = [
      riskRow("small", { grossNotional: 1, limits: [util("net", 5, 10, 0.5, "green")] }),
      riskRow("big", { grossNotional: 999, limits: [util("net", 5, 10, 0.5, "green")] }),
    ];
    expect(riskCards(rows).map((c) => c.row.bookId)).toEqual(["big", "small"]);
  });

  it("marks the breaching card and carries its folded band + worst fraction", () => {
    const cards = riskCards([riskRow("breach", { limits: [util("net", 11, 10, 1.1, "red")] })]);
    expect(cards).toHaveLength(1);
    const card = cards[0]!;
    expect(card.breaching).toBe(true);
    expect(card.band).toBe("red");
    expect(card.worstFraction).toBeCloseTo(1.1);
  });

  it("does not mutate the caller's row array", () => {
    const rows = [
      riskRow("calm", { limits: [util("net", 1, 10, 0.1, "green")] }),
      riskRow("breach", { limits: [util("net", 11, 10, 1.1, "red")] }),
    ];
    riskCards(rows);
    expect(rows.map((r) => r.bookId)).toEqual(["calm", "breach"]);
  });
});

describe("summariseRisk — the 2-second glance strip", () => {
  it("sums net/gross notional and counts the portfolios shown", () => {
    const summary = summariseRisk([
      riskRow("a", { netNotional: 100, grossNotional: 300 }),
      riskRow("b", { netNotional: -40, grossNotional: 120 }),
    ]);
    expect(summary.bookCount).toBe(2);
    expect(summary.netNotional).toBe(60);
    expect(summary.grossNotional).toBe(420);
  });

  it("keeps net DV01 null when NO portfolio reports one — never a fabricated 0", () => {
    const summary = summariseRisk([riskRow("a"), riskRow("b")]);
    expect(summary.netDv01).toBeNull();
  });

  it("sums only the portfolios that genuinely report a DV01", () => {
    const summary = summariseRisk([
      riskRow("a", { dv01: 1_250 }),
      riskRow("b", { dv01: null }),
      riskRow("c", { dv01: -250 }),
    ]);
    expect(summary.netDv01).toBe(1_000);
  });

  it("counts breaching portfolios and takes the worst band across them all", () => {
    const summary = summariseRisk([
      riskRow("a", { limits: [util("net", 1, 10, 0.1, "green")] }),
      riskRow("b", { limits: [util("net", 9, 10, 0.9, "amber")] }),
      riskRow("c", { limits: [util("net", 11, 10, 1.1, "red")] }),
    ]);
    expect(summary.breaching).toBe(1);
    expect(summary.worstBand).toBe("red");
    expect(summary.worstFraction).toBeCloseTo(1.1);
  });

  it("stays finite (and still red) when a portfolio breached a ZERO cap", () => {
    const summary = summariseRisk([
      riskRow("z", {
        limits: [util("net_notional", 5_000_000, 0, Number.POSITIVE_INFINITY, "red")],
      }),
    ]);
    expect(Number.isFinite(summary.worstFraction)).toBe(true);
    expect(summary.worstFraction).toBe(0);
    expect(summary.worstBand).toBe("red");
    expect(summary.breaching).toBe(1);
  });

  it("folds an empty asset tab into an honest zeroed summary with a null DV01", () => {
    const summary = summariseRisk([]);
    expect(summary).toEqual({
      bookCount: 0,
      netNotional: 0,
      grossNotional: 0,
      netDv01: null,
      breaching: 0,
      worstFraction: 0,
      worstBand: "green",
    });
  });
});

describe("computeIsMobile — the viewport/pointer detection rule", () => {
  it("treats any narrow viewport as mobile regardless of pointer", () => {
    expect(computeIsMobile(390, true)).toBe(true);
    expect(computeIsMobile(390, false)).toBe(true);
    expect(computeIsMobile(MOBILE_MAX_WIDTH, false)).toBe(true);
  });

  it("treats a COARSE pointer on a not-wide viewport as mobile (phone in landscape)", () => {
    expect(computeIsMobile(MOBILE_MAX_WIDTH + 1, true)).toBe(true);
    expect(computeIsMobile(COARSE_POINTER_MAX_WIDTH, true)).toBe(true);
  });

  it("keeps a FINE-pointer viewport above the phone width on the desktop Shell", () => {
    expect(computeIsMobile(MOBILE_MAX_WIDTH + 1, false)).toBe(false);
    expect(computeIsMobile(1440, false)).toBe(false);
  });

  it("keeps a WIDE desktop touchscreen on the desktop Shell", () => {
    expect(computeIsMobile(COARSE_POINTER_MAX_WIDTH + 1, true)).toBe(false);
    expect(computeIsMobile(1920, true)).toBe(false);
  });
});
