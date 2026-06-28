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
  buildCommands,
  cheatsheet,
  COMMAND_META,
  DOMAINS,
  domainOf,
  RAIL,
  railChord,
  resolveChord,
  type CommandContext,
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

describe("domains — the top-tab partition over the rail (GW-tabs)", () => {
  it("declares the three product domains in tab order", () => {
    expect(DOMAINS.map((d) => d.id)).toEqual([
      "fx-options",
      "fixed-income",
      "administration",
    ]);
    for (const d of DOMAINS) expect(d.label.length).toBeGreaterThan(0);
  });

  it("every rail entry declares a valid domain", () => {
    const valid = new Set(DOMAINS.map((d) => d.id));
    for (const r of RAIL) expect(valid.has(r.domain)).toBe(true);
  });

  it("domainOf returns each rail entry's declared domain", () => {
    for (const r of RAIL) expect(domainOf(r.id)).toBe(r.domain);
  });

  it("the three domains partition the rail (every entry in exactly one, none empty)", () => {
    const counts = new Map<string, number>();
    for (const r of RAIL) counts.set(r.domain, (counts.get(r.domain) ?? 0) + 1);
    // Sum of per-domain counts == rail length (a partition: no entry double-counted).
    const total = [...counts.values()].reduce((a, b) => a + b, 0);
    expect(total).toBe(RAIL.length);
    // Each declared domain is non-empty (so every tab has at least one workspace).
    for (const d of DOMAINS) expect(counts.get(d.id) ?? 0).toBeGreaterThan(0);
  });

  it("places ratesrisk under Fixed Income with its glyph + label", () => {
    const entry = RAIL.find((r) => r.id === "ratesrisk");
    expect(entry).toBeDefined();
    expect(entry!.domain).toBe("fixed-income");
    expect(entry!.glyph.length).toBeGreaterThan(0);
    expect(entry!.label).toBe("Rates Risk");
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
    // The eleventh-and-beyond views (admin Excel today) have no ⌘N chord at all.
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
