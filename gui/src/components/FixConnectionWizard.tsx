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

import type {
  CapabilityAction,
  CapabilityAsset,
  DeskDesc,
  FixConnection,
  FixConnectionKind,
  FixConnectionSpec,
} from "../data/contract";
import { Button } from "./Button";
import styles from "./FixConnectionWizard.module.css";

/** A selectable dialect card; FI dialects gate on a capability the caller holds. */
interface KindOption {
  readonly kind: FixConnectionKind;
  readonly name: string;
  readonly desc: string;
  /** The capability required to stand this dialect up, if any (FX-options needs none). */
  readonly cap?: { readonly action: CapabilityAction; readonly asset: CapabilityAsset };
}

/**
 * The dialects offered in the wizard's first step. FX-options needs only
 * connection administration; each fixed-income dialect additionally requires the
 * matching FI capability (`quote_respond` / `stream` on `fixed_income`) — the
 * card is disabled (never hidden) when the caller lacks it.
 */
const KIND_OPTIONS: readonly KindOption[] = [
  {
    kind: "OPTIONS",
    name: "Options",
    desc: "The FX-options RFQ dialect — QuoteRequest → Quote → lift → ExecutionReport, priced through the live engine.",
  },
  {
    kind: "FIXED_INCOME_QUOTE",
    name: "Fixed Income — Quote (RFQ)",
    desc: "The rates/OIS one-shot RFQ dialect — a QuoteRequest(263=0) is priced to a single two-way rate Quote, lifted and filled through the shared path.",
    cap: { action: "quote_respond", asset: "fixed_income" },
  },
  {
    kind: "FIXED_INCOME_STREAM",
    name: "Fixed Income — Request for Stream (RFS)",
    desc: "A MarketDataRequest(V) subscribe opens a stream priced for the CLIENT'S OWN clip — the notional the request carries. Use this when counterparties ask for a price in their size.",
    cap: { action: "stream", asset: "fixed_income" },
  },
  {
    kind: "FIXED_INCOME_ESP",
    name: "Fixed Income — Executable Streaming Price (ESP)",
    desc: "The same subscribe, but the venue streams at ITS OWN published clip and ignores any size the client carries. Shaped by the pricing group's ESP pipeline rather than its RFQ/RFS one; a lift books an ESP deal.",
    cap: { action: "stream", asset: "fixed_income" },
  },
];

/** The display label for a connection dialect kind. */
function kindDisplayLabel(kind: FixConnectionKind): string {
  return KIND_OPTIONS.find((o) => o.kind === kind)?.name ?? kind;
}

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

/** Display label for an owning desk id: "Name (id)", or the bare id if unknown. */
function deskLabel(desks: readonly DeskDesc[], deskId: string): string {
  const id = deskId.trim();
  if (id.length === 0) return "—";
  const desk = desks.find((d) => d.id === id);
  return desk ? `${desk.name} (${desk.id})` : id;
}

export interface FixConnectionWizardProps {
  /** Whether the modal is mounted/visible. */
  open: boolean;
  /** Close without creating (Esc, scrim click, Cancel). */
  onClose: () => void;
  /** Create the connection; rejects with a display message on failure. */
  onCreate: (spec: FixConnectionSpec) => Promise<FixConnection>;
  /** The current connections, for client-side uniqueness/address pre-checks. */
  existing: readonly FixConnection[];
  /**
   * The desks a connection may be routed to. The routing desk is OPTIONAL — it
   * determines which desk's users receive the venue's RFQs/deals; a blank desk
   * ("— unrouted —") accepts the session without delivering its traffic anywhere.
   * The picker shows the desk `name` but submits the stable `id`.
   */
  desks: readonly DeskDesc[];
  /**
   * Whether the caller holds `action` on `asset` — gates the fixed-income dialect
   * cards (the server enforces the same capability on create). Anonymous callers
   * run the permissive path, so this returns `true` when signed out.
   */
  can: (action: CapabilityAction, asset: CapabilityAsset) => boolean;
}

