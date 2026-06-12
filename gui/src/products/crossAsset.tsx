/**
 * Cross-asset vanilla — the GUI end of the W1 generalized `Underlying` oneof
 * (`Instrument.underlying`, proto field 1) + `Instrument.settlement_style` (field
 * 29). A single {@link ProductSpec} that books an ordinary European vanilla over a
 * NON-FX underlying: an equity (single-name / index), a commodity, or a
 * digital-asset (crypto) pair — each priced through the same `product.vanilla`
 * arm, carrying its asset-class identity on `underlying` and (for crypto) the
 * linear/inverse contract mechanics on `settlementStyle`. Follows the
 * registry-DATA pattern (`AssetClass` is now a populated dimension, not a shell
 * rewrite): the asset-class picker, the asset identity inputs, and the
 * crypto-only settlement-style selector all live in this self-contained spec.
 *
 * The wire output is byte-identical to a plain FX vanilla EXCEPT for the additive
 * `underlying` + (when INVERSE_COIN) `settlement_style` keys — LINEAR is the
 * proto3 zero value and is presence-omitted by the codec.
 */
import type {
  Instrument,
  Metal,
  OptionType,
  SettlementStyle,
  Underlying,
} from "../data/contract";
import { bookingModelsFor, crossAssetVanillaInstrument } from "../data/seed";
import styles from "../workspaces/TicketWorkspace.module.css";
import { defineProduct, withTenorAndModel, type InputBlockProps } from "./types";

/** The cross-asset class the ticket books (the populated `Underlying` arms). */
export type CrossAssetKind = "EQUITY" | "COMMODITY" | "CRYPTO" | "METAL";

/** The cross-asset vanilla ticket inputs. */
export interface CrossAssetInputs {
  /** The asset class (which `Underlying` arm is built). */
  assetKind: CrossAssetKind;
  /** Call or put on the underlying. */
  optionType: OptionType;
  /** Absolute strike (quote per 1 unit of base/asset); `0` ⇒ a placeholder ATMF hint. */
  strike: number;
  /** The asset identifier: a ticker (equity/commodity) or a coin (crypto base). */
  symbol: string;
  /** The numeraire / quote currency (equity/commodity, a 3-letter code; crypto quote). */
  currency: string;
  /** The listing venue / exchange MIC (equity/commodity); empty when unambiguous. */
  venue: string;
  /** The precious metal (METAL class only). */
  metal: Metal;
  /** Contract settlement mechanics (CRYPTO only; LINEAR for every other class). */
  settlementStyle: SettlementStyle;
}

/** The default cross-asset inputs at first render (an equity call). */
export const DEFAULT_CROSS_ASSET: CrossAssetInputs = {
  assetKind: "EQUITY",
  optionType: "CALL",
  strike: 0,
  symbol: "AAPL",
  currency: "USD",
  venue: "XNAS",
  metal: "GOLD",
  settlementStyle: "LINEAR",
};

/**
 * Build the {@link Underlying} oneof arm for the trader inputs. The crypto symbol
 * is the coin base (the quote is `currency`); the metal arm uses the selected
 * {@link Metal}; equity/commodity carry the symbol + venue + currency.
 */
export function crossAssetUnderlying(v: CrossAssetInputs): Underlying {
  const symbol = v.symbol.trim().toUpperCase();
  const currency = v.currency.trim().toUpperCase();
  const venue = v.venue.trim().toUpperCase();
  switch (v.assetKind) {
    case "EQUITY":
      return { kind: "equity", equity: { symbol: { ticker: symbol, venue }, currency }, settlementCcy: currency };
    case "COMMODITY":
      return {
        kind: "commodity",
        commodity: { symbol: { ticker: symbol, venue }, currency },
        settlementCcy: currency,
      };
    case "CRYPTO":
      return {
        kind: "digitalAsset",
        digitalAsset: { base: symbol, quote: currency },
        settlementCcy: currency,
      };
    case "METAL":
      return { kind: "metal", metal: { metal: v.metal, quote: currency }, settlementCcy: currency };
  }
}

/**
 * The settlement style the instrument carries: INVERSE_COIN is meaningful ONLY for
 * a digital-asset (crypto) underlying — every other class is forced to LINEAR so a
 * coin-margined flag never leaks onto an equity/commodity/metal contract.
 */
export function crossAssetSettlement(v: CrossAssetInputs): SettlementStyle {
  return v.assetKind === "CRYPTO" ? v.settlementStyle : "LINEAR";
}

