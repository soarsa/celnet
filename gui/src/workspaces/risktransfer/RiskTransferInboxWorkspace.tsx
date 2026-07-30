/**
 * RiskTransferInboxWorkspace — the FI Risk Transfer INBOX (docs/RISK-TRANSFER-
 * REQUIREMENTS.md §9.2). The four-eyes counterparty side: every transfer currently
 * PENDING the subscriber's accept/reject, pushed live over `streamRiskTransferInbox`
 * (subscribe on mount, dispose on unmount). Each item shows source → target, quantity,
 * price basis + reason, and a moved-risk preview. Accept books the two offsetting legs
 * (a re-attribution is single-control and never lands here); Reject declines with a
 * reason. The live server enforces approver ≠ initiator — surfaced as informational
 * text; the offline sandbox lets one principal drive both sides so the flow is
 * demonstrable end-to-end. Reads the transport via `useApp()`.
 */

import { useCallback, useEffect, useMemo, useState } from "react";

import { useApp } from "../../app/AppContext";
import type { RiskBook, RiskBookRisk, RiskTransfer } from "../../data/contract";
import { fmtClock, fmtCompact } from "../../lib/format";
import { movedNotionalOf } from "./transferModel";
import styles from "./RiskTransferInboxWorkspace.module.css";

export function RiskTransferInboxWorkspace(): React.ReactElement {
  const app = useApp();
  const { auth } = app;
  const signedIn = auth.user !== null && auth.user !== undefined;

  const [pending, setPending] = useState<RiskTransfer[]>([]);
  const [books, setBooks] = useState<RiskBook[]>([]);
  const [risk, setRisk] = useState<RiskBookRisk[]>([]);
  const [busyId, setBusyId] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const [rejectingId, setRejectingId] = useState<string | null>(null);
  const [rejectReason, setRejectReason] = useState("");

  useEffect(() => {
    if (!signedIn) {
      setPending([]);
      setBooks([]);
      setRisk([]);
      return;
    }
    let cancelled = false;
    // The book roster + risk rows name the legs and drive the moved-risk preview.
    void Promise.all([app.transport.listRiskBooks(), app.transport.listRiskBookRisk()])
      .then(([b, r]) => {
        if (cancelled) return;
        setBooks(b);
        setRisk(r);
      })
      .catch(() => {
        /* names degrade to ids; not fatal for the inbox */
      });
    // Live inbox: the mock re-pushes the current PENDING set on every transfer mutation.
    const dispose = app.transport.streamRiskTransferInbox((items) => {
      if (!cancelled) setPending(items);
    });
    return () => {
      cancelled = true;
      dispose();
    };
  }, [app.transport, signedIn]);

  const bookName = useCallback(
    (id: string): string => books.find((b) => b.id === id)?.name ?? id,
    [books],
  );
  const netOf = useCallback(
    (id: string): number => risk.find((r) => r.bookId === id)?.netNotional ?? 0,
    [risk],
  );
  const dv01Absent = useMemo(() => risk.every((r) => r.dv01 === null), [risk]);

  const refreshRisk = useCallback((): void => {
    void app.transport.listRiskBookRisk().then((r) => setRisk(r)).catch(() => undefined);
  }, [app.transport]);

  const onAccept = useCallback(
    async (id: string): Promise<void> => {
      setBusyId(id);
      setActionError(null);
      try {
        await app.transport.acceptRiskTransfer(id);
        refreshRisk();
      } catch (e: unknown) {
        setActionError(e instanceof Error ? e.message : "accept failed");
      } finally {
        setBusyId(null);
      }
    },
    [app.transport, refreshRisk],
  );

  const onReject = useCallback(
    async (id: string): Promise<void> => {
      setBusyId(id);
      setActionError(null);
      try {
        await app.transport.rejectRiskTransfer(id, rejectReason);
        setRejectingId(null);
        setRejectReason("");
      } catch (e: unknown) {
        setActionError(e instanceof Error ? e.message : "reject failed");
      } finally {
        setBusyId(null);
      }
    },
    [app.transport, rejectReason],
  );

  if (!signedIn) {
    return (
      <div className={styles.wrap}>
        <p className={styles.empty}>Sign in to review incoming transfers.</p>
      </div>
    );
  }

  return (
    <div className={styles.wrap}>
      <header className={styles.head}>
        <h1 className={styles.title}>Transfer Inbox</h1>
        <p className={styles.note}>
          Incoming desk-to-desk and trader hand-off transfers awaiting your acceptance.
          Accepting books the two offsetting legs and moves the risk.
        </p>
      </header>

      <p className={styles.fourEyes} role="note">
        <strong>Four-eyes control.</strong> On the live desk an approver must differ from the
        initiator — the server rejects a self-approval. This sandbox is single-user, so one
        principal can drive both sides to demonstrate the flow.
      </p>

      {actionError && (
        <p className={styles.error} role="alert" data-testid="inbox-error">
          {actionError}
        </p>
      )}

      {pending.length === 0 ? (
        <p className={styles.empty} data-testid="inbox-empty">
          No transfers awaiting acceptance. Desk-to-desk and trader hand-off transfers land here
          Pending; re-attributions within one desk book immediately and never appear.
        </p>
      ) : (
        <ul className={styles.list} data-testid="inbox-list">
          {pending.map((t) => {
            const moved = movedNotionalOf(netOf(t.source.riskBookId), t.quantityFull, t.partialNotional);
            const price = t.priceBasis === "AGREED" ? (t.agreedPrice ?? 0) : 100;
            return (
              <li key={t.id} className={styles.item} data-testid={`inbox-item-${t.id}`}>
                <div className={styles.itemHead}>
                  <span className={`${styles.kindBadge} ${styles[`kind_${t.kind}`]}`}>
                    {t.kind.replace(/_/g, " ").toLowerCase()}
                  </span>
                  <span className={styles.itemId}>#{t.id}</span>
                  <span className={styles.itemTime}>{fmtClock(t.initiatedAt)}</span>
                </div>

                <div className={styles.route}>
                  <span className={styles.routeBook}>{bookName(t.source.riskBookId)}</span>
                  <span className={styles.routeArrow} aria-hidden>
                    →
                  </span>
                  <span className={styles.routeBook}>{bookName(t.target.riskBookId)}</span>
                  {t.target.trader.length > 0 && (
                    <span className={styles.routeTrader}>· {t.target.trader}</span>
                  )}
                </div>

                <dl className={styles.facts}>
                  <div className={styles.fact}>
                    <dt>Quantity</dt>
                    <dd className={styles.mono}>
                      {t.quantityFull ? "Full" : fmtCompact(t.partialNotional ?? 0)}
                    </dd>
                  </div>
                  <div className={styles.fact}>
                    <dt>Moved notional</dt>
                    <dd className={styles.mono}>{fmtCompact(moved)}</dd>
                  </div>
                  <div className={styles.fact}>
                    <dt>Price</dt>
                    <dd className={styles.mono}>
                      {t.priceBasis === "MARK_TO_MARKET" ? "Mark" : t.priceBasis === "MID" ? "Mid" : "Agreed"}{" "}
                      {price.toFixed(2)}
                    </dd>
                  </div>
                  <div className={styles.fact}>
                    <dt>Initiated by</dt>
                    <dd>{t.initiatedBy}</dd>
                  </div>
                  {!dv01Absent && (
                    <div className={styles.fact}>
                      <dt>Basis risk</dt>
                      <dd className={styles.mono}>DV01-carrying</dd>
                    </div>
                  )}
                </dl>

                {t.reason.trim().length > 0 && (
                  <p className={styles.reason}>
                    <span className={styles.reasonLabel}>Reason</span> {t.reason}
                  </p>
                )}

                {rejectingId === t.id ? (
                  <div className={styles.rejectBox}>
                    <label className={styles.field}>
                      <span className={styles.fieldLabel}>Reject reason</span>
                      <input
                        className={styles.input}
                        value={rejectReason}
                        onChange={(e) => setRejectReason(e.target.value)}
                        data-testid={`inbox-reject-reason-${t.id}`}
                        placeholder="Why is this transfer declined?"
                      />
                    </label>
                    <div className={styles.actions}>
                      <button
                        type="button"
                        className={styles.dangerBtn}
                        disabled={busyId === t.id || rejectReason.trim().length === 0}
                        onClick={() => void onReject(t.id)}
                        data-testid={`inbox-reject-confirm-${t.id}`}
                      >
                        Confirm reject
                      </button>
                      <button
                        type="button"
                        className={styles.ghostBtn}
                        onClick={() => {
                          setRejectingId(null);
                          setRejectReason("");
                        }}
                      >
                        Cancel
                      </button>
                    </div>
                  </div>
                ) : (
                  <div className={styles.actions}>
                    <button
                      type="button"
                      className={styles.acceptBtn}
                      disabled={busyId === t.id}
                      onClick={() => void onAccept(t.id)}
                      data-testid={`inbox-accept-${t.id}`}
                    >
                      {busyId === t.id ? "Booking…" : "Accept & book"}
                    </button>
                    <button
                      type="button"
                      className={styles.rejectBtn}
                      disabled={busyId === t.id}
                      onClick={() => {
                        setRejectingId(t.id);
                        setRejectReason("");
                      }}
                      data-testid={`inbox-reject-${t.id}`}
                    >
                      Reject
                    </button>
                  </div>
                )}
              </li>
            );
          })}
        </ul>
      )}
    </div>
  );
}
