#!/usr/bin/env node
/**
 * Automated Sub-App Entrypoint Generator for DeskModal
 *
 * Reads dist/index.html and generates individual micro-app entrypoints for
 * each of CelNet's 6 institutional trading desks:
 *   1. dist/pricing/index.html    -> studio_pricing
 *   2. dist/markets/index.html    -> studio_markets
 *   3. dist/rfq/index.html        -> studio_distribution
 *   4. dist/risk/index.html       -> studio_risk
 *   5. dist/blotter/index.html    -> studio_blotter
 *   6. dist/analytics/index.html -> studio_policy
 *
 * Syncs the latest hashed JavaScript bundle, CSS, and version metadata
 * directly to the DeskModal plugin deployed tree.
 */

import fs from "fs";
import path from "path";
import { fileURLToPath } from "url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const GUI_ROOT = path.resolve(__dirname, "..");
const DIST_DIR = path.resolve(GUI_ROOT, "dist");
const DESKMODAL_APP_DIR = "/Users/adrian/deskmodal/plugins/celnet-studio/app";

const DESKS = [
  {
    path: "pricing",
    view: "studio_pricing",
    title: "CelNet Options Pricing Studio",
  },
  {
    path: "markets",
    view: "studio_markets",
    title: "CelNet Markets & Depth Studio",
  },
  {
    path: "rfq",
    view: "studio_distribution",
    title: "CelNet RFQ & Negotiation Studio",
  },
  {
    path: "risk",
    view: "studio_risk",
    title: "CelNet Risk Cube Studio",
  },
  {
    path: "blotter",
    view: "studio_blotter",
    title: "CelNet Blotters & Position Ledger",
  },
  {
    path: "analytics",
    view: "studio_policy",
    title: "CelNet Curve & Swaption Analytics",
  },
];

function generate() {
  const indexPath = path.join(DIST_DIR, "index.html");
  if (!fs.existsSync(indexPath)) {
    console.error(`Error: ${indexPath} does not exist. Run build first.`);
    process.exit(1);
  }

  const baseHtml = fs.readFileSync(indexPath, "utf-8");

  for (const desk of DESKS) {
    const targetDir = path.join(DIST_DIR, desk.path);
    fs.mkdirSync(targetDir, { recursive: true });

    // Patch relative asset paths from ./assets to ../assets
    let patchedHtml = baseHtml.replace(/(src|href)="\.\/assets\//g, '$1="../assets/');

    // Patch title
    patchedHtml = patchedHtml.replace(
      /<title>.*?<\/title>/,
      `<title>${desk.title}</title>`
    );

    // Inject bootstrap script configuring default view and micro-app mode
    const bootstrapScript = `
    <script>
      window.__CELNET_DEFAULT_VIEW__ = "${desk.view}";
      window.__CELNET_APP_MODE__ = true;
    </script>`;

    patchedHtml = patchedHtml.replace("</head>", `${bootstrapScript}\n  </head>`);

    const targetFile = path.join(targetDir, "index.html");
    fs.writeFileSync(targetFile, patchedHtml, "utf-8");
    console.log(`Generated: dist/${desk.path}/index.html (${desk.view})`);
  }

  // Deploy to DeskModal plugin directory if it exists
  if (fs.existsSync(DESKMODAL_APP_DIR)) {
    console.log(`Deploying generated assets to ${DESKMODAL_APP_DIR}...`);
    cpR(DIST_DIR, DESKMODAL_APP_DIR);
    console.log("Deployment complete.");
  }
}

function cpR(src, dest) {
  if (!fs.existsSync(dest)) {
    fs.mkdirSync(dest, { recursive: true });
  }
  const entries = fs.readdirSync(src, { withFileTypes: true });
  for (const entry of entries) {
    const srcPath = path.join(src, entry.name);
    const destPath = path.join(dest, entry.name);
    if (entry.isDirectory()) {
      cpR(srcPath, destPath);
    } else {
      fs.copyFileSync(srcPath, destPath);
    }
  }
}

generate();
