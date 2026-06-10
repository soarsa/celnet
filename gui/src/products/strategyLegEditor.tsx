/**
 * The per-leg editor for the vanilla / multi-leg strategy family (the leg
 * ladder): the strike-entry grammar, the template structure laws, and the
 * editable ladder UI. The grammar matches the platform's strike vocabulary
 * (the SDK's `StrikeSpec`, the CLI's `--strike/--delta`, Excel's
 * `parseStrikeOrDelta`): an absolute level (`1.0850`), a convention delta
 * (`25dC` / `25dP`), or `ATM`. A delta-keyed strike rides the wire's
 * `StrikeOrDelta.delta` arm and is SOLVED to a level server-side under the
 * request's delta convention (the engine's delta→strike inversion); the solved
 * strike is echoed back on the quote's `resolvedStrike` — the inline strike
 * solve the ticket caption advertises.
 *
 * `ATM` enters as the signed 50Δ pillar (call `+0.5`, put `−0.5`) — exactly the
 * straddle template ladder's convention; under the unadjusted delta conventions
 * the 50Δ strike IS the delta-neutral straddle strike `F·e^{σ²T/2}`. The server
 * resolves the exact level under the request's convention set.
 */
import type { OptionType, Side, StrategyKind, StrikeOrDelta } from "../data/contract";
import styles from "../workspaces/TicketWorkspace.module.css";

/** The side an editable leg takes — a leg is bought or sold, never two-way. */
export type StrategyLegSide = Extract<Side, "BUY" | "SELL">;

/**
 * One editable leg of the vanilla / strategy ticket. `strike` is the last
 * PARSED entry — always wire-valid, so the family's `toInstrument` stays total.
 * `strikeDraft` is the raw text while the field is being edited (kept so typing
 * `1.` does not snap to a canonical `1` mid-keystroke, and so an entry that
 * does NOT parse is visible to the spec's `validate` — an unparseable draft
 * gates Request quote rather than silently pricing the previous strike). It is
 * dropped on blur (the display re-canonicalizes from the committed strike) and
 * never reaches the wire — the instrument build strips it.
 */
export interface StrategyLegInputs {
  optionType: OptionType;
  strike: StrikeOrDelta;
  side: StrategyLegSide;
  /** The leg ratio (notional weight) relative to the base notional, e.g. 1, or 2 for a 1×2. */
  ratio: number;
  /** In-progress strike text; present only while the strike field is being edited. */
  strikeDraft?: string;
}

/** The structure templates of the vanilla / strategy family. */
export type StrategyTemplate = "VANILLA" | StrategyKind;

/** Trader-facing template name, for law-violation messages. */
export function templateName(template: StrategyTemplate): string {
  switch (template) {
    case "VANILLA":
      return "vanilla";
    case "RISK_REVERSAL":
      return "risk reversal";
    case "STRANGLE":
      return "strangle";
    case "STRADDLE":
      return "straddle";
    case "SEAGULL":
      return "seagull";
  }
}

/** The leg count each template's structure law requires. */
export function templateLegCount(template: StrategyTemplate): number {
  return template === "SEAGULL" ? 3 : template === "VANILLA" ? 1 : 2;
}

/** The fewest legs the editor keeps mounted (the law flags a wrong count). */
export const MIN_STRATEGY_LEGS = 1;
/** The most legs the editor allows (one beyond the largest template, so the count law is visible). */
export const MAX_STRATEGY_LEGS = 4;

// --- strike-entry grammar ----------------------------------------------------

/** A typed strike-entry failure — every arm carries what the trader can act on. */
export type StrikeEntryError =
  | { kind: "EMPTY" }
  | { kind: "UNRECOGNIZED"; raw: string }
  | { kind: "NON_POSITIVE_LEVEL"; raw: string }
  | { kind: "DELTA_OUT_OF_RANGE"; raw: string }
  | { kind: "DELTA_LETTER_MISMATCH"; raw: string; optionType: OptionType };

/** The result of parsing a strike entry: the wire strike, or a typed error. */
export type StrikeEntry =
  | { ok: true; strike: StrikeOrDelta }
  | { ok: false; error: StrikeEntryError };

const DELTA_ENTRY = /^(\d+(?:\.\d+)?)\s*D\s*([CP])$/;

