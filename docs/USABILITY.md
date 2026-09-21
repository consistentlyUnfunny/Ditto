# Usability review — GUI, 2026-09-21

Written after using the app against the real machine. Findings are grouped by
whether something is **broken** (does not work) or **confusing** (works, but the
user cannot tell what it will do). Nothing here is implemented yet.

State the review was done against: 6 skills, 3 MCP servers, 5 harnesses.
`karpathy-coder` is drifted across 4 locations holding 2 distinct versions.

---

## The root cause

**The UI exposes the engine's verb instead of the user's intent.**

Every row, whatever its state, offers the same button — *Plan a deploy* — with
the same caption. But "deploy" is three different jobs wearing one name:

| The user is thinking | The engine calls it |
|---|---|
| "Codex has this, Claude doesn't. Put it there." | deploy |
| "These two copies disagree. Make one win." | deploy |
| "I edited the source. Push it out." | deploy |

The second is the interesting one, and it has a decision at its heart — *which
version wins* — that the interface never asks. The engine currently picks
`locations.iter().find(is_ok)`, i.e. **the first copy in path order**. For
`karpathy-coder` that silently selects `~/.agents/skills` because `.agents`
sorts before `.claude`. The user is never told, and cannot change it.

That is the single biggest gap. Everything below is downstream of it.

---

## Broken

**B-1 · Server rows are dead ends.**
`detail` in `crates/inception-gui/src/main.rs` searches skills only:

```rust
inventory::skills(&found, scope).iter().find(|g| g.identity == name)
    .ok_or_else(|| format!("no skill named '{name}'"))
```

Clicking `notion`, `node_repl` or `obsidian` in the Servers section opens the
drawer and shows `no skill named 'notion'`. A third of the matrix cannot be
opened.

**B-2 · The caption is now false.**
The detail drawer says *"Copies this skill to every harness that doesn't have
it."* Since the convergence fix, a deploy also **overwrites** copies that do
have it and disagree. The text describes behaviour the app no longer has, on the
button most likely to destroy something.

**B-3 · Target selection does not exist in the GUI.**
`main.ts:407` sends every installed provider, always:

```ts
const targets = (current?.providers ?? []).map((p) => p.id);
```

The CLI has `--to`; the window has no equivalent. You cannot sync to one
harness.

**B-4 · Source selection does not exist anywhere.**
Not in the GUI, not in the CLI. Arbitrary path order decides which version of a
drifted skill survives.

**B-5 · Problem rows lead nowhere useful.**
`codex-primary-runtime` (an empty directory) opens a drawer with one unreadable
copy and still offers *Plan a deploy*, which cannot succeed. There is no way to
act on the actual problem from the UI.

**B-6 · No refresh.**
The app reads on load. Anything that changes the machine afterwards — including
its own writes from another window, or a harness updating itself — is invisible
until you switch tabs and back.

---

## Confusing

**C-1 · The comparison diff pretends to be a change.**
A drift comparison is *A versus B*. Neither side is "before". Rendering it with
`+` and `-` tells the user that the `-` lines are about to be deleted, which is
not true — nothing is being changed at that point. This is the thing that reads
worst in the current build.

**C-2 · Two different diffs look identical.**
The drawer's comparison (A vs B, nothing will happen) and the plan's preview
(before → after, something *will* happen) use the same colours and the same
signs. The one that carries consequence has no extra weight.

**C-3 · The paper panels duplicate the diff and truncate.**
The detail drawer shows two "paper" cards containing the description plus 600
characters of body, then shows a full diff of the same files underneath. The
cards add nothing the diff does not say better, and their truncation can hide
the part that differs.

**C-4 · The matrix does not show where a copy lives.**
Paths are in `title` attributes only. The deprecated-location warning the CLI
prints (`!! deprecated path`) is absent from the window entirely, so the single
most actionable finding on this machine is invisible in the GUI.

**C-5 · Rows do not look clickable.**
Hover changes the label colour and nothing else — the background tint was
removed so the tie knockouts would match the ground. There is no cursor
affordance beyond the pointer, and no disclosure marker.

**C-6 · Engine vocabulary leaks.**
The drawer header reads `drifted`, `unique`, `problem`. Those are
`DriftStatus` variant names, not things a person would say.

**C-7 · Doctor is a dead list.**
Every finding names an action — "move these to `~/.agents/skills`", "run
`inception import`" — and none of them is a button.

**C-8 · The drawer has no history.**
*Cancel* on the plan screen re-fetches the detail view from scratch. There is no
back stack, so a deep flow cannot be stepped out of.

---

## Proposed design

### 1 · The action names the intent, and the state picks the action

| Row state | What is true | Primary action |
|---|---|---|
| unique | one copy; others lack it | **Add to 4 harnesses…** |
| identical | copies agree; some lack it | **Add to 2 harnesses…** |
| drifted | copies disagree | **Resolve differences…** |
| problem | cannot be read | *none* — explain, offer **Show in folder** |

The count goes in the label. A button that says how many things it touches is
a button the user can predict.

### 2 · Resolve — a two-step flow, source first

The decision the current UI skips becomes step one, and versions are grouped by
**content**, not by location, because that is the actual choice:

```
karpathy-coder exists in 4 places, in 2 versions.
Which one should win?

  ( )  17a16cad   codex · claude · copilot · gemini          3 copies
       "Behavioral guidelines to reduce common LLM coding mistakes…"

  (•)  b1697c5e   opencode                                    1 copy
       "Execute strict, no-fluff senior developer discipline…"

       [ Compare these two ]

                                          [ Cancel ]  [ Continue ]
```

Then, and only then, what it costs:

