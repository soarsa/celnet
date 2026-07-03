/**
 * Shared helpers for the OFFLINE `fidelity` project (the mockup-parity visual-
 * fidelity gate — lane `gui-visual-regression-gate`).
 *
 * Everything here runs against the in-app `?mock` transport (src/data/mockSource.ts):
 * a fully offline, deterministic source that seeds the admin `admin@celnet.com` /
 * `password` and prices every number itself. NO `celnet-server`, NO cargo.
 *
 * DETERMINISM CONTRACT (the precondition for a visual DRIFT gate): the mock drives a
 * 10 Hz price tape (`setInterval`) plus chart RAF loops, so a naive screenshot is a
 * moving target. We install Playwright's fake clock at a FIXED instant BEFORE any
 * page script runs and only ever advance it by a FIXED budget — so the baseline run
 * and every later gate run land on the EXACT same tape/chart frame (a byte-identical
 * canvas). The residual delta a baseline tolerates (`maxDiffPixelRatio` in the config)
 * is then anti-aliasing only; a real layout/IA change shifts LARGE regions ≫ that.
 */
import { expect, type Locator, type Page } from "@playwright/test";

import { signIn } from "./helpers";

/** A fixed wall-clock instant the mock tape + any date display are frozen at. */
export const FROZEN_EPOCH = new Date("2026-07-03T12:00:00.000Z");

/** Fixed budget to advance the frozen clock so the tape settles to a live frame (ms). */
export const SETTLE_MS = 2_000;

/**
 * The mounted workspaces the per-workspace visual baseline covers, in rail order.
 * `view` is the real `?view=<id>` workspace id (savedViews codec); `name` is the
 * baseline/snapshot label. Signing in as the seeded admin mounts every one of these
 * (the four admin/ops panes require `isAdmin`). NOTE: the "Market Data" workspace's
 * id is `surface` — its baseline is named `marketdata` to match its rail label.
 */
export const FIDELITY_WORKSPACES = [
  { name: "ticket", view: "ticket" },
  { name: "stream", view: "stream" },
  { name: "marketdata", view: "surface" },
  { name: "risk", view: "risk" },
  { name: "book", view: "book" },
  { name: "quoting", view: "quoting" },
  { name: "xva", view: "xva" },
  { name: "connections", view: "connections" },
  { name: "admin", view: "admin" },
  { name: "permissions", view: "permissions" },
  { name: "refdata", view: "refdata" },
] as const;

/**
 * Install the frozen fake clock on `page`. MUST be called ONCE, before the first
 * navigation, so the app's timers (the mock tape, chart RAF, connection ticks) are
 * created against the fake clock. Idempotent-unsafe (Playwright rejects a double
 * install) — call exactly once per page.
 */
export async function installFrozenClock(page: Page): Promise<void> {
  await page.clock.install({ time: FROZEN_EPOCH });
}

/** Freeze CSS animations/transitions + hide the caret for a resting-state screenshot. */
export async function freezeAnimations(page: Page): Promise<void> {
  await page.addStyleTag({
    content:
      "*,*::before,*::after{animation:none!important;transition:none!important;animation-duration:0s!important;caret-color:transparent!important;}",
  });
}

/**
 * Open a workspace against the OFFLINE mock and return the ACTIVE pane locator, with
 * the tape settled to a deterministic frame and animations frozen — ready to shoot.
 * Requires {@link installFrozenClock} to have run on this page first.
 *
 * `?view=<id>` selects the workspace on mount (the savedViews URL codec); `?mock`
 * selects the offline transport (src/data/transportConfig.ts). We sign in as the
 * seeded admin so every workspace (including the admin/ops panes) is mounted.
 */
export async function gotoMockView(page: Page, viewId: string): Promise<Locator> {
  await page.goto(`/?view=${viewId}&mock`);
  // Fire any boot timers created during first paint, deterministically.
  await page.clock.runFor(200);
  // Server-enforced sessions: sign in through the mandatory login gate. The mock
  // resolves the login synchronously (a microtask, not a timer), so it completes
  // even with the clock frozen.
  await signIn(page);
  // The left workspaces rail confirms the Shell mounted.
  await expect(page.getByRole("complementary", { name: "workspaces" })).toBeVisible();
  // The active pane is the canvas child that is NOT inert (hidden panes carry it).
  const pane = page.locator('[aria-hidden="false"]:not([inert])').last();
  await expect(pane).toBeVisible();
  // Advance a FIXED budget so the tape settles to a representative live frame; the
  // clock then stays frozen (no further ticks) → the render is static for the shot.
  await page.clock.runFor(SETTLE_MS);
  await freezeAnimations(page);
  return pane;
}

