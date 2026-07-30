/**
 * command registry — the single source-of-truth invariant (GW1-S4).
 *
 * The plan's single-source contract: the cheatsheet == the registry projection;
 * every dispatched key is a registry binding & vice-versa; no chord collision; the
 * data-driven rail generates exactly one `⌘N` per view (uncapping the old ⌘1-5);
 * and `resolveChord` (the Shell's dispatcher) honours exactly the advertised chords.
 *
 * Oracles are reached WITHOUT replaying the dispatcher: chord uniqueness is a set
 * assertion over the registry; the cheatsheet projection is compared to a re-derived
 * filter; and `resolveChord` is driven with synthetic key events and its output id
 * is checked against the registry — never against its own internals.
 */

import { describe, expect, it } from "vitest";

import {
  ADMIN_ONLY_WORKSPACES,
  buildCommands,
  cheatsheet,
  COMMAND_META,
  domainAccessible,
  DOMAINS,
  firstAccessibleWorkspace,
  RAIL,
  railChord,
  railForDomain,
  resolveChord,
  workspaceAccessible,
  workspaceAssets,
  workspaceDomains,
  type CommandContext,
  type NavAuth,
} from "../src/lib/commands";
import { SHORTCUTS } from "../src/lib/shortcuts";

/** A spy context: every action records its id so we can assert dispatch wiring. */
function spyContext(): { ctx: CommandContext; calls: string[] } {
  const calls: string[] = [];
  const rec = (id: string) => () => calls.push(id);
  return {
    calls,
    ctx: {
      setWorkspace: (w) => calls.push(`ws-${w}`),
      openPalette: rec("palette"),
      openScopeSwitcher: rec("switch-scope"),
      showShortcuts: rec("help"),
      drillScopeDown: rec("scope-drill-down"),
      resetScope: rec("scope-reset"),
      markSurface: rec("mark-surface"),
      openRiskScenario: rec("risk-scenario"),
      saveView: rec("save-view"),
      toggleDensity: rec("toggle-density"),
      toggleAppearance: rec("toggle-appearance"),
      toggleContrast: rec("toggle-contrast"),
      canDrillScope: true,
    },
  };
}

describe("registry integrity", () => {
  it("every command has a unique id and a non-empty label", () => {
    const ids = new Set<string>();
    for (const c of COMMAND_META) {
      expect(c.label.length).toBeGreaterThan(0);
      expect(ids.has(c.id)).toBe(false);
      ids.add(c.id);
    }
  });

  it("chords are collision-free (no two commands share a display chord)", () => {
    const seen = new Set<string>();
    for (const c of COMMAND_META) {
      if (c.keys.length === 0) continue;
      const chord = c.keys.join("");
      expect(seen.has(chord)).toBe(false);
      seen.add(chord);
    }
  });

  it("the rail generates exactly one ⌘N workspace jump per view (uncaps ⌘1-5)", () => {
    for (let i = 0; i < RAIL.length; i += 1) {
      const id = `ws-${RAIL[i]!.id}`;
      const cmd = COMMAND_META.find((c) => c.id === id);
      expect(cmd).toBeDefined();
      expect(cmd!.keys).toEqual(railChord(i));
    }
    // The jumps cover every rail view and nothing more.
    const wsCmds = COMMAND_META.filter((c) => c.group === "Workspace");
    expect(wsCmds.map((c) => c.id)).toEqual(RAIL.map((r) => `ws-${r.id}`));
  });
});

