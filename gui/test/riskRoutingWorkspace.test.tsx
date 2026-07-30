/**
 * RiskRoutingWorkspace — the reworked rules-TABLE + per-rule-editor CRUD surface
 * (the old decision-tree canvas / yes-no port wiring is gone). Driven with `useApp`
 * mocked (no server): a loaded graph decompiles into table rows; "Create risk rule"
 * opens the per-rule editor; dragging a field chip adds a condition; picking a book
 * + Save adds a row and "Save routing" persists the compiled graph via
 * `updateRiskRoutingGraph`; reordering re-weights; and an error conflict disables
 * Save. Capability-gated: without the FI risk capability the table is read-only.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";

import type { DeskDesc, RiskBook, RiskRoutingGraph } from "../src/data/contract";
import {
  compileRulesToGraph,
  newRuleId,
  type RiskRule,
  type RuleCondition,
} from "../src/lib/riskRules";
import { encodeDrag } from "../src/workspaces/riskrouting/FieldPalette";

const state: { app: unknown } = { app: null };
vi.mock("../src/app/AppContext", () => ({ useApp: () => state.app }));

import { RiskRoutingWorkspace } from "../src/workspaces/riskrouting/RiskRoutingWorkspace";

function book(id: string, name: string, deskId: string | null = null): RiskBook {
  return { id, name, parentId: null, deskId, description: "", limits: null, enabled: true };
}
function cond(field: RuleCondition["field"], op: RuleCondition["op"], text: string): RuleCondition {
  return { field, op, value: { kind: "text", text } };
}
function rule(conditions: RuleCondition[], bookId: string | null): RiskRule {
  return { id: newRuleId(), conditions, bookId, enabled: true };
}

/** A loaded graph = the compiled form of a rule list (round-trips into the table). */
function graphOf(rules: RiskRule[]): RiskRoutingGraph {
  return compileRulesToGraph(rules);
}

function makeApp(opts: {
  graph: RiskRoutingGraph | null;
  books: RiskBook[];
  desks?: DeskDesc[];
  canEdit?: boolean;
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
      listDesks: vi.fn(async () => opts.desks ?? []),
      listFixConnections: vi.fn(async () => []),
    },
    auth: {
      user: { id: "u", email: "risk@celnet.com" },
      isAdmin: true,
      can: () => opts.canEdit ?? true,
    },
    setSignInOpen: vi.fn(),
  };
}

beforeEach(() => {
  state.app = null;
  vi.clearAllMocks();
});
afterEach(() => cleanup());

const TWO_RULES = (): RiskRule[] => [
  rule([cond("ccy", "eq", "EUR")], "BOOK-A"),
  rule([], "DEFAULT"),
];
const BOOKS = () => [book("BOOK-A", "A", "rates"), book("BOOK-B", "B", "rates"), book("DEFAULT", "Def")];
const DESKS: DeskDesc[] = [{ id: "rates", name: "RATES" }];

