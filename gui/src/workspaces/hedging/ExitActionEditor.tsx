/**
 * ExitActionEditor — the hedge-graph LEAF editor. Where the risk-routing editor's
 * terminal is a desk→book picker, the hedge terminal is an EXIT ACTION: an action-kind
 * picker plus exactly the venue / size / style fields that action reads
 * (docs/AUTO-HEDGING §5.3, §5.5). Every change emits a fully-formed {@link ExitAction}
 * so the compiled graph is always total.
 */
import type { ExecStyle, ExitAction, ExitActionKind, HedgeSize, HedgeSizeKind } from "../../data/contract";
import {
  EXIT_ACTION_KINDS,
  actionUsesSize,
  actionUsesStyle,
  defaultExitAction,
  exitActionHint,
  exitActionLabel,
  execStyleLabel,
  hedgeSizeLabel,
} from "../../lib/hedgeExit";
import rr from "../riskrouting/RiskRoutingWorkspace.module.css";

import { NumberField } from "../../components/NumberField";

const SIZE_KINDS: readonly HedgeSizeKind[] = ["overflow", "full", "fixed"];
const EXEC_STYLES: readonly ExecStyle[] = ["immediate", "worked"];

interface ExitActionEditorProps {
  action: ExitAction;
  readOnly: boolean;
  /** Aggregation instruments a CROSS_INTERNAL can target (advisory datalist). */
  instrumentOptions: readonly string[];
  /** LP ids an RFQ_OUT can fan to. */
  lpOptions: readonly string[];
  onChange: (next: ExitAction) => void;
}

/** A compact size chooser (kind + explicit magnitude for `fixed`). */
function SizeEditor({
  size,
  readOnly,
  onChange,
}: {
  size: HedgeSize;
  readOnly: boolean;
  onChange: (s: HedgeSize) => void;
}): React.ReactElement {
  return (
    <div className={rr.destRow}>
      <label className={rr.editorField}>
        <span className={rr.fieldLabel}>Size</span>
        <select
          className={rr.select}
          value={size.kind}
          disabled={readOnly}
          data-testid="exit-size-kind"
          onChange={(e) => onChange({ ...size, kind: e.target.value as HedgeSizeKind })}
        >
          {SIZE_KINDS.map((k) => (
            <option key={k} value={k}>
              {hedgeSizeLabel(k)}
            </option>
          ))}
        </select>
      </label>
      {size.kind === "fixed" && (
        <label className={rr.editorField}>
          <span className={rr.fieldLabel}>Fixed amount</span>
          <NumberField
            className={rr.input}
            value={size.fixed}
            disabled={readOnly}
            data-testid="exit-size-fixed"
            onChange={(e) => onChange({ ...size, fixed: Number(e.target.value) })}
          />
        </label>
      )}
    </div>
  );
}

