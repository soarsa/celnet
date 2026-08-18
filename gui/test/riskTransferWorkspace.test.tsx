/**
 * Risk-transfer workspace render tests — behaviour + accessibility, driven with
 * `useApp` mocked (no server). The ticket loads portfolios, synthesises selectable
 * position lots, infers the kind, previews the move and submits; the inbox renders the
 * live pending set with accept/reject + the four-eyes note; the audit blotter lists
 * records and expands one to its provenance.
 */
import { act } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";

import type { RiskBook, RiskBookRisk, RiskTransfer } from "../src/data/contract";

const state: { app: unknown } = { app: null };
vi.mock("../src/app/AppContext", () => ({ useApp: () => state.app }));

import { RiskTransferWorkspace } from "../src/workspaces/risktransfer/RiskTransferWorkspace";
import { RiskTransferInboxWorkspace } from "../src/workspaces/risktransfer/RiskTransferInboxWorkspace";
import { RiskTransferAuditWorkspace } from "../src/workspaces/risktransfer/RiskTransferAuditWorkspace";
import type { InitiateRiskTransferInput, ListRiskTransfersFilter } from "../src/data/contract";

const BOOKS: RiskBook[] = [
  { id: "fi-rates-emea", name: "EMEA Rates", parentId: null, deskId: "emea", description: "", limits: null, enabled: true , assetClass: "fixed_income"},
  { id: "fi-emea-sub", name: "EMEA Sub", parentId: "fi-rates-emea", deskId: null, description: "", limits: null, enabled: true , assetClass: "fixed_income"},
  { id: "fi-marex", name: "Marex FI", parentId: null, deskId: "marex", description: "", limits: null, enabled: true , assetClass: "fixed_income"},
];

function riskRow(over: Partial<RiskBookRisk> = {}): RiskBookRisk {
  return {
    bookId: "fi-rates-emea",
    name: "EMEA Rates",
    netNotional: 300_000_000,
    grossNotional: 300_000_000,
    positionCount: 3,
    delta: 1000,
    gamma: 20,
    vega: 500,
    theta: -40,
    dv01: 25_000,
    pnl: null,
    limits: [],
    ...over,
  };
}

const RISK: RiskBookRisk[] = [
  riskRow(),
  riskRow({ bookId: "fi-emea-sub", name: "EMEA Sub", netNotional: 50_000_000 }),
  riskRow({ bookId: "fi-marex", name: "Marex FI", netNotional: 20_000_000 }),
];

const AUTH = { user: { id: "u", email: "admin@celnet.com" }, isAdmin: true, can: () => true };

beforeEach(() => {
  state.app = null;
  vi.clearAllMocks();
});
afterEach(() => cleanup());