```
Using the opencode version — 3 copies will be replaced:

  ~/.agents/skills/karpathy-coder      serves codex, copilot, gemini
  ~/.claude/skills/karpathy-coder      serves claude
  ~/.codex/skills/karpathy-coder       deprecated location · remove instead?

  [ current → chosen diff ]

  Backups go to ~/.inception/backups.
                                          [ Back ]  [ Replace 3 copies ] 
```

### 3 · Add — target checkboxes that teach the write set

```
Add karpathy-coder to:

  [x] claude      → ~/.claude/skills
  [x] gemini      ┐
  [x] copilot     ├ → ~/.agents/skills   one shared file
  [x] codex       ┘

  2 writes, serving 4 harnesses
```

The brace is the minimum-write-set insight made visible at the moment it
matters, rather than explained in prose afterwards. Ticking a fourth harness
that shares the directory should visibly *not* add a write.

### 4 · Two diffs, two treatments

**Comparison** — no direction, so no `+`/`-`. Label by side and by harness:

```
  ◀ 17a16cad  codex, claude, copilot, gemini
  ▶ b1697c5e  opencode

  ◀ description: Behavioral guidelines to reduce common LLM coding mistakes…
  ▶ description: Execute strict, no-fluff senior developer discipline…
    ---
    Behavioral guidelines to reduce common LLM coding mistakes.
```

Two neutral tints, neither reading as "removed". The harness names on the
markers mean the reader never has to hold "left is which one?" in their head.

**Change preview** — direction is real, so `+`/`-` stays, but gains a header
that says what is being changed and in which direction, and a heavier treatment
than the comparison, because this one has consequences.

### 5 · Smaller fixes

- Drop the paper cards; the comparison replaces them.
- Put the deprecated-path flag on the matrix row, not only in `doctor`.
- A refresh control, and re-read after every apply.
- Human words in the header: *4 copies, 2 versions* rather than *drifted*.
- Doctor findings get the button their suggestion already describes.
- Row affordance: a chevron, or restore the hover tint by giving the knockout
  the same tint.

---

## Plan

Ordered so each phase leaves the app more usable than it found it.

### Phase 1 — unbreak  ✅ done 2026-09-21
- **T-5.10** `detail` resolves MCP servers as well as skills; server drawer shows
  transport, command, env var names (values redacted), and which file it lives
  in. *Fixes B-1.*
- **T-5.11** Fix the false caption; derive the wording from the plan rather than
  hard-coding it. *Fixes B-2.*
- **T-5.12** Problem rows: no deploy button, explain the state, offer
  **Show in folder**. *Fixes B-5.*
- **T-5.13** Refresh control, and auto re-read after apply. *Fixes B-6.*

### Phase 2 — the real fix: intent and choice
- **T-5.14** Engine: `skill_deploy` takes an explicit source; add
  `inventory::versions(group)` grouping locations by content hash.
- **T-5.15** CLI: `deploy --from <provider>` so both frontends can choose a
  source. *Fixes B-4.*
- **T-5.16** GUI: state-specific primary action with a count in the label.
- **T-5.17** GUI: the Resolve flow — version picker, then consequences.
- **T-5.18** GUI: the Add flow — target checkboxes with shared-write grouping.
  *Fixes B-3.*

### Phase 3 — make the diff readable
- **T-5.19** Split comparison rendering from change rendering; `◀`/`▶` with
  harness labels for comparisons. *Fixes C-1, C-2.*
- **T-5.20** Drop the paper cards. *Fixes C-3.*

### Phase 4 — polish  ✅ done 2026-09-21
- **T-5.21** Deprecated-path flag on matrix rows. *Fixes C-4.*
- **T-5.22** Row affordance. *Fixes C-5.*
- **T-5.23** Human vocabulary throughout. *Fixes C-6.*
- **T-5.24** Actionable doctor findings. *Fixes C-7.*
- **T-5.25** Drawer back stack. *Fixes C-8.*

Phase 1 is roughly an afternoon. Phase 2 is the substantial one and needs an
engine change first (T-5.14), since the source is currently chosen implicitly
inside the planner.

---

## Open question

**Q-9 · Should resolving offer to remove deprecated copies rather than
overwrite them?**

`~/.codex/skills/karpathy-coder` sits in a location Codex no longer documents.
Replacing it keeps a file alive that should arguably be deleted, since Codex
reads `~/.agents/skills` anyway. Offering *remove instead of replace* is
tempting, but deletion is the one irreversible action in the app and the
migration is a separate decision from resolving a conflict.

**Decided 2026-09-21 — overwrite.** Resolving drift replaces a copy in a
deprecated location rather than deleting it. `doctor` keeps suggesting the
migration as a separate, opt-in action. Deletion remains the one thing the app
does only when explicitly asked for it.


---

## Found by using it — 2026-09-21

**"already has it" was true and useless.** On the resolve screen every target
said *already has it*, because in a drift every harness does have a copy — that
is why they disagree. Beside a button about to rewrite three of them, it read as
"nothing to do here".

The label now describes the outcome *relative to the chosen version*, and
changes when the radio changes:

| | |
|---|---|
| already matches | has the chosen version, no write |
| will be replaced | has a different version |
| will be added | has no copy |

**The summary undercounted.** *"2 writes, serving 2 harnesses"* beside five
ticked boxes looked like three had been dropped. It now reads
*"2 writes · 2 harnesses change · 3 already match"*, so every ticked box is
accounted for.


**A pill is not a marker.** T-5.21 first rendered "deprecated" as a bordered
badge. Six of seven skills on this machine sit in a deprecated location, so six
badges truncated the names the column exists for — the flag was louder than the
thing it was flagging. It is now a 5px dot with the path in its tooltip, and the
legend teaches it. The repetition itself was the signal that the badge was
wrong: information that is true of nearly every row belongs in `doctor`, said
once, not stamped on each line.
