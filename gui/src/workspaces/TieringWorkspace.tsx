/**
 * TieringWorkspace — the trader-facing FI Tiering surface, SESSION-PIVOTED.
 *
 * Outbound pricing is no longer tuned per aggregated BOOK here. It is composed as
 * a per-client {@link PricingGroup} pipeline (Administration → Pricing Groups): a
 * group binds FIX sessions / users / desks (1-to-many; desk default) to an ESP and
 * an RFQ feature pipeline (RAW → ordered features → OUTBOUND). This surface pivots
 * on the INBOUND FIX SESSIONS and answers, per session: which pricing group applies
 * (an exact session bind, a desk default, or none), and what tiering that group's
 * pipeline(s) produce — summarised READ-ONLY from the pipeline's Tiering feature and
 * price-space guardrails.
 *
 * Editing tiering itself happens in the Pricing Groups builder (deep-linked from
 * here). Tiering is now the "Tiering" TAB of the consolidated "Pricing" workspace
 * (hosted by {@link PricingGroupsWorkspace}); the host passes {@link
 * TieringWorkspaceProps.onEditInPricingGroups} so the "Edit tiering in Pricing
 * Groups" affordance switches to the sibling "Pricing Groups" tab in place, rather
 * than a rail navigation. Rendered standalone (no prop) it falls back to
 * `setWorkspace("pricinggroups")`, so the deep-linked `tiering` id still resolves.
 *
 * This surface offers ONE structural edit: a PRICING MANAGER can reassign a
 * session between groups (moving `session.id` in/out of each group's
 * `memberConnectionIds`) via the `AuthService.UpdatePricingGroup` RPC — a group
 * membership edit, which the server gates on `manage_pricing·fixed_income`.
 *
 * Gating: a Fixed-Income client-pricing surface, rail-visible on the granular
 * `manage_pricing·fixed_income` capability (docs/PERMISSIONS-GRANULAR-REVIEW.md §4 —
 * the FI pricing-desk authority, distinct from super-admin). Reassigning a session and
 * the "Edit in Pricing Groups" deep-link both use the same cap; a manager holding it
 * edits WITHOUT full Administer, and a user lacking it never reaches the pane.
 */

import { useCallback, useEffect, useMemo, useState } from "react";

import { useApp } from "../app/AppContext";
import { Button } from "../components/Button";
import { Panel } from "../components/Panel";
import type {
  FeaturePipeline,
  FixConnection,
  PricingGroup,
  TieringConfig,
  TieringGuardrails,
  TieringStrategy,
} from "../data/contract";
import { capabilityDenialTitle } from "../lib/capabilityMatrix";
import { FEATURE_KIND_LABEL } from "../lib/pricingGroups";
import { TIERING_PREVIEW_RAW, TIERING_SPREAD_UNIT_LABEL, TIERING_STRATEGY_KIND_LABEL } from "../lib/tiering";
import { clientTwoWayFromTiering, hasPositionDependentSkew } from "../lib/tieringPreview";
import styles from "./TieringWorkspace.module.css";

/** How a session's applied pricing group was matched (mirrors the server resolver). */
type GroupMatch = "session" | "desk" | "none";

/** The resolved pricing group for a session plus HOW it matched. */
interface Resolved {
  group: PricingGroup | null;
  match: GroupMatch;
}

/**
 * Resolve the pricing group that prices a session, mirroring the server resolver:
 * among ENABLED groups, first the group whose `memberConnectionIds` binds this
 * session; else the group whose `memberDesks` carries the session's routing desk
 * (a desk-level default); else none (the session streams the raw composite).
 */
function resolveGroup(session: FixConnection, enabledGroups: readonly PricingGroup[]): Resolved {
  const byConn = enabledGroups.find((g) => g.memberConnectionIds.includes(session.id));
  if (byConn) return { group: byConn, match: "session" };
  if (session.desk !== "") {
    const byDesk = enabledGroups.find((g) => g.memberDesks.includes(session.desk));
    if (byDesk) return { group: byDesk, match: "desk" };
  }
  return { group: null, match: "none" };
}

/** A short human phrase for how the group matched (for the applied-group note). */
function matchLabel(r: Resolved, desk: string): string {
  switch (r.match) {
    case "session":
      return "bound to this session";
    case "desk":
      return `desk default${desk ? ` (${desk})` : ""}`;
    case "none":
      return "no group";
  }
}

/** A compact number rendering (drops trailing zeros; never scientific). */
function num(n: number): string {
  if (!Number.isFinite(n)) return "—";
  return String(Number(n.toPrecision(6)));
}

