# Handoff: permission-intel

Branch `permission-intel`, worktree `.claude/worktrees/permission-intel`. Written 2026-11-20.

- Stages A and B are approved and in `main` (merge `1ad8ae0`).
- Stage C (rules) is committed on the branch (`765474d`, `35cabcb`) and **waits for approval**; it is not merged.

Details per stage: `docs/permission-intel/stage-a.md`, `stage-b.md`, `stage-c.md`, and `corpus-results.md`.

## What the branch does

1. **Command analysis (Stage A).** The crate `crest-intel` (`intel/`) reads a request before you answer it:
   - Bash is parsed with tree-sitter-bash; PowerShell with its own tokenizer.
   - Every request gets a risk level: Safe, Caution, Unknown (couldn't check, treated like risky) or Risky.
   - Corpus of 236 commands: **0 false allows**, median 13 µs.
2. **Peek content (Stage B).** The permission peek shows one plain sentence, the risk, where, and the exact command or a syntax-coloured diff. Risky and unchecked requests make Deny the main button and turn Enter off. The queue and the 120 s hand-back are unchanged.
3. **Rules (Stage C).**
   - Local rules scoped by project, tool and pattern. They hint by default.
   - Auto-allow is opt-in per rule, needs a current dry run and an end date (30 days at most), never covers risky requests, and never allows a command kind the dry run didn't see you allow.
   - Every automatic answer is logged under Settings → Permission rules, with "Turn off rule".

## Files

New:
- `intel/` (the analysis crate, its corpus and tests).
- `src-tauri/src/rules/` (`logic.rs`, `mod.rs`, `commands.rs`, `recorder.rs`).
- `src/lib/rules.ts` and its test; `src/dashboard/pages/RulesPage.svelte`.
- `src/lib/island/highlight.ts` and its test.
- Labs, dev only and not built: `permission-lab.html` with `src/lib/island/permlab/`, and `rules-lab.html` with `src/dashboard/ruleslab/`.
- `docs/permission-intel/`, `docs/requests/permission-intel.md`.

Changed:
- `src/lib/island/PermissionPeek.svelte` (owned by this branch since Stage B).
- `src-tauri/src/permissions.rs` (analysis and rule hint on each request).
- Shared files, each one marked `permission-intel` block:
  - `src-tauri/src/server.rs`: analysis, the rules check, the answer record;
  - `src-tauri/src/lib.rs`: `mod rules;` and the rules commands;
  - `src-tauri/Cargo.toml`: the `crest-intel` dependency;
  - `src/lib/api.ts`: `intel` and `rule` on `Permission`;
  - `src/styles/tokens.css`: peek tokens;
  - `src/dashboard/Dashboard.svelte`: the page entry, its route and the "go to rules" listener.

## Contracts used

- `docs/contracts/event-store.md` v1.1. `rules/recorder.rs` turns its events into past requests for the dry run. It is a **stub** that returns nothing until the recorder session is merged; then `past_requests()` reads the store. Recorder entries count only for the time before Atoll's own answers log starts.

## Requests to other sessions

In `docs/requests/permission-intel.md`, both to adaptive-island:

1. A coral shell glow for risky requests.
2. An "Allowed by rule" notice in the pill when `permissions:auto-allowed` fires, with "Turn off rule".

## How to test

```bash
node --experimental-strip-types --test src/lib/rules.test.ts src/lib/island/highlight.test.ts
```

```bash
cd intel && cargo test --release
```

```bash
cd src-tauri && cargo test --release
```

```bash
cd src-tauri && cargo test --release rules_timing -- --ignored --nocapture
```

- Expect 5 page tests, 15 crate tests plus the corpus, and 81 app tests (10 ignored).
- Labs: `npx vite --port 1432` in the worktree, then `/permission-lab.html` and `/rules-lab.html` (`?draft`, `?empty`, `?broken`, `&theme=light`).
- Live test steps: `stage-c.md`, "Test steps".

## Open issues

- **[Unverified] Stage C has not run end to end** through Claude Code's hook in the real app; only the labs and the Rust store tests.
- **[Unverified]** "Claude Code's deny rules win over a hook's allow" is from its documentation, not tested here.
- **Seg focus ring** (`src/dashboard/ui/Seg.svelte`, Milestone 1–2, not this branch's): on a selected segment the 2 px focus outline matches the fill, so keyboard focus can't be seen there. Affects every Settings page.
- The rules page can't yet show per-kind counts in the dry run, or let you untick single kinds before allowing.
- The recorder stub: until the recorder is merged, dry runs see only Atoll's own answers log, which starts with Stage C.
- In the lab only, closing the rules page logs an `unregisterListener` error, because the stubbed Tauri has no event plugin. The real app is unaffected [Inference].
