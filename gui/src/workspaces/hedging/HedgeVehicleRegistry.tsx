/**
 * HedgeVehicleRegistry — author the firm's HEDGE-VEHICLE roster: the mapping from a class
 * of risk to the instrument the desk actually hedges it with, and that instrument's DV01
 * per unit.
 *
 * Why the registry has to exist at all: selling back the SAME security is exact — the
 * leg/fill DV01 ratio is identically 1 — but a corporate bond is not hedged with itself.
 * It is hedged with a benchmark at matching maturity, in practice a Treasury future, and
 * the moment the hedge instrument differs from the position that ratio stops being 1. The
 * size then needs a DV01 PER UNIT, and this table is the only place it comes from. That is
 * also why the server rejects an exit action naming a vehicle the registry does not carry.
 *
 * Binds to `HedgeConfig.vehicles` through the EXISTING `get_hedge_config` /
 * `set_hedge_config` pair (no separate CRUD verb). Rendered on the shared
 * {@link DataTable} + grid model so it sorts and filters like every other roster, rather
 * than as a hand-rolled table. Edits gate on the `hedge` capability.
 */
import { useMemo, useState } from "react";

import { DataTable } from "../../components/DataTable";
import { HelpButton } from "../../components/HelpButton";
import { InstrumentPicker } from "../../components/InstrumentPicker";
import { NumberField } from "../../components/NumberField";
import type { HedgeVehicleRule, InstrumentDef } from "../../data/contract";
import { useGridState } from "../../hooks/useGridState";
import type { ColumnDef } from "../../lib/grid";
import {
  defaultUnitLabel,
  formatDv01,
  matchLabel,
  maturityBucketLabel,
  newHedgeVehicleRule,
  validateHedgeVehicleRule,
} from "../../lib/hedgeVehicle";
import {
  hedgeVehicleOptions,
  type InstrumentOption,
} from "../../lib/instrumentPicker";
import styles from "./HedgingWorkspace.module.css";

interface HedgeVehicleRegistryProps {
  vehicles: readonly HedgeVehicleRule[];
  /**
   * The reference-data registry the instrument pickers are sourced from. Empty until
   * the first load resolves — the pickers then simply offer nothing and the previously
   * configured id still renders, rather than the field appearing to have been cleared.
   */
  instruments: readonly InstrumentDef[];
  /**
   * The identifiers a live composite currently covers, so the picker can mark which
   * vehicles can actually FILL. Omit when unknown.
   */
  liquidIds?: ReadonlySet<string>;
  readOnly: boolean;
  busy: boolean;
  /** A server-side rejection to surface, or `null`. */
  saveError: string | null;
  /** Commit the full replacement roster (upsert/delete resolve client-side). */
  onCommit: (vehicles: HedgeVehicleRule[]) => void;
}

type Draft = { index: number | null; rule: HedgeVehicleRule };

/** A fresh draft id that cannot collide with an existing row on first keystroke. */
function newDraftId(existing: readonly HedgeVehicleRule[]): string {
  for (let n = existing.length + 1; ; n += 1) {
    const candidate = `vehicle-${n}`;
    if (!existing.some((v) => v.id === candidate)) return candidate;
  }
}

