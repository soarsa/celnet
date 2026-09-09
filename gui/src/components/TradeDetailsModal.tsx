/**
 * TradeDetailsModal — the ONE trade-detail popup, for every blotter.
 *
 * A desk asks the same question of a client fill and of a hedge — *what exactly was this
 * trade?* — so both answer it here, in the same chrome, opened the same way. What each
 * kind actually contains is decided upstream by the {@link TradeDetails} adapters; this
 * component never branches on kind beyond a single label chip, which is what keeps the
 * two surfaces from drifting apart.
 *
 * A hedge additionally carries the ORDERS it put on the wire. That table renders here
 * rather than inline under the ledger, because a modal is where the app already puts
 * "show me everything about this one row", and an inline panel could only ever exist on
 * the one screen that grew it.
 *
 * Accessibility: `role="dialog"` + `aria-modal`, a labelled title, Escape and
 * backdrop-click to close, initial focus moved into the dialog, and focus returned to
 * the opener on close (a blotter row is a scroll position — losing it is losing your
 * place in a table of hundreds).
 */

import { useEffect, useId, useRef } from "react";
import { createPortal } from "react-dom";

import type { StreetOrder } from "../data/contract";
import { fmtAmount, humanToken, type TradeDetails } from "../lib/tradeDetails";
import styles from "./TradeDetailsModal.module.css";

export interface TradeDetailsModalProps {
  /** The trade to show, or `null` when closed. */
  readonly details: TradeDetails | null;
  /** Close the modal (clears the parent's selection). */
  readonly onClose: () => void;
  /**
   * Surface-specific detail rendered under the common sections.
   *
   * The shared adapters carry what EVERY trade has. A client fill additionally carries
   * settlement-grade terms — schedule dates, day-count, fixed frequency — that only a
   * cash/OIS deal has and that middle office genuinely needs. Those render here rather
   * than being flattened into the common shape, so unifying the CHROME does not cost the
   * ticket its content.
   */
  readonly extra?: React.ReactNode;
}

