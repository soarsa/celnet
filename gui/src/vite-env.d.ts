/// <reference types="vite/client" />

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