/**
 * Parse a strike entry under the platform grammar: an absolute level
 * (`"1.0850"`), a convention delta (`"25dC"` / `"25dP"`, signed call +, put −),
 * or `"ATM"` (the signed 50Δ pillar for the leg's type). A delta entry's C/P
 * letter must match the leg's own call/put — a `25dP` strike on a call leg is a
 * contradiction the server's sign discipline would reject, so it is surfaced
 * here as a typed, actionable error instead.
 */
export function parseStrikeEntry(raw: string, optionType: OptionType): StrikeEntry {
  const s = raw.trim().toUpperCase();
  if (s === "") return { ok: false, error: { kind: "EMPTY" } };
  if (s === "ATM") {
    return {
      ok: true,
      strike: { kind: "delta", delta: optionType === "CALL" ? 0.5 : -0.5 },
    };
  }
  const numeric = Number(s);
  if (Number.isFinite(numeric)) {
    if (numeric <= 0) return { ok: false, error: { kind: "NON_POSITIVE_LEVEL", raw } };
    return { ok: true, strike: { kind: "strike", strike: numeric } };
  }
  const m = DELTA_ENTRY.exec(s);
  if (!m) return { ok: false, error: { kind: "UNRECOGNIZED", raw } };
  const pct = Number(m[1]);
  if (!(pct > 0 && pct < 100)) {
    return { ok: false, error: { kind: "DELTA_OUT_OF_RANGE", raw } };
  }
  const letterType: OptionType = m[2] === "C" ? "CALL" : "PUT";
  if (letterType !== optionType) {
    return { ok: false, error: { kind: "DELTA_LETTER_MISMATCH", raw, optionType } };
  }
  const magnitude = pct / 100;
  return {
    ok: true,
    strike: { kind: "delta", delta: letterType === "CALL" ? magnitude : -magnitude },
  };
}

/** Honest trader-facing message for a strike-entry error. */
export function strikeEntryMessage(e: StrikeEntryError): string {
  switch (e.kind) {
    case "EMPTY":
      return "enter a strike — a level (1.0850), a delta (25dC / 25dP) or ATM";
    case "UNRECOGNIZED":
      return `\`${e.raw}\` is not a strike — use a level (1.0850), a delta (25dC / 25dP) or ATM`;
    case "NON_POSITIVE_LEVEL":
      return `\`${e.raw}\` — an absolute strike must be a positive level`;
    case "DELTA_OUT_OF_RANGE":
      return `\`${e.raw}\` — the delta must be strictly between 0 and 100`;
    case "DELTA_LETTER_MISMATCH": {
      const legWord = e.optionType === "CALL" ? "call" : "put";
      const wanted = e.optionType === "CALL" ? "dC" : "dP";
      return `\`${e.raw}\` keys the opposite type — this leg is a ${legWord}; use ${wanted} or toggle the leg`;
    }
  }
}

/**
 * Canonical display text for a committed strike — the inverse of
 * {@link parseStrikeEntry}: levels print as plain numbers, deltas as
 * `25dC` / `25dP`, and the signed 50Δ pillar prints as `ATM` (the pillar it is).
 */
export function formatStrikeEntry(strike: StrikeOrDelta): string {
  if (strike.kind === "strike") return String(strike.strike);
  const d = strike.delta;
  if (Math.abs(d) === 0.5) return "ATM";
  // toPrecision absorbs binary-float noise (e.g. 0.07·100 = 7.000…01 → 7).
  const pct = Number((Math.abs(d) * 100).toPrecision(12));
  return `${pct}d${d >= 0 ? "C" : "P"}`;
}

// --- template structure laws ---------------------------------------------------

/** A typed violation of the active template's structure law. */
export type LegLawViolation =
  | { law: "LEG_COUNT"; template: StrategyTemplate; required: number; actual: number }
  | { law: "OPPOSITE_TYPES"; template: StrategyTemplate }
  | { law: "OPPOSITE_SIDES"; template: StrategyTemplate }
  | { law: "SAME_SIDE"; template: StrategyTemplate }
  | { law: "SAME_STRIKE"; template: StrategyTemplate }
  | { law: "DISTINCT_STRIKES"; template: StrategyTemplate }
  | { law: "BOTH_TYPES"; template: StrategyTemplate }
  | { law: "BOTH_SIDES"; template: StrategyTemplate }
  | { law: "POSITIVE_RATIO"; legIndex: number };