describe("single class-parametric rail (fe-fi-migration #6 — domain-tab collapse)", () => {
  it("has NO duplicate FX/FI rail rows — the collapsed ids are gone", () => {
    const ids = new Set(RAIL.map((r) => r.id));
    // The duplicated FX/FI rows that routed to the same shared workspace at a fixed
    // lens are collapsed into ONE class-parametric row each.
    for (const gone of ["rates", "curve", "ratesrisk", "deals", "ratesbook"]) {
      expect(ids.has(gone as never)).toBe(false);
    }
    // The surviving capability rows are present exactly once each.
    for (const kept of ["ticket", "surface", "risk", "book", "quoting"]) {
      expect(RAIL.filter((r) => r.id === kept)).toHaveLength(1);
    }
    // Every rail id is unique + every glyph/label is non-empty.
    expect(ids.size).toBe(RAIL.length);
    for (const r of RAIL) {
      expect(r.glyph.length).toBeGreaterThan(0);
      expect(r.label.length).toBeGreaterThan(0);
    }
  });

  it("the collapsed Market Data row carries both asset classes (class chosen inside)", () => {
    // `curve` (FI) + `surface` (FX) collapsed into ONE cross-asset "Market Data" row.
    const md = RAIL.find((r) => r.id === "surface");
    expect(md).toBeDefined();
    expect(md!.label).toBe("Market Data");
    expect(new Set(md!.assets)).toEqual(new Set(["fx_options", "fixed_income"]));
  });

  it("every rail row declares the asset class(es) it serves (workspaceAssets)", () => {
    for (const r of RAIL) expect(workspaceAssets(r.id)).toEqual(r.assets);
    // The collapsed shared-market trading capabilities are cross-asset (both classes).
    for (const id of ["surface", "risk", "book"] as const) {
      expect(new Set(workspaceAssets(id))).toEqual(new Set(["fx_options", "fixed_income"]));
    }
    // Quoting + Streaming (no FX twin) are single-asset FI; Ticket + Stream are FX.
    expect(workspaceAssets("quoting")).toEqual(["fixed_income"]);
    expect(workspaceAssets("fistreaming")).toEqual(["fixed_income"]);
    expect(workspaceAssets("ticket")).toEqual(["fx_options"]);
    expect(workspaceAssets("stream")).toEqual(["fx_options"]);
    // Admin/ops rows serve no asset class (gated by isAdmin, no license concept).
    for (const id of ADMIN_ONLY_WORKSPACES) expect(workspaceAssets(id)).toEqual([]);
  });
});

describe("single-source cheatsheet projection", () => {
  it("the cheatsheet is exactly the chord-bearing commands, in registry order", () => {
    const projected = cheatsheet().map((c) => c.id);
    const expected = COMMAND_META.filter((c) => c.keys.length > 0).map((c) => c.id);
    expect(projected).toEqual(expected);
  });

  it("every global registry chord appears in the rendered SHORTCUTS list", () => {
    for (const c of cheatsheet()) {
      const row = SHORTCUTS.find((s) => s.id === c.id);
      expect(row, `missing cheatsheet row for ${c.id}`).toBeDefined();
      expect(row!.keys).toEqual(c.keys);
    }
  });
});

describe("resolveChord — honoured grammar == advertised grammar", () => {
  it("resolves every meta-key chord the registry advertises", () => {
    // Each non-digit meta chord round-trips its lowercase letter to its command id.
    expect(resolveChord({ key: "k", meta: true }, RAIL.length)?.id).toBe("palette");
    expect(resolveChord({ key: "p", meta: true }, RAIL.length)?.id).toBe("switch-scope");
    expect(resolveChord({ key: "?", meta: false }, RAIL.length)?.id).toBe("help");
  });

  it("⌘1..n resolves to the rail view at that index, capped to the digit grammar", () => {
    for (let i = 0; i < RAIL.length; i += 1) {
      const chord = railChord(i);
      // A view beyond the ten single-digit slots (⌘1..⌘9, ⌘0) advertises NO chord
      // and is palette-only — there is no digit keydown that could reach it.
      if (chord.length === 0) continue;
      // Drive the keydown from the digit the registry advertises for this view
      // (railChord), so the honoured grammar is checked against its single source —
      // ⌘1..⌘9 for the first nine, ⌘0 wrapping to a tenth.
      const key = chord[1]!;
      const hit = resolveChord({ key, meta: true }, RAIL.length);
      expect(hit?.id).toBe(`ws-${RAIL[i]!.id}`);
      expect(hit?.railIndex).toBe(i);
    }
    // The eleventh-and-beyond views (Permissions / Reference Data today) have no
    // ⌘N chord at all — the single-digit grammar addresses only ⌘1..⌘9 + ⌘0.
    expect(railChord(10)).toEqual([]);
    // A two-digit "chord" is never honoured (the grammar is a single keypress).
    expect(resolveChord({ key: String(RAIL.length + 1), meta: true }, RAIL.length)).toBeNull();
  });

  it("rejects the removed pair-navigator chord (⌘B) — no advertised-but-dead key", () => {
    expect(resolveChord({ key: "b", meta: true }, RAIL.length)).toBeNull();
    expect(SHORTCUTS.some((s) => s.keys.join("") === "⌘B")).toBe(false);
  });

  it("a bare digit (no meta) is not a workspace jump", () => {
    expect(resolveChord({ key: "1", meta: false }, RAIL.length)).toBeNull();
  });
});

