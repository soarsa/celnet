/**
 * Detect a Playwright run that selected ONLY the offline `fidelity` project.
 *
 * The `fidelity` project (the mockup-parity visual-regression gate) runs entirely
 * against the in-app `?mock` transport — no `celnet-server`, no cargo, no demo edge.
 * `globalSetup` is GLOBAL (it runs once per `playwright test` invocation regardless
 * of which projects are selected), so it consults this predicate to SKIP the cargo
 * demo-edge boot when the run is fidelity-only. This keeps the front-end fidelity
 * gate runnable in a gui-only T2 window with no Rust toolchain present.
 *
 * A run is treated as fidelity-only when either:
 *   • `CELNET_FIDELITY=1` is set (the `npm run fidelity` scripts set it), or
 *   • the CLI selected `--project fidelity` (or `--project=fidelity`) and did NOT
 *     also select `--project chromium` (the live demo-edge project, which needs the
 *     edge). If BOTH are selected the edge still boots — chromium requires it.
 */
export function isFidelityOnlyRun(): boolean {
  if (process.env.CELNET_FIDELITY === "1") return true;
  const argv = process.argv.join(" ");
  const selectsFidelity = /--project[=\s]+fidelity\b/.test(argv);
  const selectsChromium = /--project[=\s]+chromium\b/.test(argv);
  return selectsFidelity && !selectsChromium;
}