/**
 * Whether two legs sit on the same strike pillar. Two absolute strikes share a
 * pillar iff the levels are equal; a call/put delta pair shares one iff both
 * sit on the signed 50Δ pillar (the one delta pillar where the call and put
 * strikes coincide — the straddle's ATM). Any other delta pair (e.g. 25dC vs
 * 25dP) resolves to two DIFFERENT levels, so it is never "the same strike".
 */
function sameStrikePillar(a: StrikeOrDelta, b: StrikeOrDelta): boolean {
  if (a.kind === "strike" && b.kind === "strike") return a.strike === b.strike;
  if (a.kind === "delta" && b.kind === "delta") {
    return Math.abs(a.delta) === 0.5 && Math.abs(b.delta) === 0.5;
  }
  return false;
}

/**
 * Validate the legs against the template's structure law:
 * - vanilla — exactly 1 leg;
 * - risk reversal — 2 legs, a call against a put, one bought one sold;
 * - strangle — 2 legs, a call and a put on the same side, at distinct strikes;
 * - straddle — 2 legs, a call and a put on the same side, sharing one strike;
 * - seagull — 3 legs mixing calls with puts and bought with sold legs;
 * - every leg's ratio (notional weight) must be positive.
 * The wire `Strategy` carries arbitrary legs and the server prices the signed
 * sum regardless of `kind`, so this law is the honest client-side gate that a
 * booked structure does not belie its declared template.
 */
export function legLawViolations(
  template: StrategyTemplate,
  legs: readonly StrategyLegInputs[],
): LegLawViolation[] {
  const out: LegLawViolation[] = [];
  legs.forEach((leg, i) => {
    if (!(leg.ratio > 0)) out.push({ law: "POSITIVE_RATIO", legIndex: i });
  });
  const required = templateLegCount(template);
  if (legs.length !== required) {
    out.push({ law: "LEG_COUNT", template, required, actual: legs.length });
    return out; // the pairwise laws only read once the count is right
  }
  if (template === "RISK_REVERSAL") {
    const [a, b] = [legs[0]!, legs[1]!];
    if (a.optionType === b.optionType) out.push({ law: "OPPOSITE_TYPES", template });
    if (a.side === b.side) out.push({ law: "OPPOSITE_SIDES", template });
  }
  if (template === "STRANGLE" || template === "STRADDLE") {
    const [a, b] = [legs[0]!, legs[1]!];
    if (a.optionType === b.optionType) out.push({ law: "OPPOSITE_TYPES", template });
    if (a.side !== b.side) out.push({ law: "SAME_SIDE", template });
    const shared = sameStrikePillar(a.strike, b.strike);
    if (template === "STRADDLE" && !shared) out.push({ law: "SAME_STRIKE", template });
    if (template === "STRANGLE" && shared) out.push({ law: "DISTINCT_STRIKES", template });
  }
  if (template === "SEAGULL") {
    if (!legs.some((l) => l.optionType === "CALL") || !legs.some((l) => l.optionType === "PUT")) {
      out.push({ law: "BOTH_TYPES", template });
    }
    if (!legs.some((l) => l.side === "BUY") || !legs.some((l) => l.side === "SELL")) {
      out.push({ law: "BOTH_SIDES", template });
    }
  }
  return out;
}

/**
 * The strike entries currently drafted that do NOT parse, per leg. The shell
 * gates Request quote on these alongside the structure law (a visible bad
 * entry must never silently price the previously committed strike); blur drops
 * the draft, so an abandoned entry stops gating once its text is gone.
 */
export function strikeEntryViolations(
  legs: readonly StrategyLegInputs[],
): { legIndex: number; error: StrikeEntryError }[] {
  const out: { legIndex: number; error: StrikeEntryError }[] = [];
  legs.forEach((leg, i) => {
    if (leg.strikeDraft === undefined) return;
    const parsed = parseStrikeEntry(leg.strikeDraft, leg.optionType);
    if (!parsed.ok) out.push({ legIndex: i, error: parsed.error });
  });
  return out;
}

