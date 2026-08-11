/**
 * ReferenceDataForms — the family-aware create / edit form for an instrument
 * definition, plus the per-family default factories.
 *
 * The form holds a single working `InstrumentDef` draft. Picking a family from
 * the family `<select>` rebuilds the family sub-object with that family's
 * defaults (preserving the base fields), so only the chosen family's fields are
 * ever rendered or submitted. Vocabulary `<select>`s are driven by the contract's
 * label arrays; bond zero-coupon disables (and zeroes) the coupon inputs. On
 * submit the draft IS the `InstrumentInput` (blank `instrumentId` on create ⇒ the
 * server mints one from the name; carried on edit).
 */

import { useState } from "react";

import { Button } from "../components/Button";
import {
  BOND_DAY_COUNTS,
  BUSINESS_DAY_CONVENTIONS,
  CALENDARS,
  COUPON_TYPES,
  EXTERNAL_ID_SCHEMES,
  FREQUENCIES,
  INSTRUMENT_FAMILIES,
  INSTRUMENT_FAMILY_LABELS,
  RATES_DAY_COUNTS,
  ROLL_CONVENTIONS,
  type BondDef,
  type BrokenDate,
  type BusinessDayConvention,
  type Calendar,
  type CouponType,
  type DepositDef,
  type ExternalIdEntry,
  type ExternalIdScheme,
  type FraDef,
  type Frequency,
  type InstrumentDef,
  type InstrumentFamily,
  type InstrumentInput,
  type OisDef,
  type RatesDayCount,
  type RollConvention,
  type StirFutureDef,
  type VanillaIrsDef,
} from "../data/contract";
import styles from "./ReferenceDataWorkspace.module.css";

import { NumberField as NumberFieldBase } from "../components/NumberField";

// --- per-family default factories ------------------------------------------

function defaultDeposit(): DepositDef {
  return {
    index: "",
    tenor: "",
    dayCount: "act_360",
    businessDayConvention: "modified_following",
    calendars: ["united_states"],
    spotLagDays: 2,
  };
}

function defaultFra(): FraDef {
  return {
    floatIndex: "",
    startTenor: "",
    endTenor: "",
    accrualDayCount: "act_360",
    businessDayConvention: "modified_following",
    calendars: ["united_states"],
    spotLagDays: 2,
  };
}

function defaultStirFuture(): StirFutureDef {
  return {
    contractCode: "",
    referenceStart: "",
    referenceEnd: "",
    dayCount: "act_360",
    calendars: ["united_states"],
    convexityVol: 0,
    contractSize: 1_000_000,
  };
}

function defaultVanillaIrs(): VanillaIrsDef {
  return {
    tenor: "",
    fixedFrequency: "annual",
    fixedDayCount: "act_360",
    floatIndex: "",
    floatFrequency: "quarterly",
    floatDayCount: "act_360",
    businessDayConvention: "modified_following",
    calendars: ["united_states"],
    rollConvention: "none",
    spotLagDays: 2,
  };
}

function defaultOis(): OisDef {
  return {
    tenor: "",
    index: "",
    fixedFrequency: "annual",
    fixedDayCount: "act_360",
    floatDayCount: "act_360",
    businessDayConvention: "modified_following",
    calendars: ["united_states"],
    spotLagDays: 2,
  };
}

function defaultBond(): BondDef {
  return {
    issuer: "",
    couponRate: 0,
    couponType: "fixed",
    couponFrequency: "semi_annual",
    dayCount: "act_act",
    maturityDate: { year: 2030, month: 1, day: 1 },
    redemption: 100,
    calendars: ["united_states"],
  };
}

/** A fresh definition for `family` with empty base fields (create defaults). */
export function defaultInstrument(family: InstrumentFamily): InstrumentDef {
  const base = {
    instrumentId: "",
    name: "",
    description: "",
    currency: "USD",
    externalIds: [] as ExternalIdEntry[],
  };
  switch (family) {
    case "deposit":
      return { ...base, family, deposit: defaultDeposit() };
    case "fra":
      return { ...base, family, fra: defaultFra() };
    case "stir_future":
      return { ...base, family, stirFuture: defaultStirFuture() };
    case "vanilla_irs":
      return { ...base, family, vanillaIrs: defaultVanillaIrs() };
    case "ois":
      return { ...base, family, ois: defaultOis() };
    case "bond":
      return { ...base, family, bond: defaultBond() };
  }
}

