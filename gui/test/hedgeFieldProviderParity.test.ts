/**
 * The DRIFT GUARD between the Rust hedge-field provider declarations and their client mirror.
 *
 * `HedgeGraph::validate` refuses a condition on a field nothing populates
 * (`HedgeError::UnprovidedField`). `gui/src/lib/hedgeFields.ts` mirrors that declaration so
 * `validateHedgeGraph` can refuse the SAME graphs, with the SAME reason text, before a trader
 * hits save. Two copies of one fact is a drift hazard — so this test does not restate the
 * expected values: it PARSES `crates/celnet-hedge-routing/src/field.rs` and compares the
 * mirror against whatever the Rust actually says today.
 *
 * Consequently, changing a provider declaration (or adding a `HedgeField`) on the Rust side
 * turns this test red until the TS registry is updated to match — which is the entire point.
 * Editing this file to accommodate a mismatch defeats it; fix the mirror instead.
 */
import { existsSync, readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";

import { describe, expect, it } from "vitest";

import { HEDGE_FIELD_REGISTRY, hedgeFieldUnprovidedReason } from "../src/lib/hedgeFields";
import { at } from "./support";

const RELATIVE_FIELD_RS = "crates/celnet-hedge-routing/src/field.rs";

/**
 * Locate the Rust source by walking up from the working directory to the repo root, rather
 * than assuming a fixed depth (`import.meta.url` is not a `file:` URL under the vitest
 * transform, and the runner's cwd depends on how it was invoked). Throws rather than skips
 * when it cannot be found: a drift guard that silently no-ops is worse than none.
 */
function locateFieldRs(): string {
  let dir = process.cwd();
  for (;;) {
    const candidate = resolve(dir, RELATIVE_FIELD_RS);
    if (existsSync(candidate)) return candidate;
    const parent = dirname(dir);
    if (parent === dir) {
      throw new Error(
        `hedge field provider parity: could not find ${RELATIVE_FIELD_RS} above ${process.cwd()}`,
      );
    }
    dir = parent;
  }
}

const FIELD_RS = locateFieldRs();

/** `CounterpartyToxicity` → `counterparty_toxicity` (the wire / TS selector spelling). */
function toSnakeCase(variant: string): string {
  return variant.replace(/(?<!^)([A-Z])/g, "_$1").toLowerCase();
}

/**
 * Resolve a Rust string literal's ACTUAL value. The declarations wrap across lines with a
 * trailing `\`, which elides the newline *and* the next line's leading whitespace — so a
 * naive read would compare against text containing indentation that the compiled constant
 * does not have.
 */
function unescapeRustString(raw: string): string {
  return raw
    .replace(/\\\r?\n[ \t]*/g, "")
    .replace(/\\"/g, '"')
    .replace(/\\\\/g, "\\");
}

interface RustProvider {
  state: "computed" | "unprovided";
  text: string;
}

/** Every `HedgeField::X => FieldProvider::…` arm of `HedgeField::provider`, as declared. */
function parseRustProviders(source: string): Map<string, RustProvider> {
  const arm =
    /HedgeField::(\w+)\s*=>\s*FieldProvider::(Computed|Unprovided)\s*\{\s*(basis|reason):\s*"((?:[^"\\]|\\[\s\S])*)"/g;
  const out = new Map<string, RustProvider>();
  for (const m of source.matchAll(arm)) {
    // The regex has four mandatory capture groups, but a RegExpMatchArray types every
    // element as possibly-undefined; assert rather than destructure so a parse that
    // silently stops matching fails here with a precise message.
    const variant = at(m, 1);
    const ctor = at(m, 2);
    const key = at(m, 3);
    const literal = at(m, 4);
    // A `Computed` arm must carry `basis` and an `Unprovided` arm `reason`; anything else
    // means the parse has latched onto something that is not a provider declaration.
    const expectedKey = ctor === "Computed" ? "basis" : "reason";
    expect(key, `${variant}: ${ctor} arm should use \`${expectedKey}\``).toBe(expectedKey);
    out.set(toSnakeCase(variant), {
      state: ctor === "Computed" ? "computed" : "unprovided",
      text: unescapeRustString(literal),
    });
  }
  return out;
}

/** The `HedgeField::ALL` roster — the authoritative field set. */
function parseRustAll(source: string): string[] {
  const block = /pub const ALL:\s*\[HedgeField;\s*\d+\]\s*=\s*\[([\s\S]*?)\];/.exec(source);
  expect(block, "HedgeField::ALL should be parseable from field.rs").not.toBeNull();
  return [...at(block as RegExpExecArray, 1).matchAll(/HedgeField::(\w+)/g)].map((m) =>
    toSnakeCase(at(m, 1)),
  );
}

const source = readFileSync(FIELD_RS, "utf8");
const rustProviders = parseRustProviders(source);
const rustAll = parseRustAll(source);

describe("hedge field provider parity — Rust source vs the client mirror", () => {
  it("parses a non-trivial number of declarations out of field.rs", () => {
    // Guards the guard: a regex that silently stopped matching would make every comparison
    // below vacuous, and the test would pass while the mirror rotted.
    expect(rustProviders.size).toBeGreaterThanOrEqual(19);
    expect(rustAll.length).toBe(rustProviders.size);
    expect([...rustProviders.values()].some((p) => p.state === "unprovided")).toBe(true);
    expect([...rustProviders.values()].some((p) => p.state === "computed")).toBe(true);
  });

  it("covers exactly the same field set as HedgeField::ALL", () => {
    expect([...HEDGE_FIELD_REGISTRY.map((s) => s.field)].sort()).toEqual([...rustAll].sort());
  });

  it("declares the same provider state for every field", () => {
    const mirror = Object.fromEntries(
      HEDGE_FIELD_REGISTRY.map((s) => [s.field, s.provider.state]),
    );
    const rust = Object.fromEntries([...rustProviders].map(([f, p]) => [f, p.state]));
    expect(mirror).toEqual(rust);
  });

  it("carries the Rust reason/basis text VERBATIM, so the trader reads the server's words", () => {
    const mirror = Object.fromEntries(
      HEDGE_FIELD_REGISTRY.map((s) => [
        s.field,
        s.provider.state === "computed" ? s.provider.basis : s.provider.reason,
      ]),
    );
    const rust = Object.fromEntries([...rustProviders].map(([f, p]) => [f, p.text]));
    expect(mirror).toEqual(rust);
  });

  it("agrees with the Rust on which fields are dead, via the helper the validator uses", () => {
    for (const [field, p] of rustProviders) {
      const reason = hedgeFieldUnprovidedReason(field as never);
      if (p.state === "unprovided") {
        expect(reason, `${field} should be unprovided`).toBe(p.text);
      } else {
        expect(reason, `${field} should be computed`).toBeNull();
      }
    }
  });
});
