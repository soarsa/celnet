/**
 * Deterministic seed data for the NON-FX underlier universes the scope drill
 * navigates (book → asset class → underlier), alongside `seed.ts`'s `PAIRS`
 * (the FX universe). Same honesty contract as `PAIRS` (GUIDE.md rule 2): every
 * row is TODAY'S seeded universe — plausible, clearly-labeled indicative levels,
 * never a live mark — and the shapes are the real contract `Underlying` arms, so
 * an estate market-data feed replaces this module with zero downstream rework.
 *
 * Classes seeded here:
 *   • METAL  — the four precious metals (XAU/XAG/XPT/XPD) vs the fiat majors
 *     (the metal-vs-fiat crosses the conventions registry carries, e.g. XAUEUR/
 *     XAUJPY/XPTEUR; metal-vs-metal ratios are NOT priceable and not seeded).
 *     Each row projects onto a pair-style row via `underlyingPairProjection`
 *     (the metal leg becomes its ISO-4217 X-code, byte-identical to the FX
 *     `CcyPair` encoding).
 *   • EQUITY — a handful of index / single names with listing venue (MIC) + ccy.
 *   • COMMODITY — seeded futures-style symbols with venue + ccy.
 *   • CRYPTO — BTC/ETH vs USD/USDT. The USD-quoted rows seed the coin-margined
 *     INVERSE convention (the inverse-perpetual desk's 1/S_T payoff); the
 *     USDT-quoted rows seed the ordinary stable-margined LINEAR contract — the
 *     `Instrument.settlementStyle` mechanics, carried per row so a selection
 *     pre-targets the ticket with the right convention.
 */

import type { Metal, SettlementStyle, Underlying } from "./contract";

/** One seeded non-FX underlier: its contract identity + indicative display data. */
export interface UnderlierSeed {
  /** The contract `Underlying` arm (the asset-class identity the wire carries). */
  underlying: Underlying;
  /** Seeded indicative reference level (quote units per 1 base/asset unit). */
  refLevel: number;
  /** Display decimal places for the reference level. */
  decimals: number;
  /**
   * Contract settlement mechanics a ticket pre-target seeds
   * (INVERSE_COIN is meaningful only for the digital-asset rows).
   */
  settlementStyle: SettlementStyle;
  /** A short detail string (venue · ccy, or the crypto settlement note). */
  detail: string;
}

/** Build a precious-metal underlier row (`Underlying.metal`). */
function metal(
  m: Metal,
  quote: string,
  refLevel: number,
  decimals: number,
): UnderlierSeed {
  return {
    underlying: { kind: "metal", metal: { metal: m, quote }, settlementCcy: quote },
    refLevel,
    decimals,
    settlementStyle: "LINEAR",
    detail: "spot",
  };
}

/** Build an equity underlier row (`Underlying.equity`). */
function equity(
  ticker: string,
  venue: string,
  currency: string,
  refLevel: number,
  decimals: number,
): UnderlierSeed {
  return {
    underlying: {
      kind: "equity",
      equity: { symbol: { ticker, venue }, currency },
      settlementCcy: currency,
    },
    refLevel,
    decimals,
    settlementStyle: "LINEAR",
    detail: `${venue} · ${currency}`,
  };
}

/** Build a commodity underlier row (`Underlying.commodity`). */
function commodity(
  ticker: string,
  venue: string,
  currency: string,
  refLevel: number,
  decimals: number,
): UnderlierSeed {
  return {
    underlying: {
      kind: "commodity",
      commodity: { symbol: { ticker, venue }, currency },
      settlementCcy: currency,
    },
    refLevel,
    decimals,
    settlementStyle: "LINEAR",
    detail: `${venue} · ${currency}`,
  };
}

/** Build a digital-asset underlier row (`Underlying.digitalAsset`). */
function crypto(
  base: string,
  quote: string,
  refLevel: number,
  decimals: number,
  settlementStyle: SettlementStyle,
): UnderlierSeed {
  return {
    underlying: { kind: "digitalAsset", digitalAsset: { base, quote }, settlementCcy: quote },
    refLevel,
    decimals,
    settlementStyle,
    detail: settlementStyle === "INVERSE_COIN" ? "inverse · coin-margined" : "linear · stable-margined",
  };
}

/**
 * The seeded non-FX underlier universe, in source order (the FX universe is
 * `seed.ts`'s `PAIRS`). Honest scope: today's seeded set only — the future estate
 * market-data feed drops a larger list in here with zero downstream rework.
 */
export const ASSET_UNDERLIERS: UnderlierSeed[] = [
  // METAL — the four precious metals vs the fiat majors…
  metal("GOLD", "USD", 2331.4, 2),
  metal("GOLD", "EUR", 2165.1, 2),
  metal("GOLD", "JPY", 364710, 0),
  metal("GOLD", "GBP", 1834.2, 2),
  metal("GOLD", "CHF", 2102.7, 2),
  metal("GOLD", "AUD", 3512.5, 2),
  metal("SILVER", "USD", 29.46, 3),
  metal("SILVER", "EUR", 27.36, 3),
  metal("PLATINUM", "USD", 1012.8, 2),
  metal("PLATINUM", "EUR", 941.2, 2),
  metal("PALLADIUM", "USD", 968.3, 2),
  // NOTE: metal-vs-METAL ratios (XAU/XAG, XPT/XPD) are deliberately NOT seeded —
  // the conventions registry (and `MetalPair.quote: Ccy`, a FIAT leg) carries
  // metal-vs-fiat only ("metal base, loco-London, premium in the fiat"); a ratio
  // row here would navigate to an unpriceable underlier (honesty rule).
  // EQUITY — index + single names, with the listing venue MIC + quote ccy.
  equity("SPX", "XCBO", "USD", 5312.7, 1),
  equity("SX5E", "XEUR", "EUR", 4982.4, 1),
  equity("N225", "XOSE", "JPY", 38940, 0),
  equity("AAPL", "XNAS", "USD", 192.35, 2),
  equity("MSFT", "XNAS", "USD", 428.74, 2),
  equity("NVDA", "XNAS", "USD", 1136.2, 2),
  equity("ASML", "XAMS", "EUR", 871.6, 2),
  // COMMODITY — futures-style symbols with venue + ccy.
  commodity("BRENT", "IFEU", "USD", 82.41, 2),
  commodity("WTI", "XNYM", "USD", 78.12, 2),
  commodity("NATGAS", "XNYM", "USD", 2.87, 3),
  commodity("COPPER", "XLME", "USD", 9847, 0),
  commodity("WHEAT", "XCBT", "USD", 6.84, 2),
  // CRYPTO — BTC/ETH vs USD (inverse, coin-margined) and USDT (linear).
  crypto("BTC", "USD", 68420, 0, "INVERSE_COIN"),
  crypto("BTC", "USDT", 68455, 0, "LINEAR"),
  crypto("ETH", "USD", 3724.5, 1, "INVERSE_COIN"),
  crypto("ETH", "USDT", 3726.8, 1, "LINEAR"),
];