/** Re-key a draft to a different family, preserving the base fields. */
function changeFamily(def: InstrumentDef, family: InstrumentFamily): InstrumentDef {
  const fresh = defaultInstrument(family);
  return {
    ...fresh,
    instrumentId: def.instrumentId,
    name: def.name,
    description: def.description,
    currency: def.currency,
    externalIds: def.externalIds,
  } as InstrumentDef;
}

// --- small presentational field helpers ------------------------------------

function Field({
  label,
  htmlFor,
  children,
}: {
  label: string;
  htmlFor: string;
  children: React.ReactNode;
}): React.ReactElement {
  return (
    <div className={styles.field}>
      <label className={styles.fieldLabel} htmlFor={htmlFor}>
        {label}
      </label>
      {children}
    </div>
  );
}

function TextField({
  id,
  label,
  value,
  onChange,
  placeholder,
}: {
  id: string;
  label: string;
  value: string;
  onChange: (v: string) => void;
  placeholder?: string;
}): React.ReactElement {
  return (
    <Field label={label} htmlFor={id}>
      <input
        id={id}
        className={styles.control}
        type="text"
        value={value}
        placeholder={placeholder}
        onChange={(e) => onChange(e.target.value)}
      />
    </Field>
  );
}

function NumberField({
  id,
  label,
  value,
  onChange,
  step,
  disabled,
}: {
  id: string;
  label: string;
  value: number;
  onChange: (v: number) => void;
  step?: number;
  disabled?: boolean;
}): React.ReactElement {
  return (
    <Field label={label} htmlFor={id}>
      <NumberFieldBase
        id={id}
        className={styles.control}
        value={Number.isFinite(value) ? value : 0}
        step={step ?? 1}
        disabled={disabled}
        onChange={(e) => onChange(e.target.value === "" ? 0 : Number(e.target.value))}
      />
    </Field>
  );
}

function SelectField<T extends string>({
  id,
  label,
  value,
  options,
  onChange,
  disabled,
  labelOf,
}: {
  id: string;
  label: string;
  value: T;
  options: readonly T[];
  onChange: (v: T) => void;
  disabled?: boolean;
  labelOf?: (v: T) => string;
}): React.ReactElement {
  return (
    <Field label={label} htmlFor={id}>
      <select
        id={id}
        className={styles.control}
        value={value}
        disabled={disabled}
        onChange={(e) => onChange(e.target.value as T)}
      >
        {options.map((opt) => (
          <option key={opt} value={opt}>
            {labelOf ? labelOf(opt) : opt}
          </option>
        ))}
      </select>
    </Field>
  );
}

function CalendarsField({
  id,
  value,
  onChange,
}: {
  id: string;
  value: Calendar[];
  onChange: (v: Calendar[]) => void;
}): React.ReactElement {
  const toggle = (cal: Calendar, on: boolean): void => {
    if (on) onChange([...value, cal]);
    else onChange(value.filter((c) => c !== cal));
  };
  return (
    <fieldset id={id} className={styles.field}>
      <legend className={styles.fieldLabel}>Calendars (≥1 required)</legend>
      <div className={styles.extIds}>
        {CALENDARS.map((cal) => (
          <label key={cal} className={styles.extIdChip}>
            <input
              type="checkbox"
              checked={value.includes(cal)}
              onChange={(e) => toggle(cal, e.target.checked)}
            />{" "}
            {cal}
          </label>
        ))}
      </div>
    </fieldset>
  );
}

const MONTHS = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12] as const;

function BrokenDateField({
  idPrefix,
  label,
  value,
  onChange,
}: {
  idPrefix: string;
  label: string;
  value: BrokenDate;
  onChange: (v: BrokenDate) => void;
}): React.ReactElement {
  return (
    <div className={styles.dateCell}>
      <span className={styles.fieldLabel}>{label}</span>
      <div className={styles.dateRow}>
        <NumberFieldBase
          id={`${idPrefix}-year`}
          aria-label={`${label} year`}
          className={`${styles.control} ${styles.dateInput}`}
          value={value.year}
          onChange={(e) => onChange({ ...value, year: Number(e.target.value) })}
        />
        <select
          id={`${idPrefix}-month`}
          aria-label={`${label} month`}
          className={`${styles.control} ${styles.dateInput}`}
          value={value.month}
          onChange={(e) => onChange({ ...value, month: Number(e.target.value) })}
        >
          {MONTHS.map((m) => (
            <option key={m} value={m}>
              {m}
            </option>
          ))}
        </select>
        <NumberFieldBase
          id={`${idPrefix}-day`}
          aria-label={`${label} day`}
          className={`${styles.control} ${styles.dateInput}`}
          min={1}
          max={31}
          value={value.day}
          onChange={(e) => onChange({ ...value, day: Number(e.target.value) })}
        />
      </div>
    </div>
  );
}