export function HedgeVehicleRegistry({
  vehicles,
  instruments,
  liquidIds,
  readOnly,
  busy,
  saveError,
  onCommit,
}: HedgeVehicleRegistryProps): React.ReactElement {
  const [draft, setDraft] = useState<Draft | null>(null);

  const columns = useMemo<ColumnDef<HedgeVehicleRule>[]>(
    () => [
      {
        key: "id",
        header: "Id",
        width: 150,
        align: "left",
        accessor: (v) => v.id,
        sortValue: (v) => v.id,
        sortKey: "id",
        filter: { kind: "text" },
      },
      {
        key: "match",
        header: "Match",
        description:
          "The ANDed match axes — instrument, product, currency. An empty axis matches anything.",
        width: 240,
        align: "left",
        accessor: matchLabel,
        sortValue: matchLabel,
        sortKey: "match",
        filter: { kind: "text" },
      },
      {
        key: "maturity",
        header: "Maturity",
        description: "The half-open bucket [min, max) in years a BENCHMARK vehicle resolves through.",
        width: 110,
        align: "left",
        accessor: maturityBucketLabel,
        sortValue: (v) => v.minMaturityYears,
        sortKey: "maturity",
      },
      {
        key: "hedgeInstrument",
        header: "Hedge instrument",
        description: "The security a match actually trades to shed the risk.",
        width: 170,
        align: "left",
        accessor: (v) => v.hedgeInstrumentId,
        sortValue: (v) => v.hedgeInstrumentId,
        sortKey: "hedgeInstrument",
        filter: { kind: "text" },
      },
      {
        key: "future",
        header: "Future?",
        description: "A future is sized in WHOLE contracts, so its size always rounds.",
        width: 90,
        align: "left",
        accessor: (v) => (v.isFuture ? "yes" : "no"),
        sortValue: (v) => (v.isFuture ? 1 : 0),
        sortKey: "future",
        filter: { kind: "select" },
      },
      {
        key: "dv01",
        header: "DV01 / unit",
        description: "The DV01 of ONE unit — the divisor that turns a target DV01 into units.",
        width: 120,
        align: "right",
        accessor: (v) => formatDv01(v.dv01PerUnit),
        cell: (v) => <span className={styles.num}>{formatDv01(v.dv01PerUnit)}</span>,
        sortValue: (v) => v.dv01PerUnit,
        sortKey: "dv01",
        filter: { kind: "range" },
      },
      {
        key: "unit",
        header: "Unit",
        width: 110,
        align: "left",
        accessor: (v) => v.unitLabel,
        sortValue: (v) => v.unitLabel,
        sortKey: "unit",
      },
      ...(readOnly
        ? []
        : [
            {
              key: "actions",
              header: "Actions",
              width: 150,
              align: "left" as const,
              accessor: () => "",
              cell: (v: HedgeVehicleRule) => (
                <div className={styles.rowActions}>
                  <button
                    type="button"
                    className={styles.ghostBtn}
                    disabled={busy}
                    data-testid={`vehicle-edit-${v.id}`}
                    onClick={() =>
                      setDraft({ index: vehicles.findIndex((x) => x.id === v.id), rule: { ...v } })
                    }
                  >
                    Edit
                  </button>
                  <button
                    type="button"
                    className={styles.dangerBtn}
                    disabled={busy}
                    data-testid={`vehicle-delete-${v.id}`}
                    onClick={() => {
                      onCommit(vehicles.filter((x) => x.id !== v.id));
                      setDraft(null);
                    }}
                  >
                    Delete
                  </button>
                </div>
              ),
            },
          ]),
    ],
    [readOnly, busy, vehicles, onCommit],
  );

  const rows = useMemo(() => [...vehicles], [vehicles]);
  const grid = useGridState<HedgeVehicleRule>({
    tableId: "hedge-vehicle-registry",
    columns,
    rows,
    initialSort: { key: "id", dir: "asc" },
  });

  // EVERY problem at once, the way the hedge-graph editor surfaces its conflicts — a
  // half-corrected row that fails again on save is the worst of both worlds.
  const draftErrors = useMemo(() => {
    if (draft === null) return [];
    const others = vehicles.filter((_, i) => i !== draft.index).map((v) => v.id.trim());
    return validateHedgeVehicleRule(draft.rule, others);
  }, [draft, vehicles]);

  const setRule = (p: Partial<HedgeVehicleRule>): void =>
    setDraft((d) => (d === null ? d : { ...d, rule: { ...d.rule, ...p } }));

  // The vehicles a rule may hedge INTO — rolling futures products first (the option that
  // survives the quarterly roll), then specific delivery months, then cash bonds.
  const vehicleOptions = useMemo(
    () => hedgeVehicleOptions({ defs: instruments, liquidIds }),
    [instruments, liquidIds],
  );

  /**
   * Adopt a picked hedge instrument AND everything the definition already knows about
   * it. The DV01 per contract, the whole-lot flag and the unit label are all facts of
   * the contract's own published terms — the trader was previously retyping them from
   * memory, which is the failure this picker exists to remove. A definition that does
   * NOT carry a DV01 (every cash bond — its DV01 is a function of the live curve, not a
   * static term) leaves the field alone for the trader to supply: pre-filling a number
   * nobody derived would be a guess.
   */
  const chooseHedgeInstrument = (opt: InstrumentOption | null): void => {
    if (opt === null) {
      setRule({ hedgeInstrumentId: "" });
      return;
    }
    setRule({
      hedgeInstrumentId: opt.value,
      isFuture: opt.isFuture,
      unitLabel: opt.unitLabel,
      ...(opt.dv01PerUnit === null ? {} : { dv01PerUnit: opt.dv01PerUnit }),
    });
  };

  const onSaveDraft = (): void => {
    if (draft === null || draftErrors.length > 0) return;
    const trimmed: HedgeVehicleRule = {
      ...draft.rule,
      id: draft.rule.id.trim(),
      instrumentId: draft.rule.instrumentId.trim(),
      product: draft.rule.product.trim(),
      ccy: draft.rule.ccy.trim(),
      hedgeInstrumentId: draft.rule.hedgeInstrumentId.trim(),
      unitLabel: draft.rule.unitLabel.trim(),
    };
    onCommit(
      draft.index === null
        ? [...vehicles, trimmed]
        : vehicles.map((v, i) => (i === draft.index ? trimmed : v)),
    );
    setDraft(null);
  };

  return (
    <section
      className={styles.configPanel}
      aria-labelledby="hedge-vehicles-heading"
      data-testid="hedge-vehicles"
    >
      <div className={styles.scopeBar}>
        <h3 id="hedge-vehicles-heading" className={styles.panelHeading}>
          Hedge vehicles
        </h3>
        <HelpButton helpId="concept.hedge-vehicle-registry" subject="the hedge-vehicle registry" />
      </div>
      <p className={styles.panelNote}>
        What the desk hedges each class of risk <strong>with</strong>. Selling back the same
        security is exact — its DV01 ratio is 1 — but a corporate bond is hedged with a
        benchmark at matching maturity, in practice a Treasury future. The moment the hedge
        instrument differs from the position, the size needs a <strong>DV01 per unit</strong>,
        and this roster is the only place it comes from. A rule naming a vehicle that is not
        here is rejected by the server.
      </p>

      {saveError !== null && (
        <p className={styles.errorText} role="alert" data-testid="vehicles-save-error">
          {saveError}
        </p>
      )}

      <DataTable
        label="Hedge vehicle registry"
        columns={columns}
        grid={grid}
        rowKey={(v) => v.id}
        rowProps={(v) => ({ "data-testid": `vehicle-row-${v.id}` })}
        emptyState="No hedge vehicles registered — every hedge falls back to selling the same security back."
      />

      {!readOnly && draft === null && (
        <div className={styles.formActions}>
          <button
            type="button"
            className={styles.saveBtn}
            disabled={busy}
            data-testid="vehicle-add"
            onClick={() => setDraft({ index: null, rule: newHedgeVehicleRule(newDraftId(vehicles)) })}
          >
            + Add hedge vehicle
          </button>
        </div>
      )}

      {!readOnly && draft !== null && (
        <div className={styles.thresholdForm} data-testid="vehicle-editor">
          <div className={styles.formGrid}>
            <label className={styles.formField}>
              <span className={styles.fieldLabel}>Id</span>
              <input
                className={styles.input}
                type="text"
                value={draft.rule.id}
                data-testid="vehicle-id"
                onChange={(e) => setRule({ id: e.target.value })}
              />
            </label>
            <div className={styles.formField}>
              <span className={styles.fieldLabel}>Instrument (empty ⇒ any)</span>
              <InstrumentPicker
                label="Match instrument (empty matches any)"
                options={vehicleOptions}
                value={draft.rule.instrumentId}
                placeholder="Any instrument"
                testId="vehicle-instrument-id"
                onChange={(opt) => setRule({ instrumentId: opt?.value ?? "" })}
                onRawCommit={(raw) => setRule({ instrumentId: raw })}
              />
            </div>
            <label className={styles.formField}>
              <span className={styles.fieldLabel}>Product (empty ⇒ any)</span>
              <input
                className={styles.input}
                type="text"
                value={draft.rule.product}
                placeholder="e.g. BOND"
                data-testid="vehicle-product"
                onChange={(e) => setRule({ product: e.target.value })}
              />
            </label>
            <label className={styles.formField}>
              <span className={styles.fieldLabel}>Currency (empty ⇒ any)</span>
              <input
                className={styles.input}
                type="text"
                value={draft.rule.ccy}
                placeholder="e.g. USD"
                data-testid="vehicle-ccy"
                onChange={(e) => setRule({ ccy: e.target.value })}
              />
            </label>
            <label className={styles.formField}>
              <span className={styles.fieldLabel}>Min maturity (y)</span>
              <NumberField
                className={styles.input}
                value={draft.rule.minMaturityYears}
                data-testid="vehicle-min-maturity"
                onChange={(e) => setRule({ minMaturityYears: Number(e.target.value) })}
              />
            </label>
            <label className={styles.formField}>
              <span className={styles.fieldLabel}>Max maturity (y, exclusive)</span>
              <NumberField
                className={styles.input}
                value={draft.rule.maxMaturityYears}
                data-testid="vehicle-max-maturity"
                onChange={(e) => setRule({ maxMaturityYears: Number(e.target.value) })}
              />
            </label>
            <div className={styles.formField}>
              <span className={styles.fieldLabel}>Hedge instrument</span>
              <InstrumentPicker
                label="Hedge instrument"
                options={vehicleOptions}
                value={draft.rule.hedgeInstrumentId}
                placeholder="Search futures and bonds…"
                testId="vehicle-hedge-instrument"
                onChange={chooseHedgeInstrument}
                onRawCommit={(raw) => setRule({ hedgeInstrumentId: raw })}
              />
            </div>
            <label className={styles.formField}>
              <span className={styles.fieldLabel}>DV01 per unit</span>
              <NumberField
                className={styles.input}
                value={draft.rule.dv01PerUnit}
                data-testid="vehicle-dv01"
                onChange={(e) => setRule({ dv01PerUnit: Number(e.target.value) })}
              />
              <span className={styles.fieldHint}>
                Pre-filled from a picked future&apos;s published terms — its{" "}
                <strong>standardized</strong> DV01 at the contract&apos;s 6% notional
                yield. A futures contract has no constant basis-point value, so this
                understates the live figure whenever yields sit below 6%. Override it
                with your own if the desk sizes off the live curve.
              </span>
            </label>
            <label className={styles.formField}>
              <span className={styles.fieldLabel}>Unit label</span>
              <input
                className={styles.input}
                type="text"
                value={draft.rule.unitLabel}
                data-testid="vehicle-unit-label"
                onChange={(e) => setRule({ unitLabel: e.target.value })}
              />
            </label>
          </div>

          <label className={styles.checkField}>
            <input
              type="checkbox"
              checked={draft.rule.isFuture}
              data-testid="vehicle-is-future"
              onChange={(e) =>
                // Flipping "is a future" re-defaults the unit label with it: a contract and
                // a face amount are not interchangeable units, and a stale label misreads
                // every size computed off this row.
                setRule({ isFuture: e.target.checked, unitLabel: defaultUnitLabel(e.target.checked) })
              }
            />
            <span>Future — sized in whole contracts</span>
          </label>

          {draftErrors.length > 0 && (
            <ul className={styles.conflictList} data-testid="vehicle-errors">
              {draftErrors.map((e, i) => (
                <li key={i} className={styles.conflictError}>
                  {e}
                </li>
              ))}
            </ul>
          )}

          <div className={styles.formActions}>
            <button
              type="button"
              className={styles.saveBtn}
              disabled={busy || draftErrors.length > 0}
              data-testid="vehicle-save"
              onClick={onSaveDraft}
            >
              {draft.index === null ? "Add vehicle" : "Save vehicle"}
            </button>
            <button
              type="button"
              className={styles.ghostBtn}
              disabled={busy}
              data-testid="vehicle-cancel"
              onClick={() => setDraft(null)}
            >
              Cancel
            </button>
          </div>
        </div>
      )}
    </section>
  );
}