/** Honest trader-facing message for a structure-law violation. */
export function legLawMessage(v: LegLawViolation): string {
  switch (v.law) {
    case "LEG_COUNT": {
      const diff = v.actual - v.required;
      const fix =
        diff > 0 ? `remove ${diff} leg${diff > 1 ? "s" : ""}` : `add ${-diff} leg${diff < -1 ? "s" : ""}`;
      return `a ${templateName(v.template)} carries exactly ${v.required} leg${v.required > 1 ? "s" : ""} (${v.actual} now) — ${fix}`;
    }
    case "OPPOSITE_TYPES":
      return `a ${templateName(v.template)} pairs a call against a put — toggle one leg's type`;
    case "OPPOSITE_SIDES":
      return "a risk reversal buys one leg and sells the other — flip one side";
    case "SAME_SIDE":
      return `a ${templateName(v.template)} holds both legs on the same side — buy both or sell both`;
    case "SAME_STRIKE":
      return "a straddle's legs share one strike — set both to the same level, or both ATM";
    case "DISTINCT_STRIKES":
      return "a strangle's legs sit at distinct strikes (one shared strike is a straddle) — move a wing";
    case "BOTH_TYPES":
      return "a seagull mixes calls and puts — include at least one of each";
    case "BOTH_SIDES":
      return "a seagull finances bought legs with sold legs — include a buy and a sell";
    case "POSITIVE_RATIO":
      return `leg ${v.legIndex + 1}: the ratio (notional weight) must be positive`;
  }
}

// --- the editable leg ladder -------------------------------------------------

/** A fresh leg appended by "+ Add leg" (a bought 25Δ call, the template seed). */
function freshLeg(): StrategyLegInputs {
  return { optionType: "CALL", strike: { kind: "delta", delta: 0.25 }, side: "BUY", ratio: 1 };
}

export interface StrategyLegEditorProps {
  template: StrategyTemplate;
  legs: readonly StrategyLegInputs[];
  /** Replace the committed legs (the shell owns the per-family state). */
  onChange: (legs: StrategyLegInputs[]) => void;
}

/**
 * The editable leg ladder (stateless — the shell owns the per-family inputs).
 * Strike text is drafted per leg in the inputs (`strikeDraft`) and the parsed
 * strike committed alongside whenever the text parses under the grammar, so the
 * committed strike is always wire-valid and `toInstrument` stays total; a draft
 * that does not parse shows its typed error inline AND surfaces through the
 * spec's `validate`, gating Request quote. Structure-law violations render
 * beneath the ladder — the shell reads the same law and gates on it too.
 */
