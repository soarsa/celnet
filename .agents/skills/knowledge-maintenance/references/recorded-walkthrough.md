# Recorded-Real Walkthrough: `invariant:pure` Constraint Gate

This document describes a deterministic CI testing pattern for verifying purity claims on real code functions. The workflow operates in three phases:

## Core Testing Loop

**Phase 1** involves a real provider authoring a claim about function `lode_is_keyword`. As noted in the document, the function "reads its two args, `switch`es to a static `const` keyword table" and has "no outgoing `WRITES` edge." The provider submits an assertion that the function "has no side effects" and the constraint gate validates this by confirming an empty writes set, transitioning the claim from draft to active status.

**Phase 2** demonstrates automatic staleness detection. When the anchored function's content hash changes—due to editing—the system deterministically flips the claim to stale state with "zero LLM" involvement. This represents what the document calls the staleness mechanism.

**Phase 3** closes the maintenance loop through re-authoring. If the function remains pure after editing, the provider resubmits and the gate passes again. Crucially, the negative case shows the gate's rejection mechanism: if the function now writes to global state, the gate "hard-rejects" any purity claim and prevents it from reaching active status.

## Key Guarantees

The document establishes four core assertions:

- Claims transition through definable states (draft → active → stale → active)
- Changes to anchored symbols deterministically trigger staleness
- False purity claims are rejected with measured "constraint-gate precision = 1.0"
- Unresolved anchor references are caught and not stored

The engine half remains reproducible for CI because only the provider's authored text is recorded; verdicts remain deterministic across replay cycles.
