/**
 * "Create hedging rule" from a PRICING GROUP — a pricing group's editor button / roster
 * right-click spawns a NEW hedge exit-policy rule pre-scoped to that group, handed to the
 * Hedging → Exit Policy builder via the {@link HedgeSeedProvider} store. The structural
 * clone of the deal seed path (`flowHedgeSeed.test.tsx`), with the honest desk-only
 * asymmetry a pricing group carries.
 *
 * Covered here (real modules, `useApp` mocked so there is no server):
 *   • the pure seed helpers — `hedgeSeedFromPricingGroup` projects a group onto the seed;
 *     `hedgeRuleFromPricingGroupSeed` seeds `desk = <desk>` for EXACTLY ONE member desk
 *     and NO condition for 0 or many desks (a flat-AND rule can't OR desks); the three
 *     hint variants + the gap note;
 *   • the seed store's discriminated source union — `requestHedgeSeedFromPricingGroup`
 *     sets `pending.source.kind === "pricingGroup"` (the deal verb still sets `"deal"`);
 *   • the Hedging Exit Policy builder consumes a pricing-group seed: it opens a NEW draft
 *     rule with the desk condition pre-filled + the pricing-group hint, and consumes it
 *     once (no re-seed) — the deal path is untouched.
 */
import { act } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";

import type { PricingGroup } from "../src/data/contract";
import {
  hedgeRuleFromPricingGroupSeed,
  hedgeSeedFromPricingGroup,
  pricingGroupSeedGapNote,
  pricingGroupSeedHint,
} from "../src/lib/hedgeSeed";

const state: { app: unknown } = { app: null };
vi.mock("../src/app/AppContext", () => ({ useApp: () => state.app }));

import { HedgingWorkspace } from "../src/workspaces/hedging/HedgingWorkspace";
import { HedgeSeedProvider, useHedgeSeed } from "../src/app/HedgeSeedContext";

// --- fixtures ---------------------------------------------------------------

/** A pricing group with the given member desks (all other fields inert). */
function group(name: string, desks: string[]): PricingGroup {
  return {
    id: `pg-${name}`,
    name,
    description: "",
    memberConnectionIds: ["conn-1", "conn-2"],
    memberUserIds: ["user-1"],
    memberDesks: desks,
    espPipeline: null,
    rfqPipeline: null,
    sharePipeline: false,
    enabled: true,
    pricingSourceMode: 0,
    bookSkewWeight: null,
    lastLookMode: 0,
    lastLookToleranceBps: null,
    asyncGivebackPct: null,
  };
}

function makeApp() {
  const setWorkspace = vi.fn();
  const updateHedgePolicyGraph = vi.fn(async (g: unknown) => g);
  return {
    setWorkspace,
    updateHedgePolicyGraph,
    app: {
      setWorkspace,
      conventions: {},
      scope: undefined,
      activeDomain: "fixed_income",
      transport: {
        label: "mock",
        listRiskBooks: vi.fn(async () => []),
        getHedgePolicyGraph: vi.fn(async () => null),
        // The Exit Policy tab loads the engine config for the leaf editor's hedge-VEHICLE
        // picker (a named vehicle must be a registry row).
        getHedgeConfig: vi.fn(async () => ({ vehicles: [], exitModes: [] })),
        updateHedgePolicyGraph,
      },
      auth: {
        user: { id: "u", email: "admin@celnet.com" },
        isAdmin: true,
        can: () => true,
      },
      setSignInOpen: vi.fn(),
    },
  };
}

/** A probe that mirrors the seed store's pending source + can fire a pricing-group seed. */
function PgSeedProbe({ g }: { g: PricingGroup }): React.ReactElement {
  const { pending, requestHedgeSeedFromPricingGroup } = useHedgeSeed();
  const kind = pending?.source.kind ?? "";
  const groupId = pending?.source.kind === "pricingGroup" ? pending.source.group.groupId : "";
  return (
    <div>
      <span data-testid="pg-seed-kind">{kind}</span>
      <span data-testid="pg-seed-group">{groupId}</span>
      <button
        data-testid="pg-seed-fire"
        onClick={() => requestHedgeSeedFromPricingGroup(hedgeSeedFromPricingGroup(g))}
      >
        seed
      </button>
    </div>
  );
}

beforeEach(() => {
  state.app = null;
  vi.clearAllMocks();
});
afterEach(() => cleanup());

// --- pure helpers -----------------------------------------------------------

