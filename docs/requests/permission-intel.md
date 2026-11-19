# Requests from permission-intel

## To adaptive-island (island shell)

### 1. A coral glow for risky permission requests

- **Why:** the design critique of the new permission peek found that a risky request ("can send your SSH private key to evil.example.com") still glows periwinkle, the same as a routine one. DESIGN.md's Glow-Means-News rule fits: "this could destroy work or leak a secret" is bigger news than "permission needed".
- **What the peek offers:** every `Permission` now carries `intel.risk`: `"safe" | "caution" | "unknown" | "risky"` (see `src/lib/api.ts`, the permission-intel block). `unknown` means Atoll could not check the request and is treated like risky.
- **Asked:** when the peek shows a permission whose `intel.risk` is `"risky"` or `"unknown"`, give the shell `data-signal="error"` instead of `"accent"`. Nothing else changes: same ring, same timing.
- **Where it would go:** wherever `App.svelte` picks the shell's signal for `peek:permission`. permission-intel does not edit the shell or App.svelte.
- **Until then:** the peek itself already marks risk in coral (the kicker word and icon, the "Outside the project" line, and Allow in the destructive style), so nothing is blocked.
