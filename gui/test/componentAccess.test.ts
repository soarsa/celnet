/**
 * componentAccess — the pure component → capability mapping + the Read/Write
 * projection toggles behind the Permissions page. These assert COMPONENT_ACCESS
 * maps each component to exactly the right capabilities, that a toggle is a
 * faithful projection over its capability set (on / off / mixed), that a click
 * round-trips through the deny-wins overlay algebra, and that the self-lockout
 * inputs are what the page guards on.
 */

import { describe, expect, it } from "vitest";

import { CAPABILITY_ACTIONS, type Capability } from "../src/data/contract";
import {
  COMPONENT_ACCESS,
  COMPONENT_SECTIONS,
  type ComponentAccess,
  capKey,
  componentAdvancedCaps,
  componentReadCaps,
  componentWriteCaps,
  isReadOnlyComponent,
  overlayFromCapabilities,
  resolveEffective,
  setOverlayFor,
  toggleState,
  toggleTarget,
} from "../src/lib/capabilityMatrix";

const byId = (id: string): ComponentAccess => {
  const c = COMPONENT_ACCESS.find((x) => x.id === id);
  if (!c) throw new Error(`no component \`${id}\``);
  return c;
};

const keysOf = (caps: readonly Capability[]): Set<string> =>
  new Set(caps.map((c) => capKey(c.action, c.asset)));

describe("COMPONENT_ACCESS — the component → capability spec", () => {
  it("maps each FX-options component to view (Read) + its own actions (Write)", () => {
    expect(keysOf(componentReadCaps(byId("ticket")))).toEqual(new Set([capKey("view", "fx_options")]));
    expect(keysOf(componentWriteCaps(byId("ticket")))).toEqual(
      new Set([capKey("price", "fx_options"), capKey("execute", "fx_options")]),
    );
    expect(keysOf(componentWriteCaps(byId("stream")))).toEqual(
      new Set([capKey("stream", "fx_options"), capKey("execute", "fx_options")]),
    );
  });

  it("maps each fixed-income component to the fixed_income asset", () => {
    for (const id of ["rates", "curve", "ratesrisk", "quoting", "deals", "ratesbook", "book"]) {
      for (const cap of componentAdvancedCaps(byId(id))) {
        expect(cap.asset).toBe("fixed_income");
      }
    }
    expect(keysOf(componentWriteCaps(byId("quoting")))).toEqual(
      new Set([
        capKey("quote_respond", "fixed_income"),
        capKey("rfq_respond", "fixed_income"),
        capKey("ioi_respond", "fixed_income"),
        capKey("execute", "fixed_income"),
      ]),
    );
    expect(keysOf(componentWriteCaps(byId("ratesbook")))).toEqual(
      new Set([capKey("book", "fixed_income")]),
    );
  });

  it("read-only components have no write actions (Surface/Risk/Curve/Rates Risk/Deals/Book)", () => {
    for (const id of ["surface", "risk", "curve", "ratesrisk", "deals", "book"]) {
      expect(isReadOnlyComponent(byId(id))).toBe(true);
      expect(componentWriteCaps(byId(id))).toHaveLength(0);
    }
    for (const id of ["ticket", "stream", "rates", "quoting", "ratesbook", "administration"]) {
      expect(isReadOnlyComponent(byId(id))).toBe(false);
    }
  });

  it("Administration is one cross-asset toggle governing administer on BOTH assets (Read == Write)", () => {
    const admin = byId("administration");
    const expected = new Set([
      capKey("administer", "fx_options"),
      capKey("administer", "fixed_income"),
    ]);
    expect(keysOf(componentReadCaps(admin))).toEqual(expected);
    expect(keysOf(componentWriteCaps(admin))).toEqual(expected);
  });

  it("every component belongs to a declared section and every action label is known", () => {
    const sections = new Set(COMPONENT_SECTIONS.map((s) => s.id));
    for (const c of COMPONENT_ACCESS) {
      expect(sections.has(c.section)).toBe(true);
      for (const a of [...c.readActions, ...c.writeActions]) {
        expect(CAPABILITY_ACTIONS).toContain(a);
      }
    }
  });
});

