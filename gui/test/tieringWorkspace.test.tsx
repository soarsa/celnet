/**
 * TieringWorkspace — the trader-facing FI Tiering surface, SESSION-PIVOTED.
 *
 * Book-level tiering is gone: outbound pricing is composed per client as a
 * {@link PricingGroup} feature pipeline, and this surface pivots on the INBOUND
 * FIX SESSIONS, answering per session which pricing group prices it and what
 * tiering that group's pipeline produces (read-only). We drive the workspace with
 * `useApp` mocked so we inject a fixed session + pricing-group roster and observe
 * the admin reassign mutation. Covers:
 *  (a) the sign-in gate for an anonymous session;
 *  (b) consolidation — `tiering` has NO standalone rail row (it is the "Tiering" tab
 *      of the "Pricing" workspace), but is still a valid Fixed-Income navigable id
 *      resolving to its `pricinggroups` host via the consolidated alias;
 *  (c) the session roster resolves each session's pricing group (exact session
 *      bind → desk default → none);
 *  (d) the detail pane shows the resolved group + a tiering summary read from the
 *      group's pipeline;
 *  (e) the admin reassign <select> moves the session between groups'
 *      `memberConnectionIds` via `transport.updatePricingGroup`;
 *  (f) a non-admin sees the assign control disabled (read-only).
 */

import { render, screen, fireEvent, waitFor, within } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { FixConnection, PricingGroup, TieringConfig } from "../src/data/contract";
import {
  ADMIN_ONLY_WORKSPACES,
  RAIL,
  workspaceDomains,
} from "../src/lib/commands";

const state: { app: unknown } = { app: null };
vi.mock("../src/app/AppContext", () => ({ useApp: () => state.app }));

import { TieringWorkspace } from "../src/workspaces/TieringWorkspace";

// --- fixtures ---------------------------------------------------------------

/** A FLAT_MARKUP tiering config @ 25 price-bps (the worked-example baseline). */
function flat25(): TieringConfig {
  return {
    unit: "PRICE_BPS",
    strategies: [
      {
        kind: "FLAT_MARKUP",
        halfSpread: 25,
        kappa: 0,
        sMax: 0,
        smoothingWeight: 0,
        expectedSpread: 0,
        maxDivergence: 0,
        coreSpread: 0,
        maxOutputSpread: 0,
        spreadScaleFactor: 0,
      },
    ],
    guardrails: { hMin: 0, hMax: 1, sMax: 0.5, spreadFloor: 0.01 },
    stalePolicy: "SUPPRESS",
  };
}

function session(overrides: Partial<FixConnection> = {}): FixConnection {
  return {
    id: "sess",
    name: "Session",
    kind: "FIXED_INCOME_QUOTE",
    bindAddr: "127.0.0.1:9099",
    senderCompId: "CELNET",
    targetCompId: "CPTY",
    enabled: true,
    running: true,
    boundAddr: "127.0.0.1:9099",
    desk: "",
    // no order route configured for this fixture (contract.ts:1852-1857)
    orderEndpoint: "",
    ...overrides,
  };
}

/** Three sessions: bound-by-session, matched-by-desk, and unmatched. */
const SESSIONS: FixConnection[] = [
  session({ id: "sess-alpha", name: "Alpha FIX", targetCompId: "ALPHA", desk: "rates", running: true }),
  session({ id: "sess-bravo", name: "Bravo FIX", targetCompId: "BRAVO", desk: "credit", running: false }),
  session({ id: "sess-gamma", name: "Gamma FIX", targetCompId: "GAMMA", desk: "", enabled: false, running: false }),
];

function group(overrides: Partial<PricingGroup> = {}): PricingGroup {
  return {
    id: "grp",
    name: "GROUP",
    description: "",
    memberConnectionIds: [],
    memberUserIds: [],
    memberDesks: [],
    espPipeline: null,
    rfqPipeline: null,
    sharePipeline: false,
    enabled: true,
    pricingSourceMode: 0,
    bookSkewWeight: null,
    lastLookMode: 0,
    lastLookToleranceBps: null,
    asyncGivebackPct: null,
    ...overrides,
  };
}