export function StrategyLegEditor({ template, legs, onChange }: StrategyLegEditorProps) {
  const isVanilla = template === "VANILLA";

  const replaceLeg = (i: number, leg: StrategyLegInputs): void => {
    onChange(legs.map((l, k) => (k === i ? leg : l)));
  };
  const editStrike = (i: number, raw: string): void => {
    const leg = legs[i]!;
    const parsed = parseStrikeEntry(raw, leg.optionType);
    replaceLeg(i, {
      ...leg,
      strike: parsed.ok ? parsed.strike : leg.strike,
      strikeDraft: raw,
    });
  };
  const settleStrike = (i: number): void => {
    // Blur: drop the draft so the display re-canonicalizes from the committed
    // strike (an abandoned bad entry visibly snaps back, never lingers).
    const leg = legs[i]!;
    if (leg.strikeDraft === undefined) return;
    const { strikeDraft: _dropped, ...settled } = leg;
    replaceLeg(i, settled);
  };
  const toggleType = (i: number, ot: OptionType): void => {
    const leg = legs[i]!;
    if (leg.optionType === ot) return;
    // A delta strike keeps its pillar and re-signs to the new type's discipline
    // (call +, put −); the draft drops so the display letter follows.
    const strike: StrikeOrDelta =
      leg.strike.kind === "delta"
        ? {
            kind: "delta",
            delta: ot === "CALL" ? Math.abs(leg.strike.delta) : -Math.abs(leg.strike.delta),
          }
        : leg.strike;
    const { strikeDraft: _dropped, ...rest } = leg;
    replaceLeg(i, { ...rest, optionType: ot, strike });
  };
  const addLeg = (): void => {
    if (legs.length >= MAX_STRATEGY_LEGS) return;
    onChange([...legs, freshLeg()]);
  };
  const removeLeg = (i: number): void => {
    if (legs.length <= MIN_STRATEGY_LEGS) return;
    onChange(legs.filter((_, k) => k !== i));
  };

  const violations = legLawViolations(template, legs);

  return (
    <>
      <ul className={styles.legs} role="list" aria-label="strategy legs">
        {legs.map((leg, i) => {
          const draft = leg.strikeDraft;
          const entry = draft !== undefined ? parseStrikeEntry(draft, leg.optionType) : undefined;
          const entryError = entry && !entry.ok ? entry.error : undefined;
          return (
            <li className={styles.legStack} key={i} role="listitem">
              <div className={styles.leg}>
                <span className={styles.legNo}>LEG {i + 1}</span>
                {!isVanilla && (
                  <div
                    className={styles.toggleGroup}
                    role="tablist"
                    aria-label={`leg ${i + 1} side`}
                  >
                    {(["BUY", "SELL"] as StrategyLegSide[]).map((side) => (
                      <button
                        key={side}
                        role="tab"
                        aria-selected={leg.side === side}
                        className={`${styles.modeTab} ${leg.side === side ? styles.modeActive : ""} ${
                          leg.side === side ? (side === "SELL" ? styles.sell : styles.buy) : ""
                        }`}
                        onClick={() => replaceLeg(i, { ...leg, side })}
                      >
                        {side === "BUY" ? "Buy" : "Sell"}
                      </button>
                    ))}
                  </div>
                )}
                <div
                  className={styles.toggleGroup}
                  role="tablist"
                  aria-label={`leg ${i + 1} option type`}
                >
                  {(["CALL", "PUT"] as OptionType[]).map((ot) => (
                    <button
                      key={ot}
                      role="tab"
                      aria-selected={leg.optionType === ot}
                      className={`${styles.modeTab} ${leg.optionType === ot ? styles.modeActive : ""}`}
                      onClick={() => toggleType(i, ot)}
                    >
                      {ot === "CALL" ? "Call" : "Put"}
                    </button>
                  ))}
                </div>
                <label className={styles.productField}>
                  <span>K</span>
                  <input
                    className="num"
                    type="text"
                    value={draft ?? formatStrikeEntry(leg.strike)}
                    aria-label={`leg ${i + 1} strike`}
                    aria-invalid={entryError !== undefined}
                    placeholder="1.0850 · 25dC · ATM"
                    onChange={(ev) => editStrike(i, ev.target.value)}
                    onBlur={() => settleStrike(i)}
                  />
                </label>
                {!isVanilla && (
                  <label className={styles.productField}>
                    <span>×</span>
                    <input
                      className="num"
                      type="number"
                      min={0}
                      step={0.25}
                      value={leg.ratio}
                      aria-label={`leg ${i + 1} ratio`}
                      onChange={(ev) => {
                        const n = Number(ev.target.value);
                        replaceLeg(i, { ...leg, ratio: Number.isFinite(n) ? n : 0 });
                      }}
                    />
                  </label>
                )}
                {!isVanilla && (
                  <button
                    className={styles.basketLegRemove}
                    aria-label={`remove leg ${i + 1}`}
                    disabled={legs.length <= MIN_STRATEGY_LEGS}
                    onClick={() => removeLeg(i)}
                  >
                    Remove
                  </button>
                )}
              </div>
              {entryError && (
                <span className={styles.legEntryError}>{strikeEntryMessage(entryError)}</span>
              )}
            </li>
          );
        })}
      </ul>
      {!isVanilla && (
        <button
          className={styles.basketAddLeg}
          aria-label="add leg"
          disabled={legs.length >= MAX_STRATEGY_LEGS}
          onClick={addLeg}
        >
          + Add leg
        </button>
      )}
      {violations.length > 0 && (
        <ul className={styles.legLawList} role="list" aria-label="structure law violations">
          {violations.map((v, k) => (
            <li key={k} className={styles.legLawItem}>
              {legLawMessage(v)}
            </li>
          ))}
        </ul>
      )}
    </>
  );
}
