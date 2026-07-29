/**
 * RiskRoutingWorkspace — the decision-tree canvas. Driven with `useApp` mocked (no
 * server): a valid graph loads, an in-editor mutation makes it dirty + saveable and
 * round-trips through `updateRiskRoutingGraph`; a cyclic or dangling graph is caught
 * client-side (Save disabled, issues surfaced) exactly as the server would reject
 * it; and the live trace lands the default fill on the DEFAULT book.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";

import type { RiskBook, RiskRoutingGraph, RoutingNode } from "../src/data/contract";

const state: { app: unknown } = { app: null };
vi.mock("../src/app/AppContext", () => ({ useApp: () => state.app }));

import { RiskRoutingWorkspace } from "../src/workspaces/riskrouting/RiskRoutingWorkspace";

function book(id: string, name: string): RiskBook {
  return { id, name, parentId: null, deskId: null, description: "", limits: null, enabled: true };
}

/** A minimal VALID graph: ccy == EUR → BOOK-A else DEFAULT. */
function validGraph(): RiskRoutingGraph {
  const nodes: RoutingNode[] = [
    {
      kind: "condition",
      id: 0,
      condition: {
        field: "ccy",
        op: "eq",
        value: { kind: "text", text: "EUR" },
        onTrue: 1,
        onFalse: 2,
      },
    },
    { kind: "book", id: 1, bookId: "BOOK-A" },
    { kind: "book", id: 2, bookId: "DEFAULT" },
  ];
  return { entry: 0, nodes };
}

function makeApp(opts: {
  graph: RiskRoutingGraph | null;
  books: RiskBook[];
  isAdmin?: boolean;
  onUpdate?: (g: RiskRoutingGraph) => void;
}) {
  const update = vi.fn(async (g: RiskRoutingGraph) => {
    opts.onUpdate?.(g);
    return g;
  });
  return {
    transport: {
      getRiskRoutingGraph: vi.fn(async () => opts.graph),
      updateRiskRoutingGraph: update,
      listRiskBooks: vi.fn(async () => opts.books),
      listDesks: vi.fn(async () => []),
      listFixConnections: vi.fn(async () => []),
    },
    auth: {
      user: { id: "u", email: "admin@celnet.com" },
      isAdmin: opts.isAdmin ?? true,
      can: () => true,
    },
    setSignInOpen: vi.fn(),
  };
}

beforeEach(() => {
  state.app = null;
  vi.clearAllMocks();
});
afterEach(() => cleanup());

describe("RiskRoutingWorkspace", () => {
  it("shows the empty-state prompt and palette when no graph exists", async () => {
    state.app = makeApp({ graph: null, books: [book("DEFAULT", "Default")] });
    render(<RiskRoutingWorkspace />);

    expect(await screen.findByText(/routing graph is empty/i)).toBeInTheDocument();
    // Field palette chips are always present (draggable sources).
    expect(screen.getByTestId("palette-field-ccy")).toBeInTheDocument();
    expect(screen.getByTestId("palette-book-chip")).toBeInTheDocument();
  });

  it("loads a valid graph as clean (Save disabled) and reports it valid", async () => {
    state.app = makeApp({ graph: validGraph(), books: [book("BOOK-A", "A"), book("DEFAULT", "Def")] });
    render(<RiskRoutingWorkspace />);

    expect(await screen.findByTestId("validation-status")).toHaveTextContent(/valid/i);
    expect(screen.getByTestId("save-graph")).toBeDisabled();
  });

  it("edits a node and round-trips the mutation through updateRiskRoutingGraph", async () => {
    let saved: RiskRoutingGraph | null = null;
    state.app = makeApp({
      graph: validGraph(),
      books: [book("BOOK-A", "A"), book("DEFAULT", "Def")],
      onUpdate: (g) => {
        saved = g;
      },
    });
    render(<RiskRoutingWorkspace />);

    // Select the entry condition node, then change its operator (eq → ne).
    fireEvent.click(await screen.findByTestId("node-0"));
    fireEvent.change(screen.getByTestId("op-select"), { target: { value: "ne" } });

    const saveBtn = screen.getByTestId("save-graph");
    expect(saveBtn).toBeEnabled();
    fireEvent.click(saveBtn);

    await screen.findByText(/routing graph saved/i);
    expect(saved).not.toBeNull();
    const node0 = saved!.nodes.find((n) => n.id === 0);
    expect(node0?.kind === "condition" && node0.condition.op).toBe("ne");
  });

  it("catches a cyclic graph client-side (issues surfaced, Save disabled)", async () => {
    const cyclic: RiskRoutingGraph = {
      entry: 0,
      nodes: [
        {
          kind: "condition",
          id: 0,
          condition: { field: "ccy", op: "eq", value: { kind: "text", text: "EUR" }, onTrue: 1, onFalse: 1 },
        },
        {
          kind: "condition",
          id: 1,
          condition: { field: "ccy", op: "eq", value: { kind: "text", text: "USD" }, onTrue: 0, onFalse: 0 },
        },
      ],
    };
    state.app = makeApp({ graph: cyclic, books: [book("DEFAULT", "Def")] });
    render(<RiskRoutingWorkspace />);

    expect(await screen.findByTestId("validation-status")).toHaveTextContent(/issue/i);
    expect(screen.getByTestId("save-graph")).toBeDisabled();
  });

  it("catches a dangling edge client-side", async () => {
    const dangling: RiskRoutingGraph = {
      entry: 0,
      nodes: [
        {
          kind: "condition",
          id: 0,
          condition: { field: "ccy", op: "eq", value: { kind: "text", text: "EUR" }, onTrue: 77, onFalse: 1 },
        },
        { kind: "book", id: 1, bookId: "DEFAULT" },
      ],
    };
    state.app = makeApp({ graph: dangling, books: [book("DEFAULT", "Def")] });
    render(<RiskRoutingWorkspace />);

    expect(await screen.findByTestId("validation-status")).toHaveTextContent(/issue/i);
    expect(screen.getByTestId("save-graph")).toBeDisabled();
  });

  it("live-traces the default fill to the DEFAULT book (server-parity walk)", async () => {
    state.app = makeApp({ graph: validGraph(), books: [book("BOOK-A", "A"), book("DEFAULT", "Def")] });
    render(<RiskRoutingWorkspace />);

    // Default fill has ccy "" ≠ EUR ⇒ the false branch lands on DEFAULT ("Def").
    expect(await screen.findByTestId("trace-landing")).toHaveTextContent("Def");
  });

  it("is read-only for a non-admin (no Save button)", async () => {
    state.app = makeApp({
      graph: validGraph(),
      books: [book("BOOK-A", "A"), book("DEFAULT", "Def")],
      isAdmin: false,
    });
    render(<RiskRoutingWorkspace />);

    await screen.findByTestId("validation-status");
    expect(screen.queryByTestId("save-graph")).toBeNull();
  });
});
