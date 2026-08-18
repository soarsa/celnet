/**
 * RiskBooksWorkspace — the hierarchical risk-book tree editor. These tests drive the
 * workspace with `useApp` mocked (no server): the tree renders every book, an admin
 * gets the create + edit affordances, and a non-admin gets a read-only view.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, within } from "@testing-library/react";

import type { DeskDesc, RiskBook } from "../src/data/contract";

const state: { app: unknown } = { app: null };
vi.mock("../src/app/AppContext", () => ({ useApp: () => state.app }));

import { RiskBooksWorkspace } from "../src/workspaces/RiskBooksWorkspace";

function book(overrides: Partial<RiskBook> = {}): RiskBook {
  return {
    id: "fx-emea",
    name: "FX EMEA",
    parentId: null,
    deskId: null,
    description: "",
    limits: null,
    enabled: true,
    assetClass: "fx_options",
    ...overrides,
  };
}

function makeApp(opts: {
  isAdmin: boolean;
  books: RiskBook[];
  desks?: DeskDesc[];
  createRiskBook?: ReturnType<typeof vi.fn>;
  updateRiskBook?: ReturnType<typeof vi.fn>;
}) {
  return {
    transport: {
      listRiskBooks: vi.fn(async () => opts.books),
      // The portfolio editor now also reads the firm hedge config (for the per-scope
      // RISK-MODEL binding), so the stub must answer it or the effect throws.
      getHedgeConfig: vi.fn(async () => ({
        killSwitch: false,
        execution: "lp_panel_then_composite" as const,
        deskEnabled: [],
        maxClip: 0,
        maxHedgesPerInterval: 0,
        dailyExternalNotionalCap: 0,
        lpPanels: [],
        compositeSpreadBp: 0.5,
        vehicles: [],
        exitModes: [],
        hedgingModels: [],
      })),
      setHedgeConfig: vi.fn(async () => undefined),
      listDesks: vi.fn(async () => opts.desks ?? []),
      createRiskBook:
        opts.createRiskBook ?? vi.fn(async (b: RiskBook) => ({ ...b, id: b.id || "minted" })),
      updateRiskBook: opts.updateRiskBook ?? vi.fn(async (_id: string, b: RiskBook) => b),
      deleteRiskBook: vi.fn(async () => true),
    },
    auth: { user: { id: "u", email: "admin@celnet.com" }, isAdmin: opts.isAdmin, can: () => opts.isAdmin },
    setSignInOpen: vi.fn(),
  };
}

beforeEach(() => {
  state.app = null;
  vi.clearAllMocks();
});
afterEach(() => cleanup());

describe("RiskBooksWorkspace", () => {
  it("renders the book tree (parent + nested sub-book)", async () => {
    state.app = makeApp({
      isAdmin: true,
      books: [
        book(),
        book({ id: "fx-emea-vanilla", name: "FX EMEA Vanilla", parentId: "fx-emea" }),
        book({ id: "fx-apac", name: "FX APAC" }),
      ],
    });
    render(<RiskBooksWorkspace />);

    const tree = await screen.findByRole("navigation", { name: /risk portfolio tree/i });
    expect(within(tree).getByText("FX EMEA")).toBeInTheDocument();
    expect(within(tree).getByText("FX EMEA Vanilla")).toBeInTheDocument();
    expect(within(tree).getByText("FX APAC")).toBeInTheDocument();
  });

  it("gives an admin the create affordance and an editable name field", async () => {
    state.app = makeApp({ isAdmin: true, books: [book({ name: "FX EMEA" })] });
    render(<RiskBooksWorkspace />);

    expect(await screen.findByTestId("new-risk-book")).toBeInTheDocument();
    // The first book auto-selects; its name populates the editable input.
    const nameInput = await screen.findByDisplayValue("FX EMEA");
    expect(nameInput).not.toBeDisabled();
  });

  it("opens a book's editor when its tree row is clicked", async () => {
    state.app = makeApp({
      isAdmin: true,
      books: [book({ id: "fx-apac", name: "FX APAC" }), book({ name: "FX EMEA" })],
    });
    render(<RiskBooksWorkspace />);

    const tree = await screen.findByRole("navigation", { name: /risk portfolio tree/i });
    fireEvent.click(within(tree).getByText("FX EMEA"));
    expect(await screen.findByDisplayValue("FX EMEA")).toBeInTheDocument();
  });

  it("is read-only for a non-admin (no create; name field disabled)", async () => {
    state.app = makeApp({ isAdmin: false, books: [book({ name: "FX EMEA" })] });
    render(<RiskBooksWorkspace />);

    await screen.findByRole("navigation", { name: /risk portfolio tree/i });
    expect(screen.queryByTestId("new-risk-book")).not.toBeInTheDocument();
    expect(await screen.findByDisplayValue("FX EMEA")).toBeDisabled();
  });
});