/** A price rendering for the worked client-price readout (fixed 2 dp, e.g. 99.30). */
function px(n: number): string {
  if (!Number.isFinite(n)) return "—";
  return n.toFixed(2);
}

/** The Tiering feature (if any) carried by a pipeline, plus its ordered feature kinds. */
function pipelineTiering(p: FeaturePipeline): TieringConfig | null {
  const feat = p.features.find((f) => f.kind === "TIERING" && f.tiering !== null);
  return feat ? feat.tiering : null;
}

/** A one-line summary of a single tiering strategy's key parameters. */
function strategyParams(s: TieringStrategy, unitLabel: string): string {
  switch (s.kind) {
    case "FLAT_MARKUP":
      return `H ${num(s.halfSpread)} ${unitLabel}`;
    case "INVENTORY_SKEW":
      return `H ${num(s.halfSpread)} ${unitLabel} · κ ${num(s.kappa)} · sMax ${num(s.sMax)}`;
    case "SCALED_SMOOTHED_SPREAD":
      return `c ${num(s.coreSpread)} · m ${num(s.maxOutputSpread)} · w ${num(
        s.smoothingWeight,
      )} · e ${num(s.expectedSpread)} · f ${num(s.spreadScaleFactor)}`;
    default:
      return "";
  }
}

/** A one-line summary of the pipeline's price-space guardrails. */
function guardrailLine(g: TieringGuardrails): string {
  return `h_min ${num(g.hMin)} · h_max ${num(g.hMax)} · s_max ${num(g.sMax)} · spread_floor ${num(
    g.spreadFloor,
  )}`;
}

/** The result of the last reassign attempt (inline success / failure feedback). */
type AssignState =
  | { kind: "idle" }
  | { kind: "saving" }
  | { kind: "ok"; message: string }
  | { kind: "error"; message: string };

/** A read-only render of one pipeline's ordered features + its Tiering summary. */
function PipelineSummary({ label, pipeline }: { label: string; pipeline: FeaturePipeline | null }): React.ReactElement {
  if (pipeline === null) {
    return (
      <div className={styles.pipeBlock}>
        <span className={styles.pipeHead}>{label}</span>
        <span className={styles.noTier}>No custom pipeline — this mode uses the book-default pricing.</span>
      </div>
    );
  }
  const tiering = pipelineTiering(pipeline);
  const unitLabel = tiering ? TIERING_SPREAD_UNIT_LABEL[tiering.unit] : "";
  // The worked client-price example: feed the reference sample raw two-way through
  // this pipeline's TIERING config, clamped by the guardrails the summary above shows
  // (the pipeline's, falling back to the config's own). A pipeline with no TIERING
  // feature streams the raw composite unmarked. INVENTORY_SKEW's position-linear lean
  // is shown as a note, not a fabricated number (see tieringPreview).
  const previewTiering =
    tiering !== null ? { ...tiering, guardrails: pipeline.guardrails ?? tiering.guardrails } : null;
  const client = clientTwoWayFromTiering(previewTiering, TIERING_PREVIEW_RAW.bid, TIERING_PREVIEW_RAW.offer);
  const positionDependent = hasPositionDependentSkew(tiering);
  return (
    <div className={styles.pipeBlock}>
      <span className={styles.pipeHead}>{label}</span>
      <div className={styles.chips}>
        {pipeline.features.length === 0 ? (
          <span className={styles.noTier}>No features (pass-through).</span>
        ) : (
          pipeline.features.map((f, i) => (
            <span key={`${f.kind}-${i}`} className={styles.chip}>
              {FEATURE_KIND_LABEL[f.kind]}
            </span>
          ))
        )}
      </div>
      {tiering === null ? (
        <span className={styles.noTier}>No tiering in this group&apos;s pipeline.</span>
      ) : (
        <div className={styles.tierWrap}>
          <span className={styles.metaLine}>Spread unit: {unitLabel}</span>
          {tiering.strategies.length === 0 ? (
            <span className={styles.noTier}>Tiering enabled with no strategies.</span>
          ) : (
            tiering.strategies.map((s, i) => (
              <span key={`${s.kind}-${i}`} className={styles.stratLine}>
                <span className={styles.stratName}>{TIERING_STRATEGY_KIND_LABEL[s.kind]}</span>{" "}
                <span className={styles.mono}>{strategyParams(s, unitLabel)}</span>
              </span>
            ))
          )}
        </div>
      )}
      {pipeline.guardrails && (
        <span className={styles.guardLine}>
          Guardrails: <span className={styles.mono}>{guardrailLine(pipeline.guardrails)}</span>
        </span>
      )}
      <div className={styles.clientPrice} aria-label={`${label} client price preview`}>
        <span className={styles.clientTag}>SAMPLE</span>
        <span className={styles.mono}>
          LP {px(TIERING_PREVIEW_RAW.bid)} / {px(TIERING_PREVIEW_RAW.offer)}
        </span>
        <span className={styles.clientArrow} aria-hidden="true">
          →
        </span>
        {tiering === null ? (
          <span className={styles.clientRaw}>raw composite — no client markup</span>
        ) : (
          <span className={`${styles.mono} ${styles.clientOut}`}>
            client {px(client.bid)} / {px(client.offer)}
          </span>
        )}
        {positionDependent && (
          <span className={styles.clientNote}>+ inventory skew (position-dependent)</span>
        )}
      </div>
    </div>
  );
}