/**
 * The INVERSE of {@link crossAssetUnderlying}: seed the ticket inputs from a
 * contract `Underlying` arm (the universe-navigator → ticket pre-target path),
 * so a selection re-uses this spec's one wire-building seam rather than
 * duplicating it. Returns `null` for the FX arm — an FX underlier never
 * re-points the ticket at the cross-asset spec (FX flows are untouched).
 * Round-trip law (gated by tests): for every non-FX `u`,
 * `crossAssetUnderlying(crossAssetInputsFor(u, s)!) === u` and
 * `crossAssetSettlement(...)` reproduces `s` (crypto) / LINEAR (other classes).
 */
export function crossAssetInputsFor(
  underlying: Underlying,
  settlementStyle: SettlementStyle,
): CrossAssetInputs | null {
  switch (underlying.kind) {
    case "fx":
      return null;
    case "metal":
      return {
        ...DEFAULT_CROSS_ASSET,
        assetKind: "METAL",
        metal: underlying.metal.metal,
        currency: underlying.metal.quote,
        settlementStyle: "LINEAR",
      };
    case "equity":
      return {
        ...DEFAULT_CROSS_ASSET,
        assetKind: "EQUITY",
        symbol: underlying.equity.symbol.ticker,
        venue: underlying.equity.symbol.venue,
        currency: underlying.equity.currency,
        settlementStyle: "LINEAR",
      };
    case "commodity":
      return {
        ...DEFAULT_CROSS_ASSET,
        assetKind: "COMMODITY",
        symbol: underlying.commodity.symbol.ticker,
        venue: underlying.commodity.symbol.venue,
        currency: underlying.commodity.currency,
        settlementStyle: "LINEAR",
      };
    case "digitalAsset":
      return {
        ...DEFAULT_CROSS_ASSET,
        assetKind: "CRYPTO",
        symbol: underlying.digitalAsset.base,
        currency: underlying.digitalAsset.quote,
        venue: "",
        settlementStyle,
      };
  }
}

const ASSET_LABEL: Record<CrossAssetKind, string> = {
  EQUITY: "Equity",
  COMMODITY: "Commodity",
  CRYPTO: "Crypto",
  METAL: "Metal",
};

