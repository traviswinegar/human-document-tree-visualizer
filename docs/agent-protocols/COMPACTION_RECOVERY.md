---
id: compaction-recovery
cluster: agent-protocols
node_type: topic_root
pinned: true
version: v0.20.0
spec_version: 0.4
status: stable
provenance: copied verbatim from E:\Development\ai-studio\docs\agent-protocols\COMPACTION_RECOVERY.md
last_verified_against:
  ref: 8ff1063c
  date: 2026-06-01
---

# Compaction Recovery Protocol

## brief {#compaction-recovery-brief node:domain_brief}
Reusable protocol for AI coding agents doing multi-phase work that spans more context than fits in a single window. Maintains an external ledger (markdown file) so a post-compaction agent can walk back to the last verified position and resume without re-deriving state or trusting a summary that may have lost critical detail. Battle-tested across ~50 commits spanning four phases of AI Studio's QA Loop hardening pass with zero working-memory items lost.

## problem {#compaction-recovery-problem node:architecture_note}
Long-running agent work — refactors, hardening passes, multi-phase implementations — accumulates more context than a single window holds. When the harness compacts, the agent loses:

- What it was about to do next
- What it had just finished
- What it noticed but has not fixed yet
- Why it made specific design decisions

Without a recovery protocol, the post-compaction agent either re-derives state from scratch (slow, error-prone) or trusts a summary that may have lost critical detail (silently wrong).

## ledger-structure {#compaction-recovery-ledger-structure node:architecture_note}
The external ledger is a markdown file (e.g. `HARDENING_LOG.md`, `MIGRATION_LOG.md`, `REFACTOR_LOG.md`) that holds five sections in order:

- **Current Position** header — the step the agent is on RIGHT NOW
- **Walk plan** — ordered list of files or tasks, with seeded issues at each
- **Completed entries** — for each finished step, a verification triple: commit hash + test path + source file:line
- **Catch-all phase** — discoveries that are not part of the current step; logged here, never held in working memory
- **Recovery protocol** — verbatim instructions for the post-compaction self

Update the Current Position optimistically at the start of each step (the header is allowed to lead reality). Update the completed entry pessimistically — only after the commit lands.

## first-principle {#compaction-recovery-first-principle node:decision}
> **The document is a hypothesis. The code is truth.**
> When they disagree, the code wins and the document gets corrected to match.

This is what makes recovery robust. The agent never trusts the ledger blindly — it always cross-checks against the actual codebase. Without this principle, a ledger entry that drifted from the code silently misdirects the recovering agent. With it, the first verification failure surfaces the actual position and the ledger is corrected rather than trusted.

## walk-back-validation {#compaction-recovery-walk-back node:project_rule}
**On every session start — compaction recovery, fresh start, or after a long pause — perform walk-back validation before touching code:**

1. Read the entire ledger.
2. Find the Current Position. Note the step.
3. Walk back through all prior completed entries in the current phase. For each, run the verification triple:
   - Does the named commit exist? `git log --oneline | grep "<id>"`
   - Does the named test exist and pass? `flutter test <path>`
   - Does the named source line match what the entry claims? Read the file.
4. The first entry where verification fails is the actual current position. Discard the in-progress assumption. Restart that step from scratch.
5. If all prior entries verify clean, restart the named Current Position step from scratch.

The walk-back catches: mid-documentation failure (commit missing, grep returns empty); mid-fix failure (commit landed but log not updated — verification succeeds, advance past it); documentation drift (log says line 175, code shows line 182 — correct the log and continue).

## test-first-per-fix {#compaction-recovery-test-first node:project_rule}
**Every fix begins with a failing test that demonstrates the bug.**

The failing test is a recoverable checkpoint independent of the ledger:

- If compaction hits between writing the test and the fix: recovery sees the failing test in the file system, knows exactly what the work is, and restarts the fix.
- If compaction hits after the fix passes but before commit: the passing test plus the uncommitted source change tell the story.
- If compaction hits after commit but before log update: the commit message plus the diff are the source of truth for what happened.

