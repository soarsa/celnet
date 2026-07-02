---
name: git-local-only
description: Celnet git is local-first; pushing is allowed ONLY to github.com/soarsa/celnet (authorized 2026-06-05).
metadata:
  node_type: memory
  type: feedback
  originSessionId: b315eccc-f521-4987-b5b5-1a21d5710edb
---

Celnet git is **local-first with one sanctioned remote**. Commit locally as much as needed. Pushing is permitted **only** to `github.com/soarsa/celnet` (the `origin` remote, owner `soarsa`).

**Why:** Originally local-only (user instruction 30 May 2026). On **2026-06-05** the user explicitly lifted that, authorizing push to `github.com/soarsa/celnet` specifically. The local-only deny rules (`git push` / `git remote add` / `git remote set-url`) were removed from `.claude/settings.json`; CLAUDE.md guardrail #1 updated to match.

**How to apply:** `init`/`add`/`commit`/`branch`/`log`/`push` are fine. `origin` must point at `https://github.com/soarsa/celnet` and **nowhere else** — do not add any other remote or push elsewhere without a fresh explicit instruction. See [[no-mocks-policy]].
