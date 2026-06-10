/**
 * Option on a listed future — the {@link ProductSpec} for the
 * `product.listedFutureOption` arm (proto field 31). The ticket's tenor/expiry is
 * the OPTION's; the named listed future (ticker + venue MIC) must OUTLIVE the
 * option (`futureExpiryYears >= expiryYears > 0`, validity-checked server-side),
 * which this spec guarantees BY CONSTRUCTION: the trader enters how long the
 * future settles AFTER the option (a lag ≥ 0), so the booked
 * `futureExpiryYears = tenorYears + lag` can never undercut the option at any
 * tenor selection — no stale-state validity hole. The quoted futures price
 * already embodies the underlying's carry, so every asset class prices by the
 * same futures-measure closed form; the `margining` toggle selects the premium
 * convention (equity-style upfront/discounted vs futures-style
 * daily-margined/undiscounted). Follows the {@link asianSpec} template.
 */
import type { Instrument, Margining, OptionType } from "../data/contract";
import { listedFutureOptionInstrument, type ListedFutureTerms } from "../data/seed";
import { fmtRate } from "../lib/format";
import styles from "../workspaces/TicketWorkspace.module.css";
import { defineProduct, withTenorAndModel, type InputBlockProps } from "./types";

/** The listed-future-option ticket inputs. */
export interface ListedFutureOptionInputs {
  optionType: OptionType;
  /** Strike as an absolute level; `0` ⇒ default to the ATM-forward when built. */
  strike: number;
  /** The listed future's trading symbol / ticker, e.g. "6E". */
  futureTicker: string;
  /** The listing venue / exchange MIC, e.g. "XCME"; empty when unambiguous. */
  futureVenue: string;
  /**
   * How long the future settles AFTER the option expiry (years, `≥ 0`):
   * `futureExpiryYears = tenorYears + futureLagYears`, so the future outlives
   * the option by construction (`0` ⇒ the future settles with the option).
   */
  futureLagYears: number;
  /** The premium margining convention. */
  margining: Margining;
}

/** The default listed-future-option inputs (an ATMF call on a co-terminal future). */
export const DEFAULT_LISTED_FUTURE_OPTION: ListedFutureOptionInputs = {
  optionType: "CALL",
  strike: 0,
  futureTicker: "6E",
  futureVenue: "XCME",
  futureLagYears: 0,
  margining: "EQUITY_STYLE",
};

/**
 * Build the `ListedFutureTerms` from the ticket inputs: strike ATMF-defaulted
 * (the futures price at the option expiry IS the forward), the symbol
 * trimmed/uppercased (matching the cross-asset spec's symbol hygiene), and the
 * future's expiry derived as `tenorYears + lag` (validity by construction).
 */
export function listedFutureTerms(
  v: ListedFutureOptionInputs,
  tenorYears: number,
  atmForward: number,
): ListedFutureTerms {
  return {
    futureSymbol: {
      ticker: v.futureTicker.trim().toUpperCase(),
      venue: v.futureVenue.trim().toUpperCase(),
    },
    futureExpiryYears: tenorYears + Math.max(0, v.futureLagYears),
    optionType: v.optionType,
    strike: v.strike > 0 ? v.strike : atmForward,
    margining: v.margining,
  };
}

const MARGINING_LABEL: Record<Margining, string> = {
  EQUITY_STYLE: "Equity-style (upfront premium)",
  FUTURES_STYLE: "Futures-style (daily margined)",
};

function ListedFutureOptionInputBlock({
  value,
  onChange,
  ctx,
}: InputBlockProps<ListedFutureOptionInputs>) {
  const { atmForward, tenorYears, pipDecimals } = ctx;
  const futureExpiry = tenorYears + Math.max(0, value.futureLagYears);
  return (
    <div className={styles.product}>
      <div className={styles.productRow}>
        <span className={styles.productLabel}>Future</span>
        <label className={styles.productField}>
          <span>Symbol</span>
          <input
            type="text"
            value={value.futureTicker}
            aria-label="future symbol"
            placeholder="6E"
            onChange={(ev) => onChange({ ...value, futureTicker: ev.target.value })}
          />
        </label>
        <label className={styles.productField}>
          <span>Venue</span>
          <input
            type="text"
            value={value.futureVenue}
            aria-label="future venue"
            placeholder="XCME"
            onChange={(ev) => onChange({ ...value, futureVenue: ev.target.value })}
          />
        </label>
        <label className={styles.productField}>
          <span>Settles after option</span>
          <input
            className="num"
            type="number"
            min={0}
            step={1 / 12}
            value={value.futureLagYears}
            aria-label="future lag years"
            onChange={(ev) =>
              onChange({ ...value, futureLagYears: Math.max(0, Number(ev.target.value)) })
            }
          />
          <span>{`y · future expiry ${futureExpiry.toFixed(3)}y`}</span>
        </label>
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
            step={Math.pow(10, -pipDecimals)}
            value={value.strike}
            aria-label="strike"
            placeholder={atmForward.toFixed(pipDecimals)}
            onChange={(ev) => onChange({ ...value, strike: Math.max(0, Number(ev.target.value)) })}
          />
          <span>{value.strike > 0 ? "" : `ATMF ${fmtRate(atmForward, pipDecimals)}`}</span>
        </label>
      </div>
      <div className={styles.productRow}>
        <span className={styles.productLabel}>Margining</span>
        <div className={styles.toggleGroup} role="tablist" aria-label="margining">
          {(["EQUITY_STYLE", "FUTURES_STYLE"] as Margining[]).map((mg) => (
            <button
              key={mg}
              role="tab"
              aria-selected={value.margining === mg}
              className={`${styles.modeTab} ${value.margining === mg ? styles.modeActive : ""}`}
              onClick={() => onChange({ ...value, margining: mg })}
            >
              {MARGINING_LABEL[mg]}
            </button>
          ))}
        </div>
      </div>
      <p className={styles.productNote}>
        Option on the named listed future — the quoted futures price embodies the carry, so it
        prices by the same exact futures-measure closed form for every asset class.{" "}
        {value.margining === "EQUITY_STYLE"
          ? "Equity-style: the premium is paid upfront, so the value is discounted."
          : "Futures-style: the premium is margined daily like the future, so the value is undiscounted."}{" "}
        The future settles {value.futureLagYears > 0 ? `${value.futureLagYears.toFixed(3)}y after` : "with"}{" "}
        the option, so it outlives the option by construction.
      </p>
    </div>
  );
}

/** The listed-future-option {@link ProductSpec}. */
export const listedFutureOptionSpec = defineProduct<ListedFutureOptionInputs>({
  id: "LISTED_FUTURE_OPTION",
  label: "Future option (listed)",
  group: "Vanilla & strategies",
  assetClass: "FX",
  summary:
    "Vanilla on a named listed future — futures-measure closed form, equity- or futures-style premium margining.",
  keywords: ["future", "futures", "listed", "margined", "margining", "exchange", "option on future"],
  kind: "listedFutureOption",
  defaults: DEFAULT_LISTED_FUTURE_OPTION,
  allowedModels: ["DEFAULT"],
  toInstrument: (inputs: ListedFutureOptionInputs, ctx): Instrument =>
    withTenorAndModel(
      listedFutureOptionInstrument(
        ctx.pair,
        ctx.tenorYears,
        ctx.notionalMm,
        listedFutureTerms(inputs, ctx.tenorYears, ctx.atmForward),
      ),
      ctx,
    ),
  InputBlock: ListedFutureOptionInputBlock,
});
