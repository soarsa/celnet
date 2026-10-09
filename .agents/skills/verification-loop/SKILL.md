# The verification loop (canonical — do not reinvent)

This is THE approach for verifying agent work in this repository. It is recorded
as graph-anchored knowledge (a `decision` claim on `lode_review_request`) so it
cannot drift. lodestar's own rule (`knowledge_review`: never self-judge,
distinct-family panel — tested by `seam_review_cross_family_adjudicates_same_family_rejected`)
is the gold standard; this generalizes it to all agent work AND shifts the
expensive judgment LEFT.

## Front-load the judgment; keep the post-check cheap

The wasteful pattern is *latent*: author produces full work → judge adversarially
re-derives a critique → rework. The judgment is spent **per artifact, after the
fact**, and flawed work is produced only to be discarded. Invert it:

**Phase 1 — judge as collaborator, UP FRONT, once (the expensive reasoning).**
Before authoring, the critical persona produces the **binding, reusable
directives**: the **verified seam pack** (the real signatures/seams, re-grounded
against the live graph — so an author can't misread them, e.g. "`lode_cypher_execute`
takes a `store`, not a `gbuf`"), the **frozen contracts**, the **decidable
acceptance assertions** (the `spec:satisfies` `acceptance.json` — schema/edge/
invariant/perf), and the **known traps + non-negotiables** (determinism, leak,
model-free, default-off, never-silent-PASS). Authors build TO this. It is
produced **O(1)** and **reused across every author and increment** — the flaw is
*prevented*, not caught late.

**Phase 2 — verify CHEAPLY, post (the authority, ~no reasoning).** The
**deterministic gate is the authority**: `knowledge_check` (the decidable
acceptance assertions), the ASan/UBSan/LeakSanitizer build+suite, and the
benchmark floors. These are mechanical — ~0 tokens, no re-derivation. A Agent
critical persona does ONLY a **light, targeted confirmation of the non-decidable
residue** (the behavioral/`judged` parts the gate must defer), never a full
adversarial re-review of what the contract + gate already cover.

> Economics: expensive judgment is O(1) up-front and reused, instead of
> O(authors x increments) latent re-critique; the post-check is mechanical
> instead of a full LLM re-judge. Same rigor, a fraction of the effort/tokens.
> This is lodestar's own model — "the spec is the verifier, never the generator":
> the contract/acceptance is the up-front directive; the gate is the cheap check.

## The rules (hold in both phases)

1. **Author != reviewer.** Never self-review.
2. **Gate on the artifact, not self-reports.** Read the real diff/file; re-ground
   cited facts against the live graph (the knowledge layer self-invalidates — a
   cited claim/seam can go stale mid-reindex; re-pull, never trust a cached id).
3. **The deterministic gate is the AUTHORITY; model review is ADVISORY.** No
   model verdict can mint a PASS — only refute or advise. Never a silent PASS.
4. **Refute-default; fix-forward.** Default to refuted on uncertainty; work that
   fails its gate is repaired in place, never merged on a self-report.
5. **Perspective-diverse, not redundant.** Distinct lenses for distinct risks
   (fit / determinism+leak / security), never N copies of one check.

## Judge tier — current and deferred (a tracked clause, not a sticky note)

- **Current:** a **Agent critical persona** (independent, refute-default). Sound
  because rule 3 keeps the deterministic gate authoritative.
- **Deferred (tracked):** evolve the reviewer to a genuine **cross-family** model
  via lodestar's judge transport (ollama gemma/qwen or a non-Agent API), per the
  product's never-self / distinct-family rule. Recorded in the verification
  `decision` claim + the v0.7.0 charter so it is never forgotten. It strengthens
  the *advisory* layer, never the *authority* layer.
