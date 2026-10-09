/// <reference types="vite/client" />

// Transport selection (src/data/transportConfig.ts). Both optional. The DEFAULT
// is the LIVE WebSocket mirror against celnet-server; the in-app mock is an
// explicit offline opt-in (`?mock` URL flag or VITE_CELNET_TRANSPORT=mock). One
// contract, two transports — never mixed at runtime.
interface ImportMetaEnv {
  /** "mock" forces the offline in-app source; otherwise the live WS transport. */
  readonly VITE_CELNET_TRANSPORT?: string;
  /** Override the live `celnet-server` WS endpoint, e.g. "ws://127.0.0.1:8081". */
  readonly VITE_CELNET_WS_URL?: string;
}

interface ImportMeta {
  readonly env: ImportMetaEnv;
}

// Build-time stamp constants injected by Vite `define` (vite.config.ts). Real
// values: the git short SHA (or package version off a git tree) and the UTC
// build time. Surfaced in the status ribbon (Login-footer signature).
declare const __CELNET_BUILD_HASH__: string;
declare const __CELNET_BUILD_TIME__: string;

// Typed CSS Modules: every `*.module.css` import is a class-name map. Keeps the
// design-system styling type-safe under `strict` without a heavy build plugin.
declare module "*.module.css" {
  // Intentionally a loose record whose lookups are always `string`. Unknown class
  // names returning a value (rather than `undefined` under noUncheckedIndexedAccess)
  // keeps `className={styles.x}` typed as a plain string for strict consumers.
  interface CssModuleClasses {
    readonly [key: string]: string;
  }
  const classes: CssModuleClasses;
  export default classes;
}
