/**
 * Mockup-vs-live STRUCTURAL FIDELITY report — MEASURES the parity gap between the
 * redesign mockups (`docs/gui-redesign/mockups/*.html`, the TARGET information
 * architecture) and the live `?mock` app, for the shell + a few key views.
 *
 * This is the MEASURE half of the `gui-visual-regression-gate` lane: it does not
 * reshape the shell (that is `gui-shell-ia-realization`, pending an operator
 * decision) — it turns "the live shell is missing the mockup's IA" from an assertion
 * into DATA. For each view pair it captures BOTH renders (a side-by-side artifact)
 * and checks whether the live view carries the mockup's two shell-level primitives:
 *   (a) a GROUPED labelled capability rail (mockup `.rail .grp`: Price / Market data /
 *       Distribute / Risk / Operate) — live is a FLAT icon rail; and
 *   (b) a persistent RIGHT CONTEXT rail (mockup `aside.ctx`: Contribution / Feed
 *       health / Book risk / Activity) — live has none.
 * Both are currently ABSENT from the live shell; the report records exactly which
 * views match the mockup IA and, for those that don't, the specific missing element.
 *
 * Runs under the OFFLINE `fidelity` project (NO server, NO cargo). Output artifact:
 * `test-results/fidelity/` (gitignored) — `report.html` (side-by-side + findings),
 * `report.json` (machine-readable), and the per-view `*-mockup.png` / `*-live.png`.
 * The run stays GREEN: the measured live gap is data, not a test failure; the hard
 * assertions are the mockup POSITIVE CONTROL + that the report artifact was written.
 */
