// Notifications phase-6 parity for the Excel add-in: the desk notification push
// contract (`NotificationService.StreamNotifications`) decodes EVERY
// `NotificationKind` — including the phase-5 additive `ORDER_RECEIVED`=8 / `FILL`=9
// arms a FIX-venue firm-order lift emits — and labels each one first-class, with NO
// silent default that would drop the new arms into a generic bucket. These are the
// same numeric-enum JSON frames the server's `notification_to_json` produces and the
// GUI's `notificationFromWire` decodes; the add-in mirrors that contract exactly
// (CLAUDE.md rule 9: one contract, two encodings).

import { describe, expect, it } from "vitest";
import { notificationFromWire, notificationKindLabel } from "../src/contract/wsCodec";
import { notificationKind, deskRequestKind } from "../src/contract/enums";
import type { NotificationKind } from "../src/contract/contract";

// Wire tag ↔ string member table (proto `NotificationKind`, named members start at
// tag 1). The exhaustiveness fixture for the parity assertions.
const KIND_BY_TAG: ReadonlyArray<readonly [number, NotificationKind]> = [
  [1, "RFQ_RECEIVED"],
  [2, "IOI_RECEIVED"],
  [3, "REQUEST_WITHDRAWN"],
  [4, "REQUEST_EXPIRED"],
  [5, "QUOTE_ACCEPTED"],
  [6, "QUOTE_REJECTED"],
  [7, "MANUAL_INTERVENTION_REQUIRED"],
  [8, "ORDER_RECEIVED"],
  [9, "FILL"],
];

describe("notification kind codec", () => {
  it("maps every wire tag 1..9 to its member and back (reversible)", () => {
    for (const [tag, member] of KIND_BY_TAG) {
      expect(notificationKind.fromWire(tag)).toBe(member);
      expect(notificationKind.toWire(member)).toBe(tag);
    }
  });

  it("decodes the phase-5 ORDER_RECEIVED=8 / FILL=9 arms first-class", () => {
    expect(notificationKind.fromWire(8)).toBe("ORDER_RECEIVED");
    expect(notificationKind.fromWire(9)).toBe("FILL");
  });

  it("clamps the unspecified(0) / unknown tag to the first member (proto3 reader)", () => {
    expect(notificationKind.fromWire(0)).toBe("RFQ_RECEIVED");
    expect(notificationKind.fromWire(999)).toBe("RFQ_RECEIVED");
  });
});

describe("notificationKindLabel", () => {
  it("labels ORDER_RECEIVED / FILL with human-readable, cross-client-parity text", () => {
    expect(notificationKindLabel("ORDER_RECEIVED")).toBe("Order in");
    expect(notificationKindLabel("FILL")).toBe("Fill");
  });

  it("gives every kind a non-empty label — no silent default bucket", () => {
    for (const [, member] of KIND_BY_TAG) {
      expect(notificationKindLabel(member).length).toBeGreaterThan(0);
    }
    // All nine labels are distinct.
    const labels = KIND_BY_TAG.map(([, m]) => notificationKindLabel(m));
    expect(new Set(labels).size).toBe(labels.length);
  });
});

describe("notificationFromWire", () => {
  it("decodes a FILL notification (kind=9) field-for-field", () => {
    const wire = {
      notification_id: "notif-fill-1",
      kind: 9,
      at_nanos: 1_700_000_000_000_000_000,
      request_id: "ord-7",
      desk: "g10",
      counterparty: "ACME",
      request_kind: 1, // RFQ
      headline: "Filled",
      detail: "5y OIS 25mm",
      alert_worthy: true,
    };
    const n = notificationFromWire(wire);
    expect(n.kind).toBe("FILL");
    expect(notificationKindLabel(n.kind)).toBe("Fill");
    expect(n.requestId).toBe("ord-7");
    expect(n.requestKind).toBe("RFQ");
    expect(n.desk).toBe("g10");
    expect(n.alertWorthy).toBe(true);
    expect(n.detail).toBe("5y OIS 25mm");
    // 64-bit nanos survive as a bigint without precision loss.
    expect(n.atNanos).toBe(1_700_000_000_000_000_000n);
  });

  it("decodes an ORDER_RECEIVED notification (kind=8) and omits absent optionals", () => {
    const wire = {
      notification_id: "notif-ord-1",
      kind: 8,
      at_nanos: 42,
      desk: "g10",
      counterparty: "ACME",
      request_kind: 1,
      headline: "Order in",
      // no request_id / detail / reason / alert_worthy
    };
    const n = notificationFromWire(wire);
    expect(n.kind).toBe("ORDER_RECEIVED");
    expect(notificationKindLabel(n.kind)).toBe("Order in");
    expect(n.requestId).toBeUndefined();
    expect(n.detail).toBeUndefined();
    expect(n.reason).toBeUndefined();
    expect(n.alertWorthy).toBe(false);
  });

  it("decodes the manual-intervention reason only when present as a positive ordinal", () => {
    const withReason = notificationFromWire({
      notification_id: "n",
      kind: 7,
      at_nanos: 1,
      desk: "g10",
      counterparty: "ACME",
      request_kind: 2, // IOI
      headline: "Needs pricing",
      alert_worthy: true,
      reason: 2, // CREDIT_RISK_BREAK
    });
    expect(withReason.kind).toBe("MANUAL_INTERVENTION_REQUIRED");
    expect(withReason.reason).toBe("CREDIT_RISK_BREAK");
    expect(withReason.requestKind).toBe("IOI");
    expect(deskRequestKind.toWire(withReason.requestKind)).toBe(2);
  });
});
