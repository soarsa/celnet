/**
 * FixConnectionWizard — the guided "new inbound FIX acceptor" creation flow.
 *
 * A scrim+blur modal (the ScopeSwitcher/ReconnectOverlay material) with a four-
 * step stepper and per-step validation, building a {@link FixConnectionSpec} and
 * handing it to `onCreate` (the Connections workspace's `create`, which calls
 * `FixAdminService.CreateConnection`). On success it closes; a server rejection
 * (validation, uniqueness, address conflict, bind failure) is surfaced inline on
 * the review step so the operator can correct and retry.
 *
 * Kind is `OPTIONS` only today: SPOT FX is shown DISABLED with a "phase 2" note
 * rather than offered as a non-working choice (the data model/registry/API are
 * already kind-generic, so it lights up when the spot dialect lands).
 *
 * Accessibility: `role="dialog"` + `aria-modal`, a labelled title, Esc to close,
 * a focus trap over the panel, and initial focus on the first control.
 */

import { useCallback, useEffect, useId, useMemo, useRef, useState } from "react";

import type { FixConnection, FixConnectionKind, FixConnectionSpec } from "../data/contract";
import { Button } from "./Button";
import styles from "./FixConnectionWizard.module.css";

/** The demo venue/counterparty identities prefilled on the CompIDs step. */
const DEFAULT_SENDER = "CELNET";
const DEFAULT_TARGET = "CELNET-CPTY";
/** The valid TCP port range. */
const PORT_MIN = 1;
const PORT_MAX = 65535;

interface StepDef {
  readonly key: string;
  readonly title: string;
}

const STEPS: readonly StepDef[] = [
  { key: "kind", title: "Dialect" },
  { key: "identity", title: "Identity & bind" },
  { key: "compids", title: "CompIDs" },
  { key: "review", title: "Review" },
];

export interface FixConnectionWizardProps {
  /** Whether the modal is mounted/visible. */
  open: boolean;
  /** Close without creating (Esc, scrim click, Cancel). */
  onClose: () => void;
  /** Create the connection; rejects with a display message on failure. */
  onCreate: (spec: FixConnectionSpec) => Promise<FixConnection>;
  /** The current connections, for client-side uniqueness/address pre-checks. */
  existing: readonly FixConnection[];
}

