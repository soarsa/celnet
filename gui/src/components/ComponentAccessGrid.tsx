/**
 * ComponentAccessGrid — the per-user component-access editor that is the PRIMARY
 * surface of the Permissions page. Rows are trader-facing COMPONENTS (Ticket,
 * Rates Book, Administration, …) grouped into sections; columns are Read and
 * Write toggle widgets. Each toggle is a PROJECTION over the component's
 * underlying capability set (`lib/capabilityMatrix.COMPONENT_ACCESS`):
 *
 *   - Read   = the `view` capability for the component's asset. Because `view` is
 *     SHARED across every component of one asset, toggling any one Read moves all
 *     its same-asset siblings — one underlying capability (the section caption
 *     says so, so it doesn't read as a bug).
 *   - Write  = ALL of the component's write actions. ON ⇒ every action effective;
 *     OFF ⇒ none; MIXED ⇒ some but not all (the advanced view edits the mix
 *     precisely). Clicking ON grants the whole set; clicking OFF/MIXED's inverse
 *     denies it (deny-wins, visually obvious).
 *
 * Every edit round-trips through {@link useCapabilityEditor}'s overlay and Saves
 * via `SetUserCapabilities`, refreshing from the server's resolved `effective`
 * set — the component view is sugar over the one capability contract, never a
 * parallel model. Affordance discipline: a read-only component's Write toggle and
 * the signed-in admin's own Administration toggle are DISABLED with an
 * explanatory tooltip, never hidden.
 */

import { useState } from "react";

import { Button } from "./Button";
import type { Capability, UserDesc } from "../data/contract";
import type { CelnetTransport } from "../data/transport";
import { useCapabilityEditor } from "../hooks/useCapabilityEditor";
import {
  ACTION_LABELS,
  ASSET_LABELS,
  COMPONENT_ACCESS,
  COMPONENT_SECTIONS,
  type ComponentAccess,
  type ToggleState,
  capKey,
  componentAdvancedCaps,
  componentReadCaps,
  componentWriteCaps,
  isReadOnlyComponent,
  overlayStateAt,
  resolveCell,
  toggleState,
  toggleTarget,
  type OverlayMap,
  type OverlayState,
} from "../lib/capabilityMatrix";
import styles from "./ComponentAccessGrid.module.css";

interface ComponentAccessGridProps {
  /** The user whose component access is being edited. */
  user: UserDesc;
  /** The transport (live WS or offline mock). */
  transport: CelnetTransport;
  /** The signed-in admin's id — guards self-lockout on Administration. */
  signedInUserId: string | undefined;
}

/** A short, non-colour-only label for a toggle's resolved state. */
const STATE_LABEL: Record<ToggleState, string> = {
  on: "On",
  off: "Off",
  mixed: "Mixed",
};

/** Whether any capability in the set carries an active `deny` overlay. */
function hasDeny(overlay: OverlayMap, caps: readonly Capability[]): boolean {
  return caps.some((c) => overlayStateAt(overlay, c.action, c.asset) === "deny");
}

interface AccessSwitchProps {
  /** The accessible name (component + Read/Write). */
  label: string;
  /** The resolved projection state. */
  state: ToggleState;
  /** Whether an active deny is producing the off/mixed state (deny-wins styling). */
  deny: boolean;
  disabled: boolean;
  /** The tooltip — the reason when disabled, otherwise the action hint. */
  title: string;
  onToggle: () => void;
}

/**
 * One Read/Write projection toggle, a real keyboard-operable `switch`. ON/OFF map
 * to `aria-checked`; the MIXED case (which `switch` cannot express via
 * `aria-checked`) is carried in the accessible name AND a visible "Mixed" tag, so
 * indeterminacy is never conveyed by colour alone.
 */
function AccessSwitch({
  label,
  state,
  deny,
  disabled,
  title,
  onToggle,
}: AccessSwitchProps): React.ReactElement {
  const ariaLabel = state === "mixed" ? `${label}, mixed` : label;
  return (
    <button
      type="button"
      role="switch"
      aria-checked={state === "on"}
      aria-label={ariaLabel}
      className={styles.switch}
      data-state={state}
      data-deny={deny}
      disabled={disabled}
      title={title}
      onClick={onToggle}
    >
      <span className={styles.track} aria-hidden="true">
        <span className={styles.thumb} />
      </span>
      <span className={styles.stateText}>{STATE_LABEL[state]}</span>
    </button>
  );
}

interface AdvancedCellProps {
  cap: Capability;
  overlay: OverlayMap;
  role: UserDesc["role"];
  disabled: boolean;
  title: string;
  onCycle: () => void;
}