/**
 * Two enabled groups: TIER1 is bound to `sess-alpha` explicitly and carries a
 * shared TIERING pipeline (Flat 25); DESK-CREDIT is a desk-level default for the
 * `credit` desk (so it prices `sess-bravo`). `sess-gamma` matches neither.
 */
const GROUPS: PricingGroup[] = [
  group({
    id: "tier1",
    name: "TIER1",
    memberConnectionIds: ["sess-alpha"],
    sharePipeline: true,
    espPipeline: {
      features: [
        {
          kind: "TIERING",
          unit: "PRICE_BPS",
          shift: 0,
          reference: null,
          tiering: flat25(),
          axeSide: "BUY",
          magnitude: 0,
          kappa: 0,
          sMax: 0,
          skew: 0,
          triggered: false,
        },
      ],
      guardrails: { hMin: 0, hMax: 1, sMax: 0.5, spreadFloor: 0.01 },
    },
  }),
  group({ id: "desk-credit", name: "DESK-CREDIT", memberDesks: ["credit"] }),
];

function makeApp(opts: {
  user: { id: string; email: string } | null;
  isAdmin: boolean;
  canManagePricing: boolean;
  sessions?: FixConnection[];
  groups?: PricingGroup[];
  updatePricingGroup?: ReturnType<typeof vi.fn>;
}) {
  const groups = opts.groups ?? GROUPS;
  return {
    transport: {
      listFixConnections: vi.fn(async () => opts.sessions ?? SESSIONS),
      listPricingGroups: vi.fn(async () => groups),
      updatePricingGroup:
        opts.updatePricingGroup ??
        vi.fn(async (_id: string, spec: PricingGroup) => spec),
    },
    auth: {
      user: opts.user,
      isAdmin: opts.isAdmin,
      can: () => opts.canManagePricing,
    },
    setSignInOpen: vi.fn(),
    setWorkspace: vi.fn(),
  };
}

beforeEach(() => {
  state.app = null;
  vi.clearAllMocks();
});

describe("TieringWorkspace — rail registration (consolidated into Pricing)", () => {
  it("has NO standalone rail row but resolves to the pricinggroups host, FI, not admin-only", () => {
    // Consolidated: no rail row of its own (it is the "Pricing" workspace's Tiering tab).
    expect(RAIL.some((r) => r.id === "tiering")).toBe(false);
    // Still a valid Fixed-Income navigable id via the alias, NOT admin-only.
    expect(workspaceDomains("tiering")).toEqual(["fixed_income"]);
    expect(ADMIN_ONLY_WORKSPACES.has("tiering")).toBe(false);
    // The consolidated host is the single "Pricing" surface.
    expect(RAIL.find((r) => r.id === "pricinggroups")?.label).toBe("Pricing");
  });
});

describe("TieringWorkspace — sign-in gate", () => {
  it("shows a sign-in card for an anonymous session", () => {
    state.app = makeApp({ user: null, isAdmin: false, canManagePricing: true });
    render(<TieringWorkspace />);
    expect(screen.getByRole("button", { name: "Sign in" })).toBeInTheDocument();
  });
});

describe("TieringWorkspace — session → group resolution", () => {
  it("badges each session with its resolved group (session bind, desk default, none)", async () => {
    state.app = makeApp({
      user: { id: "trader", email: "trader@celnet.com" },
      isAdmin: true,
      canManagePricing: true,
    });
    render(<TieringWorkspace />);

    // The roster lists every session (async load).
    await screen.findByRole("button", { name: /Alpha FIX/ });
    const roster = screen.getByRole("region", { name: "select a FIX session" });
    expect(within(roster).getByRole("button", { name: /Bravo FIX/ })).toBeInTheDocument();
    expect(within(roster).getByRole("button", { name: /Gamma FIX/ })).toBeInTheDocument();

    // Alpha binds TIER1 by session; Bravo resolves DESK-CREDIT as the credit desk
    // default; Gamma matches neither ⇒ "No group".
    expect(within(roster).getByText("TIER1")).toBeInTheDocument();
    expect(within(roster).getByText("DESK-CREDIT")).toBeInTheDocument();
    expect(within(roster).getByText("No group")).toBeInTheDocument();
  });
});