function CrossAssetInputBlock({ value, onChange }: InputBlockProps<CrossAssetInputs>) {
  const isCrypto = value.assetKind === "CRYPTO";
  const isMetal = value.assetKind === "METAL";
  return (
    <div className={styles.product}>
      <div className={styles.productRow}>
        <span className={styles.productLabel}>Asset class</span>
        <div className={styles.toggleGroup} role="tablist" aria-label="asset class">
          {(["EQUITY", "COMMODITY", "CRYPTO", "METAL"] as CrossAssetKind[]).map((ak) => (
            <button
              key={ak}
              role="tab"
              aria-selected={value.assetKind === ak}
              className={`${styles.modeTab} ${value.assetKind === ak ? styles.modeActive : ""}`}
              onClick={() => onChange({ ...value, assetKind: ak })}
            >
              {ASSET_LABEL[ak]}
            </button>
          ))}
        </div>
      </div>
      <div className={styles.productRow}>
        {isMetal ? (
          <label className={styles.productField}>
            <span>Metal</span>
            <select
              aria-label="metal"
              value={value.metal}
              onChange={(ev) => onChange({ ...value, metal: ev.target.value as Metal })}
            >
              {(["GOLD", "SILVER", "PLATINUM", "PALLADIUM"] as Metal[]).map((m) => (
                <option key={m} value={m}>
                  {m}
                </option>
              ))}
            </select>
          </label>
        ) : (
          <label className={styles.productField}>
            <span>{isCrypto ? "Coin" : "Symbol"}</span>
            <input
              type="text"
              value={value.symbol}
              aria-label="symbol"
              placeholder={isCrypto ? "BTC" : "AAPL"}
              onChange={(ev) => onChange({ ...value, symbol: ev.target.value })}
            />
          </label>
        )}
        <label className={styles.productField}>
          <span>{isCrypto ? "Quote" : "Currency"}</span>
          <input
            type="text"
            value={value.currency}
            aria-label="currency"
            placeholder={isCrypto ? "USDT" : "USD"}
            onChange={(ev) => onChange({ ...value, currency: ev.target.value })}
          />
        </label>
        {!isCrypto && !isMetal && (
          <label className={styles.productField}>
            <span>Venue</span>
            <input
              type="text"
              value={value.venue}
              aria-label="venue"
              placeholder="XNAS"
              onChange={(ev) => onChange({ ...value, venue: ev.target.value })}
            />
          </label>
        )}
      </div>
      <div className={styles.productRow}>
        <span className={styles.productLabel}>Option</span>
        <div className={styles.toggleGroup} role="tablist" aria-label="option type">
          {(["CALL", "PUT"] as OptionType[]).map((ot) => (
            <button
              key={ot}
              role="tab"
              aria-selected={value.optionType === ot}
              className={`${styles.modeTab} ${value.optionType === ot ? styles.modeActive : ""}`}
              onClick={() => onChange({ ...value, optionType: ot })}
            >
              {ot === "CALL" ? "Call" : "Put"}
            </button>
          ))}
        </div>
        <label className={styles.productField}>
          <span>Strike</span>
          <input
            className="num"
            type="number"
            min={0}
            value={value.strike}
            aria-label="strike"
            onChange={(ev) => onChange({ ...value, strike: Math.max(0, Number(ev.target.value)) })}
          />
        </label>
      </div>
      {isCrypto && (
        <div className={styles.productRow}>
          <span className={styles.productLabel}>Settlement</span>
          <div className={styles.toggleGroup} role="tablist" aria-label="settlement style">
            {(["LINEAR", "INVERSE_COIN"] as SettlementStyle[]).map((ss) => (
              <button
                key={ss}
                role="tab"
                aria-selected={value.settlementStyle === ss}
                className={`${styles.modeTab} ${value.settlementStyle === ss ? styles.modeActive : ""}`}
                onClick={() => onChange({ ...value, settlementStyle: ss })}
              >
                {ss === "LINEAR" ? "Linear (stable-margined)" : "Inverse (coin-margined)"}
              </button>
            ))}
          </div>
        </div>
      )}
      <p className={styles.productNote}>
        Cross-asset vanilla over the W1 `Underlying` seam: {ASSET_LABEL[value.assetKind]} priced
        through the generalized cost-of-carry path.{" "}
        {isCrypto
          ? "INVERSE_COIN books the coin-margined 1/S_T payoff (the inverse-perpetual desk's convention); LINEAR is the ordinary stablecoin-margined contract."
          : "Quote-currency-margined (LINEAR) settlement."}
      </p>
    </div>
  );
}

/** The cross-asset vanilla {@link ProductSpec}. */
export const crossAssetSpec = defineProduct<CrossAssetInputs>({
  id: "CROSS_ASSET_VANILLA",
  label: "Cross-asset vanilla",
  group: "Cross-asset (equity / commodity / crypto)",
  assetClass: "EQUITY",
  // The cross-asset vanilla BUILDER for the three true cross-asset classes. METAL
  // is intentionally excluded: a metal underlier structures through the FX-native
  // specs (the full 24-arm set over the FX engine, XAUUSD-as-a-pair), so listing it
  // here too would duplicate the vanilla card on a metal underlier.
  applicableClasses: ["EQUITY", "COMMODITY", "CRYPTO"],
  summary:
    "European vanilla over an equity / commodity / crypto / metal underlying — the W1 cross-asset Underlying seam.",
  keywords: [
    "equity",
    "commodity",
    "crypto",
    "digital asset",
    "metal",
    "cross-asset",
    "underlying",
    "inverse",
    "coin-margined",
    "settlement style",
  ],
  kind: "vanilla",
  defaults: DEFAULT_CROSS_ASSET,
  // Booking-matrix consistency: a cross-asset vanilla books under the `vanilla`
  // product arm, so it carries exactly the vanilla booking models (no per-spec
  // drift — the registry invariant ties allowedModels to bookingModelsFor(kind)).
  // The default closed form is the generalized cost-of-carry path; the LSV route is
  // selectable identically to an FX vanilla at the contract level.
  allowedModels: bookingModelsFor("vanilla"),
  toInstrument: (inputs: CrossAssetInputs, ctx): Instrument =>
    withTenorAndModel(
      crossAssetVanillaInstrument(ctx.tenorYears, ctx.notionalMm, {
        optionType: inputs.optionType,
        strike: { kind: "strike", strike: inputs.strike > 0 ? inputs.strike : ctx.atmForward },
        underlying: crossAssetUnderlying(inputs),
        settlementStyle: crossAssetSettlement(inputs),
      }),
      ctx,
    ),
  InputBlock: CrossAssetInputBlock,
});