export function FixConnectionWizard({
  open,
  onClose,
  onCreate,
  existing,
}: FixConnectionWizardProps): React.ReactElement | null {
  const titleId = useId();
  const panelRef = useRef<HTMLDivElement | null>(null);
  const firstFieldRef = useRef<HTMLInputElement | null>(null);

  const [step, setStep] = useState(0);
  const [kind, setKind] = useState<FixConnectionKind>("OPTIONS");
  const [name, setName] = useState("");
  const [host, setHost] = useState("127.0.0.1");
  const [port, setPort] = useState("9100");
  const [senderCompId, setSenderCompId] = useState(DEFAULT_SENDER);
  const [targetCompId, setTargetCompId] = useState(DEFAULT_TARGET);
  const [enabled, setEnabled] = useState(true);
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Reset to a clean first step whenever the modal (re)opens.
  useEffect(() => {
    if (!open) return;
    setStep(0);
    setKind("OPTIONS");
    setName("");
    setHost("127.0.0.1");
    setPort("9100");
    setSenderCompId(DEFAULT_SENDER);
    setTargetCompId(DEFAULT_TARGET);
    setEnabled(true);
    setSubmitting(false);
    setError(null);
  }, [open]);

  // Focus the first control on open.
  useEffect(() => {
    if (open) firstFieldRef.current?.focus();
  }, [open, step]);

  const portNum = Number.parseInt(port, 10);
  const bindAddr = `${host.trim()}:${port.trim()}`;

  // --- per-step validation -------------------------------------------------
  const nameTrimmed = name.trim();
  const nameClash = useMemo(
    () => existing.some((c) => c.name === nameTrimmed),
    [existing, nameTrimmed],
  );
  const addrClash = useMemo(
    () => existing.some((c) => c.enabled && c.bindAddr === bindAddr),
    [existing, bindAddr],
  );

  const identityError = useMemo((): string | null => {
    if (nameTrimmed.length === 0) return "Name is required.";
    if (nameClash) return `A connection named "${nameTrimmed}" already exists.`;
    if (host.trim().length === 0) return "Host is required.";
    if (!Number.isInteger(portNum) || portNum < PORT_MIN || portNum > PORT_MAX) {
      return `Port must be an integer ${PORT_MIN}–${PORT_MAX}.`;
    }
    if (enabled && addrClash) {
      return `Address ${bindAddr} is already used by an enabled connection.`;
    }
    return null;
  }, [nameTrimmed, nameClash, host, portNum, enabled, addrClash, bindAddr]);

  const compIdError = useMemo((): string | null => {
    if (senderCompId.trim().length === 0) return "SenderCompID is required.";
    if (targetCompId.trim().length === 0) return "TargetCompID is required.";
    return null;
  }, [senderCompId, targetCompId]);

  const stepValid = useMemo((): boolean => {
    switch (STEPS[step]?.key) {
      case "kind":
        return kind === "OPTIONS";
      case "identity":
        return identityError === null;
      case "compids":
        return compIdError === null;
      default:
        return true;
    }
  }, [step, kind, identityError, compIdError]);

  const isLast = step === STEPS.length - 1;

  const submit = useCallback(async (): Promise<void> => {
    setSubmitting(true);
    setError(null);
    const spec: FixConnectionSpec = {
      name: nameTrimmed,
      kind,
      bindAddr,
      senderCompId: senderCompId.trim(),
      targetCompId: targetCompId.trim(),
      enabled,
    };
    try {
      await onCreate(spec);
      onClose();
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : "could not create the connection");
      setSubmitting(false);
    }
  }, [nameTrimmed, kind, bindAddr, senderCompId, targetCompId, enabled, onCreate, onClose]);

  const advance = useCallback((): void => {
    if (!stepValid) return;
    if (isLast) void submit();
    else setStep((s) => Math.min(s + 1, STEPS.length - 1));
  }, [stepValid, isLast, submit]);

  const back = useCallback((): void => setStep((s) => Math.max(s - 1, 0)), []);

  // Esc closes; Enter advances (when the active control isn't a textarea/select).
  const onKeyDown = useCallback(
    (e: React.KeyboardEvent<HTMLDivElement>): void => {
      if (e.key === "Escape") {
        e.stopPropagation();
        onClose();
        return;
      }
      if (e.key === "Enter" && !submitting) {
        e.preventDefault();
        advance();
        return;
      }
      // Minimal focus trap: keep Tab within the panel.
      if (e.key === "Tab" && panelRef.current) {
        const focusables = panelRef.current.querySelectorAll<HTMLElement>(
          'button:not([disabled]), input:not([disabled]), [tabindex]:not([tabindex="-1"])',
        );
        const first = focusables[0];
        const last = focusables[focusables.length - 1];
        if (!first || !last) return;
        const active = document.activeElement;
        if (e.shiftKey && active === first) {
          e.preventDefault();
          last.focus();
        } else if (!e.shiftKey && active === last) {
          e.preventDefault();
          first.focus();
        }
      }
    },
    [onClose, advance, submitting],
  );

  if (!open) return null;

  return (
    <div
      className={styles.scrim}
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <div
        ref={panelRef}
        className={styles.panel}
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        onKeyDown={onKeyDown}
      >
        <header className={styles.head}>
          <h2 id={titleId} className={styles.title}>
            New FIX connection
          </h2>
          <ol className={styles.stepper} aria-label="Wizard progress">
            {STEPS.map((s, i) => (
              <li
                key={s.key}
                className={[
                  styles.stepDot,
                  i === step ? styles.stepCurrent : "",
                  i < step ? styles.stepDone : "",
                ]
                  .filter(Boolean)
                  .join(" ")}
                aria-current={i === step ? "step" : undefined}
              >
                <span className={styles.stepIndex}>{i + 1}</span>
                <span className={styles.stepLabel}>{s.title}</span>
              </li>
            ))}
          </ol>
        </header>

        <div className={styles.body}>
          {STEPS[step]?.key === "kind" && (
            <fieldset className={styles.fieldset}>
              <legend className={styles.legend}>Which dialect does this acceptor speak?</legend>
              <div className={styles.kindGrid}>
                <button
                  type="button"
                  className={[styles.kindCard, kind === "OPTIONS" ? styles.kindCardOn : ""]
                    .filter(Boolean)
                    .join(" ")}
                  aria-pressed={kind === "OPTIONS"}
                  onClick={() => setKind("OPTIONS")}
                >
                  <span className={styles.kindName}>Options</span>
                  <span className={styles.kindDesc}>
                    The FX-options RFQ dialect — QuoteRequest → Quote → lift →
                    ExecutionReport, priced through the live engine.
                  </span>
                </button>
                <button
                  type="button"
                  className={`${styles.kindCard} ${styles.kindCardDisabled}`}
                  disabled
                  aria-disabled="true"
                  title="SPOT FX acceptors arrive in a later phase"
                >
                  <span className={styles.kindName}>
                    SPOT FX <span className={styles.soon}>phase 2</span>
                  </span>
                  <span className={styles.kindDesc}>
                    Strike/expiry-less spot two-way quoting. Coming soon — the
                    model, API and UI are already kind-generic.
                  </span>
                </button>
              </div>
            </fieldset>
          )}

          {STEPS[step]?.key === "identity" && (
            <div className={styles.fields}>
              <label className={styles.field}>
                <span className={styles.fieldLabel}>Name</span>
                <input
                  ref={firstFieldRef}
                  className={styles.input}
                  value={name}
                  onChange={(e) => setName(e.target.value)}
                  placeholder="e.g. Bank A — Options"
                  aria-invalid={nameClash}
                />
              </label>
              <div className={styles.fieldRow}>
                <label className={styles.field}>
                  <span className={styles.fieldLabel}>Host</span>
                  <input
                    className={styles.input}
                    value={host}
                    onChange={(e) => setHost(e.target.value)}
                    placeholder="127.0.0.1"
                  />
                </label>
                <label className={styles.fieldNarrow}>
                  <span className={styles.fieldLabel}>Port</span>
                  <input
                    className={styles.input}
                    value={port}
                    onChange={(e) => setPort(e.target.value)}
                    inputMode="numeric"
                    placeholder="9100"
                  />
                </label>
              </div>
              <label className={styles.checkRow}>
                <input
                  type="checkbox"
                  checked={enabled}
                  onChange={(e) => setEnabled(e.target.checked)}
                />
                <span>Bind immediately (enabled) and on every startup</span>
              </label>
              {identityError && <p className={styles.error}>{identityError}</p>}
            </div>
          )}

          {STEPS[step]?.key === "compids" && (
            <div className={styles.fields}>
              <p className={styles.hint}>
                FIX session identities. The acceptor rejects any peer whose
                SenderCompID is not the TargetCompID below.
              </p>
              <label className={styles.field}>
                <span className={styles.fieldLabel}>SenderCompID (our venue)</span>
                <input
                  ref={firstFieldRef}
                  className={styles.input}
                  value={senderCompId}
                  onChange={(e) => setSenderCompId(e.target.value)}
                />
              </label>
              <label className={styles.field}>
                <span className={styles.fieldLabel}>TargetCompID (expected counterparty)</span>
                <input
                  className={styles.input}
                  value={targetCompId}
                  onChange={(e) => setTargetCompId(e.target.value)}
                />
              </label>
              {compIdError && <p className={styles.error}>{compIdError}</p>}
            </div>
          )}

          {STEPS[step]?.key === "review" && (
            <div className={styles.review}>
              <dl className={styles.summary}>
                <div className={styles.summaryRow}>
                  <dt>Dialect</dt>
                  <dd>Options</dd>
                </div>
                <div className={styles.summaryRow}>
                  <dt>Name</dt>
                  <dd>{nameTrimmed}</dd>
                </div>
                <div className={styles.summaryRow}>
                  <dt>Bind</dt>
                  <dd>{bindAddr}</dd>
                </div>
                <div className={styles.summaryRow}>
                  <dt>SenderCompID</dt>
                  <dd>{senderCompId.trim()}</dd>
                </div>
                <div className={styles.summaryRow}>
                  <dt>TargetCompID</dt>
                  <dd>{targetCompId.trim()}</dd>
                </div>
                <div className={styles.summaryRow}>
                  <dt>On create</dt>
                  <dd>{enabled ? "bind now + persist (auto-loads on restart)" : "saved, not bound"}</dd>
                </div>
              </dl>
              {error && <p className={styles.error}>{error}</p>}
            </div>
          )}
        </div>

        <footer className={styles.foot}>
          <Button variant="ghost" onClick={onClose} disabled={submitting}>
            Cancel
          </Button>
          <div className={styles.footRight}>
            {step > 0 && (
              <Button variant="secondary" onClick={back} disabled={submitting}>
                Back
              </Button>
            )}
            <Button variant="primary" onClick={advance} disabled={!stepValid || submitting}>
              {isLast ? (submitting ? "Creating…" : "Create connection") : "Next"}
            </Button>
          </div>
        </footer>
      </div>
    </div>
  );
}