// ---------------------------------------------------------------------------
// Structural IA fidelity checks (mockup ↔ live). The mockup encodes the TARGET
// information architecture; these predicates MEASURE whether the live shell carries
// its two shell-level primitives. Both are currently ABSENT from the live shell — the
// gate MEASURES that gap rather than asserting it away (lane: MEASURE + GATE only; the
// shell reshape is `gui-shell-ia-realization`, pending an operator decision).
// ---------------------------------------------------------------------------

/** The mockup's grouped-rail section labels (`.rail .grp`), in order. */
export const MOCKUP_RAIL_GROUPS = ["Price", "Market data", "Distribute", "Risk", "Operate"] as const;

/** The mockup's right context-rail section labels (`.ctx .grouphdr .label`), in order. */
export const MOCKUP_CTX_SECTIONS = ["Contribution", "Feed health", "Book risk", "Activity"] as const;

/** One structural primitive's presence + the supporting evidence. */
export interface IaCheck {
  /** Whether the primitive is present. */
  present: boolean;
  /** A short human-readable evidence string (counts / labels found). */
  evidence: string;
}

/** The two shell-level IA primitives, measured on one page (mockup or live). */
export interface ShellIa {
  groupedRail: IaCheck;
  rightContextRail: IaCheck;
}

/**
 * Measure the mockup's shell IA (the positive control / target). The mockup uses
 * literal `.rail .grp` group headers and a `aside.ctx` right rail, so both resolve
 * by their authored class names.
 */
export async function measureMockupIa(mock: Page): Promise<ShellIa> {
  const groups = mock.locator(".rail .grp");
  const groupCount = await groups.count();
  const ctx = mock.locator(".ctx");
  const ctxCount = await ctx.count();
  const ctxSections = await ctx.locator(".grouphdr .label").allInnerTexts();
  return {
    groupedRail: {
      present: groupCount >= 2,
      evidence: `${groupCount} '.rail .grp' group header(s): ${(await groups.allInnerTexts())
        .map((s) => s.trim())
        .join(", ")}`,
    },
    rightContextRail: {
      present: ctxCount >= 1,
      evidence: `${ctxCount} 'aside.ctx' rail with ${ctxSections.length} section(s): ${ctxSections
        .map((s) => s.trim())
        .join(", ")}`,
    },
  };
}

/**
 * Measure the LIVE shell's IA against the mockup contract. The live app uses hashed
 * CSS-module class names, so we probe by STRUCTURE/semantics, not the mockup's class
 * names:
 *   • grouped rail — grouping elements (ARIA groups / headings / any `grp`-classed
 *     divider) WITHIN the `aria-label="workspaces"` rail. The live rail is a FLAT
 *     list of icon buttons with no group headers ⇒ 0.
 *   • right context rail — a persistent complementary/`aside` landmark OTHER than the
 *     left workspaces rail, carrying any of the mockup's context section titles
 *     (Contribution / Feed Health / Book Risk / Activity). The live shell has none.
 */
export async function measureLiveIa(page: Page): Promise<ShellIa> {
  const rail = page.getByRole("complementary", { name: "workspaces" });
  // Grouping structure inside the rail: ARIA groups, headings, or any element whose
  // class encodes a group divider (mockup parlance `grp`). A flat button rail ⇒ 0.
  const railGroups = rail.locator(
    ':scope [role="group"], :scope h1, :scope h2, :scope h3, :scope h4, :scope [class*="grp" i], :scope [class*="group" i]',
  );
  const railGroupCount = await railGroups.count();

  // A persistent right-context rail: any aside/complementary that is NOT the left
  // workspaces rail and surfaces a mockup context section.
  const asides = page.locator('aside, [role="complementary"]');
  const asideCount = await asides.count();
  let ctxLikeCount = 0;
  const ctxTitlesFound: string[] = [];
  for (let i = 0; i < asideCount; i++) {
    const el = asides.nth(i);
    const label = (await el.getAttribute("aria-label")) ?? "";
    if (label.toLowerCase() === "workspaces") continue; // the left rail — not context
    const text = (await el.innerText().catch(() => "")) ?? "";
    const hits = MOCKUP_CTX_SECTIONS.filter((s) =>
      text.toLowerCase().includes(s.toLowerCase()),
    );
    if (hits.length > 0) {
      ctxLikeCount++;
      ctxTitlesFound.push(...hits);
    }
  }

  return {
    groupedRail: {
      present: railGroupCount >= 2,
      evidence: `${railGroupCount} grouping element(s) inside the workspaces rail (flat icon rail ⇒ 0; mockup has ${MOCKUP_RAIL_GROUPS.length}: ${MOCKUP_RAIL_GROUPS.join(", ")})`,
    },
    rightContextRail: {
      present: ctxLikeCount >= 1,
      evidence:
        ctxLikeCount >= 1
          ? `${ctxLikeCount} context aside(s) carrying: ${ctxTitlesFound.join(", ")}`
          : `no persistent right context rail (mockup has aside.ctx: ${MOCKUP_CTX_SECTIONS.join(", ")})`,
    },
  };
}