export function ExitActionEditor({
  action,
  readOnly,
  instrumentOptions,
  lpOptions,
  onChange,
}: ExitActionEditorProps): React.ReactElement {
  // Swapping the kind resets to that kind's sensible defaults but preserves any
  // still-relevant shared fields (instrument, size) so a re-pick is not destructive.
  const setKind = (kind: ExitActionKind): void => {
    const base = defaultExitAction(kind);
    onChange({ ...base, instrument: action.instrument, size: action.size });
  };
  const patch = (p: Partial<ExitAction>): void => onChange({ ...action, ...p });

  const addLp = (lp: string): void => {
    const t = lp.trim();
    if (t.length === 0 || action.lps.includes(t)) return;
    patch({ lps: [...action.lps, t] });
  };
  const removeLp = (lp: string): void => patch({ lps: action.lps.filter((x) => x !== lp) });

  return (
    <div data-testid="exit-action-editor">
      <div className={rr.destRow}>
        <label className={rr.editorField}>
          <span className={rr.fieldLabel}>Exit action</span>
          <select
            className={rr.select}
            value={action.kind}
            disabled={readOnly}
            data-testid="exit-action-kind"
            onChange={(e) => setKind(e.target.value as ExitActionKind)}
          >
            {EXIT_ACTION_KINDS.map((k) => (
              <option key={k} value={k}>
                {exitActionLabel(k)}
              </option>
            ))}
          </select>
        </label>
      </div>
      <p className={rr.ruleEditorHint}>{exitActionHint(action.kind)}</p>

      {action.kind === "cross_internal" && (
        <>
          <label className={rr.editorField}>
            <span className={rr.fieldLabel}>Aggregation instrument</span>
            <input
              className={rr.input}
              type="text"
              list="hedge-agg-instruments"
              value={action.instrument}
              disabled={readOnly}
              data-testid="exit-instrument"
              placeholder="e.g. AGG-OIS"
              onChange={(e) => patch({ instrument: e.target.value })}
            />
            <datalist id="hedge-agg-instruments">
              {instrumentOptions.map((i) => (
                <option key={i} value={i} />
              ))}
            </datalist>
          </label>
          <SizeEditor size={action.size} readOnly={readOnly} onChange={(s) => patch({ size: s })} />
        </>
      )}

      {action.kind === "skew" && (
        <div className={rr.destRow}>
          <label className={rr.toggle}>
            <input
              type="checkbox"
              checked={action.toEdge}
              disabled={readOnly}
              data-testid="exit-to-edge"
              onChange={(e) => patch({ toEdge: e.target.checked })}
            />
            <span>Lean to the band edge (auto)</span>
          </label>
          {!action.toEdge && (
            <label className={rr.editorField}>
              <span className={rr.fieldLabel}>Skew (bp)</span>
              <NumberField
                className={rr.input}
                value={action.skewBp ?? 0}
                disabled={readOnly}
                data-testid="exit-skew-bp"
                onChange={(e) => patch({ skewBp: Number(e.target.value) })}
              />
            </label>
          )}
        </div>
      )}

      {action.kind === "submit_market_order" && (
        <>
          <SizeEditor size={action.size} readOnly={readOnly} onChange={(s) => patch({ size: s })} />
          <label className={rr.editorField}>
            <span className={rr.fieldLabel}>Execution style</span>
            <select
              className={rr.select}
              value={action.style}
              disabled={readOnly}
              data-testid="exit-style"
              onChange={(e) => patch({ style: e.target.value as ExecStyle })}
            >
              {EXEC_STYLES.map((s) => (
                <option key={s} value={s}>
                  {execStyleLabel(s)}
                </option>
              ))}
            </select>
          </label>
        </>
      )}

      {action.kind === "rfq_out" && (
        <>
          <div className={rr.editorField}>
            <span className={rr.fieldLabel}>LPs to fan to</span>
            <div className={rr.tagRow}>
              {action.lps.length === 0 && <span className={rr.tagEmpty}>No LPs yet</span>}
              {action.lps.map((lp) => (
                <span key={lp} className={rr.tag}>
                  {lp}
                  {!readOnly && (
                    <button
                      type="button"
                      className={rr.tagX}
                      aria-label={`Remove ${lp}`}
                      onClick={() => removeLp(lp)}
                    >
                      ×
                    </button>
                  )}
                </span>
              ))}
            </div>
            {!readOnly && (
              <select
                className={rr.select}
                value=""
                data-testid="exit-lp-add"
                onChange={(e) => {
                  if (e.target.value) addLp(e.target.value);
                }}
              >
                <option value="">+ add LP…</option>
                {lpOptions
                  .filter((o) => !action.lps.includes(o))
                  .map((o) => (
                    <option key={o} value={o}>
                      {o}
                    </option>
                  ))}
              </select>
            )}
          </div>
          <SizeEditor size={action.size} readOnly={readOnly} onChange={(s) => patch({ size: s })} />
        </>
      )}

      {action.kind === "split" && (
        <>
          <label className={rr.toggle}>
            <input
              type="checkbox"
              checked={action.internalFirst}
              disabled={readOnly}
              data-testid="exit-internal-first"
              onChange={(e) => patch({ internalFirst: e.target.checked })}
            />
            <span>Net internally first, then externalise the residual</span>
          </label>
          <SizeEditor size={action.size} readOnly={readOnly} onChange={(s) => patch({ size: s })} />
          <label className={rr.editorField}>
            <span className={rr.fieldLabel}>Execution style (external leg)</span>
            <select
              className={rr.select}
              value={action.style}
              disabled={readOnly}
              onChange={(e) => patch({ style: e.target.value as ExecStyle })}
            >
              {EXEC_STYLES.map((s) => (
                <option key={s} value={s}>
                  {execStyleLabel(s)}
                </option>
              ))}
            </select>
          </label>
        </>
      )}

      {action.kind === "escalate" && (
        <label className={rr.editorField}>
          <span className={rr.fieldLabel}>Escalation reason</span>
          <input
            className={rr.input}
            type="text"
            value={action.reason}
            disabled={readOnly}
            data-testid="exit-reason"
            placeholder="e.g. toxic / illiquid overflow — desk to hand-manage"
            onChange={(e) => patch({ reason: e.target.value })}
          />
        </label>
      )}

      {/* WAREHOUSE + unused-field guards keep the size/style helpers imported. */}
      {!actionUsesSize(action.kind) && !actionUsesStyle(action.kind) && action.kind === "warehouse" && (
        <p className={rr.ruleEditorHint}>No parameters — the risk is simply held.</p>
      )}

      {action.kind === "clear_risk" && (
        <p className={rr.ruleEditorHint} data-testid="exit-clear-risk-note">
          No parameters — the book&rsquo;s entire net is flattened to zero via the live composite.
        </p>
      )}
    </div>
  );
}
