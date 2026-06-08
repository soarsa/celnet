/**
 * Shared e2e helpers: navigate to the app pinned to the LIVE demo-edge WS mirror,
 * switch workspaces via the rail, and run an axe accessibility scan asserting zero
 * serious/critical violations on a view.
 */
import { expect, type Locator, type Page } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";

import { readWsUrl } from "./wsUrl";

/** Workspace ids ↔ their rail button labels (Shell.tsx RAIL). */
const RAIL_LABEL: Record<string, string> = {
  ticket: "Ticket",
  stream: "Stream",
  surface: "Surface",
  risk: "Risk",
  book: "Book",
};

/**
 * Open the app against the live edge. `?ws=<url>` pins the live WebSocket transport
 * to the booted edge (NOT the mock) — the resolved transport is the production WS
 * client, so every rendered number is server-computed.
 */
export async function openLive(page: Page): Promise<void> {
  const ws = encodeURIComponent(readWsUrl());
  await page.goto(`/?ws=${ws}`);
  // The shell renders synchronously; wait for the workspace rail to be live.
  await expect(page.getByRole("complementary", { name: "workspaces" })).toBeVisible();
  // Confirm we are on the LIVE transport: the ribbon's transport-seam label reads
  // "live ws://…" (the live WsTransport), NOT "mock/replay".
  const transportLabel = page.locator('[title="transport seam"]');
  await expect(transportLabel).toBeVisible();
  await expect(transportLabel).toContainText("ws://");
}

/**
 * Switch to a workspace via its rail button and return the ACTIVE pane locator.
 * Every workspace stays mounted (Shell toggles `display`), so all queries must be
 * scoped to the active pane — the only `.canvas` child without the `inert`
 * attribute — or a hidden pane's text would match. Waits for the pane to settle.
 */
export async function gotoWorkspace(
  page: Page,
  id: keyof typeof RAIL_LABEL,
): Promise<Locator> {
  const rail = page.getByRole("complementary", { name: "workspaces" });
  await rail.getByRole("button", { name: RAIL_LABEL[id], exact: false }).click();
  // The active pane is the canvas child that is NOT inert (hidden panes carry it).
  const pane = page.locator('[aria-hidden="false"]:not([inert])').last();
  await expect(pane).toBeVisible();
  return pane;
}

/**
 * Open the vol-cube heatmap: the cube is a SIBLING pivot inside the Surface
 * workspace (a "Surface | Cube" view toggle), not a top-level rail view. Go to
 * Surface, flip to the Cube chip, and wait for a real cube table to render. The
 * cube self-loads its own per-pair surfaces, so it renders regardless of the
 * marking-surface state.
 */
export async function gotoCube(page: Page): Promise<Locator> {
  const pane = await gotoWorkspace(page, "surface");
  // The "Cube" view chip lives in the surface view-toggle group.
  await pane.getByRole("button", { name: "Cube", exact: true }).click();
  // Wait for the actual heat grid (the smile pivot's tenor×delta table).
  const cube = page.getByRole("table", { name: "tenor by delta vol cube" });
  await expect(cube).toBeVisible();
  return pane;
}

/**
 * Open the scope switcher's pair-universe LEAF view (the virtualised, searchable
 * listbox) — GW1 absorbed the standalone "Pairs" overlay into the ONE breadcrumb
 * scope control. From the firm root the scope drills firm → desk → book → pair;
 * to reach the pair leaf directly we click the active pair crumb (when present) or
 * the drill button, then drill down to the pair listbox. The simplest stable path
 * to the pair-universe listbox is the ⌘P scope/underlier switcher, then drilling
 * down to the terminal pair level. This is the data-dense listbox where option
 * a11y issues hide.
 */
export async function openUniverseNavigator(page: Page): Promise<Locator> {
  // Drill down to the pair (terminal) level: firm → desk → book → pair. Each drill
  // step opens the switcher and selects the first option, descending one level.
  for (const level of ["desk", "book"] as const) {
    await page.getByRole("button", { name: `drill into a ${level}` }).click();
    const dialog = page.getByRole("dialog");
    await expect(dialog).toBeVisible();
    await dialog.getByRole("option").first().click();
  }
  // Now the tail is a book → the drill button enters the pair leaf (the universe).
  await page.getByRole("button", { name: "drill into a pair" }).click();
  const dialog = page.getByRole("dialog", { name: "Pairs" });
  await expect(dialog).toBeVisible();
  await expect(dialog.getByRole("listbox", { name: "currency pairs" })).toBeVisible();
  return dialog;
}

/**
 * Run axe on the current page and assert ZERO serious/critical violations. We
 * scan the WCAG 2.1 A/AA tags. Moderate/minor findings are reported (logged) but
 * do not fail the gate — the bar the task sets is serious/critical = 0.
 */
export async function expectNoSeriousA11y(page: Page, context: string): Promise<void> {
  // Freeze CSS animations/transitions before scanning so axe samples the RESTING
  // UI — the persistent, accessible state — rather than a sub-500ms price-flash
  // wash transient (a moving target axe would sample non-deterministically). This
  // is the canonical axe-on-an-animated-app practice; the resting colours are the
  // real contract. The override is scoped to this scan (a style tag on the page).
  await page.addStyleTag({
    content:
      "*,*::before,*::after{animation:none!important;transition:none!important;animation-duration:0s!important;}",
  });
  const results = await new AxeBuilder({ page })
    .withTags(["wcag2a", "wcag2aa", "wcag21a", "wcag21aa"])
    .analyze();
  const serious = results.violations.filter(
    (v) => v.impact === "serious" || v.impact === "critical",
  );
  if (serious.length > 0) {
    const summary = serious
      .map((v) => {
        const nodes = v.nodes
          .map((n) => `      ${n.target.join(" ")}\n        ${n.failureSummary ?? ""}`)
          .join("\n");
        return `  [${v.impact}] ${v.id}: ${v.help} (${v.nodes.length} node(s))\n${nodes}`;
      })
      .join("\n");
    throw new Error(`axe found ${serious.length} serious/critical violation(s) on ${context}:\n${summary}`);
  }
  process.stdout.write(
    `[a11y] ${context}: 0 serious/critical (${results.violations.length} moderate/minor noted)\n`,
  );
}
