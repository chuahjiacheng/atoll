# Permission intelligence — Stage C: rules (approved 2026-11-20)

Branch `permission-intel`, commits `765474d` (engine) and `35cabcb` (page and peek). Builds on Stage A (`stage-a.md`) and Stage B (`stage-b.md`). Not merged into `main` yet.

## What changed for you

- **Settings → Permission rules** (new page, after Agents):
  - **Your rules.** Each rule shows what it covers, where, and a status line: "Hint only: still asks you", "Allows without asking until 27 Nov", "Auto-allow ended: asks again" or "Paused". Each has **Edit** and an on/off switch that stops it at once.
  - **The editor:** a name, what it covers (Commands / File edits / Web pages), where (this project or every project), which ones (one per line), a **dry run**, and **Allow without asking**.
  - **Answered by rules:** every request a rule allowed for you, newest first, with the command kinds, the rule, the project and when. Each row has **Turn off rule**.
- **The peek:**
  - When a rule matches but still asks, a line says so and why, for example: "Rule “npm test, tail in proj” matches. Still asks: its dry run never saw “npm run build” allowed."
  - **Make a rule…** opens Settings with a rule drafted from the waiting request and runs its dry run straight away. The request itself keeps waiting in the island.

## The rules about rules

1. **A rule only hints by default.** It shows the line in the peek; you still answer.
2. **Auto-allow is opt-in per rule** and needs:
   - a dry run of the rule exactly as it is now (editing the rule voids the dry run: "The rule changed since its dry run. Run it again."),
   - at least one request like it that **you** allowed,
   - an end date: 1, 7 or 30 days (default 7, never more than 30). After that it goes back to hinting.
   - If the dry run found requests you said no to, the button stays off until you tick "I checked them; let it answer anyway".
   - The button says what you are agreeing to ("Allow until 27 Nov"), with one line above it: "Until 27 Nov, Atoll answers yes for you to npm test, tail in proj. Anything else, and anything risky, still asks."
3. **Never wider than the dry run.** A rule auto-allows only the command kinds its dry run saw you allow ("npm test", not "npm test --watch" unless you allowed that too). Anything else it covers only hints.
4. **Never risky.** A request rated Risky, or one Atoll couldn't check (including a timed-out check), always asks, whatever the rule says.
5. **No shells or wrappers.** Patterns like `bash`, `sh`, `sudo`, `env`, `eval`, `xargs`, `powershell` or `cmd` are refused, because they can run anything.

## What the hook can return, and the fail-safes

A PermissionRequest hook can answer `allow` or `deny`, or say nothing (Claude Code then shows its own prompt).

