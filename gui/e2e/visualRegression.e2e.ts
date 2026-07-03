/**
 * Per-workspace VISUAL-REGRESSION baseline — the layout/IA DRIFT gate (lane
 * `gui-visual-regression-gate`). Before this spec nothing in the GUI caught layout
 * or information-architecture drift (no `toHaveScreenshot` / Chromatic / Percy
 * anywhere); a workspace could silently lose a panel, a column, or a whole rail and
 * every green vitest still passed. This pins a committed pixel baseline for each
 * mounted workspace and FAILS the gate when the rendered layout drifts from it.
 *
 * OFFLINE by construction — it runs under the `fidelity` Playwright project against
 * the in-app `?mock` transport (NO server, NO cargo; the demo-edge boot is skipped
 * for a fidelity-only run). It signs in as the seeded admin so every workspace —
 * including the four admin/ops panes — mounts, then for each workspace navigates
 * `?view=<id>&mock`, settles the deterministic (clock-frozen) render, and asserts the
 * screenshot against `<name>.png`.
 *
 * Baselines: `e2e/visualRegression.e2e.ts-snapshots/` (committed — they ARE the gate
 * reference). Regenerate after an intentional UI change with:
 *   npm --prefix gui run fidelity:update
 */
import { expect, test } from "@playwright/test";

import { FIDELITY_WORKSPACES, gotoMockView, installFrozenClock } from "./fidelityHelpers";

test.describe("workspace visual-regression baseline (offline ?mock)", () => {
  for (const ws of FIDELITY_WORKSPACES) {
    test(`workspace ${ws.name} matches its committed baseline`, async ({ page }) => {
      // Freeze time BEFORE the first navigation so the mock tape + chart frames are
      // deterministic; then open the workspace and settle to a fixed frame.
      await installFrozenClock(page);
      await gotoMockView(page, ws.view);

      // The whole viewport (shell chrome + the active workspace) is the drift target:
      // a lost panel, a broken rail, or a shifted column all move pixels here.
      await expect(page).toHaveScreenshot(`${ws.name}.png`, {
        fullPage: false,
        // A layout/IA change shifts large regions; this ratio absorbs only AA noise.
        maxDiffPixelRatio: 0.01,
      });
    });
  }
});