describe("RiskTransferWorkspace (ticket)", () => {
  function makeApp(initiate = vi.fn(async (): Promise<RiskTransfer> => bookedTransfer())) {
    return {
      transport: {
        listRiskBooks: vi.fn(async () => BOOKS),
        listRiskBookRisk: vi.fn(async () => RISK),
        listDesks: vi.fn(async () => [
          { id: "emea", name: "EMEA" },
          { id: "marex", name: "Marex" },
        ]),
        listUsers: vi.fn(async () => []),
        initiateRiskTransfer: initiate,
      },
      auth: AUTH,
    };
  }

  function bookedTransfer(): RiskTransfer {
    return {
      id: "xfer-1",
      kind: "RE_ATTRIBUTE",
      source: { riskBookId: "fi-rates-emea", deskId: "emea", trader: "admin@celnet.com", positionIds: [] },
      target: { riskBookId: "fi-emea-sub", deskId: "emea", trader: "", positionIds: [] },
      quantityFull: true,
      partialNotional: null,
      priceBasis: "MID",
      agreedPrice: null,
      reason: "",
      initiatedBy: "admin@celnet.com",
      initiatedAt: 1n,
      state: "BOOKED",
      approver: null,
      decidedAt: 1n,
      transferPrice: 100,
      provenance: null,
    };
  }

  it("loads portfolios, synthesises positions, infers the kind and previews + submits", async () => {
    const initiate = vi.fn(async (_input: InitiateRiskTransferInput) => bookedTransfer());
    state.app = makeApp(initiate);
    await act(async () => {
      render(<RiskTransferWorkspace />);
    });
    expect(screen.getByRole("heading", { name: /risk transfer/i })).toBeInTheDocument();

    // Pick the source portfolio → position lots appear.
    const source = await screen.findByTestId("xfer-source");
    await act(async () => {
      fireEvent.change(source, { target: { value: "fi-rates-emea" } });
    });
    fireEvent.click(await screen.findByTestId("xfer-select-all"));

    // Pick a same-desk target → kind = re-attribution.
    fireEvent.change(screen.getByTestId("xfer-target"), { target: { value: "fi-emea-sub" } });
    expect(screen.getByTestId("xfer-kind")).toHaveTextContent(/re attribute/i);
    expect(screen.getByTestId("xfer-preview")).toBeInTheDocument();

    // Submit initiates the transfer and surfaces the BOOKED result.
    fireEvent.click(screen.getByTestId("xfer-submit"));
    await waitFor(() => expect(initiate).toHaveBeenCalledTimes(1));
    expect(initiate.mock.calls[0]![0]).toMatchObject({ kind: "RE_ATTRIBUTE", quantityFull: true });
    expect(await screen.findByTestId("xfer-result")).toHaveTextContent(/booked/i);
  });

  it("infers DESK_TO_DESK for a cross-desk target", async () => {
    state.app = makeApp();
    await act(async () => {
      render(<RiskTransferWorkspace />);
    });
    fireEvent.change(await screen.findByTestId("xfer-source"), { target: { value: "fi-rates-emea" } });
    fireEvent.change(await screen.findByTestId("xfer-target"), { target: { value: "fi-marex" } });
    expect(screen.getByTestId("xfer-kind")).toHaveTextContent(/desk to desk/i);
  });

  it("requires a reason before an AGREED transfer can submit", async () => {
    state.app = makeApp();
    await act(async () => {
      render(<RiskTransferWorkspace />);
    });
    fireEvent.change(await screen.findByTestId("xfer-source"), { target: { value: "fi-rates-emea" } });
    fireEvent.click(await screen.findByTestId("xfer-select-all"));
    fireEvent.change(screen.getByTestId("xfer-target"), { target: { value: "fi-emea-sub" } });
    fireEvent.click(screen.getByTestId("xfer-basis-AGREED"));
    // Reason empty → submit disabled.
    expect(screen.getByTestId("xfer-submit")).toBeDisabled();
    fireEvent.change(screen.getByTestId("xfer-reason"), { target: { value: "control-approved cross" } });
    expect(screen.getByTestId("xfer-submit")).not.toBeDisabled();
  });
});

describe("RiskTransferInboxWorkspace", () => {
  function makeApp(pending: RiskTransfer[], accept = vi.fn(async () => pending[0]!)) {
    return {
      transport: {
        listRiskBooks: vi.fn(async () => BOOKS),
        listRiskBookRisk: vi.fn(async () => RISK),
        streamRiskTransferInbox: (cb: (p: RiskTransfer[]) => void) => {
          cb(pending);
          return () => undefined;
        },
        acceptRiskTransfer: accept,
        rejectRiskTransfer: vi.fn(async () => pending[0]!),
      },
      auth: AUTH,
    };
  }

  function pendingTransfer(): RiskTransfer {
    return {
      id: "xfer-9",
      kind: "DESK_TO_DESK",
      source: { riskBookId: "fi-rates-emea", deskId: "emea", trader: "a@x", positionIds: [1n] },
      target: { riskBookId: "fi-marex", deskId: "marex", trader: "", positionIds: [] },
      quantityFull: true,
      partialNotional: null,
      priceBasis: "MARK_TO_MARKET",
      agreedPrice: null,
      reason: "hedge hand-over",
      initiatedBy: "a@x",
      initiatedAt: 1n,
      state: "PENDING",
      approver: null,
      decidedAt: null,
      transferPrice: null,
      provenance: null,
    };
  }

  it("surfaces the four-eyes rule and the empty state", async () => {
    state.app = makeApp([]);
    await act(async () => {
      render(<RiskTransferInboxWorkspace />);
    });
    expect(screen.getByRole("note")).toHaveTextContent(/four-eyes/i);
    expect(await screen.findByTestId("inbox-empty")).toBeInTheDocument();
  });

  it("renders a pending transfer and accepts it", async () => {
    const accept = vi.fn(async () => ({ ...pendingTransfer(), state: "BOOKED" as const }));
    state.app = makeApp([pendingTransfer()], accept);
    await act(async () => {
      render(<RiskTransferInboxWorkspace />);
    });
    const item = await screen.findByTestId("inbox-item-xfer-9");
    expect(within(item).getByText(/EMEA Rates/)).toBeInTheDocument();
    expect(within(item).getByText(/Marex FI/)).toBeInTheDocument();
    fireEvent.click(screen.getByTestId("inbox-accept-xfer-9"));
    await waitFor(() => expect(accept).toHaveBeenCalledWith("xfer-9"));
  });
});

