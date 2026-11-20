# Requests from permission-intel

## To adaptive-island (island shell)

### 1. A coral glow for risky permission requests

- **Why:** the design critique of the new permission peek found that a risky request ("can send your SSH private key to evil.example.com") still glows periwinkle, the same as a routine one. DESIGN.md's Glow-Means-News rule fits: "this could destroy work or leak a secret" is bigger news than "permission needed".
- **What the peek offers:** every `Permission` now carries `intel.risk`: `"safe" | "caution" | "unknown" | "risky"` (see `src/lib/api.ts`, the permission-intel block). `unknown` means Atoll could not check the request and is treated like risky.
- **Asked:** when the peek shows a permission whose `intel.risk` is `"risky"` or `"unknown"`, give the shell `data-signal="error"` instead of `"accent"`. Nothing else changes: same ring, same timing.
- **Where it would go:** wherever `App.svelte` picks the shell's signal for `peek:permission`. permission-intel does not edit the shell or App.svelte.
- **Until then:** the peek itself already marks risk in coral (the kicker word and icon, the "Outside the project" line, and Allow in the destructive style), so nothing is blocked.

### 2. An "Allowed by rule" line when a rule answers for the user (Stage C)

- **Why:** the brief says every automatic decision must be visible, with a one-click way to disable the rule. Settings → Permission rules already lists each one with "Turn off rule", but the user is usually in another app; the island is where they would notice it happened.
- **What permission-intel emits:** the Tauri event `permissions:auto-allowed`, payload `{ ruleId, ruleName, summary, at }` (`AutoAllowed` in `src/lib/rules.ts`; `onAutoAllowed()` subscribes). It fires once per request a rule allowed. No peek is opened for it, and nothing waits on the island.
- **Asked:** on that event, show a short, quiet notice in the pill (like a "done" pulse, not a permission peek): `Allowed by rule “<ruleName>”` with `summary` as the detail, and one action, **Turn off rule**, which calls `rulesApi.disable(ruleId)` from `src/lib/rules.ts` (Tauri command `rules_disable`). It should never take focus or beat a waiting permission request in the arbiter.
- **Where it would go:** the pill and the arbiter are adaptive-island's. permission-intel does not edit them.
- **Until then:** Settings → Permission rules → "Answered by rules" shows every automatic answer with "Turn off rule", and the peek's hint line shows when a rule matched but still asked.
