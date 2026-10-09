# The design-token + mockup loop (Visual Knowledge)

How an agent maintains **Visual Knowledge** — the design-system half of the
knowledge layer — using the same author→gate→(optional)review loop as code
claims. The operator guide is
[`docs/guides/visual-knowledge.md`](../../../docs/guides/visual-knowledge.md); the
host-side drivers are `tools/visual/` (`README.md` there).

> **Same model, one evidence_pack.** Visual claims live in the **same Tier-1
> model** as code claims: one `evidence_pack` returns code + visual claims. The
> symbolic kinds are gated **exactly like `invariant:pure`** (deterministic, in
> the engine). The neural `ui:matches-mockup` verdict goes through the **same
> never-self / cross-family** review seam as every other claim — it never widens
> trust. **No Figma, no commercial dependency**, anywhere.

## The deterministic kinds (Stage-1, no model)

Author these like any constraint claim; the engine's design gate decides
PASS/FAIL/DEFER off the graph + the in-repo `design-tokens.json` (W3C DTCG) + the
anchored component's source. Present+clean ⇒ PASS, present+violating ⇒ FAIL with
the concrete refuting fact, **absent ⇒ DEFER** (the present-guard — never a
silent pass).

| Kind | What it proves | Refute |
|---|---|---|
| `design:token` | the component references DTCG tokens, not a raw color/spacing/typography literal **where a token exists** | the raw literal with no `CONSUMES_TOKEN` edge |
| `ui:component:<tag>` | uses the design-system wrapper, not the raw element **where a wrapper exists** (`ui:component:button`) | the raw `<button>`/`<input>` tag |
| `a11y:<fact>` | a statically-decidable a11y fact (`a11y:labeled` = accessible-name prop present; `a11y:contrast` = the consumed fg/bg **token pair**'s `$value`s meet the WCAG ratio) | the missing prop / the failing token pair |

Scope is **only what is checkable now**: token *adherence* (not "the right token
was chosen"), wrapper *usage* (a known raw tag with a known wrapper), and
statically-decidable a11y facts (never a rendered-DOM scan — that is host-side).
Deeper TSX/CSS-in-JS extraction **defers**; do not over-claim.

## The neural kind (Stage-2 seam, opt-in)

`ui:matches-mockup` is the one kind the graph cannot decide — it needs a render
and a vision model. It is **default-off** and behind the existing review seam.
Never author it as fact off your own reasoning: drive it through
`tools/visual/visual-verify.sh`, which records a **cross-family** verdict the
engine adjudicates. A **refute** marks the claim `contradicted`; an **affirm**
**never** overrides a symbolic refute (pixels-match ≠ conformance). With no
provider installed the seam returns a **recorded/stub** verdict — say so; never
present a stub as a live judgment.

## The agent loop (editing a CelNet component toward a moved mockup)

1. **Read.** `evidence_pack(PriceCell)` returns the design rules *with the code* —
   the consumed tokens, the props/spec, the target mockup hash, and the alignment
   state ("token-adherence active, matches-mockup STALE since M→M'"). A **new
   mockup hash flips `ui:matches-mockup` stale even when code is unchanged** — the
   implementation now lags a moved target. That is how a mockup **drives**
   direction.
2. **(Optional) ingest Storybook.** If a built `index.json` exists,
   `sh tools/visual/storybook-ingest.sh anchor storybook-static/index.json design:token,a11y:labeled`
   anchors the deterministic design kinds to the real component qns, carrying the
   story id as the render-state. Absent manifest ⇒ no-op; hand-anchor instead.
3. **Edit + propose.** Change the component toward the mockup; author
   `ui:matches-mockup(PriceCell → M')` as a **draft** claim pinned to the in-repo
   target hash.
4. **Gate (deterministic, free).** The Stage-1 design gate refutes immediately if
   the edit introduced a raw literal (`design:token` FAIL), used a raw element
   where a wrapper exists (`ui:component` FAIL), or dropped an accessible name
   (`a11y:*` FAIL). No model, no cost. Fix and re-put until it clears, or DEFER.
5. **Judge (opt-in).** If a host installed the visual provider,
   `sh tools/visual/visual-verify.sh verify <claim_id> <story_id> <target.png>`
   renders the story (Storybook + Playwright), screenshots it, runs axe-core, and
   records the cross-family vision verdict. With no provider, the visual claim
   stays on its deterministic result and you say so.
6. **Travel.** The verdict is appended to the content-addressed event log, so the
   team's agents share one source of design truth — lossless union across merges.

## Setup + self-test

```sh
sh tools/run-selftests.sh                          # includes both visual self-tests
cp tools/visual/lodestar.visual.toml.example tools/lodestar.judge.toml   # then edit [visual]/[storybook]
sh tools/visual/visual-verify.sh --self-test       # capability matrix (no render, no model)
```

The design target is **always** an in-repo, content-addressed asset (PNG/spec or
a designated baseline story snapshot) hashed by `kn_hash`. Never a Figma artifact.
The open tooling — Storybook/test-runner (MIT), Playwright (Apache-2.0), axe-core
(MPL-2.0) — is host-side only and never linked into the pure-C engine.
