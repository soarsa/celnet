/**
 * Perpetual (no-expiry) American option — the {@link ProductSpec} for the
 * `product.perpetualOption` arm (proto field 30). The ONE tenorless, expiryless
 * product on the contract: the holder may exercise at any time with no terminal
 * date, so the product is American by construction (no exercise-style field) and
 * the instrument carries NO tenor and `expiryYears = 0` exactly (the server's
 * term validator rejects any other expiry on this arm as INVALID_ARGUMENT). The
 * spec declares {@link ProductSpec.noExpiry}, so the ticket shell disables the
 * expiry controls honestly instead of offering a tenor the contract cannot
 * carry. Follows the {@link asianSpec} template — an `Inputs` shape, the
 * `*Terms` transform, defaults, a `toInstrument` delegating to the `data/seed`
 * wire builder, and a self-contained `InputBlock`.
 */
import type { Instrument, OptionType } from "../data/contract";
import { perpetualInstrument, type PerpetualTerms } from "../data/seed";
import { fmtRate } from "../lib/format";
import styles from "../workspaces/TicketWorkspace.module.css";
import { defineProduct, withModel, type InputBlockProps } from "./types";

/** The perpetual-option ticket inputs. */
export interface PerpetualInputs {
  optionType: OptionType;
  /**
   * Strike as an absolute level; `0` ⇒ default to SPOT when built. (There is no
   * forward at an expiry that does not exist, so the at-the-money reference for
   * a perpetual is the spot level, not an ATM-forward.)
   */
  strike: number;
}

/** The default perpetual inputs at first render (an at-the-spot call). */
export const DEFAULT_PERPETUAL: PerpetualInputs = {
  optionType: "CALL",
  strike: 0,
};

/**
 * Build the `PerpetualTerms` from the ticket inputs, defaulting a `0` strike to
 * the SPOT level (a perpetual has no expiry ⇒ no ATM-forward to default to).
 */
export function perpetualTerms(p: PerpetualInputs, spot: number): PerpetualTerms {
  return {
    optionType: p.optionType,
    strike: p.strike > 0 ? p.strike : spot,
  };
}

function PerpetualInputBlock({ value, onChange, ctx }: InputBlockProps<PerpetualInputs>) {
  const { spot, pipDecimals } = ctx;
  return (
    <div className={styles.product}>
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
            placeholder={spot.toFixed(pipDecimals)}
            onChange={(ev) => onChange({ ...value, strike: Math.max(0, Number(ev.target.value)) })}
          />
          <span>{value.strike > 0 ? "" : `spot ${fmtRate(spot, pipDecimals)}`}</span>
        </label>
      </div>
      <p className={styles.productNote}>
        Perpetual American {value.optionType === "CALL" ? "call" : "put"}: no expiry —
        exercisable at any time, and exercise is the only way the contract ends. Priced in exact
        closed form (the value is time-homogeneous: theta is identically zero); the booked
        instrument carries no tenor and an expiry of exactly 0, the contract's canonical
        no-expiry shape. Refused under a negative discount rate, and a call is refused when the
        carry exceeds the discount rate (r_for &lt; 0) — no finite value exists in either case.
      </p>
    </div>
  );
}

/** The perpetual-option {@link ProductSpec}. */
export const perpetualSpec = defineProduct<PerpetualInputs>({
  id: "PERPETUAL",
  label: "Perpetual (no expiry)",
  group: "Path-dependent",
  assetClass: "FX",
  summary:
    "Perpetual American option — no expiry, exercisable at any time; exact free-boundary closed form.",
  keywords: ["perpetual", "no expiry", "expiryless", "american", "free boundary", "everlasting"],
  kind: "perpetualOption",
  defaults: DEFAULT_PERPETUAL,
  allowedModels: ["DEFAULT"],
  noExpiry: {
    reason:
      "A perpetual has no expiry — the contract encodes exactly 0 with no tenor; exercise is the only way it ends.",
  },
  // The one tenorless builder: no tenor stamp (withModel, not withTenorAndModel)
  // and the canonical `expiryYears = 0` from the seed builder.
  toInstrument: (inputs: PerpetualInputs, ctx): Instrument =>
    withModel(
      perpetualInstrument(ctx.pair, ctx.notionalMm, perpetualTerms(inputs, ctx.spot)),
      ctx,
    ),
  InputBlock: PerpetualInputBlock,
});
