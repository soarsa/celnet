/**
 * Corporate Actions — the GUI client for `CorporateActionsService`. Covers:
 *  1. the three CA enum codecs (proto-tag round-trip + unknown-tag clamp);
 *  2. the wsCodec request encoders / response decoders (the flat wire shape, ISO
 *     dates, tag-carried enums, optional `response_deadline`) round-tripped against
 *     the SAME field tables the server's generated_codec uses;
 *  3. the MockTransport CA lifecycle — confirm then apply a full call SHORTENS the
 *     shown schedule (14 → 1 flow), realises the face, and drops the pool factor;
 *  4. the workspace render + `refdata` gating — a view·FI user sees the read-only
 *     inbox with Confirm/Apply DISABLED; a `refdata` holder can drive them.
 */
import { act } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";

import * as e from "../src/data/enums";
import {
  corporateActionFromWire,
  instrumentScheduleFlowFromWire,
  listInstrumentScheduleToWire,
  listInstrumentScheduleResponseFromWire,
  listCorporateActionsToWire,
  listCorporateActionsResponseFromWire,
  confirmCorporateActionToWire,
  confirmCorporateActionResponseFromWire,
  applyCorporateActionToWire,
  applyCorporateActionResponseFromWire,
} from "../src/data/wsCodec";
import type {
  CapabilityAction,
  CapabilityAsset,
  CorporateAction,
} from "../src/data/contract";
import { MockTransport } from "../src/data/mockSource";

// A wire `CorporateActionDesc` exactly as the server's generated_codec emits it:
// flat, ISO-string dates, and `caev`/`camv`/`status` as their proto enum TAGS.
const WIRE_CA = {
  ca_id: "ca-mcal-atlas-2026",
  isin: "US04966QAX86",
  caev: 3, // MCAL
  camv: 1, // MAND
  status: 3, // CONFIRMED
  announcement_date: "2026-08-10",
  record_date: "2026-09-01",
  ex_date: "2026-08-31",
  payment_date: "2026-09-15",
  cash_per_100: 101.5,
  redeemed_fraction: 1,
  target_instrument: "",
  target_units_per_100: 0,
  source_ref: "MT564/CALL-ATLAS-2026",
  source_priority: 40,
  source: "vendor-ca-feed",
};

describe("CA enum codecs — proto-tag round-trip", () => {
  it("corpEventType maps every member to its proto tag (1..10) and back", () => {
    const members = [
      "REDM",
      "INTR",
      "MCAL",
      "PCAL",
      "PRED",
      "DRAW",
      "BPUT",
      "TEND",
      "EXOF",
      "CONV",
    ] as const;
    members.forEach((m, i) => {
      expect(e.corpEventType.toWire(m)).toBe(i + 1);
      expect(e.corpEventType.fromWire(i + 1)).toBe(m);
    });
    // The proto3 0/UNSPECIFIED sentinel clamps to the first named member.
    expect(e.corpEventType.fromWire(0)).toBe("REDM");
    expect(e.corpEventType.fromWire(99)).toBe("REDM");
  });

  it("corpMandatory and corpActionStatus round-trip at their offsets", () => {
    (["MAND", "VOLU", "CHOS"] as const).forEach((m, i) => {
      expect(e.corpMandatory.toWire(m)).toBe(i + 1);
      expect(e.corpMandatory.fromWire(i + 1)).toBe(m);
    });
    (["ANNOUNCED", "ELECTED", "CONFIRMED", "APPLIED", "REVERSED", "CANCELLED"] as const).forEach(
      (m, i) => {
        expect(e.corpActionStatus.toWire(m)).toBe(i + 1);
        expect(e.corpActionStatus.fromWire(i + 1)).toBe(m);
      },
    );
    expect(e.corpActionStatus.fromWire(0)).toBe("ANNOUNCED");
  });
});