export function FixConnectionWizard({
  open,
  onClose,
  onCreate,
  existing,
  desks,
  can,
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
  // Where WE dial to send this counterparty a hedge order. Optional: a member may
  // legitimately quote us without being tradeable-to.
  const [orderEndpoint, setOrderEndpoint] = useState("");
  const [desk, setDesk] = useState("");
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
    setOrderEndpoint("");
    setTargetCompId(DEFAULT_TARGET);
    setDesk("");
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
    // The routing desk is OPTIONAL — a blank desk is a valid, intentionally-
    // unrouted connection (the review step flags it), so it never blocks the step.
    return null;
  }, [nameTrimmed, nameClash, host, portNum, enabled, addrClash, bindAddr]);

  const compIdError = useMemo((): string | null => {
    if (senderCompId.trim().length === 0) return "SenderCompID is required.";
    if (targetCompId.trim().length === 0) return "TargetCompID is required.";
    return null;
  }, [senderCompId, targetCompId]);

  // Whether the caller may select a given dialect (FX-options always; an FI
  // dialect only when the caller holds its capability).
  const kindEnabled = useCallback(
    (opt: KindOption): boolean => opt.cap === undefined || can(opt.cap.action, opt.cap.asset),
    [can],
  );

  const selectedKindEnabled = useMemo(
    (): boolean => KIND_OPTIONS.some((o) => o.kind === kind && kindEnabled(o)),
    [kind, kindEnabled],
  );

  const stepValid = useMemo((): boolean => {
    switch (STEPS[step]?.key) {
      case "kind":
        return selectedKindEnabled;
      case "identity":
        return identityError === null;
      case "compids":
        return compIdError === null;
      default:
        return true;
    }
  }, [step, selectedKindEnabled, identityError, compIdError]);

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
      desk: desk.trim(),
      orderEndpoint: orderEndpoint.trim(),
    };
    try {
      await onCreate(spec);
      onClose();
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : "could not create the connection");
      setSubmitting(false);
    }
  }, [
    nameTrimmed,
    kind,
    bindAddr,
    senderCompId,
    targetCompId,
    desk,
    enabled,
    orderEndpoint,
    onCreate,
    onClose,
  ]);

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
                {KIND_OPTIONS.map((opt) => {
                  const enabled = kindEnabled(opt);
                  const selected = kind === opt.kind;
                  const tip = enabled
                    ? undefined
                    : `Requires the ${opt.cap?.action.replace("_", " ")} capability on fixed income`;
                  return (
                    <button
                      key={opt.kind}
                      type="button"
                      className={[
                        styles.kindCard,
                        selected ? styles.kindCardOn : "",
                        enabled ? "" : styles.kindCardDisabled,
                      ]
                        .filter(Boolean)
                        .join(" ")}
                      aria-pressed={selected}
                      disabled={!enabled}
                      aria-disabled={enabled ? undefined : "true"}
                      title={tip}
                      onClick={() => {
                        if (enabled) setKind(opt.kind);
                      }}
                    >
                      <span className={styles.kindName}>{opt.name}</span>
                      <span className={styles.kindDesc}>{opt.desc}</span>
                      {!enabled && (
                        <span className={styles.kindDesc}>
                          You lack the {opt.cap?.action.replace("_", " ")} capability for
                          fixed income — ask an administrator to grant it.
                        </span>
                      )}
                    </button>
                  );
                })}
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
              <label className={styles.field}>
                <span className={styles.fieldLabel}>Routing desk (optional)</span>
                <select
                  className={styles.input}
                  value={desk}
                  onChange={(e) => setDesk(e.target.value)}
                  aria-label="Routing desk"
                >
                  <option value="">— unrouted —</option>
                  {desks.map((d) => (
                    <option key={d.id} value={d.id}>
                      {d.name}
                    </option>
                  ))}
                </select>
                <p className={styles.hint}>
                  The routing desk determines which desk&apos;s users receive this
                  venue&apos;s RFQs and executed deals. Leave it unrouted to accept
                  the session without delivering its traffic to any desk.
                </p>
                {desk.trim().length === 0 && (
                  <p className={styles.warn} role="status">
                    ⚠ Unrouted — no desk&apos;s users will receive this connection&apos;s
                    RFQs or deals.
                  </p>
                )}
                {desks.length === 0 && (
                  <p className={styles.hint}>
                    No desks defined yet — create one in the Admin workspace to route
                    this connection&apos;s traffic to a desk.
                  </p>
                )}
              </label>
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
              <p className={styles.hint}>
                The bind address above is where this counterparty reaches <em>us</em>.
                The order route below is where <em>we</em> reach <em>it</em> — the
                address we dial to send it a hedge order. Leave it blank if this
                counterparty only ever quotes: a hedge order to a member with no order
                route is answered <code>no_order_endpoint</code> and is never filled
                from its standing quote.
              </p>
              <label className={styles.field}>
                <span className={styles.fieldLabel}>Order route (optional, host:port)</span>
                <input
                  className={styles.input}
                  value={orderEndpoint}
                  placeholder="e.g. 127.0.0.1:5701"
                  onChange={(e) => setOrderEndpoint(e.target.value)}
                />
              </label>
            </div>
          )}

          {STEPS[step]?.key === "review" && (
            <div className={styles.review}>
              <dl className={styles.summary}>
                <div className={styles.summaryRow}>
                  <dt>Dialect</dt>
                  <dd>{kindDisplayLabel(kind)}</dd>
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
                  <dt>Order route</dt>
                  <dd>
                    {orderEndpoint.trim().length === 0 ? (
                      <span className={styles.warn}>
                        ⚠ None — we cannot send this counterparty orders
                      </span>
                    ) : (
                      orderEndpoint.trim()
                    )}
                  </dd>
                </div>
                <div className={styles.summaryRow}>
                  <dt>Routing desk</dt>
                  <dd>
                    {desk.trim().length === 0 ? (
                      <span className={styles.warn}>
                        ⚠ Unrouted — traffic reaches no desk
                      </span>
                    ) : (
                      deskLabel(desks, desk)
                    )}
                  </dd>
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