import { existsSync, mkdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

import { expect, test, type Page } from "@playwright/test";

import {
  gotoMockView,
  installFrozenClock,
  measureLiveIa,
  measureMockupIa,
  MOCKUP_CTX_SECTIONS,
  MOCKUP_RAIL_GROUPS,
  type ShellIa,
} from "./fidelityHelpers";

/** Where the report + side-by-side screenshots are written (gitignored). */
const REPORT_DIR = fileURLToPath(new URL("../test-results/fidelity/", import.meta.url));

/** The committed redesign mockups (the TARGET IA), served over `file://`. */
const MOCKUP_DIR = new URL("../../docs/gui-redesign/mockups/", import.meta.url);

interface ViewPair {
  /** Stable key (screenshot/report id). */
  key: string;
  /** Human label. */
  label: string;
  /** The mockup html file (the target). */
  mockup: string;
  /** The live `?view=<id>` workspace to compare against. */
  view: string;
}

/** The shell + key views the task calls out (00-shell↔Shell, 06↔risk, 15↔ticket, 07↔xva). */
const VIEW_PAIRS: readonly ViewPair[] = [
  { key: "shell", label: "Shell · Price & Model", mockup: "00-shell.html", view: "ticket" },
  { key: "risk", label: "Risk & Scenario", mockup: "06-risk-scenario.html", view: "risk" },
  { key: "ticket", label: "Rates Ticket", mockup: "15-rates-ticket.html", view: "ticket" },
  { key: "xva", label: "XVA", mockup: "07-xva.html", view: "xva" },
] as const;

interface Finding extends ViewPair {
  mockShot: string;
  liveShot: string;
  mockIa: ShellIa;
  liveIa: ShellIa;
  /** Live carries BOTH shell-level primitives ⇒ matches the mockup IA. */
  matchesMockupIa: boolean;
  /** The specific mockup primitives ABSENT from the live view. */
  missing: string[];
}

test("mockup-vs-live IA fidelity report (offline ?mock)", async ({ page, context }) => {
  test.setTimeout(120_000);
  mkdirSync(REPORT_DIR, { recursive: true });

  // The live app runs on the frozen clock (deterministic tape); the mockup page is a
  // static file, so it keeps real timers (its own page — the clock install is per-page).
  await installFrozenClock(page);
  const mock: Page = await context.newPage();

  const findings: Finding[] = [];
  for (const pair of VIEW_PAIRS) {
    // --- TARGET: the mockup (file://) -------------------------------------
    const mockUrl = new URL(pair.mockup, MOCKUP_DIR).href;
    await mock.goto(mockUrl, { waitUntil: "load" });
    await mock.waitForTimeout(300); // settle static layout (no fake clock on this page)
    const mockShot = `${pair.key}-mockup.png`;
    await mock.screenshot({ path: join(REPORT_DIR, mockShot), fullPage: false });
    const mockIa = await measureMockupIa(mock);

    // --- LIVE: the `?mock` app --------------------------------------------
    await gotoMockView(page, pair.view);
    const liveShot = `${pair.key}-live.png`;
    await page.screenshot({ path: join(REPORT_DIR, liveShot), fullPage: false });
    const liveIa = await measureLiveIa(page);

    const missing: string[] = [];
    if (!liveIa.groupedRail.present) missing.push("grouped labelled rail (.rail .grp)");
    if (!liveIa.rightContextRail.present) missing.push("right context rail (aside.ctx)");

    findings.push({
      ...pair,
      mockShot,
      liveShot,
      mockIa,
      liveIa,
      matchesMockupIa: liveIa.groupedRail.present && liveIa.rightContextRail.present,
      missing,
    });
  }
  await mock.close();

  // --- write the machine-readable + human artifacts -------------------------
  const reportJson = join(REPORT_DIR, "report.json");
  const reportHtml = join(REPORT_DIR, "report.html");
  writeFileSync(
    reportJson,
    JSON.stringify(
      {
        generatedAt: new Date().toISOString(),
        contract: {
          groupedRail: { selector: ".rail .grp", groups: MOCKUP_RAIL_GROUPS },
          rightContextRail: { selector: "aside.ctx", sections: MOCKUP_CTX_SECTIONS },
        },
        findings,
      },
      null,
      2,
    ),
    "utf8",
  );
  writeFileSync(reportHtml, renderHtml(findings), "utf8");

  // --- concise console summary ----------------------------------------------
  const line = "─".repeat(72);
  let out = `\n${line}\n[fidelity] mockup-vs-live IA report → ${reportHtml}\n${line}\n`;
  for (const f of findings) {
    const verdict = f.matchesMockupIa ? "MATCH" : "GAP";
    out += `  ${f.key.padEnd(8)} ${verdict.padEnd(6)} live=${f.view}  vs  ${f.mockup}\n`;
    out += `      grouped-rail : mockup=${yn(f.mockIa.groupedRail.present)} live=${yn(
      f.liveIa.groupedRail.present,
    )}  — ${f.liveIa.groupedRail.evidence}\n`;
    out += `      right-context: mockup=${yn(f.mockIa.rightContextRail.present)} live=${yn(
      f.liveIa.rightContextRail.present,
    )}  — ${f.liveIa.rightContextRail.evidence}\n`;
    if (f.missing.length > 0) out += `      MISSING live : ${f.missing.join("; ")}\n`;
  }
  const gaps = findings.filter((f) => !f.matchesMockupIa).length;
  out += `${line}\n  ${gaps}/${findings.length} view(s) do NOT yet carry the mockup IA (grouped rail + right context rail).\n${line}\n`;
  process.stdout.write(out);

  // --- hard gates: the checker's positive control + the artifact ------------
  // The mockup MUST carry both primitives on every pair (validates the checker and
  // pins the target); if a mockup regressed, this fails.
  for (const f of findings) {
    expect(f.mockIa.groupedRail.present, `${f.mockup} grouped rail`).toBe(true);
    expect(f.mockIa.rightContextRail.present, `${f.mockup} context rail`).toBe(true);
  }
  // The report artifacts must exist (the deliverable of this spec).
  expect(existsSync(reportJson)).toBe(true);
  expect(existsSync(reportHtml)).toBe(true);
});

function yn(b: boolean): string {
  return b ? "yes" : "NO ";
}

/** Render a self-contained side-by-side HTML report (embeds the captured PNGs). */
function renderHtml(findings: readonly Finding[]): string {
  const rows = findings
    .map((f) => {
      const chip = (label: string, mock: boolean, live: boolean): string =>
        `<tr><td>${label}</td><td class="${mock ? "ok" : "no"}">${mock ? "present" : "absent"}</td>` +
        `<td class="${live ? "ok" : "no"}">${live ? "present" : "absent"}</td></tr>`;
      return `
      <section class="pair">
        <h2>${f.label} <span class="${f.matchesMockupIa ? "ok" : "no"}">${
          f.matchesMockupIa ? "MATCHES mockup IA" : "GAP vs mockup IA"
        }</span></h2>
        <table class="ia">
          <thead><tr><th>shell primitive</th><th>mockup (target)</th><th>live (?mock)</th></tr></thead>
          <tbody>
            ${chip("grouped labelled rail (.rail .grp)", f.mockIa.groupedRail.present, f.liveIa.groupedRail.present)}
            ${chip("right context rail (aside.ctx)", f.mockIa.rightContextRail.present, f.liveIa.rightContextRail.present)}
          </tbody>
        </table>
        <p class="ev"><b>live grouped-rail:</b> ${f.liveIa.groupedRail.evidence}</p>
        <p class="ev"><b>live right-context:</b> ${f.liveIa.rightContextRail.evidence}</p>
        ${f.missing.length > 0 ? `<p class="miss">Missing in live: ${f.missing.join("; ")}</p>` : ""}
        <div class="sbs">
          <figure><figcaption>mockup — ${f.mockup}</figcaption><img src="${f.mockShot}" alt="mockup ${f.key}"/></figure>
          <figure><figcaption>live — ?view=${f.view}&amp;mock</figcaption><img src="${f.liveShot}" alt="live ${f.key}"/></figure>
        </div>
      </section>`;
    })
    .join("\n");
  return `<!doctype html>
<html lang="en"><head><meta charset="utf-8"/>
<title>Celnet — mockup-vs-live IA fidelity</title>
<style>
  body{font:14px/1.5 system-ui,sans-serif;margin:0;padding:24px;background:#0f1115;color:#e6e8ee}
  h1{font-size:20px} h2{font-size:16px;margin:0 0 8px;display:flex;gap:10px;align-items:center}
  .ok{color:#3ecf8e} .no{color:#ff6b6b} span.ok,span.no{font-size:12px;border:1px solid;border-radius:6px;padding:1px 8px}
  section.pair{border:1px solid #262a33;border-radius:10px;padding:16px;margin:16px 0;background:#161922}
  table.ia{border-collapse:collapse;margin:6px 0 10px} table.ia th,table.ia td{border:1px solid #262a33;padding:4px 12px;text-align:left}
  .ev{color:#9aa2b1;font-size:12px;margin:2px 0} .miss{color:#ff8f8f;font-size:12px;font-weight:600}
  .sbs{display:grid;grid-template-columns:1fr 1fr;gap:14px;margin-top:12px}
  figure{margin:0} figcaption{font-size:11px;color:#9aa2b1;margin-bottom:4px}
  img{width:100%;height:auto;border:1px solid #2c313c;border-radius:6px;background:#0b0d11}
  .lead{color:#9aa2b1;max-width:70ch}
</style></head>
<body>
  <h1>Celnet — mockup-vs-live information-architecture fidelity</h1>
  <p class="lead">Structural parity of the live <code>?mock</code> app against the redesign mockups
  (<code>docs/gui-redesign/mockups</code>). Two shell-level primitives are measured per view: a grouped
  labelled capability rail (mockup <code>.rail .grp</code>) and a persistent right context rail
  (mockup <code>aside.ctx</code>). This report MEASURES the gap; reshaping the shell is a separate lane.</p>
  ${rows}
</body></html>`;
}