describe("wsCodec — CA request/response round-trip", () => {
  it("decodes a wire CorporateActionDesc (flat, ISO dates, tag enums)", () => {
    const ca = corporateActionFromWire(WIRE_CA);
    expect(ca.caId).toBe("ca-mcal-atlas-2026");
    expect(ca.isin).toBe("US04966QAX86");
    expect(ca.caev).toBe("MCAL");
    expect(ca.camv).toBe("MAND");
    expect(ca.status).toBe("CONFIRMED");
    expect(ca.exDate).toBe("2026-08-31");
    expect(ca.paymentDate).toBe("2026-09-15");
    expect(ca.cashPer100).toBeCloseTo(101.5);
    expect(ca.sourcePriority).toBe(40);
    // MAND ⇒ no election deadline on the wire ⇒ undefined.
    expect(ca.responseDeadline).toBeUndefined();
  });

  it("decodes an optional response_deadline only when present", () => {
    const withDeadline = corporateActionFromWire({
      ...WIRE_CA,
      camv: 2, // VOLU
      response_deadline: "2026-11-01",
    });
    expect(withDeadline.camv).toBe("VOLU");
    expect(withDeadline.responseDeadline).toBe("2026-11-01");
  });

  it("decodes an InstrumentScheduleFlow and the schedule response", () => {
    const flow = instrumentScheduleFlowFromWire({ date: "2030-03-15", coupon: 3, principal: 100 });
    expect(flow).toEqual({ date: "2030-03-15", coupon: 3, principal: 100 });

    const resp = listInstrumentScheduleResponseFromWire({
      instrument_id: "corp-atlas-6-2032",
      flows: [
        { date: "2026-03-15", coupon: 3, principal: 0 },
        { date: "2026-09-15", coupon: 3, principal: 100 },
      ],
      pool_factor: 0.5,
    });
    expect(resp.instrumentId).toBe("corp-atlas-6-2032");
    expect(resp.flows).toHaveLength(2);
    expect(resp.poolFactor).toBe(0.5);
  });

  it("encodes the four requests to the flat wire shape (no session_token — the connection injects it)", () => {
    expect(listInstrumentScheduleToWire({ instrumentId: "x" })).toEqual({ instrument_id: "x" });
    // Absent ISIN filter ⇒ omitted; present ⇒ carried.
    expect(listCorporateActionsToWire({})).toEqual({});
    expect(listCorporateActionsToWire({ isin: "US04966QAX86" })).toEqual({
      isin: "US04966QAX86",
    });
    expect(confirmCorporateActionToWire({ caId: "ca-1" })).toEqual({ ca_id: "ca-1" });
    expect(applyCorporateActionToWire({ caId: "ca-1", heldFace: 1_000_000 })).toEqual({
      ca_id: "ca-1",
      held_face: 1_000_000,
    });
  });

  it("decodes the list / confirm / apply responses", () => {
    const list = listCorporateActionsResponseFromWire({ actions: [WIRE_CA] });
    expect(list.actions).toHaveLength(1);
    expect(list.actions[0]!.caev).toBe("MCAL");

    const confirmed = confirmCorporateActionResponseFromWire({ action: WIRE_CA });
    expect(confirmed.action.status).toBe("CONFIRMED");

    const applied = applyCorporateActionResponseFromWire({
      instrument_id: "corp-atlas-6-2032",
      face_delta: -1_000_000,
      cash: 1_015_000,
      remaining_flows: 1,
      action: { ...WIRE_CA, status: 4 },
    });
    expect(applied.instrumentId).toBe("corp-atlas-6-2032");
    expect(applied.faceDelta).toBe(-1_000_000);
    expect(applied.cash).toBe(1_015_000);
    expect(applied.remainingFlows).toBe(1);
    expect(applied.action.status).toBe("APPLIED");
  });
});

describe("MockTransport — the CA lifecycle", () => {
  it("lists the seeded inbox and joins to the instrument registry by ISIN", async () => {
    const t = new MockTransport({ tickMs: 60_000 });
    const { actions } = await t.listCorporateActions({});
    // A coupon, a full call, a maturity, and a voluntary tender.
    const caevs = actions.map((a) => a.caev).sort();
    expect(caevs).toContain("INTR");
    expect(caevs).toContain("MCAL");
    expect(caevs).toContain("REDM");
    // The ISIN filter narrows to one instrument's events.
    const atlas = await t.listCorporateActions({ isin: "US04966QAX86" });
    expect(atlas.actions.every((a) => a.isin === "US04966QAX86")).toBe(true);
  });

  it("applying a full call SHORTENS the schedule, realises face, and zeroes the pool factor", async () => {
    const t = new MockTransport({ tickMs: 60_000 });
    // The callable bond's pre-event schedule is a multi-year coupon ladder.
    const before = await t.listInstrumentSchedule({ instrumentId: "corp-atlas-6-2032" });
    expect(before.flows.length).toBeGreaterThan(1);
    expect(before.poolFactor).toBe(1);

    // Apply is refused before confirmation (mirrors the server lifecycle).
    await expect(
      t.applyCorporateAction({ caId: "ca-mcal-atlas-2026", heldFace: 1_000_000 }),
    ).rejects.toThrow(/confirmed/);

    const confirmed = await t.confirmCorporateAction({ caId: "ca-mcal-atlas-2026" });
    expect(confirmed.action.status).toBe("CONFIRMED");

    const applied = await t.applyCorporateAction({
      caId: "ca-mcal-atlas-2026",
      heldFace: 1_000_000,
    });
    expect(applied.faceDelta).toBe(-1_000_000); // realised
    expect(applied.cash).toBeCloseTo((1_000_000 * 101.5) / 100);
    expect(applied.remainingFlows).toBe(1); // shortened to the call cashflow
    expect(applied.action.status).toBe("APPLIED");

    const after = await t.listInstrumentSchedule({ instrumentId: "corp-atlas-6-2032" });
    expect(after.flows).toHaveLength(1);
    expect(after.flows.length).toBeLessThan(before.flows.length);
    expect(after.poolFactor).toBe(0);
  });

  it("applying a coupon pays cash without changing face", async () => {
    const t = new MockTransport({ tickMs: 60_000 });
    await t.confirmCorporateAction({ caId: "ca-intr-ust425-2026h2" });
    const applied = await t.applyCorporateAction({
      caId: "ca-intr-ust425-2026h2",
      heldFace: 2_000_000,
    });
    expect(applied.faceDelta).toBe(0);
    expect(applied.cash).toBeCloseTo((2_000_000 * 2.125) / 100);
  });
});