/** The orders a hedge sent, or an honest statement of why there are none. */
function OrdersTable({
  orders,
  kind,
}: {
  readonly orders: readonly StreetOrder[];
  readonly kind: TradeDetails["kind"];
}): React.ReactElement | null {
  if (kind !== "hedge") return null;
  if (orders.length === 0) {
    return (
      <section className={styles.section}>
        <h3 className={styles.sectionTitle}>Orders sent to market</h3>
        <p className={styles.noOrders} data-testid="trade-details-no-orders">
          No street order was recorded against this hedge. An internalised decision never
          asks the street, so this is a fact about what it did — not a failed load.
        </p>
      </section>
    );
  }
  const filled = orders.filter((o) => o.filledQty > 0).length;
  return (
    <section className={styles.section}>
      <h3 className={styles.sectionTitle}>
        Orders sent to market
        <span className={styles.sectionCount}>
          {orders.length} order{orders.length === 1 ? "" : "s"} · {filled} filled
        </span>
      </h3>
      <div className={styles.ordersScroll}>
        <table className={styles.orders}>
          <caption className={styles.ordersCaption}>
            One row per order actually sent, oldest first — the order a shed walked its panel in.
          </caption>
          <thead>
            <tr>
              <th scope="col">Order</th>
              <th scope="col">Provider</th>
              <th scope="col">Instrument</th>
              <th scope="col">Side</th>
              <th scope="col" className={styles.num}>
                Requested
              </th>
              <th scope="col" className={styles.num}>
                Filled
              </th>
              <th scope="col" className={styles.num}>
                Fill px
              </th>
              <th scope="col">Outcome</th>
            </tr>
          </thead>
          <tbody>
            {orders.map((o) => (
              <tr key={o.orderId} data-testid={`trade-details-order-${o.orderId}`}>
                <td className={styles.mono}>{o.orderId}</td>
                <td>
                  {o.lpId ?? (
                    <span
                      className={styles.composite}
                      title="No provider was credited — the street showed no firm price, so this went to the composite backstop or nowhere at all."
                    >
                      composite
                    </span>
                  )}
                </td>
                <td className={styles.mono}>{o.instrument}</td>
                <td>{o.side}</td>
                <td className={styles.num}>{fmtAmount(o.requestedQty)}</td>
                <td className={styles.num}>{fmtAmount(o.filledQty)}</td>
                <td className={styles.num}>
                  {o.filledPrice === undefined ? "—" : o.filledPrice.toFixed(6)}
                </td>
                <td>
                  <span className={styles.outcome}>{humanToken(o.outcome)}</span>
                  {o.reason !== undefined && o.reason !== "" && (
                    // Printed, never tooltipped: this string is the whole diagnosis when a
                    // hedge fires and the book does not move.
                    <span className={styles.reason} data-testid="trade-details-order-reason">
                      {humanToken(o.reason)}
                    </span>
                  )}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </section>
  );
}

export function TradeDetailsModal({
  details,
  onClose,
  extra,
}: TradeDetailsModalProps): React.ReactElement | null {
  const titleId = useId();
  const dialogRef = useRef<HTMLDivElement | null>(null);
  // The element that had focus when the modal opened — a blotter row. Returning focus
  // there on close keeps the reader's place in a table of hundreds.
  const openerRef = useRef<Element | null>(null);

  const open = details !== null;

  useEffect(() => {
    if (!open) return;
    openerRef.current = document.activeElement;
    dialogRef.current?.focus();
    const onKey = (e: KeyboardEvent): void => {
      if (e.key === "Escape") {
        e.stopPropagation();
        onClose();
      }
    };
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("keydown", onKey);
      const opener = openerRef.current;
      if (opener instanceof HTMLElement && document.contains(opener)) opener.focus();
    };
  }, [open, onClose]);

  if (details === null) return null;

  return createPortal(
    <div
      className={styles.scrim}
      data-testid="trade-details-scrim"
      // A backdrop click closes; a click INSIDE must not, so the dialog stops propagation.
      onMouseDown={onClose}
    >
      <div
        ref={dialogRef}
        className={styles.dialog}
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        tabIndex={-1}
        data-testid="trade-details-modal"
        data-trade-id={details.id}
        onMouseDown={(e) => e.stopPropagation()}
      >
        <header className={styles.head}>
          <div className={styles.headMain}>
            <span
              className={details.kind === "hedge" ? styles.kindHedge : styles.kindClient}
              data-testid="trade-details-kind"
            >
              {details.kind === "hedge" ? "Hedge" : "Client trade"}
            </span>
            <h2 id={titleId} className={styles.title}>
              {details.title}
            </h2>
            <p className={styles.subtitle}>{details.subtitle}</p>
          </div>
          <button
            type="button"
            className={styles.close}
            onClick={onClose}
            data-testid="trade-details-close"
          >
            Close
          </button>
        </header>

        <div className={styles.body}>
          <div className={styles.grid}>
            {details.sections.map((section) => (
              <section key={section.title} className={styles.section}>
                <h3 className={styles.sectionTitle}>{section.title}</h3>
                <dl className={styles.rows}>
                  {section.rows.map((row) => (
                    <div key={row.label} className={styles.row}>
                      <dt className={styles.label} title={row.hint}>
                        {row.label}
                        {row.hint !== undefined && (
                          <span className={styles.hintMark} aria-hidden="true">
                            ⓘ
                          </span>
                        )}
                      </dt>
                      <dd className={row.mono ? `${styles.value} ${styles.mono}` : styles.value}>
                        {row.value}
                      </dd>
                    </div>
                  ))}
                </dl>
              </section>
            ))}
          </div>
          {extra}
          <OrdersTable orders={details.orders} kind={details.kind} />
        </div>
      </div>
    </div>,
    document.body,
  );
}