describe("pricing-group hedge seed — pure helpers", () => {
  it("projects a PricingGroup onto the seed facts", () => {
    const s = hedgeSeedFromPricingGroup(group("GROUP-A", ["g10-rates"]));
    expect(s).toEqual({
      groupId: "pg-GROUP-A",
      name: "GROUP-A",
      desks: ["g10-rates"],
      connectionCount: 2,
      userCount: 1,
    });
  });

  it("seeds a single `desk =` condition for EXACTLY ONE member desk", () => {
    const rule = hedgeRuleFromPricingGroupSeed(hedgeSeedFromPricingGroup(group("A", ["g10-rates"])));
    expect(rule.conditions).toHaveLength(1);
    const c = rule.conditions[0]!;
    expect(c.field).toBe("desk");
    expect(c.op).toBe("eq");
    expect(c.value).toEqual({ kind: "text", text: "g10-rates" });
    // The action is the safe WAREHOUSE default — the trader picks the real one.
    expect(rule.action.kind).toBe("warehouse");
    expect(rule.enabled).toBe(true);
  });

  it("seeds NO condition for ZERO member desks (session/user scoped)", () => {
    const rule = hedgeRuleFromPricingGroupSeed(hedgeSeedFromPricingGroup(group("A", [])));
    expect(rule.conditions).toEqual([]);
    expect(rule.action.kind).toBe("warehouse");
  });

  it("seeds NO condition for MANY member desks (a flat-AND rule cannot OR desks)", () => {
    const rule = hedgeRuleFromPricingGroupSeed(
      hedgeSeedFromPricingGroup(group("A", ["g10-rates", "em-rates", "credit"])),
    );
    expect(rule.conditions).toEqual([]);
  });

  it("renders the three honest hint variants", () => {
    const one = pricingGroupSeedHint(hedgeSeedFromPricingGroup(group("GROUP-A", ["g10-rates"])));
    expect(one).toContain('pricing group "GROUP-A"');
    expect(one).toContain("desk g10-rates");
    expect(one).toContain("pick an exit action and Save.");

    const many = pricingGroupSeedHint(
      hedgeSeedFromPricingGroup(group("GROUP-B", ["g10-rates", "em-rates"])),
    );
    expect(many).toContain("spans 2 desks");
    expect(many).toContain("g10-rates, em-rates");
    expect(many).toContain("add a desk condition");
    expect(many).toContain("pick an exit action and Save.");

    const none = pricingGroupSeedHint(hedgeSeedFromPricingGroup(group("GROUP-C", [])));
    expect(none).toContain("no desk members");
    expect(none).toContain("add conditions");
    expect(none).toContain("pick an exit action and Save.");
  });

  it("gap note states only desk membership maps into the hedge vocabulary at Firm scope", () => {
    const note = pricingGroupSeedGapNote();
    expect(note).toContain("desk membership");
    expect(note).toContain("no currency, product or counterparty");
    expect(note).toContain("Firm");
  });
});

// --- the seed store's discriminated source union ----------------------------

describe("HedgeSeed store — pricing-group source", () => {
  it("requestHedgeSeedFromPricingGroup sets a pricingGroup source", async () => {
    await act(async () => {
      render(
        <HedgeSeedProvider>
          <PgSeedProbe g={group("GROUP-A", ["g10-rates"])} />
        </HedgeSeedProvider>,
      );
    });
    expect(screen.getByTestId("pg-seed-kind")).toHaveTextContent("");
    await act(async () => {
      fireEvent.click(screen.getByTestId("pg-seed-fire"));
    });
    expect(screen.getByTestId("pg-seed-kind")).toHaveTextContent("pricingGroup");
    expect(screen.getByTestId("pg-seed-group")).toHaveTextContent("pg-GROUP-A");
  });
});

// --- the Exit Policy builder consumes the pricing-group seed ----------------

describe("Hedging Exit Policy builder consumes a pricing-group seed", () => {
  it("opens a NEW draft pre-scoped to the group's single desk + shows the hint, consumed once", async () => {
    state.app = makeApp().app;
    await act(async () => {
      render(
        <HedgeSeedProvider>
          <PgSeedProbe g={group("GROUP-A", ["g10-rates"])} />
          <HedgingWorkspace />
        </HedgeSeedProvider>,
      );
    });
    await screen.findByTestId("hedge-create-rule");
    expect(screen.queryByTestId("hedge-rule-editor")).toBeNull();

    await act(async () => {
      fireEvent.click(screen.getByTestId("pg-seed-fire"));
    });

    expect(await screen.findByTestId("hedge-rule-editor")).toBeInTheDocument();
    const hint = await screen.findByTestId("hedge-seed-hint");
    expect(hint).toHaveTextContent('pricing group "GROUP-A"');
    expect(hint).toHaveTextContent("desk g10-rates");

    // The draft is pre-scoped Desk = g10-rates.
    const preview = screen.getByTestId("hedge-rule-preview");
    expect(preview).toHaveTextContent("Desk");
    expect(preview).toHaveTextContent("g10-rates");

    // Nothing auto-saved; the seed is consumed exactly once (store now empty).
    expect((state.app as ReturnType<typeof makeApp>["app"]).transport.updateHedgePolicyGraph).not
      .toHaveBeenCalled();
    await waitFor(() => expect(screen.getByTestId("pg-seed-kind")).toHaveTextContent(""));
  });

  it("a multi-desk group seeds a NO-condition draft (default rule) with the spanning hint", async () => {
    state.app = makeApp().app;
    await act(async () => {
      render(
        <HedgeSeedProvider>
          <PgSeedProbe g={group("GROUP-B", ["g10-rates", "em-rates"])} />
          <HedgingWorkspace />
        </HedgeSeedProvider>,
      );
    });
    await screen.findByTestId("hedge-create-rule");
    await act(async () => {
      fireEvent.click(screen.getByTestId("pg-seed-fire"));
    });

    await screen.findByTestId("hedge-rule-editor");
    const hint = screen.getByTestId("hedge-seed-hint");
    expect(hint).toHaveTextContent("spans 2 desks");
    // No conditions ⇒ the preview reads as the catch-all default rule.
    expect(screen.getByTestId("hedge-rule-preview")).toHaveTextContent("Otherwise");
  });
});