describe("buildCommands — dispatch wiring & context gating", () => {
  it("dispatching each built command invokes the matching context action", () => {
    const { ctx, calls } = spyContext();
    for (const cmd of buildCommands(ctx)) cmd.run();
    // Every workspace jump fired, plus the global/scope/action commands.
    for (const r of RAIL) expect(calls).toContain(`ws-${r.id}`);
    expect(calls).toContain("palette");
    expect(calls).toContain("switch-scope");
    expect(calls).toContain("scope-reset");
  });

  it("omits the drill-down command at the terminal scope (no dead action offered)", () => {
    const { ctx } = spyContext();
    const terminal = buildCommands({ ...ctx, canDrillScope: false });
    expect(terminal.some((c) => c.id === "scope-drill-down")).toBe(false);
    const drillable = buildCommands({ ...ctx, canDrillScope: true });
    expect(drillable.some((c) => c.id === "scope-drill-down")).toBe(true);
  });

  it("every built command id exists in the registry (no orphan handler)", () => {
    const { ctx } = spyContext();
    const known = new Set(COMMAND_META.map((c) => c.id));
    for (const cmd of buildCommands(ctx)) expect(known.has(cmd.id)).toBe(true);
  });
});

describe("domain layer — DOMAINS / workspaceDomains / domainAccessible / railForDomain", () => {
  /** A NavAuth whose `can` admits exactly the given set of `action·asset` keys. */
  function navAuth(opts: { isAdmin: boolean; allow?: ReadonlySet<string> }): NavAuth {
    return {
      isAdmin: opts.isAdmin,
      can: (action, asset) => opts.allow?.has(`${action}·${asset}`) ?? false,
    };
  }
  const signedOut: NavAuth = { isAdmin: false, can: () => true };

  it("DOMAINS is exactly FX / FI / Administration, in bar order", () => {
    expect(DOMAINS.map((d) => d.id)).toEqual(["fx_options", "fixed_income", "admin"]);
    expect(DOMAINS.map((d) => d.label)).toEqual(["FX Options", "Fixed Income", "Administration"]);
  });

  describe("workspaceDomains (derived from served assets — Model A)", () => {
    it("shared cross-asset rows appear under BOTH trading domains", () => {
      for (const id of ["surface", "risk", "book"] as const) {
        expect([...workspaceDomains(id)].sort()).toEqual(["fixed_income", "fx_options"]);
      }
    });

    it("single-asset rows appear under their one trading domain only", () => {
      // Ticket is now FX-only (FI prices on the Streaming hub) — off the FI tab.
      expect(workspaceDomains("ticket")).toEqual(["fx_options"]);
      expect(workspaceDomains("stream")).toEqual(["fx_options"]);
      expect(workspaceDomains("xva")).toEqual(["fx_options"]);
      expect(workspaceDomains("excel")).toEqual(["fx_options"]);
      expect(workspaceDomains("quoting")).toEqual(["fixed_income"]);
      expect(workspaceDomains("fistreaming")).toEqual(["fixed_income"]);
    });

    it("admin/ops rows appear under the single admin domain", () => {
      for (const id of ADMIN_ONLY_WORKSPACES) {
        expect(workspaceDomains(id)).toEqual(["admin"]);
      }
    });

    it("derives membership from `assets` — no row's domains diverge from its served assets", () => {
      for (const r of RAIL) {
        const doms = workspaceDomains(r.id);
        if (ADMIN_ONLY_WORKSPACES.has(r.id)) {
          expect(doms).toEqual(["admin"]);
        } else {
          expect([...doms].sort()).toEqual([...r.assets].sort());
        }
      }
    });
  });

  describe("domainAccessible", () => {
    it("the Administration tab requires isAdmin (hidden signed out)", () => {
      expect(domainAccessible("admin", navAuth({ isAdmin: true }))).toBe(true);
      expect(domainAccessible("admin", navAuth({ isAdmin: false }))).toBe(false);
      expect(domainAccessible("admin", signedOut)).toBe(false);
    });

    it("a trading tab follows the class view capability", () => {
      const fxOnly = navAuth({ isAdmin: false, allow: new Set(["view·fx_options"]) });
      expect(domainAccessible("fx_options", fxOnly)).toBe(true);
      expect(domainAccessible("fixed_income", fxOnly)).toBe(false);
    });

    it("signed out: both trading tabs are accessible (permissive can)", () => {
      expect(domainAccessible("fx_options", signedOut)).toBe(true);
      expect(domainAccessible("fixed_income", signedOut)).toBe(true);
    });
  });

  describe("railForDomain (structural membership, RAIL order)", () => {
    it("FX Options = the FX-only rows + the shared rows, in RAIL order", () => {
      expect(railForDomain("fx_options").map((r) => r.id)).toEqual([
        "ticket",
        "stream",
        "surface",
        "risk",
        "book",
        "xva",
        "excel",
      ]);
    });

    it("Fixed Income = Streaming (primary FI surface, top) + Tiering + the shared rows + quoting + Pricing Groups (moved off Admin), in RAIL order", () => {
      expect(railForDomain("fixed_income").map((r) => r.id)).toEqual([
        "fistreaming",
        "aggbook",
        "tiering",
        "riskbooks",
        "riskdashboard",
        "riskrouting",
        "risktransfer",
        "transferinbox",
        "transferaudit",
        "surface",
        "risk",
        "book",
        "quoting",
        // Pricing Groups is a ManagePricing·FI client-pricing surface, moved off the
        // Administration tab onto Fixed Income (viewCap-gated); it sits at its RAIL
        // position (after the admin/ops block) so it trails the FI rows.
        "pricinggroups",
      ]);
    });

    it("Administration = exactly the admin/ops rows (Pricing Groups moved to Fixed Income)", () => {
      expect(railForDomain("admin").map((r) => r.id)).toEqual([
        "connections",
        "admin",
        "permissions",
        "refdata",
      ]);
    });

    it("every RAIL row belongs to at least one domain and only to its derived domains", () => {
      for (const r of RAIL) {
        for (const d of workspaceDomains(r.id)) {
          expect(railForDomain(d).some((x) => x.id === r.id)).toBe(true);
        }
      }
    });
  });

  describe("firstAccessibleWorkspace(auth, domain) — domain-aware landing", () => {
    it("prefers a row WITHIN the given domain", () => {
      const both = navAuth({
        isAdmin: false,
        allow: new Set(["view·fx_options", "view·fixed_income"]),
      });
      // FI tab lands on the first FI row in RAIL order — the Streaming hub, the
      // primary FI surface (Ticket is now FX-only, so it no longer leads FI).
      expect(firstAccessibleWorkspace(both, "fixed_income")).toBe("fistreaming");
      // FX tab leads with Ticket (unchanged).
      expect(firstAccessibleWorkspace(both, "fx_options")).toBe("ticket");
    });

    it("falls back to the GLOBAL first accessible when the domain has no reachable row", () => {
      // A non-admin: the Administration domain has no reachable row, so a request for
      // it falls back to the global first accessible (Ticket via a trading view).
      const trader = navAuth({
        isAdmin: false,
        allow: new Set(["view·fx_options", "view·fixed_income"]),
      });
      expect(firstAccessibleWorkspace(trader, "admin")).toBe("ticket");
    });

    it("without a domain arg behaves exactly as before (global first accessible)", () => {
      const fiOnly = navAuth({ isAdmin: false, allow: new Set(["view·fixed_income"]) });
      expect(firstAccessibleWorkspace(fiOnly)).toBe(firstAccessibleWorkspace(fiOnly, "fixed_income"));
    });
  });
});