function ExternalIdsField({
  value,
  onChange,
}: {
  value: ExternalIdEntry[];
  onChange: (v: ExternalIdEntry[]) => void;
}): React.ReactElement {
  const add = (): void => onChange([...value, { scheme: EXTERNAL_ID_SCHEMES[0]!, value: "" }]);
  const remove = (idx: number): void => onChange(value.filter((_, i) => i !== idx));
  const patch = (idx: number, next: ExternalIdEntry): void =>
    onChange(value.map((e, i) => (i === idx ? next : e)));
  return (
    <div className={styles.extIdEditor}>
      <span className={styles.sectionTitle}>External identifiers</span>
      {value.map((entry, idx) => (
        <div key={idx} className={styles.extIdRow}>
          <select
            aria-label={`external id ${idx + 1} scheme`}
            className={styles.control}
            value={entry.scheme}
            onChange={(e) => patch(idx, { ...entry, scheme: e.target.value as ExternalIdScheme })}
          >
            {EXTERNAL_ID_SCHEMES.map((s) => (
              <option key={s} value={s}>
                {s.toUpperCase()}
              </option>
            ))}
          </select>
          <input
            aria-label={`external id ${idx + 1} value`}
            className={`${styles.control} ${styles.extIdValue}`}
            type="text"
            value={entry.value}
            placeholder="identifier, e.g. US91282CHK24"
            onChange={(e) => patch(idx, { ...entry, value: e.target.value })}
          />
          <Button type="button" variant="ghost" onClick={() => remove(idx)}>
            Remove
          </Button>
        </div>
      ))}
      <div>
        <Button type="button" variant="secondary" onClick={add}>
          Add identifier
        </Button>
      </div>
    </div>
  );
}

// --- the family field blocks -----------------------------------------------

const rdc = (v: RatesDayCount): RatesDayCount => v;

function dayCountLabel(v: RatesDayCount): string {
  return v;
}

function DepositFields({
  def,
  set,
}: {
  def: DepositDef;
  set: (d: DepositDef) => void;
}): React.ReactElement {
  return (
    <>
      <TextField id="dep-index" label="Index" value={def.index} onChange={(v) => set({ ...def, index: v })} placeholder="e.g. USD-SOFR" />
      <TextField id="dep-tenor" label="Tenor" value={def.tenor} onChange={(v) => set({ ...def, tenor: v })} placeholder="e.g. 3M" />
      <SelectField id="dep-daycount" label="Day count" value={def.dayCount} options={RATES_DAY_COUNTS} labelOf={dayCountLabel} onChange={(v) => set({ ...def, dayCount: rdc(v) })} />
      <SelectField id="dep-bdc" label="Business-day convention" value={def.businessDayConvention} options={BUSINESS_DAY_CONVENTIONS} onChange={(v) => set({ ...def, businessDayConvention: v as BusinessDayConvention })} />
      <NumberField id="dep-spot" label="Spot lag (days)" value={def.spotLagDays} onChange={(v) => set({ ...def, spotLagDays: v })} />
      <CalendarsField id="dep-cals" value={def.calendars} onChange={(v) => set({ ...def, calendars: v })} />
    </>
  );
}

function FraFields({ def, set }: { def: FraDef; set: (d: FraDef) => void }): React.ReactElement {
  return (
    <>
      <TextField id="fra-index" label="Float index" value={def.floatIndex} onChange={(v) => set({ ...def, floatIndex: v })} placeholder="e.g. USD-SOFR" />
      <TextField id="fra-start" label="Start tenor" value={def.startTenor} onChange={(v) => set({ ...def, startTenor: v })} placeholder="e.g. 3M" />
      <TextField id="fra-end" label="End tenor" value={def.endTenor} onChange={(v) => set({ ...def, endTenor: v })} placeholder="e.g. 6M" />
      <SelectField id="fra-daycount" label="Accrual day count" value={def.accrualDayCount} options={RATES_DAY_COUNTS} labelOf={dayCountLabel} onChange={(v) => set({ ...def, accrualDayCount: rdc(v) })} />
      <SelectField id="fra-bdc" label="Business-day convention" value={def.businessDayConvention} options={BUSINESS_DAY_CONVENTIONS} onChange={(v) => set({ ...def, businessDayConvention: v as BusinessDayConvention })} />
      <NumberField id="fra-spot" label="Spot lag (days)" value={def.spotLagDays} onChange={(v) => set({ ...def, spotLagDays: v })} />
      <CalendarsField id="fra-cals" value={def.calendars} onChange={(v) => set({ ...def, calendars: v })} />
    </>
  );
}

