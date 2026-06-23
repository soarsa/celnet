# tools/visual — host-side VISUAL drivers (vendored from lodestar)

Reference drivers for the **VISUAL / design-knowledge seam** of the lodestar
verified-knowledge layer. Vendored verbatim from
`github.com/soarsa/lodestar/tools/visual/` so this repo's GUI design system can be
held to the same deterministic, model-free design-adherence gate the engine ships.

Everything here is **additive and DEFAULT-OFF**. No gate, justfile recipe, GUI build,
or CI step depends on it today; the GUI build stays green with these files absent.
The verified VISUAL knowledge for this repo is currently authored directly through the
lodestar MCP (`knowledge_put`, `put_derive_rule`) — see "What is already live" below —
and these scripts are the optional host-side automation around that same seam.

## Files

| File | What it is |
| --- | --- |
| `storybook-ingest.sh` | SYMBOLIC, DETERMINISTIC, NO-MODEL ingest of a built Storybook `index.json` into component-anchored DRAFT design claims (`design:token` / `ui:component:<tag>` / `a11y:*`). Resolves each story's component to a real graph qualified-name via `knowledge_put`; an unresolved anchor is **deferred, never guessed**. Absent manifest ⇒ clean no-op (exit 3). |
| `visual-verify.sh` | Host-side reference driver for the `ui:matches-mockup` VISUAL-VERIFIER seam + rendered-DOM `a11y:rendered` facts. Two stages: (1) deterministic pixel diff over isolated Storybook stories (Playwright + pixelmatch/ImageMagick), (2) **only on a Stage-1 delta** a comparative vision-model judgment with conformal abstention — never gate-grade, marked "measured-with-caveats". Plus an axe-core 4.12.x rendered-a11y pass over the built static Storybook. DEFAULT-OFF; missing tools ⇒ honest UNSUPPORTED, never a fabricated PASS. |
| `lodestar.visual.toml.example` | Config template for both drivers. `provider = "off"` by default. Copy to `tools/lodestar.judge.toml` (the single config surface `common.sh` reads) or point `LODESTAR_VISUAL_CONFIG` at it. |

All third-party deps are permissively licensed and free (Storybook MIT, Playwright
Apache-2.0, axe-core MPL-2.0, ImageMagick Apache-2.0). **No commercial dependency** —
consistent with the repo guardrail. The engine itself stays pure-C, model-free, and
air-gapped; these are *host-side* reference drivers outside that boundary.

## Prerequisite (not yet vendored)

`storybook-ingest.sh` and `visual-verify.sh` source `tools/lib/common.sh` (line 41 /
the judge-tooling shared helper: config locate/read, `engine_bin`, `engine_project`,
`json_get`, `json_escape`, `log`/`warn`/`die`). That shared lib is **not yet present**
in this repo, so the scripts are not runnable standalone here — they are vendored as the
reference automation to wire up when the judge-tooling base lands. Until then, author
VISUAL claims directly via the lodestar MCP (the path used for "What is already live").

## What is already live (authored via the MCP, no script needed)

The design system is already token-driven and the VISUAL "why" layer is seeded with
**gate-passing** claims anchored to the real `Component` graph nodes:

- `a11y:role` / `a11y:labeled` — DataGrid (APG `role="grid"`), StatusBadge
  (`role="img"`), CommandPalette (dialog+listbox), SignInDialog (labelledby-to-title).
- `ui:component:<tag>` — DataGrid (`grid`), CommandPalette (`dialog`),
  StatusBadge (`badge`), GreeksStrip (`strip`).

Each was dry-run with `knowledge_check` first and only `knowledge_put` on a Stage-1
gate **pass** (the gate decides these from the indexed `a11y_role`/`a11y_labeled`
Component properties + the Component label).

## The design-token contract these drivers protect

`gui/design-tokens.json` (W3C **DTCG** format) is the single styling source of truth — a
verbatim formalization of `gui/src/design/tokens.css` (+ `appearance.ts`/`density.ts`/
`global.css`). Components consume it **only** through CSS-Module classes that reference
the emitted CSS custom properties (`var(--bg-base)`, `var(--accent)`, `var(--space-3)`,
`var(--r-md)`, `var(--row-h)`, …), never raw color/spacing literals. A repo-wide scan of
all 46 `gui/src` CSS Modules found **zero** raw hex/hsl color literals and exactly one
`rgba()` (a text-shadow in `CubeWorkspace.module.css`); the only inline TSX color
literals are token-read *fallbacks* (`getPropertyValue('--…') || '#…'` in
`Sparkline.tsx` / `SmileChart.tsx`) that read the token first. Editing a token value in
`tokens.css` therefore propagates everywhere with no per-component restyle.

### Why the raw-literal styling-DRIFT derive-rule is not yet expressible

A `put_derive_rule` that flags "components using raw color/spacing literals instead of
tokens" by **direct graph match** is **not soundly expressible** over the current
lodestar schema, so none was shipped (a rule that silently matches nothing — or
mis-fires — would be a placeholder, which the guardrails forbid). Concretely:

- `Component` nodes carry only an **identifier** token bag (`bt`) that is **length-capped**
  and **excludes string literals** — so neither `var(--token)` references nor raw `#rrggbb`
  / `rgba(…)` literals (which live in CSS string declarations) appear as queryable facts.
  A `bt CONTAINS '#'` rule matches **zero** real drift; a `bt CONTAINS 'styles'` heuristic
  yields false positives because the cap drops the `styles` token from large components
  (DataGrid/CommandPalette do use CSS Modules but would be mis-flagged).
- The 46 `*.module.css` files are opaque `File` nodes: their **declarations are not
  parsed** into the graph, and Components have **no resolved `IMPORTS` edge** to their
  `.module.css` (the indexer resolves TS/JS symbol imports only). So neither the literal
  nor the token-reference is on any node/edge property.

Soundly expressing this needs an **indexer capability that does not exist today**: a
CSS-aware pass that parses `*.module.css` declarations into queryable
`uses_raw_color_literal` / `references_token` Component properties (or token-reference
edges). Once that lands, the rule becomes the trivial, deterministic
`MATCH (c:Component) WHERE c.file_path CONTAINS 'gui/src' AND c.uses_raw_color_literal = true RETURN c`
emitting a `kind:derived` drift claim every index. Until then the contract is held by the
filesystem scan above + the per-component VISUAL claims, not by a derive-rule.