describe("TieringWorkspace — resolved pricing detail", () => {
  it("shows the resolved group + a tiering summary from its pipeline", async () => {
    state.app = makeApp({
      user: { id: "trader", email: "trader@celnet.com" },
      isAdmin: true,
      canManagePricing: true,
    });
    render(<TieringWorkspace />);

    // The first ENABLED session (Alpha) is selected by default ⇒ its detail shows.
    await screen.findByRole("button", { name: /Alpha FIX/ });
    const detail = screen.getByRole("region", { name: "session pricing" });

    // The applied group + how it matched.
    const applied = within(detail).getByLabelText("applied pricing group");
    expect(within(applied).getByText("TIER1")).toBeInTheDocument();
    expect(within(applied).getByText(/Matched as bound to this session/)).toBeInTheDocument();

    // The shared pipeline's TIERING feature summarises read-only (Flat 25).
    const summary = within(detail).getByLabelText("pipeline tiering summary");
    expect(within(summary).getByText("ESP & RFQ (shared)")).toBeInTheDocument();
    expect(within(summary).getByText("TIERING")).toBeInTheDocument();
    expect(within(summary).getByText("Flat markup")).toBeInTheDocument();
    expect(within(summary).getByText(/H\s*25/)).toBeInTheDocument();

    // The worked client-price readout: the reference sample raw two-way widened by
    // Flat 25 price-bps ⇒ client 99.30 / 99.80.
    expect(within(summary).getByText(/LP\s*99\.50\s*\/\s*99\.60/)).toBeInTheDocument();
    expect(within(summary).getByText(/client\s*99\.30\s*\/\s*99\.80/)).toBeInTheDocument();
  });
});

describe("TieringWorkspace — admin reassign", () => {
  it("moves the session into the target group's memberConnectionIds", async () => {
    const updatePricingGroup = vi.fn(async (_id: string, spec: PricingGroup) => spec);
    state.app = makeApp({
      user: { id: "admin", email: "admin@celnet.com" },
      isAdmin: true,
      canManagePricing: true,
      updatePricingGroup,
    });
    render(<TieringWorkspace />);

    // Select the currently-unmatched Gamma session.
    fireEvent.click(await screen.findByRole("button", { name: /Gamma FIX/ }));

    // The admin-only assign control starts at "No group" (unbound) and is enabled.
    const select = screen.getByLabelText("Assign to group") as HTMLSelectElement;
    expect(select).toBeEnabled();
    expect(select.value).toBe("");

    // Reassign Gamma into TIER1 ⇒ updatePricingGroup adds it to TIER1's members.
    fireEvent.change(select, { target: { value: "tier1" } });

    await waitFor(() => expect(updatePricingGroup).toHaveBeenCalledTimes(1));
    const [id, spec] = updatePricingGroup.mock.calls[0]!;
    expect(id).toBe("tier1");
    expect((spec as PricingGroup).memberConnectionIds).toEqual(
      expect.arrayContaining(["sess-alpha", "sess-gamma"]),
    );
  });
});

describe("TieringWorkspace — read-only without Manage-Pricing", () => {
  it("disables the assign control for a signed-in user lacking manage_pricing·FI", async () => {
    state.app = makeApp({
      user: { id: "viewer", email: "viewer@celnet.com" },
      isAdmin: false,
      canManagePricing: false,
    });
    render(<TieringWorkspace />);

    await screen.findByRole("button", { name: /Alpha FIX/ });

    // The assign <select> is disabled and the Manage-only deep-link is absent.
    expect(screen.getByLabelText("Assign to group")).toBeDisabled();
    expect(
      screen.queryByRole("button", { name: /Edit tiering in Pricing Groups/ }),
    ).toBeNull();
  });
});