function StirFutureFields({
  def,
  set,
}: {
  def: StirFutureDef;
  set: (d: StirFutureDef) => void;
}): React.ReactElement {
  return (
    <>
      <TextField id="stir-code" label="Contract code" value={def.contractCode} onChange={(v) => set({ ...def, contractCode: v })} placeholder="e.g. SR3" />
      <TextField id="stir-start" label="Reference start" value={def.referenceStart} onChange={(v) => set({ ...def, referenceStart: v })} placeholder="e.g. 2026-03-18" />
      <TextField id="stir-end" label="Reference end" value={def.referenceEnd} onChange={(v) => set({ ...def, referenceEnd: v })} placeholder="e.g. 2026-06-17" />
      <SelectField id="stir-daycount" label="Day count" value={def.dayCount} options={RATES_DAY_COUNTS} labelOf={dayCountLabel} onChange={(v) => set({ ...def, dayCount: rdc(v) })} />
      <NumberField id="stir-conv" label="Convexity vol" value={def.convexityVol} step={0.0001} onChange={(v) => set({ ...def, convexityVol: v })} />
      <NumberField id="stir-size" label="Contract size" value={def.contractSize} onChange={(v) => set({ ...def, contractSize: v })} />
      <CalendarsField id="stir-cals" value={def.calendars} onChange={(v) => set({ ...def, calendars: v })} />
    </>
  );
}

function VanillaIrsFields({
  def,
  set,
}: {
  def: VanillaIrsDef;
  set: (d: VanillaIrsDef) => void;
}): React.ReactElement {
  return (
    <>
      <TextField id="irs-tenor" label="Tenor" value={def.tenor} onChange={(v) => set({ ...def, tenor: v })} placeholder="e.g. 10Y" />
      <SelectField id="irs-fixfreq" label="Fixed frequency" value={def.fixedFrequency} options={FREQUENCIES} onChange={(v) => set({ ...def, fixedFrequency: v as Frequency })} />
      <SelectField id="irs-fixdc" label="Fixed day count" value={def.fixedDayCount} options={RATES_DAY_COUNTS} labelOf={dayCountLabel} onChange={(v) => set({ ...def, fixedDayCount: rdc(v) })} />
      <TextField id="irs-floatindex" label="Float index" value={def.floatIndex} onChange={(v) => set({ ...def, floatIndex: v })} placeholder="e.g. USD-SOFR" />
      <SelectField id="irs-floatfreq" label="Float frequency" value={def.floatFrequency} options={FREQUENCIES} onChange={(v) => set({ ...def, floatFrequency: v as Frequency })} />
      <SelectField id="irs-floatdc" label="Float day count" value={def.floatDayCount} options={RATES_DAY_COUNTS} labelOf={dayCountLabel} onChange={(v) => set({ ...def, floatDayCount: rdc(v) })} />
      <SelectField id="irs-bdc" label="Business-day convention" value={def.businessDayConvention} options={BUSINESS_DAY_CONVENTIONS} onChange={(v) => set({ ...def, businessDayConvention: v as BusinessDayConvention })} />
      <SelectField id="irs-roll" label="Roll convention" value={def.rollConvention} options={ROLL_CONVENTIONS} onChange={(v) => set({ ...def, rollConvention: v as RollConvention })} />
      <NumberField id="irs-spot" label="Spot lag (days)" value={def.spotLagDays} onChange={(v) => set({ ...def, spotLagDays: v })} />
      <CalendarsField id="irs-cals" value={def.calendars} onChange={(v) => set({ ...def, calendars: v })} />
    </>
  );
}