describe("RiskTransferWorkspace consolidated tabs", () => {
  // A transport carrying every method the three tabs use, so switching tabs mounts a
  // working panel (only the active tab mounts).
  function fullTransport() {
    return {
      listRiskBooks: vi.fn(async () => BOOKS),
      listRiskBookRisk: vi.fn(async () => RISK),
      listDesks: vi.fn(async () => [
        { id: "emea", name: "EMEA" },
        { id: "marex", name: "Marex" },
      ]),
      listUsers: vi.fn(async () => []),
      initiateRiskTransfer: vi.fn(),
      streamRiskTransferInbox: (cb: (p: RiskTransfer[]) => void) => {
        cb([]);
        return () => undefined;
      },
      acceptRiskTransfer: vi.fn(),
      rejectRiskTransfer: vi.fn(),
      listRiskTransfers: vi.fn(async () => [] as RiskTransfer[]),
    };
  }

  it("renders all three tabs for a risk_transfer holder and switches between them", async () => {
    state.app = { transport: fullTransport(), auth: AUTH }; // admin can() => true
    await act(async () => {
      render(<RiskTransferWorkspace />);
    });
    // All three tab toggles present; default lands on the initiate ticket.
    expect(screen.getByTestId("risk-transfer-tab-ticket")).toBeInTheDocument();
    expect(screen.getByTestId("risk-transfer-tab-inbox")).toBeInTheDocument();
    expect(screen.getByTestId("risk-transfer-tab-audit")).toBeInTheDocument();
    expect(await screen.findByTestId("xfer-source")).toBeInTheDocument();

    // Switch to Inbox → the four-eyes note + empty state mount.
    await act(async () => {
      fireEvent.click(screen.getByTestId("risk-transfer-tab-inbox"));
    });
    expect(await screen.findByTestId("inbox-empty")).toBeInTheDocument();
    expect(screen.queryByTestId("xfer-source")).not.toBeInTheDocument();

    // Switch to Audit → the audit blotter (state filter) mounts.
    await act(async () => {
      fireEvent.click(screen.getByTestId("risk-transfer-tab-audit"));
    });
    expect(await screen.findByTestId("audit-state")).toBeInTheDocument();
  });

  it("opens on the Inbox tab when deep-linked via initialTab", async () => {
    state.app = { transport: fullTransport(), auth: AUTH };
    await act(async () => {
      render(<RiskTransferWorkspace initialTab="inbox" />);
    });
    expect(await screen.findByTestId("inbox-empty")).toBeInTheDocument();
    expect(screen.queryByTestId("xfer-source")).not.toBeInTheDocument();
  });

  it("hides the write-class tabs from a view-only FI trader, landing on Audit", async () => {
    const viewOnly = {
      user: { id: "u", email: "trader@celnet.com" },
      isAdmin: false,
      can: (action: string, asset: string) => action === "view" && asset === "fixed_income",
    };
    state.app = { transport: fullTransport(), auth: viewOnly };
    await act(async () => {
      // Deep-linked to the initiate ticket, but the trader cannot view it → clamps to Audit.
      render(<RiskTransferWorkspace initialTab="ticket" />);
    });
    expect(screen.queryByTestId("risk-transfer-tab-ticket")).not.toBeInTheDocument();
    expect(screen.queryByTestId("risk-transfer-tab-inbox")).not.toBeInTheDocument();
    expect(screen.getByTestId("risk-transfer-tab-audit")).toBeInTheDocument();
    expect(await screen.findByTestId("audit-state")).toBeInTheDocument();
    expect(screen.queryByTestId("xfer-source")).not.toBeInTheDocument();
  });
});