describe("toggleState — the Read/Write projection over a capability set", () => {
  it("a trader with no overlay: Write is ON when the role allows every action", () => {
    // Trader holds price + execute on FX, so Ticket Write projects to ON.
    const state = toggleState("TRADER", new Map(), componentWriteCaps(byId("ticket")));
    expect(state).toBe("on");
  });

  it("ON when all allowed, OFF when all denied, MIXED when some are and some are not", () => {
    const ticketWrite = componentWriteCaps(byId("ticket")); // [price·FX, execute·FX]
    // Deny BOTH → off.
    const allDenied = overlayFromCapabilities([], ticketWrite);
    expect(toggleState("TRADER", allDenied, ticketWrite)).toBe("off");
    // Deny ONE of the two → mixed.
    const oneDenied = overlayFromCapabilities([], [{ action: "execute", asset: "fx_options" }]);
    expect(toggleState("TRADER", oneDenied, ticketWrite)).toBe("mixed");
  });

  it("a read-only component's empty write set resolves to OFF (the disabled affordance)", () => {
    expect(toggleState("TRADER", new Map(), componentWriteCaps(byId("surface")))).toBe("off");
  });

  it("Administration Write is OFF for a trader (no administer) and ON for an admin", () => {
    const adminWrite = componentWriteCaps(byId("administration"));
    expect(toggleState("TRADER", new Map(), adminWrite)).toBe("off");
    expect(toggleState("ADMIN", new Map(), adminWrite)).toBe("on");
  });
});

describe("toggleTarget — a click projects the whole set", () => {
  it("ON → deny (turn off); OFF/MIXED → grant (turn on, mixed resolves all-on first)", () => {
    expect(toggleTarget("on")).toBe("deny");
    expect(toggleTarget("off")).toBe("grant");
    expect(toggleTarget("mixed")).toBe("grant");
  });
});

describe("toggle round-trip through the deny-wins overlay algebra", () => {
  it("turning Write OFF denies every action — deny wins in the resolved effective set", () => {
    const ratesbookWrite = componentWriteCaps(byId("ratesbook")); // [book·FI]
    // Trader starts ON; click → deny all.
    const next = setOverlayFor(new Map(), ratesbookWrite, toggleTarget("on"));
    const eff = keysOf(resolveEffective("TRADER", next));
    expect(eff.has(capKey("book", "fixed_income"))).toBe(false); // denied, deny-wins
    // Sibling FI affordances are untouched.
    expect(eff.has(capKey("price", "fixed_income"))).toBe(true);
    expect(eff.has(capKey("view", "fixed_income"))).toBe(true);
  });

  it("turning an indeterminate Write ON grants the whole set (resolves the mix)", () => {
    const ticketWrite = componentWriteCaps(byId("ticket"));
    // Start mixed: execute denied, price inherited (allowed).
    const mixed = overlayFromCapabilities([], [{ action: "execute", asset: "fx_options" }]);
    expect(toggleState("TRADER", mixed, ticketWrite)).toBe("mixed");
    // Click a mixed toggle → grant all.
    const next = setOverlayFor(mixed, ticketWrite, toggleTarget("mixed"));
    const eff = keysOf(resolveEffective("TRADER", next));
    expect(eff.has(capKey("price", "fx_options"))).toBe(true);
    expect(eff.has(capKey("execute", "fx_options"))).toBe(true);
    expect(toggleState("TRADER", next, ticketWrite)).toBe("on");
  });

  it("Read toggles the SHARED view capability — it moves every same-asset component", () => {
    const ticketRead = componentReadCaps(byId("ticket")); // [view·FX]
    // Turn Ticket Read OFF (deny view·FX).
    const next = setOverlayFor(new Map(), ticketRead, toggleTarget("on"));
    // Stream + Surface + Risk (all FX) now read OFF too — one underlying capability.
    for (const id of ["stream", "surface", "risk"]) {
      expect(toggleState("TRADER", next, componentReadCaps(byId(id)))).toBe("off");
    }
    // Fixed-income Read is unaffected (a different view capability).
    expect(toggleState("TRADER", next, componentReadCaps(byId("rates")))).toBe("on");
  });

  it("turning Administration ON for a trader grants administer on BOTH assets", () => {
    const adminWrite = componentWriteCaps(byId("administration"));
    const next = setOverlayFor(new Map(), adminWrite, toggleTarget("off"));
    const eff = keysOf(resolveEffective("TRADER", next));
    expect(eff.has(capKey("administer", "fx_options"))).toBe(true);
    expect(eff.has(capKey("administer", "fixed_income"))).toBe(true);
  });
});

describe("self-lockout guard inputs", () => {
  it("the Administration row's capability set is the administer cells a self-guard disables", () => {
    // The page disables a toggle whose caps include `administer` when editing self.
    const adminCaps = componentAdvancedCaps(byId("administration"));
    expect(adminCaps.every((c) => c.action === "administer")).toBe(true);
    // No non-admin component carries administer, so none is ever self-locked.
    for (const c of COMPONENT_ACCESS) {
      if (c.id === "administration") continue;
      expect(componentAdvancedCaps(c).some((cap) => cap.action === "administer")).toBe(false);
    }
  });
});