function OisFields({ def, set }: { def: OisDef; set: (d: OisDef) => void }): React.ReactElement {
  return (
    <>
      <TextField id="ois-tenor" label="Tenor" value={def.tenor} onChange={(v) => set({ ...def, tenor: v })} placeholder="e.g. 5Y" />
      <TextField id="ois-index" label="Index" value={def.index} onChange={(v) => set({ ...def, index: v })} placeholder="e.g. SOFR" />
      <SelectField id="ois-fixfreq" label="Fixed frequency" value={def.fixedFrequency} options={FREQUENCIES} onChange={(v) => set({ ...def, fixedFrequency: v as Frequency })} />
      <SelectField id="ois-fixdc" label="Fixed day count" value={def.fixedDayCount} options={RATES_DAY_COUNTS} labelOf={dayCountLabel} onChange={(v) => set({ ...def, fixedDayCount: rdc(v) })} />
      <SelectField id="ois-floatdc" label="Float day count" value={def.floatDayCount} options={RATES_DAY_COUNTS} labelOf={dayCountLabel} onChange={(v) => set({ ...def, floatDayCount: rdc(v) })} />
      <SelectField id="ois-bdc" label="Business-day convention" value={def.businessDayConvention} options={BUSINESS_DAY_CONVENTIONS} onChange={(v) => set({ ...def, businessDayConvention: v as BusinessDayConvention })} />
      <NumberField id="ois-spot" label="Spot lag (days)" value={def.spotLagDays} onChange={(v) => set({ ...def, spotLagDays: v })} />
      <CalendarsField id="ois-cals" value={def.calendars} onChange={(v) => set({ ...def, calendars: v })} />
    </>
  );
}

function BondFields({ def, set }: { def: BondDef; set: (d: BondDef) => void }): React.ReactElement {
  const isZero = def.couponType === "zero";
  const onCouponType = (t: CouponType): void => {
    if (t === "zero") set({ ...def, couponType: t, couponFrequency: "", couponRate: 0 });
    else set({ ...def, couponType: t, couponFrequency: def.couponFrequency || "semi_annual" });
  };
  return (
    <>
      <TextField id="bond-issuer" label="Issuer" value={def.issuer} onChange={(v) => set({ ...def, issuer: v })} placeholder="e.g. US Treasury" />
      <SelectField id="bond-coupontype" label="Coupon type" value={def.couponType} options={COUPON_TYPES} onChange={(v) => onCouponType(v as CouponType)} />
      <NumberField id="bond-couponrate" label="Coupon rate (%)" value={def.couponRate} step={0.01} disabled={isZero} onChange={(v) => set({ ...def, couponRate: v })} />
      <SelectField
        id="bond-couponfreq"
        label="Coupon frequency"
        value={(def.couponFrequency || "semi_annual") as Frequency}
        options={FREQUENCIES}
        disabled={isZero}
        onChange={(v) => set({ ...def, couponFrequency: v as Frequency })}
      />
      <SelectField id="bond-daycount" label="Day count" value={def.dayCount} options={BOND_DAY_COUNTS} labelOf={dayCountLabel} onChange={(v) => set({ ...def, dayCount: rdc(v) })} />
      <NumberField id="bond-redemption" label="Redemption" value={def.redemption} step={0.01} onChange={(v) => set({ ...def, redemption: v })} />
      <CalendarsField id="bond-cals" value={def.calendars} onChange={(v) => set({ ...def, calendars: v })} />
      <div className={styles.field}>
        <span className={styles.sectionTitle}>Schedule dates</span>
        <div className={styles.dateRow}>
          <BrokenDateField idPrefix="bond-maturity" label="Maturity (required)" value={def.maturityDate} onChange={(v) => set({ ...def, maturityDate: v })} />
          <BrokenDateField idPrefix="bond-issue" label="Issue (optional)" value={def.issueDate ?? def.maturityDate} onChange={(v) => set({ ...def, issueDate: v })} />
          <BrokenDateField idPrefix="bond-dated" label="Dated (optional)" value={def.datedDate ?? def.maturityDate} onChange={(v) => set({ ...def, datedDate: v })} />
          <BrokenDateField idPrefix="bond-firstcoupon" label="First coupon (optional)" value={def.firstCouponDate ?? def.maturityDate} onChange={(v) => set({ ...def, firstCouponDate: v })} />
        </div>
      </div>
    </>
  );
}

// --- the form --------------------------------------------------------------

export interface InstrumentFormProps {
  /** The def to edit, or `null` for a fresh create form. */
  editing: InstrumentDef | null;
  onCreate: (input: InstrumentInput) => Promise<unknown>;
  onUpdate: (input: InstrumentInput) => Promise<unknown>;
  /** Called after a successful submit (the workspace clears `editing`). */
  onDone: () => void;
  /** The shared run/clear-after-await wrapper (surfaces errors). */
  run: (action: () => Promise<unknown>) => Promise<void>;
}