describe("navigation gating — workspaceAccessible (slice 5c / #6 per-workspace-asset)", () => {
  /** A NavAuth whose `can` admits exactly the given set of `action·asset` keys. */
  function navAuth(opts: { isAdmin: boolean; allow?: ReadonlySet<string> }): NavAuth {
    return {
      isAdmin: opts.isAdmin,
      can: (action, asset) => opts.allow?.has(`${action}·${asset}`) ?? false,
    };
  }

  // The signed-out identity: `can` is permissive (returns true) and not admin.
  const signedOut: NavAuth = { isAdmin: false, can: () => true };

  describe("workspaceAccessible", () => {
    it("admin-only workspaces require isAdmin", () => {
      const admin = navAuth({ isAdmin: true });
      const trader = navAuth({
        isAdmin: false,
        allow: new Set(["view·fx_options", "view·fixed_income"]),
      });
      for (const id of ADMIN_ONLY_WORKSPACES) {
        expect(workspaceAccessible(id, admin)).toBe(true);
        expect(workspaceAccessible(id, trader)).toBe(false);
      }
    });

    it("a cross-asset (class-parametric) workspace is reachable via EITHER class", () => {
      const fxOnly = navAuth({ isAdmin: false, allow: new Set(["view·fx_options"]) });
      const fiOnly = navAuth({ isAdmin: false, allow: new Set(["view·fixed_income"]) });
      // Market Data / Risk / Book each serve BOTH classes — a view on either FX or
      // FI reaches them (the denied class is gated per-lens inside).
      for (const id of ["surface", "risk", "book"] as const) {
        expect(workspaceAccessible(id, fxOnly)).toBe(true);
        expect(workspaceAccessible(id, fiOnly)).toBe(true);
      }
    });

    it("single-asset workspaces follow their one class's view capability", () => {
      const fxOnly = navAuth({ isAdmin: false, allow: new Set(["view·fx_options"]) });
      const fiOnly = navAuth({ isAdmin: false, allow: new Set(["view·fixed_income"]) });
      // Ticket + Stream are FX-only; Quoting + Streaming are FI-only; Excel/XVA FX.
      expect(workspaceAccessible("ticket", fxOnly)).toBe(true);
      expect(workspaceAccessible("ticket", fiOnly)).toBe(false);
      expect(workspaceAccessible("stream", fxOnly)).toBe(true);
      expect(workspaceAccessible("stream", fiOnly)).toBe(false);
      expect(workspaceAccessible("quoting", fiOnly)).toBe(true);
      expect(workspaceAccessible("quoting", fxOnly)).toBe(false);
      expect(workspaceAccessible("fistreaming", fiOnly)).toBe(true);
      expect(workspaceAccessible("fistreaming", fxOnly)).toBe(false);
      expect(workspaceAccessible("excel", fiOnly)).toBe(false);
      expect(workspaceAccessible("excel", fxOnly)).toBe(true);
    });

    it("signed-out reaches every non-admin workspace", () => {
      for (const r of RAIL) {
        const expected = !ADMIN_ONLY_WORKSPACES.has(r.id);
        expect(workspaceAccessible(r.id, signedOut)).toBe(expected);
      }
    });

    // Per-feature rail visibility (docs/PERMISSIONS-GRANULAR-REVIEW.md §4): a row with
    // a viewCap is visible ONLY to a holder of that fine-grained capability.
    const RISK_ROWS = ["riskbooks", "riskdashboard", "riskrouting"] as const;
    const PRICING_ROWS = ["tiering", "pricinggroups"] as const;

    it("a risk_manage·FI seat SEES the risk management rows and NOT the pricing rows", () => {
      const riskMgr = navAuth({
        isAdmin: false,
        allow: new Set(["view·fixed_income", "risk_manage·fixed_income"]),
      });
      for (const id of RISK_ROWS) expect(workspaceAccessible(id, riskMgr)).toBe(true);
      for (const id of PRICING_ROWS) expect(workspaceAccessible(id, riskMgr)).toBe(false);
    });

    it("a manage_pricing·FI seat SEES the pricing rows and NOT the risk management rows", () => {
      const priceMgr = navAuth({
        isAdmin: false,
        allow: new Set(["view·fixed_income", "manage_pricing·fixed_income"]),
      });
      for (const id of PRICING_ROWS) expect(workspaceAccessible(id, priceMgr)).toBe(true);
      for (const id of RISK_ROWS) expect(workspaceAccessible(id, priceMgr)).toBe(false);
    });

    it("a plain FI trader (view only) sees NONE of the management surfaces", () => {
      const trader = navAuth({
        isAdmin: false,
        allow: new Set(["view·fx_options", "view·fixed_income"]),
      });
      for (const id of [...RISK_ROWS, ...PRICING_ROWS]) {
        expect(workspaceAccessible(id, trader)).toBe(false);
      }
      // …but still reaches the ordinary trading surfaces (Agg Book stays view·FI).
      expect(workspaceAccessible("aggbook", trader)).toBe(true);
      expect(workspaceAccessible("book", trader)).toBe(true);
    });

    it("an admin holding grant_all sees every viewCap-gated management surface", () => {
      const admin = navAuth({
        isAdmin: true,
        allow: new Set([
          "risk_manage·fixed_income",
          "manage_pricing·fixed_income",
          "manage_liquidity·fixed_income",
        ]),
      });
      for (const id of [...RISK_ROWS, ...PRICING_ROWS]) {
        expect(workspaceAccessible(id, admin)).toBe(true);
      }
    });
  });

  describe("firstAccessibleWorkspace", () => {
    it("returns the first RAIL workspace the identity can reach", () => {
      const fiOnly = navAuth({ isAdmin: false, allow: new Set(["view·fixed_income"]) });
      // Ticket + Stream now lead the rail but are FX-only, so a FI-only trader skips
      // them and lands on the first FI-serving row — the Streaming hub.
      expect(firstAccessibleWorkspace(fiOnly)).toBe("fistreaming");
      expect(RAIL[0]!.id).toBe("ticket");
    });

    it("an admin reaches the first RAIL entry (Ticket)", () => {
      const admin = navAuth({
        isAdmin: true,
        allow: new Set(["view·fx_options", "view·fixed_income"]),
      });
      expect(firstAccessibleWorkspace(admin)).toBe(RAIL[0]!.id);
    });

    it("an identity with no asset view and not admin reaches nothing (null)", () => {
      // No view on either asset, not admin: every trading workspace gates on a view
      // of a class it serves, admin panes on isAdmin, so nothing is reachable. The
      // AppContext redirect treats a null target as "leave the workspace as-is".
      const none = navAuth({ isAdmin: false });
      expect(firstAccessibleWorkspace(none)).toBeNull();
    });
  });
});
