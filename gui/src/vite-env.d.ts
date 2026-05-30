/// <reference types="vite/client" />

// Build-time transport selection (src/data/transportConfig.ts). Both optional;
// unset ⇒ the deterministic in-app mock (the standalone default). One contract,
// two transports — chosen at build time, never mixed at runtime.
interface ImportMetaEnv {
  /** "ws" selects the live WebSocket transport; anything else keeps the mock. */
  readonly VITE_CELNET_TRANSPORT?: string;
  /** The `celnet-server` WS mirror endpoint, e.g. "ws://127.0.0.1:8081". */
  readonly VITE_CELNET_WS_URL?: string;
}

interface ImportMeta {
  readonly env: ImportMetaEnv;
}

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