/** Props for the Tiering surface. */
interface TieringWorkspaceProps {
  /**
   * Switch the consolidated Pricing workspace to its "Pricing Groups" tab — supplied
   * by the host ({@link PricingGroupsWorkspace}) so the "Edit tiering in Pricing
   * Groups" affordance moves tabs in place. Omitted when rendered standalone (a
   * deep-linked `tiering` id), where it falls back to `setWorkspace("pricinggroups")`.
   */
  onEditInPricingGroups?: () => void;
}

export function TieringWorkspace({
  onEditInPricingGroups,
}: TieringWorkspaceProps = {}): React.ReactElement {
  const app = useApp();
  const { auth } = app;
  const signedIn = auth.user !== undefined && auth.user !== null;
  // The FI pricing-desk capability the server gates session reassignment on (a group
  // membership edit). Admin holds it via grant-all; rail visibility uses the same cap,
  // so anyone reaching the pane may reassign. `can` is permissive signed-out.
  const canManagePricing = auth.can("manage_pricing", "fixed_income");
  const readOnly = !canManagePricing;

  const [sessions, setSessions] = useState<FixConnection[]>([]);
  const [groups, setGroups] = useState<PricingGroup[]>([]);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [assignState, setAssignState] = useState<AssignState>({ kind: "idle" });

  // Load (and reload) both rosters. `listFixConnections` / `listPricingGroups` are
  // authenticated-only, so any signed-in user may read them. Default selection lands
  // on the first ENABLED (running-or-armed) session, else the first session.
  const reloadGroups = useCallback(async (): Promise<PricingGroup[]> => {
    const list = await app.transport.listPricingGroups();
    setGroups(list);
    return list;
  }, [app.transport]);

  const reloadAll = useCallback(async (): Promise<void> => {
    const [sess, grps] = await Promise.all([
      app.transport.listFixConnections(),
      app.transport.listPricingGroups(),
    ]);
    setSessions(sess);
    setGroups(grps);
    setLoadError(null);
    setSelectedId((prev) => {
      if (prev && sess.some((s) => s.id === prev)) return prev;
      const first = sess.find((s) => s.enabled) ?? sess[0];
      return first ? first.id : null;
    });
  }, [app.transport]);

  useEffect(() => {
    if (!signedIn) {
      setSessions([]);
      setGroups([]);
      setSelectedId(null);
      return;
    }
    let cancelled = false;
    void reloadAll().catch((e: unknown) => {
      if (cancelled) return;
      setLoadError(e instanceof Error ? e.message : "failed to load FIX sessions and pricing groups");
    });
    return () => {
      cancelled = true;
    };
  }, [reloadAll, signedIn]);

  const enabledGroups = useMemo(() => groups.filter((g) => g.enabled), [groups]);

  const selectedSession = useMemo(
    () => sessions.find((s) => s.id === selectedId) ?? null,
    [sessions, selectedId],
  );

  // Clear stale reassign feedback whenever the selected session changes.
  useEffect(() => {
    setAssignState({ kind: "idle" });
  }, [selectedId]);

  const resolved = useMemo<Resolved | null>(
    () => (selectedSession ? resolveGroup(selectedSession, enabledGroups) : null),
    [selectedSession, enabledGroups],
  );

  // The group the session is EXPLICITLY bound to (its `memberConnectionIds`), which
  // drives the assign <select>'s value — a desk-default match leaves it unbound ("").
  const boundGroupId = resolved && resolved.match === "session" && resolved.group ? resolved.group.id : "";

  const reassign = useCallback(
    async (nextGroupId: string): Promise<void> => {
      if (!selectedSession || !canManagePricing) return;
      const sid = selectedSession.id;
      // Build the COMPLETE updated specs immutably: strip this session from every
      // group that currently lists it (except the target), and add it to the chosen
      // group. Other members / pipelines / fields are preserved verbatim.
      const updates: PricingGroup[] = [];
      for (const g of groups) {
        if (g.id !== nextGroupId && g.memberConnectionIds.includes(sid)) {
          updates.push({ ...g, memberConnectionIds: g.memberConnectionIds.filter((x) => x !== sid) });
        }
      }
      const target = nextGroupId === "" ? undefined : groups.find((g) => g.id === nextGroupId);
      if (target && !target.memberConnectionIds.includes(sid)) {
        updates.push({ ...target, memberConnectionIds: [...target.memberConnectionIds, sid] });
      }
      if (updates.length === 0) {
        setAssignState({ kind: "idle" });
        return;
      }
      setAssignState({ kind: "saving" });
      try {
        for (const u of updates) {
          await app.transport.updatePricingGroup(u.id, u);
        }
        await reloadGroups();
        setAssignState({
          kind: "ok",
          message: target ? `Assigned to ${target.name}.` : "Session binding cleared.",
        });
      } catch (e: unknown) {
        setAssignState({
          kind: "error",
          message: e instanceof Error ? e.message : "failed to reassign the session",
        });
      }
    },
    [app.transport, groups, canManagePricing, reloadGroups, selectedSession],
  );

  // --- the sign-in gate ----------------------------------------------------
  if (!signedIn) {
    return (
      <div className={styles.wrap}>
        <div className={styles.gate}>
          <h2 className={styles.gateTitle}>Session tiering</h2>
          <p className={styles.gateHint}>
            Sign in to see which pricing group prices each FIX session and how its tiering is
            composed.
          </p>
          <Button variant="primary" onClick={() => app.setSignInOpen(true)}>
            Sign in
          </Button>
        </div>
      </div>
    );
  }

  return (
    <div className={styles.wrap}>
      <div className={styles.head}>
        <div className={styles.headMain}>
          <span className={styles.title}>Tiering</span>
          <span className={styles.note}>
            Inbound FIX sessions on the left, the pricing group applied to each (an exact session
            bind, a desk default, or none), and its tiering summarised from the group&apos;s pipeline.
            Reassign a session or deep-link into Pricing Groups to edit the pricing itself.
          </span>
        </div>
      </div>

      {readOnly && (
        <p className={styles.permBanner} role="note">
          <span className={styles.permGlyph} aria-hidden="true">
            🔒︎
          </span>
          You don&apos;t have the{" "}
          <strong>Manage Pricing · Fixed Income</strong> capability{" "}
          <span className={styles.permHint}>
            ({capabilityDenialTitle("manage_pricing", "fixed_income")})
          </span>{" "}
          — sessions and their applied pricing are shown below read-only.
        </p>
      )}

      {loadError && <p className={styles.banner}>{loadError}</p>}

      {sessions.length === 0 ? (
        <div className={styles.empty}>
          No FIX sessions are defined — an administrator can add one under{" "}
          <strong>Administration → Connections</strong>.
        </div>
      ) : (
        <div className={styles.body}>
          {/* LEFT — the session roster, each row badged with its resolved group. */}
          <section className={styles.roster} aria-label="select a FIX session">
            <h3 className={styles.rosterHead}>Sessions</h3>
            <ul className={styles.bookList}>
              {sessions.map((s) => {
                const r = resolveGroup(s, enabledGroups);
                const active = selectedId === s.id;
                return (
                  <li key={s.id}>
                    <button
                      type="button"
                      className={`${styles.bookBtn} ${active ? styles.bookBtnActive : ""}`}
                      aria-pressed={active}
                      onClick={() => setSelectedId(s.id)}
                    >
                      <span className={styles.bookRow}>
                        <span className={styles.bookName}>{s.name}</span>
                        {!s.enabled && <span className={styles.bookOff}>off</span>}
                      </span>
                      <span className={styles.sessionSub}>
                        <span>
                          {s.senderCompId} → {s.targetCompId}
                        </span>
                        {s.desk !== "" && <span className={styles.deskTag}>{s.desk}</span>}
                      </span>
                      <span className={styles.bookMeta}>
                        <span
                          className={`${styles.tierBadge} ${s.running ? styles.tierOn : styles.tierOff}`}
                        >
                          <span className={styles.tierDot} aria-hidden="true" />
                          {s.running ? "Live" : "Idle"}
                        </span>
                        <span
                          className={`${styles.groupChip} ${r.group ? "" : styles.groupChipNone}`}
                          title={r.group ? matchLabel(r, s.desk) : "No pricing group applies"}
                        >
                          {r.group ? r.group.name : "No group"}
                        </span>
                      </span>
                    </button>
                  </li>
                );
              })}
            </ul>
          </section>

          {/* RIGHT — the selected session's applied pricing + read-only tiering. */}
          <section className={styles.editorPane} aria-label="session pricing">
            {selectedSession === null || resolved === null ? (
              <Panel title="Session pricing" glyph="⚙">
                <div className={styles.empty}>Select a session to view its applied pricing group.</div>
              </Panel>
            ) : (
              <Panel
                title={selectedSession.name}
                glyph="⚙"
                actions={
                  assignState.kind === "ok" ? (
                    <span className={styles.okBadge} role="status" aria-live="polite">
                      ✓ Saved
                    </span>
                  ) : (
                    <span className={styles.editorId}>{selectedSession.id}</span>
                  )
                }
              >
                <div className={styles.detailStack}>
                  {/* Session identity. */}
                  <div className={styles.readonly} aria-label="session identity">
                    <div className={styles.roRow}>
                      <span className={styles.roLabel}>Comp IDs</span>
                      <span className={`${styles.roValue} ${styles.mono}`}>
                        {selectedSession.senderCompId} → {selectedSession.targetCompId}
                      </span>
                    </div>
                    <div className={styles.roRow}>
                      <span className={styles.roLabel}>Desk</span>
                      <span className={styles.roValue}>
                        {selectedSession.desk !== "" ? selectedSession.desk : "— unrouted"}
                      </span>
                    </div>
                    <div className={styles.roRow}>
                      <span className={styles.roLabel}>State</span>
                      <span className={styles.roValue}>
                        {selectedSession.running ? "Live" : "Idle"}
                        {!selectedSession.enabled && " · disabled"}
                      </span>
                    </div>
                  </div>

                  {/* Applied pricing group. */}
                  <div className={styles.readonly} aria-label="applied pricing group">
                    <div className={styles.roRow}>
                      <span className={styles.roLabel}>Applied pricing group</span>
                      <span className={styles.roValue}>
                        {resolved.group ? resolved.group.name : "None"}
                      </span>
                    </div>
                    <span className={styles.matchNote}>
                      {resolved.group
                        ? `Matched as ${matchLabel(resolved, selectedSession.desk)}${
                            resolved.group.enabled ? "" : " · group disabled"
                          }.`
                        : "No group — this session streams the raw composite (no per-session tiering)."}
                    </span>
                  </div>

                  {/* Read-only tiering summary from the group's pipeline(s). */}
                  {resolved.group && (
                    <div className={styles.pipeGrid} aria-label="pipeline tiering summary">
                      {resolved.group.sharePipeline ? (
                        <PipelineSummary label="ESP & RFQ (shared)" pipeline={resolved.group.espPipeline} />
                      ) : (
                        <>
                          <PipelineSummary label="ESP (streaming)" pipeline={resolved.group.espPipeline} />
                          <PipelineSummary label="RFQ / RFS" pipeline={resolved.group.rfqPipeline} />
                        </>
                      )}
                    </div>
                  )}

                  {/* Reassign control (admin) + deep-link to the editor. */}
                  <div className={styles.assign}>
                    <div className={styles.assignRow}>
                      <label className={styles.assignLabel} htmlFor="tier-assign">
                        Assign to group
                      </label>
                      <select
                        id="tier-assign"
                        className={styles.select}
                        value={boundGroupId}
                        disabled={!canManagePricing || assignState.kind === "saving"}
                        title={canManagePricing ? undefined : "Manage-Pricing capability required to reassign — view only"}
                        onChange={(e) => void reassign(e.target.value)}
                      >
                        <option value="">No group (clear session binding)</option>
                        {enabledGroups.map((g) => (
                          <option key={g.id} value={g.id}>
                            {g.name}
                          </option>
                        ))}
                      </select>
                      {canManagePricing ? (
                        <Button
                          variant="ghost"
                          onClick={() =>
                            onEditInPricingGroups
                              ? onEditInPricingGroups()
                              : app.setWorkspace("pricinggroups")
                          }
                          title="Open the Pricing Groups builder to edit this pipeline"
                        >
                          Edit tiering in Pricing Groups →
                        </Button>
                      ) : (
                        <span className={styles.dirtyHint}>Manage-Pricing capability required to reassign — view only</span>
                      )}
                    </div>
                    {assignState.kind === "error" && (
                      <p className={styles.banner} role="alert">
                        {assignState.message}
                      </p>
                    )}
                    {assignState.kind === "ok" && (
                      <span className={styles.dirtyHint} aria-live="polite">
                        {assignState.message}
                      </span>
                    )}
                  </div>
                </div>
              </Panel>
            )}
          </section>
        </div>
      )}
    </div>
  );
}
