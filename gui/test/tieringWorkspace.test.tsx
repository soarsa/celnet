/**
 * TieringWorkspace — the trader-facing FI Tiering surface (server commit 8404bc9).
 *
 * Drives the workspace with `useApp` mocked so we inject a fixed book roster + a
 * capability and observe the retune mutation. Covers:
 *  (a) the sign-in gate for an anonymous session;
 *  (b) rail registration — `tiering` is a Fixed-Income row, NOT admin-only, so a
 *      non-admin trader can reach it;
 *  (c) the trader flow — select a book, ENABLE tiering (Flat 25 price-bps), Apply →
 *      `transport.updateBookTiering(id, config)` fires and the book round-trips;
 *  (d) the permission state — a signed-in FI user WITHOUT `quote_respond·fixed_income`
 *      sees a clear permission note + read-only summary, never the editor.
 */

import { render, screen, fireEvent, within } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { AggregatedBookDesc } from "../src/data/contract";
import {
  ADMIN_ONLY_WORKSPACES,
  RAIL,
  workspaceDomains,
} from "../src/lib/commands";

const state: { app: unknown } = { app: null };
vi.mock("../src/app/AppContext", () => ({ useApp: () => state.app }));

import { TieringWorkspace } from "../src/workspaces/TieringWorkspace";

function book(overrides: Partial<AggregatedBookDesc> = {}): AggregatedBookDesc {
  return {
    id: "us-treasuries",
    name: "US Treasuries",
    memberConnectionIds: ["LP-SIM-01", "LP-SIM-02"],
    scopeMode: "ALL_MEMBERS_QUOTE",
    instrumentIds: [],
    params: {
      stalenessTauMs: 2000,
      maxQuoteAgeMs: 5000,
      divergenceGating: true,
      minContributors: 2,
      depthLevels: 1,
    },
    enabled: true,
    tiering: null,
    ...overrides,
  };
}

function makeApp(opts: {
  user: { id: string; email: string } | null;
  canRetune: boolean;
  books: AggregatedBookDesc[];
  updateBookTiering?: ReturnType<typeof vi.fn>;
}) {
  return {
    transport: {
      listAggregatedBooks: vi.fn(async () => opts.books),
      updateBookTiering:
        opts.updateBookTiering ??
        vi.fn(async (id: string, tiering: unknown) => ({
          ...book({ id }),
          tiering,
        })),
    },
    auth: {
      user: opts.user,
      isAdmin: false,
      can: () => opts.canRetune,
    },
    setSignInOpen: vi.fn(),
  };
}

beforeEach(() => {
  state.app = null;
  vi.clearAllMocks();
});

describe("TieringWorkspace — rail registration", () => {
  it("is a Fixed-Income row and is NOT admin-only", () => {
    const row = RAIL.find((r) => r.id === "tiering");
    expect(row).toBeDefined();
    expect(row!.label).toBe("Tiering");
    expect(workspaceDomains("tiering")).toEqual(["fixed_income"]);
    expect(ADMIN_ONLY_WORKSPACES.has("tiering")).toBe(false);
  });
});

describe("TieringWorkspace — sign-in gate", () => {
  it("shows a sign-in card for an anonymous session", () => {
    state.app = makeApp({ user: null, canRetune: true, books: [book()] });
    render(<TieringWorkspace />);
    expect(screen.getByRole("button", { name: "Sign in" })).toBeInTheDocument();
  });
});

describe("TieringWorkspace — trader retune flow", () => {
  it("enables Flat tiering and applies it to the selected book", async () => {
    const updateBookTiering = vi.fn(async (id: string, tiering: unknown) => ({
      ...book({ id }),
      tiering,
    }));
    state.app = makeApp({
      user: { id: "trader", email: "trader@celnet.com" },
      canRetune: true,
      books: [book()],
      updateBookTiering,
    });
    render(<TieringWorkspace />);

    // The book loads (async) — its roster button is present.
    const bookBtn = await screen.findByRole("button", { name: /US Treasuries/ });
    expect(bookBtn).toBeInTheDocument();

    // Apply is disabled while nothing has changed (draft == stored null tiering).
    const applyBtn = screen.getByRole("button", { name: /Apply tiering/ });
    expect(applyBtn).toBeDisabled();

    // Enable outbound tiering (the editor seeds a default Flat-markup config).
    fireEvent.click(screen.getByLabelText("enable outbound tiering"));

    // Now dirty + valid ⇒ Apply is enabled; clicking it calls the RPC.
    expect(applyBtn).toBeEnabled();
    fireEvent.click(applyBtn);

    expect(updateBookTiering).toHaveBeenCalledTimes(1);
    const [id, config] = updateBookTiering.mock.calls[0]!;
    expect(id).toBe("us-treasuries");
    expect(config).not.toBeNull();
    expect((config as { strategies: { kind: string }[] }).strategies[0]!.kind).toBe(
      "FLAT_MARKUP",
    );

    // The applied state round-trips into the UI.
    expect(await screen.findByText("✓ Applied")).toBeInTheDocument();
  });
});

describe("TieringWorkspace — permission state", () => {
  it("shows a permission note + read-only summary without the capability", async () => {
    state.app = makeApp({
      user: { id: "viewer", email: "viewer@celnet.com" },
      canRetune: false,
      books: [book()],
    });
    render(<TieringWorkspace />);

    expect(
      screen.getByText(/don't have permission to retune tiering/i),
    ).toBeInTheDocument();
    // Selecting the (default-selected) book shows the read-only summary, NOT the editor.
    const ro = await screen.findByLabelText("current tiering (read-only)");
    expect(within(ro).getByText("Disabled")).toBeInTheDocument();
    expect(
      screen.queryByLabelText("outbound tiering configuration"),
    ).toBeNull();
    expect(screen.queryByRole("button", { name: /Apply tiering/ })).toBeNull();
  });
});