describe("RiskRoutingWorkspace (rules table + editor)", () => {
  it("renders the loaded graph as table rows (description + destination)", async () => {
    state.app = makeApp({ graph: graphOf(TWO_RULES()), books: BOOKS(), desks: DESKS });
    render(<RiskRoutingWorkspace />);

    expect(await screen.findByTestId("rules-table")).toBeInTheDocument();
    expect(screen.getByTestId("rule-desc-0")).toHaveTextContent(/Currency = EUR/);
    expect(screen.getByTestId("rule-desc-1")).toHaveTextContent(/Otherwise/);
    // Destination resolves desk-scoped.
    expect(screen.getByTestId("rule-row-0")).toHaveTextContent("RATES / A");
  });

  it("‘Create risk rule’ opens the per-rule editor", async () => {
    state.app = makeApp({ graph: graphOf(TWO_RULES()), books: BOOKS(), desks: DESKS });
    render(<RiskRoutingWorkspace />);

    fireEvent.click(await screen.findByTestId("create-rule"));
    expect(await screen.findByTestId("rule-editor")).toBeInTheDocument();
  });

  it("dragging a field chip into the editor adds a condition row", async () => {
    state.app = makeApp({ graph: graphOf(TWO_RULES()), books: BOOKS(), desks: DESKS });
    render(<RiskRoutingWorkspace />);

    fireEvent.click(await screen.findByTestId("create-rule"));
    const area = await screen.findByTestId("rule-condition-area");
    const data = encodeDrag({ kind: "field", field: "product" });
    const dataTransfer = { getData: (_t: string) => data };
    fireEvent.dragOver(area, { dataTransfer });
    fireEvent.drop(area, { dataTransfer });

    expect(await screen.findByTestId("cond-row-0")).toBeInTheDocument();
    expect(screen.getByTestId("cond-field-0")).toBeInTheDocument();
  });

  it("building a rule + Save adds a table row and persists the compiled graph", async () => {
    let saved: RiskRoutingGraph | null = null;
    state.app = makeApp({
      graph: graphOf(TWO_RULES()),
      books: BOOKS(),
      desks: DESKS,
      onUpdate: (g) => {
        saved = g;
      },
    });
    render(<RiskRoutingWorkspace />);

    // Open the editor, drag in a condition, pick a destination, save the rule.
    fireEvent.click(await screen.findByTestId("create-rule"));
    const area = await screen.findByTestId("rule-condition-area");
    const data = encodeDrag({ kind: "field", field: "product" });
    const dataTransfer = { getData: (_t: string) => data };
    fireEvent.drop(area, { dataTransfer });

    fireEvent.change(await screen.findByTestId("book-desk-select"), { target: { value: "rates" } });
    fireEvent.change(screen.getByTestId("book-target-select"), { target: { value: "BOOK-B" } });
    fireEvent.click(screen.getByTestId("rule-save"));

    // Back on the table: the new rule slotted ABOVE the default (3 rows now).
    expect(await screen.findByTestId("rule-row-2")).toBeInTheDocument();
    expect(screen.getByTestId("rule-desc-1")).toHaveTextContent(/Product/);

    // Persist the whole compiled graph.
    const saveBtn = screen.getByTestId("save-graph");
    expect(saveBtn).toBeEnabled();
    fireEvent.click(saveBtn);
    await screen.findByText(/routing rules saved/i);
    expect(saved).not.toBeNull();
    // The compiled graph is a first-match-wins spine ending at a book leaf.
    expect(saved!.nodes.some((n) => n.kind === "book" && n.bookId === "BOOK-B")).toBe(true);
  });

  it("dragging a row re-weights the rules (priority order)", async () => {
    state.app = makeApp({
      graph: graphOf([
        rule([cond("ccy", "eq", "EUR")], "BOOK-A"),
        rule([cond("ccy", "eq", "GBP")], "BOOK-B"),
        rule([], "DEFAULT"),
      ]),
      books: BOOKS(),
      desks: DESKS,
    });
    render(<RiskRoutingWorkspace />);

    await screen.findByTestId("rules-table");
    expect(screen.getByTestId("rule-desc-0")).toHaveTextContent(/EUR/);

    // Drag row 0 onto row 1 → the GBP rule becomes the highest priority.
    const row0 = screen.getByTestId("rule-row-0");
    const row1 = screen.getByTestId("rule-row-1");
    fireEvent.dragStart(row0);
    fireEvent.dragOver(row1);
    fireEvent.drop(row1);

    expect(screen.getByTestId("rule-desc-0")).toHaveTextContent(/GBP/);
    expect(screen.getByTestId("rule-desc-1")).toHaveTextContent(/EUR/);
  });

  it("an error conflict disables Save routing", async () => {
    state.app = makeApp({ graph: graphOf(TWO_RULES()), books: BOOKS(), desks: DESKS });
    render(<RiskRoutingWorkspace />);

    // Delete the default (row 1) → no catch-all rule ⇒ an error conflict.
    fireEvent.click(await screen.findByTestId("rule-delete-1"));

    expect(screen.getByTestId("validation-status")).toHaveTextContent(/issue/i);
    expect(screen.getByTestId("save-graph")).toBeDisabled();
  });

  it("is read-only without the FI risk capability (table visible, no Create/Save)", async () => {
    state.app = makeApp({ graph: graphOf(TWO_RULES()), books: BOOKS(), desks: DESKS, canEdit: false });
    render(<RiskRoutingWorkspace />);

    expect(await screen.findByTestId("rules-table")).toBeInTheDocument();
    expect(screen.queryByTestId("create-rule")).toBeNull();
    expect(screen.queryByTestId("save-graph")).toBeNull();
    expect(screen.getByTestId("rule-toggle-0")).toBeDisabled();
  });
});