// --- workspace render + refdata gating --------------------------------------

const state: { app: unknown } = { app: null };
vi.mock("../src/app/AppContext", () => ({ useApp: () => state.app }));

// eslint-disable-next-line import/first
import { CorporateActionsWorkspace } from "../src/workspaces/CorporateActionsWorkspace";

function makeApp(canRefdata: boolean) {
  const transport = new MockTransport({ tickMs: 60_000 });
  return {
    transport,
    auth: {
      user: { id: "u", email: canRefdata ? "steward@celnet.com" : "trader@celnet.com" },
      isAdmin: canRefdata,
      can: (action: CapabilityAction, _asset: CapabilityAsset) =>
        action === "refdata" ? canRefdata : true,
    },
    setSignInOpen: vi.fn(),
  };
}

async function renderCa(canRefdata: boolean): Promise<void> {
  state.app = makeApp(canRefdata);
  await act(async () => {
    render(<CorporateActionsWorkspace />);
  });
  // Let the mount-time inbox + registry loads resolve.
  await screen.findByTestId("ca-row-ca-mcal-atlas-2026");
}

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  state.app = null;
});

describe("CorporateActionsWorkspace — render + refdata gating", () => {
  it("a view·FI user (no refdata) sees the inbox read-only: Confirm/Apply DISABLED", async () => {
    await renderCa(false);
    // The list is visible to any FI viewer.
    expect(screen.getByTestId("ca-row-ca-mcal-atlas-2026")).toBeTruthy();
    expect(screen.getByTestId("ca-readonly-note")).toBeTruthy();
    // The row lifecycle controls are disabled (never hidden) for a non-refdata user.
    expect(
      (screen.getByTestId("ca-confirm-ca-mcal-atlas-2026") as HTMLButtonElement).disabled,
    ).toBe(true);
    expect(
      (screen.getByTestId("ca-apply-ca-mcal-atlas-2026") as HTMLButtonElement).disabled,
    ).toBe(true);
  });

  it("a refdata holder can select a call, confirm it, then apply → the schedule shortens", async () => {
    await renderCa(true);
    expect(screen.queryByTestId("ca-readonly-note")).toBeNull();

    // Select the full-call CA → its instrument schedule loads (the pre-event ladder).
    await act(async () => {
      fireEvent.click(screen.getByTestId("ca-row-ca-mcal-atlas-2026").querySelector("button")!);
    });
    const beforeCount = (await screen.findByTestId("ca-flow-count")).textContent ?? "";
    const beforeFlows = Number(beforeCount.replace(/\D+/g, ""));
    expect(beforeFlows).toBeGreaterThan(1);

    // Confirm (announced → confirmed), then Apply against the default held face.
    await act(async () => {
      fireEvent.click(screen.getByTestId("ca-detail-confirm"));
    });
    // After confirm the detail Apply becomes enabled.
    const applyBtn = await screen.findByTestId("ca-detail-apply");
    expect((applyBtn as HTMLButtonElement).disabled).toBe(false);
    await act(async () => {
      fireEvent.click(applyBtn);
    });

    // The apply note appears and the schedule shortens to the single call cashflow.
    await screen.findByTestId("ca-apply-note");
    const afterCount = screen.getByTestId("ca-flow-count").textContent ?? "";
    const afterFlows = Number(afterCount.replace(/\D+/g, ""));
    expect(afterFlows).toBe(1);
    expect(afterFlows).toBeLessThan(beforeFlows);
    expect(screen.getByTestId("ca-pool-factor").textContent).toBe("0.0000");
  });
});