- **Atoll returns a plain `allow` and nothing else.** It never sends `updatedPermissions`, so a rule's answer is never written into Claude Code's settings. Turning a rule off stops it at once.
- **Claude Code's own deny rules still win** over a hook's allow [Unverified: from Claude Code's documentation, not tested here].
- **Fail-safes:**

| Situation | What happens |
|---|---|
| The rules file can't be read | It is renamed `rules.json.broken-<time>`, a banner says so, and **no rule acts** until you save again |
| The analysis times out (50 ms) or fails | "Couldn't check": treated like risky, so never auto-allowed |
| A rule has expired | Hint only; the status line says "Auto-allow ended: asks again" |
| The rule changed since its dry run | It cannot auto-allow until you run the dry run again (checked in Rust when saving, not only on the page) |
| Atoll isn't running | The bridge finds no listener and Claude Code shows its own prompt (unchanged from Milestone 3) |

## What is recorded

`%LOCALAPPDATA%\Crest\permissions\`:

- `rules.json`: your rules.
- `answers.jsonl`: one line per answered request: when, project folder, tool, **command kinds only** ("npm test", "edit src/lib"), risk, allow or deny, and whether you or a rule answered. **Never the full command or file contents.** A rule's answers also keep the one-sentence summary for the log. The newest 10,000 lines are kept.

The dry run reads that log. The recorder session's history (`docs/contracts/event-store.md` v1.1) is coded against but stubbed: `rules/recorder.rs` returns nothing until the recorder is merged, and the page says "the recorder is not connected yet". Its entries will only count for the time before Atoll's own log starts, so nothing is counted twice.

## Results

| Check | Result |
|---|---|
| Rules check on the hook's path (50 rules, one request), release | **7.9 µs** per request |
| Dry run over the full 10,000-answer log, release | **3.9 ms** |
| Corpus (Stage A) | 236 / 236, **false allows 0** (unchanged) |
| Crate unit tests | 15 / 15 |
| App crate tests | 81 / 81 (10 ignored: probes, the latency run and the rules timing run) |
| Rules tests inside those | 12, including: auto-allows only what the dry run saw, never risky, expiry, edits void the dry run, shells refused, the damaged file, and that the answers log never holds the command text |
| Page tests | rules 2 / 2 (the page's fingerprint matches Rust's), highlighter 3 / 3 |
| svelte-check | no findings in the changed files (34 errors / 1 warning before and after, all pre-existing elsewhere) |
| Production build | builds |
| Contrast, dark theme (measured) | status green 9.5 : 1, "Denied" red 7.0 : 1, "Save rule" 8 : 1 |

### Impeccable

- **Detector:** two kinds of hit, both dismissed. "Low contrast" on the glass buttons measured the card behind the button, not its fill (real ratio about 8 : 1). Inter is the font DESIGN.md asks for.
- **Audit fixes:** "Where" could not switch back from "Every project"; an existing rule's end date showed no day option; Edit and "Turn off rule" now name their rule for screen readers; the scroll to the editor respects Reduce motion.
- **Critique:** A scored 27 / 40. B (browser evidence) checked tab order and contrast. Fixed since:
  - The moment you turn on auto-allow is now spelled out (the line and button above), and the "you said no before" box sits in that warning and blocks the button, instead of an error after Save.
  - Drafted names no longer start with "Allow" (most rules only hint): "npm test, tail in proj", "Edits in src/**", "Pages from docs.rs".
  - The damaged-file message drops the parser text and says what happens next.
  - Log rows show the command kinds.
  - Focus goes back to Add a rule or Edit after Cancel or Save.
  - The Name field is wider; the confirm box is 18 px in the accent colour; "1 of these was".
- **Not fixed, outside this branch:** the focus ring on a *selected* segment in Seg (`src/dashboard/ui/Seg.svelte`, Milestone 1–2) is drawn in the same colour as its fill, so it can't be seen. This affects every Settings page; noted in the handoff.
- **Not done:** listing each allowable kind with its own counts, or unticking single kinds before allowing.

## Test steps

1. Run the app from this branch (`npm run app` in the worktree). Open Settings → **Permission rules**: an empty page that says "No rules yet".
2. In a Claude Code session, let it ask for a routine command (for example `npm test`) a few times and **Allow** it in the island.
3. On the next such request, click **Make a rule…** in the peek. Settings opens with "npm test in <project>" and a dry run that counts your answers. The request is still waiting in the island; answer it.
4. Save the rule (hint only). On the next `npm test` request the peek still asks. Ask for `npm run build`: the peek should say the rule matches but still asks, and why.
5. Edit the rule, run the dry run, turn on **Allow without asking**, pick 1 day, and click **Allow until …**. The next `npm test` should go through without a peek and show up under **Answered by rules**.
6. Click **Turn off rule** there. The next `npm test` asks again.
7. Try a risky command the rule's program could run (for example `npm test; rm -rf ~`): it must always ask.
8. Quit Atoll, put `{` into `%LOCALAPPDATA%\Crest\permissions\rules.json`, and start it: a red banner, no rules acting, and a `rules.json.broken-…` file beside it.

The user ran these steps in the real app and approved Stage C on 2026-11-20. Before that, they were checked only in the labs and the Rust store tests.