describe("RiskTransferAuditWorkspace", () => {
  function auditTransfer(): RiskTransfer {
    return {
      id: "xfer-5",
      kind: "DESK_TO_DESK",
      source: { riskBookId: "fi-rates-emea", deskId: "emea", trader: "a@x", positionIds: [11n, 12n] },
      target: { riskBookId: "fi-marex", deskId: "marex", trader: "b@y", positionIds: [] },
      quantityFull: true,
      partialNotional: null,
      priceBasis: "MID",
      agreedPrice: null,
      reason: "",
      initiatedBy: "a@x",
      initiatedAt: 1_700_000_000_000_000_000n,
      state: "BOOKED",
      approver: "b@y",
      decidedAt: 1_700_000_000_500_000_000n,
      transferPrice: 100,
      provenance: {
        transferId: "xfer-5",
        kind: "DESK_TO_DESK",
        initiatedBy: "a@x",
        initiatedAt: 1_700_000_000_000_000_000n,
        approver: "b@y",
        decidedAt: 1_700_000_000_500_000_000n,
        sourceBookId: "fi-rates-emea",
        targetBookId: "fi-marex",
        positionIds: [11n, 12n],
        quantityFull: true,
        partialNotional: null,
        transferPrice: 100,
        priceBasis: "MID",
        reason: "",
        realizedPnlSource: 0,
        riskMoved: { notionalBase: 5_000_000, risk: { dv01: 12, delta: 0, gamma: 0, vega: 0, theta: 0 } },
      },
    };
  }

  it("lists transfers and expands one to its provenance", async () => {
    const list = vi.fn(async (_filter: ListRiskTransfersFilter) => [auditTransfer()]);
    state.app = {
      transport: {
        listRiskBooks: vi.fn(async () => BOOKS),
        listDesks: vi.fn(async () => [{ id: "emea", name: "EMEA" }, { id: "marex", name: "Marex" }]),
        listRiskTransfers: list,
      },
      auth: AUTH,
    };
    await act(async () => {
      render(<RiskTransferAuditWorkspace />);
    });
    const rowEl = await screen.findByTestId("audit-row-xfer-5");
    expect(within(rowEl).getByText(/EMEA Rates → Marex FI/)).toBeInTheDocument();
    fireEvent.click(rowEl);
    const detail = await screen.findByTestId("audit-detail-xfer-5");
    expect(within(detail).getByText(/Provenance/)).toBeInTheDocument();
    expect(within(detail).getByText(/Moved notional/)).toBeInTheDocument();
  });

  it("re-queries when the state filter changes", async () => {
    const list = vi.fn(async (_filter: ListRiskTransfersFilter) => [] as RiskTransfer[]);
    state.app = {
      transport: {
        listRiskBooks: vi.fn(async () => BOOKS),
        listDesks: vi.fn(async () => []),
        listRiskTransfers: list,
      },
      auth: AUTH,
    };
    await act(async () => {
      render(<RiskTransferAuditWorkspace />);
    });
    await waitFor(() => expect(list).toHaveBeenCalled());
    const calls = list.mock.calls.length;
    fireEvent.change(screen.getByTestId("audit-state"), { target: { value: "BOOKED" } });
    await waitFor(() => expect(list.mock.calls.length).toBeGreaterThan(calls));
    expect(list.mock.calls.at(-1)![0]).toMatchObject({ states: ["BOOKED"] });
  });
});