/**
 * One tri-state cell in a component's advanced view — the precise per-action
 * editor behind the disclosure. Reuses the same `resolveCell` resolution and the
 * deny-wins marker the full matrix uses, so a mixed Write toggle can be unpicked
 * action by action.
 */
function AdvancedCell({
  cap,
  overlay,
  role,
  disabled,
  title,
  onCycle,
}: AdvancedCellProps): React.ReactElement {
  const overlayState = overlayStateAt(overlay, cap.action, cap.asset);
  const cell = resolveCell(role, cap.action, cap.asset, overlayState);
  const OVERLAY_LABEL: Record<OverlayState, string> = {
    inherit: "Inherit",
    grant: "Grant",
    deny: "Deny",
  };
  const ariaLabel =
    `${ACTION_LABELS[cap.action]} on ${ASSET_LABELS[cap.asset]}: ` +
    `${cell.allowed ? "allowed" : "blocked"}, overlay ${OVERLAY_LABEL[overlayState]}` +
    `${cell.denyOverrides ? ", deny overrides role default" : ""}` +
    `${disabled ? "" : ". Activate to cycle the overlay."}`;
  return (
    <button
      type="button"
      className={styles.advCell}
      data-allowed={cell.allowed}
      data-overlay={overlayState}
      data-deny-override={cell.denyOverrides}
      disabled={disabled}
      title={title}
      aria-label={ariaLabel}
      onClick={onCycle}
    >
      <span className={styles.advAction}>{ACTION_LABELS[cap.action]}</span>
      <span className={styles.advEffect}>
        <span aria-hidden="true">{cell.allowed ? "✓ " : "✕ "}</span>
        {cell.allowed ? "Allowed" : "Blocked"}
        {cell.denyOverrides ? " · deny wins" : ""}
      </span>
      <span className={styles.advOverlay} data-overlay={overlayState}>
        {OVERLAY_LABEL[overlayState]}
        {overlayState === "inherit" ? " (role default)" : ""}
      </span>
    </button>
  );
}

interface ComponentRowProps {
  component: ComponentAccess;
  editor: ReturnType<typeof useCapabilityEditor>;
  role: UserDesc["role"];
  isSelf: boolean;
}

/** Self-lockout: a cap set is self-locked when it touches the signed-in admin's
 * own `administer` (never let an admin remove their own administration access). */
function selfLocks(isSelf: boolean, caps: readonly Capability[]): boolean {
  return isSelf && caps.some((c) => c.action === "administer");
}

/** One component row: the Read/Write toggles plus an advanced per-action drawer. */
function ComponentRow({ component, editor, role, isSelf }: ComponentRowProps): React.ReactElement {
  const [expanded, setExpanded] = useState(false);

  const readCaps = componentReadCaps(component);
  const writeCaps = componentWriteCaps(component);
  const readState = toggleState(role, editor.overlay, readCaps);
  const writeState = toggleState(role, editor.overlay, writeCaps);
  const readOnly = isReadOnlyComponent(component);

  const saving = editor.isSaving;
  const readSelfLock = selfLocks(isSelf, readCaps);
  const writeSelfLock = selfLocks(isSelf, writeCaps);

  const readDisabled = saving || readSelfLock;
  const writeDisabled = saving || readOnly || writeSelfLock;

  const readTitle = readSelfLock
    ? "You cannot change your own administration access."
    : saving
      ? "Saving…"
      : `Toggle ${component.label} read access`;
  const writeTitle = writeSelfLock
    ? "You cannot change your own administration access."
    : readOnly
      ? "This view has no write actions"
      : saving
        ? "Saving…"
        : `Toggle ${component.label} write access`;

  const advancedCaps = componentAdvancedCaps(component);
  const advId = `adv-${component.id}`;

  return (
    <>
      <tr className={styles.row}>
        <th scope="row" className={styles.rowHead}>
          <button
            type="button"
            className={styles.disclosure}
            aria-expanded={expanded}
            aria-controls={advId}
            onClick={() => setExpanded((e) => !e)}
            title={expanded ? "Hide capability actions" : "Show capability actions"}
          >
            <span className={styles.caret} aria-hidden="true" data-open={expanded}>
              ▸
            </span>
            <span className={styles.compLabel}>{component.label}</span>
          </button>
        </th>
        <td className={styles.toggleCell}>
          <AccessSwitch
            label={`${component.label} Read`}
            state={readState}
            deny={hasDeny(editor.overlay, readCaps)}
            disabled={readDisabled}
            title={readTitle}
            onToggle={() => editor.setCaps(readCaps, toggleTarget(readState))}
          />
        </td>
        <td className={styles.toggleCell}>
          {readOnly ? (
            <span className={styles.readOnlyTag} title={writeTitle}>
              Read-only
            </span>
          ) : (
            <AccessSwitch
              label={`${component.label} Write`}
              state={writeState}
              deny={hasDeny(editor.overlay, writeCaps)}
              disabled={writeDisabled}
              title={writeTitle}
              onToggle={() => editor.setCaps(writeCaps, toggleTarget(writeState))}
            />
          )}
        </td>
      </tr>
      {expanded && (
        <tr className={styles.advRow}>
          <td colSpan={3} className={styles.advWrap} id={advId}>
            <p className={styles.advHint}>
              The underlying capabilities for <strong>{component.label}</strong>. Activate a cell to
              cycle its overlay: inherit, then grant, then deny.
            </p>
            <div className={styles.advGrid}>
              {advancedCaps.map((cap) => {
                const cellSelfLock = isSelf && cap.action === "administer";
                const cellDisabled = saving || cellSelfLock;
                const cellTitle = cellSelfLock
                  ? "You cannot change your own administration access."
                  : saving
                    ? "Saving…"
                    : "Cycle overlay: inherit → grant → deny";
                return (
                  <AdvancedCell
                    key={capKey(cap.action, cap.asset)}
                    cap={cap}
                    overlay={editor.overlay}
                    role={role}
                    disabled={cellDisabled}
                    title={cellTitle}
                    onCycle={() => editor.cycle(capKey(cap.action, cap.asset))}
                  />
                );
              })}
            </div>
          </td>
        </tr>
      )}
    </>
  );
}