export function InstrumentForm({
  editing,
  onCreate,
  onUpdate,
  onDone,
  run,
}: InstrumentFormProps): React.ReactElement {
  const [draft, setDraft] = useState<InstrumentDef>(editing ?? defaultInstrument("ois"));
  const [localError, setLocalError] = useState<string | null>(null);
  const isEditing = editing !== null;

  const name = draft.name.trim();
  const currency = draft.currency.trim();
  const familyDef =
    draft.family === "deposit"
      ? draft.deposit
      : draft.family === "fra"
        ? draft.fra
        : draft.family === "stir_future"
          ? draft.stirFuture
          : draft.family === "vanilla_irs"
            ? draft.vanillaIrs
            : draft.family === "ois"
              ? draft.ois
              : draft.bond;
  const calendarCount = familyDef.calendars.length;
  const canSubmit = name.length > 0 && currency.length > 0 && calendarCount > 0;

  const submit = (e: React.FormEvent<HTMLFormElement>): void => {
    e.preventDefault();
    setLocalError(null);
    if (!canSubmit) {
      setLocalError("Name, currency and at least one calendar are required.");
      return;
    }
    const payload: InstrumentInput = { ...draft, name, currency } as InstrumentInput;
    void run(async () => {
      if (isEditing) await onUpdate(payload);
      else await onCreate(payload);
      onDone();
    });
  };

  return (
    <form className={styles.form} onSubmit={submit}>
      <div className={styles.grid}>
        <SelectField
          id="rd-family"
          label="Family"
          value={draft.family}
          options={INSTRUMENT_FAMILIES}
          labelOf={(f) => INSTRUMENT_FAMILY_LABELS[f]}
          disabled={isEditing}
          onChange={(f) => setDraft((d) => changeFamily(d, f as InstrumentFamily))}
        />
        <TextField id="rd-name" label="Name" value={draft.name} onChange={(v) => setDraft((d) => ({ ...d, name: v }) as InstrumentDef)} placeholder="e.g. USD SOFR OIS 5Y" />
        <TextField id="rd-ccy" label="Currency" value={draft.currency} onChange={(v) => setDraft((d) => ({ ...d, currency: v }) as InstrumentDef)} placeholder="e.g. USD" />
        <TextField id="rd-desc" label="Description" value={draft.description} onChange={(v) => setDraft((d) => ({ ...d, description: v }) as InstrumentDef)} placeholder="free text" />
      </div>

      <div className={styles.grid}>
        {draft.family === "deposit" && (
          <DepositFields def={draft.deposit} set={(x) => setDraft((d) => ({ ...d, family: "deposit", deposit: x }) as InstrumentDef)} />
        )}
        {draft.family === "fra" && (
          <FraFields def={draft.fra} set={(x) => setDraft((d) => ({ ...d, family: "fra", fra: x }) as InstrumentDef)} />
        )}
        {draft.family === "stir_future" && (
          <StirFutureFields def={draft.stirFuture} set={(x) => setDraft((d) => ({ ...d, family: "stir_future", stirFuture: x }) as InstrumentDef)} />
        )}
        {draft.family === "vanilla_irs" && (
          <VanillaIrsFields def={draft.vanillaIrs} set={(x) => setDraft((d) => ({ ...d, family: "vanilla_irs", vanillaIrs: x }) as InstrumentDef)} />
        )}
        {draft.family === "ois" && (
          <OisFields def={draft.ois} set={(x) => setDraft((d) => ({ ...d, family: "ois", ois: x }) as InstrumentDef)} />
        )}
        {draft.family === "bond" && (
          <BondFields def={draft.bond} set={(x) => setDraft((d) => ({ ...d, family: "bond", bond: x }) as InstrumentDef)} />
        )}
      </div>

      <ExternalIdsField value={draft.externalIds} onChange={(v) => setDraft((d) => ({ ...d, externalIds: v }) as InstrumentDef)} />

      {localError && <p className={styles.formError}>{localError}</p>}

      <div className={styles.formActions}>
        <Button type="submit" variant="primary" disabled={!canSubmit}>
          {isEditing ? "Save instrument" : "Create instrument"}
        </Button>
        {isEditing && (
          <Button type="button" variant="ghost" onClick={onDone}>
            Cancel
          </Button>
        )}
      </div>
    </form>
  );
}