## atomic-commits {#compaction-recovery-atomic-commits node:architecture_note}
One commit per logical issue. Commit message format includes a phase or issue identifier (e.g. `Phase 11 hardening #3:`, `Issue #42:`, `Migration step 5:`). The git log itself becomes part of the recovery story:

```
git log --oneline | grep "Phase 11 hardening"
```

This tells exactly which seeded issues are done, in order, with their resolution captured in commit-message form.

## catch-all-discipline {#compaction-recovery-catch-all node:guardrail}
**Never fix an off-topic discovery inline, and never hold it in working memory.**

When working on issue N, discoveries of problems unrelated to N must be handled as follows:

- Log the discovery to the designated catch-all phase with a `discovered while working on Phase X #N` provenance note.
- Do not fix inline. Inline fixes balloon the commit, mix concerns, and break the recovery story.
- Do not hold in working memory. Working memory is exactly what compaction destroys.

The catch-all becomes its own prioritized backlog later.

**Single exception:** if not fixing the discovered issue would block forward progress on the current fix (e.g. it is a critical bug in code the current fix depends on), promote it to an in-current-phase issue and address it next. Otherwise, defer unconditionally.

## reset-point-rule {#compaction-recovery-reset-point node:project_rule}
**If recovery determines the current step is half-done, restart the entire step — not from the estimated midpoint.**

Rationale: better to redo small work than to corrupt larger work; mid-step state is the hardest to verify; restarting from a known-good boundary is always cheap when test-first discipline is in place.

Implication: each step should be small enough that redoing it entirely is acceptable. If a step is so large that redoing it costs hours, it is effectively three steps and should be split before proceeding.

## empirical-results {#compaction-recovery-empirical-results node:architecture_note}
Results from the AI Studio QA Loop hardening pass that originated this protocol — a single-day stretch across ~50 commits spanning phases 11 through 14:

- 168 → 276 tests (+108 new tests, all green)
- Phase 11: 7/7 seeded issues closed
- Phase 12: 7/7 seeded issues closed
- Phase 13: 7/7 seeded issues closed
- Phase 14: 42 catalogued + 42 closed (39 by direct fix, 3 by architectural trio)
- Compaction events: multiple, including one bedtime pause plus a fresh-morning session start
- Items lost to working-memory drop: zero
- Recovery time per session start: approximately 3 minutes (read log, walk back, verify, resume)

The protocol's return on investment scales with project size. For a 50-commit day, the approximately 10% overhead (writing to the ledger, walking back on start) pays back many times over in avoided rework.

## scope-limits {#compaction-recovery-scope-limits node:architecture_note}
This protocol is not a substitute for good testing (tests catch regressions; this protocol catches lost progress) and not a substitute for good architecture (scattered work across 30 files is not salvageable by any recovery protocol). It is also not optimized for happy-path single-session work — the overhead only justifies itself when the work spans multiple context windows with discrete, verifiable units.

Use the protocol when:

- The work spans more than a single context window.
- The work has multiple discrete units (issues, bugs, refactor steps) that can be sequenced.
- Each unit has clear acceptance criteria (passing test, working feature, satisfied invariant).
- Atomic commits per unit are feasible.

Skip the protocol when:

- The work is exploratory ("figure out why this is slow") with no defined units.
- The work is intrinsically interactive, with state living in the conversation rather than the code.

## adoption-checklist {#compaction-recovery-adoption-checklist node:architecture_note}
Steps to initialize the protocol for a new project:

1. Create the ledger file at a stable path.
2. Write the **Current Position** header (one step's worth).
3. Write the **Walk plan** as a table (file plus seeded issues per row).
4. Write the **Recovery Protocol** verbatim into the file so any future agent knows what to do.
5. Write the **Editing rules** (last-step log update, test-first, commit-per-issue, no scope creep).
6. Optional: write a one-line quote at the top: _"The document is a hypothesis. The code is truth."_

Setup cost: approximately 30 minutes for a substantial project. From then on, every commit plus log update is the protocol running.

## see-also
- [ref:advisory-recovery]

---

> **Local adaptation note (human-document-tree):** the verification triple's test
> command is `cargo test <path>` for the Rust backend and the frontend's test
> runner for the web side — not `flutter test`. Everything else applies verbatim.