export function ComponentAccessGrid({
  user,
  transport,
  signedInUserId,
}: ComponentAccessGridProps): React.ReactElement {
  const editor = useCapabilityEditor(transport, user);
  const isSelf = user.id === signedInUserId;
  const roleLabel = user.role === "ADMIN" ? "Administrator" : "Trader";

  return (
    <section className={styles.root} aria-labelledby="component-access-heading">
      <header className={styles.head}>
        <div>
          <h3 id="component-access-heading" className={styles.title}>
            Access — {user.email}
          </h3>
          <p className={styles.subtitle}>
            Role baseline: <strong>{roleLabel}</strong>. Read grants the right to see a component;
            Write grants its actions. Grants widen the role, denies narrow it — a deny always wins.
          </p>
        </div>
        <div className={styles.headActions}>
          <Button
            variant="ghost"
            onClick={() => editor.reset()}
            disabled={!editor.isDirty || editor.isSaving}
            title={editor.isDirty ? "Discard unsaved changes" : "No unsaved changes"}
          >
            Reset
          </Button>
          <Button
            variant="primary"
            onClick={() => void editor.save()}
            disabled={!editor.isDirty || editor.isSaving || editor.isLoading}
            title={
              editor.isSaving
                ? "Saving…"
                : editor.isDirty
                  ? "Save access (ends the user's live sessions)"
                  : "No unsaved changes to save"
            }
          >
            {editor.isSaving ? "Saving…" : "Save access"}
          </Button>
        </div>
      </header>

      {editor.error && (
        <p className={styles.error} role="alert">
          {editor.error}
        </p>
      )}
      {editor.savedNote && (
        <p className={styles.savedNote} role="status">
          {editor.savedNote}
        </p>
      )}

      {editor.isLoading ? (
        <p className={styles.loading}>Loading access…</p>
      ) : (
        <table className={styles.table}>
          <thead>
            <tr>
              <th scope="col" className={styles.compHead}>
                Component
              </th>
              <th scope="col" className={styles.toggleHead}>
                Read
              </th>
              <th scope="col" className={styles.toggleHead}>
                Write
              </th>
            </tr>
          </thead>
          {COMPONENT_SECTIONS.map((section) => {
            const rows = COMPONENT_ACCESS.filter((c) => c.section === section.id);
            return (
              <tbody key={section.id} className={styles.section}>
                <tr>
                  <th scope="colgroup" colSpan={3} className={styles.sectionHead}>
                    <span className={styles.sectionTitle}>{section.label}</span>
                    {section.id !== "administration" && (
                      <span className={styles.sectionNote}>
                        Read is the shared view permission for this asset — toggling any row&apos;s
                        Read moves every component in this section.
                      </span>
                    )}
                  </th>
                </tr>
                {rows.map((component) => (
                  <ComponentRow
                    key={component.id}
                    component={component}
                    editor={editor}
                    role={user.role}
                    isSelf={isSelf}
                  />
                ))}
              </tbody>
            );
          })}
        </table>
      )}
    </section>
  );
}
