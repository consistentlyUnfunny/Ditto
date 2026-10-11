# TODO

Tasks carry IDs (`T-n.n`) and cite the requirement they satisfy. Checked when the
cited acceptance test passes, not when the code compiles.

**Read first:** [DESIGN.md](DESIGN.md) → [docs/RESEARCH.md](docs/RESEARCH.md) →
[docs/PROVIDER-SPEC.md](docs/PROVIDER-SPEC.md) → [docs/REQUIREMENTS.md](docs/REQUIREMENTS.md)

---

## M0 — Toolchain ✅ done 2026-09-15

- [x] **T-0.1** VS Build Tools 2022, MSVC 14.44.35207 + Windows 11 SDK
- [x] **T-0.2** rustup 1.29.1 / rustc 1.98.1 / cargo 1.98.1, `stable-x86_64-pc-windows-msvc`
- [x] **T-0.3** Compile + link verified with a throwaway binary
- [x] **T-0.4** Research all five harnesses → `docs/RESEARCH.md`
- [x] **T-0.5** Provider spec format v1 → `docs/PROVIDER-SPEC.md`
- [x] **T-0.6** Requirements → `docs/REQUIREMENTS.md`
- [x] **T-0.7** Draft all five provider specs → `providers/*.provider.toml`

---

## M1 — Read-only inventory  *(substantially complete)*

Goal: see everything, touch nothing. Proves the spec format survives real files.

**Status 2026-09-15:** engine + CLI working against the dev machine. 73 tests,
`cargo clippy -- -D warnings` clean. `status`, `list skills`, `list mcp`, `drift`,
`providers` and `validate` all implemented. Remaining: T-1.2 (CI), T-1.18 (`show`),
T-1.19 (`--json`), T-1.23 (hazard regression tests).

### Scaffolding
- [x] **T-1.1** Cargo workspace: `inception-core` (lib, no Tauri dep), `inception-cli` (bin). *NF-5.1*
- [ ] **T-1.2** `git init`, `.gitignore`, CI running `cargo test` + `cargo clippy -- -D warnings`. *NF-5.2*
- [x] **T-1.3** Error type. No `unwrap` outside tests. *NF-5.5*

### Spec loading
- [x] **T-1.4** Deserialize a provider spec; reject unknown keys via `#[serde(deny_unknown_fields)]`. *F-1.1, F-1.3*
- [x] **T-1.5** Embed built-in specs in the binary; load user overrides from `~/.inception/providers/`. *F-1.2*
- [x] **T-1.6** `inception validate` covering every rule in PROVIDER-SPEC §9. *F-1.3*
- [x] **T-1.7** All five built-in specs validated by a unit test (wire into CI with T-1.2). *F-1.5*
- [x] **T-1.8** Path placeholder expansion: `{home} {user_home} {config} {project}`, `~/`, env overrides. *F-2.3, NF-3.5*

### Reading
- [x] **T-1.9** Format-agnostic tree: parse TOML / JSONC / YAML into one traversable model.
- [x] **T-1.10** JSON Pointer resolution incl. `~0`/`~1` escaping and `{project_path}` substitution. *NF-3.3*
  - ⚠️ Highest-risk item in M1. Windows paths as object keys. Test first.
- [x] **T-1.11** Candidate path resolution — read all that exist, first-wins. *F-2.2*
- [x] **T-1.12** Provider detection. *F-2.1*
- [x] **T-1.13** Distinguish absent / zero-byte / `null`. *F-2.4*
- [x] **T-1.14** MCP reader: transport strategies (`type-value`, `key-presence`, `url-presence`), field map, `argv-array` splitting, `extra` capture. *F-3.4*
- [x] **T-1.15** Skill scanner: walk dirs, apply `exclude`, parse frontmatter preserving unknown keys. *F-3.2, F-3.3*

### Commands
- [x] **T-1.16** `inception status`. *F-8*
- [x] **T-1.17** `inception list [skills|mcp]`. *F-3.1*
- [ ] **T-1.18** `inception show <name>` — canonical form + per-provider rendering.
- [ ] **T-1.19** `--json` on all read commands. *F-8.1*

### M1 acceptance
- [x] **T-1.20** Finds all five harnesses, incl. Gemini's post-migration layout. *MVP #1*
- [x] **T-1.21** Lists 7 user skills, excludes the 6 in `.system/`, reports the empty `codex-primary-runtime` dir. *MVP #2*
- [x] **T-1.22** Reads all three MCP servers (`notion` local-scope via enumerated pointer, `node_repl` w/ 12 env vars + preserved `startup_timeout_sec`, `obsidian` argv-array split). *MVP #2*
- [x] **T-1.23** Regression test per hazard in RESEARCH §4. *NF-5.3*

---

## M2 — Round-trip fidelity ⭐ **the moat**  *(complete bar T-2.4)*

Goal: prove we can write without wrecking. **Do not skip ahead to the GUI.**
This is the milestone that makes the project different from five existing apps.

**Status 2026-09-16:** 111 tests, clippy clean. `Editor` wraps `toml_edit` and the
`jsonc-parser` CST; byte-identical round-trip holds over an 11-file corpus of real
configs plus ~660 generated perturbations. The competitive claim is pinned by an
executable head-to-head test. Only T-2.4 remains, and it is blocked on external
information (Q-1).

- [x] **T-2.1** Corpus: copy every real config off the dev machine into `tests/corpus/`, secrets scrubbed. Add hand-built edge cases.
- [x] **T-2.2** Property test — parse → emit unchanged ⇒ **byte-identical**, all formats. *NF-1.1, NF-1.2*
- [x] **T-2.3** TOML writer on `toml_edit`. Literal `'...'` strings stay literal. *NF-1.4*
  - Acceptance: round-trip `~/.codex/config.toml` with its `[projects.'D:\...']` keys byte-identically.
- [ ] **T-2.4** ⚠️ **Resolve the Gemini open question.** Populate a real `mcp_config.json` and confirm its schema before any write path targets it. Until then the fallback candidate is read-only. *See `gemini-cli.provider.toml`*
- [x] **T-2.5** JSONC writer on `jsonc-parser` CST. Comments in `.json` survive. *NF-1.5*
- [~] **T-2.6** YAML writer — **deliberately not built.** No provider declares a
  YAML config file, and skills deploy by copying the directory rather than
  rewriting frontmatter, so nothing needs one. `Editor::parse` refuses YAML with
  an explicit error rather than silently mangling it. Revisit only if a provider
  spec needs it.
- [x] **T-2.7** Surgical upsert: adding one entry touches only that entry's lines. *NF-1.3*
- [x] **T-2.8** Unknown-field preservation, all three rules in PROVIDER-SPEC §6. *F-3.4*
  - Rule 3 (never strip a field an update didn't set) is the one that will bite.
- [x] **T-2.9** Line endings preserved per file. *NF-3.4*
- [x] **T-2.10** Unparseable file ⇒ report and abort, never write. *NF-1.6*
- [x] **T-2.11** Atomic write + backup to `~/.inception/backups/`. *NF-2.1, NF-2.2*
- [x] **T-2.12** Fuzz the writers against the corpus.

---

## M3 — Deploy  *(core complete; provenance and import outstanding)*

**Status 2026-09-16:** 178 tests, clippy clean. Emitter, capability gates,
minimum-write-set planner, diff preview, confirm-then-apply, backups, removal and
`doctor` all working against the dev machine. `deploy`, `remove` and `doctor` are
wired into the CLI.

Outstanding: the **lockfile** half of provenance (T-3.9, T-3.10, T-3.15).
The design question these hung on — where an *imported* artifact lives once it
is no longer just "the copy in Codex's folder" — was **answered on 2026-10-10**
and built in M7. `import` exists; the lockfile is now unblocked.

- [x] **T-3.1** Emit canonical → provider shape (inverse of T-1.14).
- [x] **T-3.2** Capability check; hard error on unsupported transport. *F-5.4*
- [x] **T-3.3** Report dropped `extra` fields per target. *F-5.5*
- [x] **T-3.4** Minimum-write-set planner, per PROVIDER-SPEC §7 selection rule. *F-5.2*
- [x] **T-3.5** Diff preview + confirmation. *F-5.3*
- [x] **T-3.6** Skill deploy — copy dir incl. `scripts/`, `references/`, `assets/`. *F-5.1, F-5.6*
- [x] **T-3.22** **Convergence.** A deploy overwrites every divergent copy a
  target also reads, instead of installing alongside it. Found by using the GUI:
  deploying `karpathy-coder` wrote it to `~/.agents/skills` but left OpenCode's
  private copy in place, so OpenCode saw two versions and the matrix reported
  everything as drifted. Pinned by
  `deploying_overwrites_a_divergent_copy_the_target_also_reads`.
- [x] **T-3.23** Skill writes that replace an existing copy show a diff of what
  they destroy, in both the CLI and the GUI.
- [x] **T-3.7** MCP deploy.
- [x] **T-3.8** Surgical removal. *F-5.7*
- [ ] **T-3.9** Lockfile `~/.inception/lock.toml`. *F-6.1, F-6.2* — unblocked by M7; the library is where a managed artifact comes from
- [ ] **T-3.10** Detect edited-in-place managed artifacts; offer re-import / overwrite / leave. *F-6.3* — unblocked by M7
- [x] **T-3.11** Partial-failure reporting. *NF-2.4*
- [~] **T-3.12** Drift detection + diff. *F-4.1–F-4.3* — **skills only.** F-4.4
  (same-named MCP servers whose canonical form differs) was never implemented;
  this was ticked in error on 2026-09-16 and corrected on 2026-09-20.
- [~] **T-3.13** Detection, keychain store and `Ref` resolution at emit are **done** (`secrets.rs`). `import` now exists for **skills** (M7); promoting a literal still waits on MCP servers being library artifacts (Q-12). *F-7.1–F-7.3*
- [x] **T-3.14** Redaction everywhere. *F-7.4*
- [ ] **T-3.15** Prefer native indirection (`bearer_token_env_var`). *F-7.5* — needs a spec field to declare it; not expressible in schema v1
- [x] **T-3.16** `inception doctor`. *F-8, NF-4.4*

### M3 acceptance — the full MVP bar
- [x] **T-3.17** `drift` reports `karpathy-coder` drifted, `minimalist-ui` identical. *MVP #3, F-4.5*
- [x] **T-3.18** `doctor` flags deprecated `~/.codex/skills` and the plaintext Obsidian key. *MVP #4*
- [~] **T-3.19** `deploy obsidian --to codex,claude-code` writes both, and the `config.toml` diff contains **only** the new `[mcp_servers.obsidian]` block — verified on the dev machine. The keychain half waits on MCP import (Q-12). *MVP #6*
- [x] **T-3.20** `deploy karpathy-coder --to all` ⇒ exactly 2 writes, with stated reason. *MVP #7*
- [x] **T-3.21** Every write reversible from backups. *MVP #8*

---

## M4 — Tauri GUI  *(matrix, drift, deploy and doctor working)*

**Status 2026-09-20:** Tauri 2.11 shell in `crates/inception-gui`, frontend in
`ui/` (Vite + vanilla TypeScript, zero runtime dependencies). Fonts bundled
locally, so the app makes no network requests. Build with
`cd crates/inception-gui/ui && npm run build`, then `cargo run -p inception-gui`.

**Verified by running it:** the matrix renders the real machine — five columns,
seven rows, the `karpathy-coder` drift staple and the `codex-primary-runtime`
problem marker. **Not yet verified interactively:** the detail drawer, deploy
preview and apply flow are wired and type-check, but no one has clicked them.

- [x] **T-4.1** Tauri shell over `inception-core`; zero duplicated logic. *F-9.1*
- [x] **T-4.2** Inventory view grouped by skill/server, showing presence per provider.
- [x] **T-4.3** Drift view with side-by-side diff.
- [x] **T-4.4** Deploy flow with the same preview + confirm as the CLI. *F-9.2*
- [x] **T-4.7** Drift drawer shows a real line-by-line diff of the two copies.
- [ ] **T-4.5** File viewer/editor, diff before save. *F-9.3* — the drawer shows content read-only; editing is not built
- [ ] **T-4.6** Windows installer; verify on a clean machine. — needs the Tauri CLI (`cargo install tauri-cli`), not installed

---

## M5a — GUI usability  *(all four phases done 2026-09-21)*

Full review with reasoning in [docs/USABILITY.md](docs/USABILITY.md). Root cause:
the UI shows the engine's verb ("deploy") instead of the user's intent, and the
decision at the heart of resolving drift — **which version wins** — is not in the
interface at all. The planner currently picks the first copy in path order.

### Phase 1 — unbreak  *(done 2026-09-21)*
- [x] **T-5.10** `detail` resolves MCP servers too; server rows currently error.
- [x] **T-5.11** Fix the caption that still claims deploy only adds, never replaces.
- [x] **T-5.12** Problem rows: explain, offer Show in folder, no deploy button.
- [x] **T-5.13** Refresh control; re-read after apply.

### Phase 2 — intent and choice  *(done 2026-09-21)*
- [x] **T-5.14** Engine: explicit source for `skill_deploy`; `inventory::versions()` grouping by hash.
- [x] **T-5.15** CLI: `deploy --from <provider>`. Picks that harness's *own*
  copy in preference to a shared one — "OpenCode's version" must mean the copy in
  `~/.config/opencode/skills`, not whatever happens to sit in `~/.agents/skills`
  that OpenCode also reads. Pinned by
  `source_for_prefers_a_providers_own_directory_over_a_shared_one`.
- [x] **T-5.16** GUI: state-specific primary action, with a count in the label.
- [x] **T-5.17** GUI: Resolve flow — pick the winning version, then see the cost.
- [x] **T-5.18** GUI: Add flow — target checkboxes showing the shared write set.

### Phase 3 — diff clarity  *(done 2026-09-21)*
- [x] **T-5.19** Separate comparison rendering from change rendering; drop +/- for comparisons.
- [x] **T-5.20** Remove the paper cards the comparison supersedes. *(landed early, in Phase 1; dead CSS cleaned up in Phase 3.)*

### Phase 4 — polish  *(done 2026-09-21)*
- [x] **T-5.21** Deprecated-path flag on matrix rows — a 5px amber dot, not a
  badge. The first attempt used a "deprecated" pill, and with six of seven
  skills in a deprecated location six repeated pills truncated the names the
  column exists for. `doctor` states it once, in words; the matrix marks it.
- [x] **T-5.22** Row click affordance.
- [x] **T-5.23** Human vocabulary instead of DriftStatus names.
- [x] **T-5.24** Actionable doctor findings.
- [x] **T-5.25** Drawer back stack.

---

## M6 — Project scope  *(2026-09-22)*

Paths verified against primary docs and recorded in
[docs/RESEARCH.md](docs/RESEARCH.md) §2 and §6. The specs were corrected in the
same pass — they had declared only canonical project directories, which would
have made the planner commit five skill directories to a repo instead of two.

- [x] **T-6.1** Research project-scope skill and MCP paths; correct all five specs.
- [x] **T-6.2** Path normalisation — case-fold and normalise separators before
  comparing or deduplicating project paths. Windows records the same repo several
  ways, and the dev machine already has two entries for this one.
- [x] **T-6.3** Project discovery from `~/.claude.json` projects and Codex's
  `[projects.'...']` trust table, filtered to directories that still exist.
  Which files, and where in them, comes from `[projects]` in the specs. Surfaced
  as `inception projects`; ten found on the dev machine, the duplicate casings of
  this repo correctly collapsed into one.
- [x] **T-6.4** Scope picker in the GUI title bar; `--project` already exists on
  every CLI read command. The title bar names the project and changes colour,
  because the consequences of a write differ between the two scopes.
- [x] **T-6.5** Scope transfer: "also add to this project" / "also add at user
  scope". The operation that makes project scope worth having. Implemented as an
  ordinary deploy whose source sits in the other scope — same planner, same
  preview, so the transfer cannot disagree with a deploy.
- [x] **T-6.6** Git awareness in the preview: each write says when it lands
  inside a working tree, and names the repository. It does **not** claim whether
  `.gitignore` covers it — a partial gitignore implementation would be worse than
  an honest silence. Note that a dotfiles home is a working tree too, and the dev
  machine has `C:/Users/User/.git`, so this fires at user scope there as well.
- [x] **T-6.7** `source_for` follows the spec's declared precedence rather than a
  shared/private heuristic. Gemini reads `.agents/skills` *before* its own
  directory, so the old rule answered "Gemini's version" backwards for one of the
  five. A deprecated directory stays the last resort regardless of position.
- [x] **T-6.8** Doctor: flag a repo where **one harness loads two**
  standing-instruction files. Stated per harness rather than as "AGENTS.md and
  CLAUDE.md", because Claude Code now reads both — so the problem is not that the
  two files exist, it is that one agent is handed both. Declared as data
  (`[[instructions]]`) so no filename enters the engine (F-1.1). These files are
  never read, written or merged; see Q-10.
- [x] **T-6.9** *(found while building M6)* `installed` is a fact about the
  machine, not about the project you are looking at. Judged per scope, a fresh
  repository reported no harnesses at all and offered the user nothing to do.
- [x] **T-6.10** *(found while building M6)* Codex's project `.agents/skills` was
  not marked `shared`, so a project deploy planned **three** writes instead of
  two — the same class of mistake T-6.1 was meant to fix, one scope down.
- [x] **T-6.11** *(found while building M6)* The planner emitted writes for
  destinations that already held exactly the same content, so a repeat deploy
  looked like fresh work and `apply` quietly reported "already up to date". Now
  dropped, alongside the self-copy case the planner already filtered — which is
  what makes "nothing to do" mean something.
- [x] **T-6.12** *(found while building M6)* A tab click in the GUI registered
  another listener on Refresh, so after *n* switches one click fired *n*+1 scans.

---

## M7 — The library and recency  *(2026-10-10)*

Answers **Q-7**. The library is the one thing that makes "which copy is
canonical?" answerable: `inventory::default_source` is the single place the
engine has to answer it, and before this its answer was "the first readable copy
in path order" — arbitrary, which is why the UI has to *show* the winner.

The shape matters more than the feature. The library is **a location, not a
mode**: `use_library = false` stops one directory being scanned, and every
ranking then falls through to the behaviour that shipped before it. A mode flag
would have meant a second set of rules in the planner, in drift detection and in
the resolve flow — four more places for the preview and the write to disagree,
which is the class of bug every one of M6's five findings belonged to.

- [x] **T-7.1** `~/.inception/config.toml` and `Settings` (`settings.rs`).
  `use_library` defaults on; `deny_unknown_fields`, so a typo is an error rather
  than a setting that silently does nothing.
- [x] **T-7.2** `SkillLocation.library`, and `inventory::skills` takes the
  library directory. Passed in rather than read from settings inside the engine,
  so the scan stays a function of what it is handed and tests can point it at a
  temporary directory.
- [x] **T-7.3** `default_source` prefers the library. *F-6.5*
- [x] **T-7.4** `source_for` still never returns it — the library has no
  `providers`, so it falls out of that filter without a special case. "OpenCode's
  version" must keep meaning the copy OpenCode reads (T-5.15).
- [x] **T-7.5** `plan::skill_import`, an ordinary `Step::Skill` write, so import
  inherits the preview, the replacement diff, backups and `apply` rather than
  growing its own. *F-6.6*
- [x] **T-7.6** `inception import <name> [--from <harness>]`.
- [x] **T-7.7** GUI: library column in the matrix, kept outside the harness
  track — the tie geometry divides that track by the number of harnesses, so
  folding the library in would shift every staple by half a column. Plus an
  "Add to library" action in the drawer.
- [x] **T-7.8** The library is user scope only, but still a *source* at project
  scope, so a transfer into a repository comes out of it. *F-6.7*
- [x] **T-7.9** Doctor names the library when it is the side that differs.
  "differs between codex" says nothing about what the other side is.
- [x] **T-7.10** **Recency as a label.** `SkillEntry.modified` (newest mtime in
  the directory, matching what the planner already compares), `when::relative`
  for the words, shown in `list`, `drift`, the version picker and the
  comparison header. Never consulted when choosing. *F-6.8*

### Found by using it (2026-10-11)

- [x] **T-7.11** The library switch existed only in `~/.inception/config.toml`,
  so it was a setting you had to already know about to find. The column header
  now opens a panel that explains the library and toggles it. `Settings` gained
  `save()`, going through the same atomic write as everything else — a
  half-written settings file would lock you out of your own tool.
- [x] **T-7.12** **You can add a project.** Discovery reads what a *harness*
  recorded, which by definition cannot include a repo you have never opened an
  agent in — exactly the repo you would open the picker to set up. Hand-added
  paths live in `settings.projects`, deduplicated with `projects::key` so one
  repo cannot appear twice under two Windows spellings.
  `inception projects --add <dir>` / `--forget <dir>`, plus an input and a
  remove control in the picker.
  - Only a hand-added project can be removed. A harness's record is that
    harness's config; dropping it from our list would make the row vanish and
    then come back on the next scan, so `forget` says so rather than no-op.
  - Kept in our own settings rather than written into a harness's project
    table: editing another tool's records to store ours would plant a trust
    grant the user never asked for. The `C:\Users\User` entry found in M6 is
    what that looks like from the other side.

- [x] **T-7.13** Native folder browser on the project picker, via
  `tauri-plugin-dialog`. Driven from **Rust**, not from the plugin's JS
  binding, for two reasons: the frontend keeps its zero-dependency build, and
  the plugin's command is never exposed to the webview, so nothing running in
  the window can open file dialogs on its own. The picker call sits inside
  `spawn_blocking` — the blocking picker parks its thread until the user
  answers, and the main thread is the one the dialog needs in order to appear.
  Choosing a folder adds it immediately; cancelling does nothing.

### Deliberately not done

- **Auto-import.** A deploy reads the library and never writes to it. Adoption
  is a decision; if deploying filled the library, it would collect copies nobody
  chose and "canonical" would mean "whatever I touched last".
- **MCP servers in the library.** See Q-12 — this needs a decision, not just
  code.
*(The native folder browser listed here as skipped was built the same day —
see T-7.13.)*
- **`~/.inception/` as cache.** F-6.4 was amended rather than preserved. The
  library is data; pretending otherwise would promise a rescan can rebuild
  something it cannot.

---

## M5 — Later

- [ ] **T-5.1** Provider spec editor with live validation. *F-9.4*
- [ ] **T-5.2** Skill migration assistant (`~/.codex/skills` → `~/.agents/skills`).
- [ ] **T-5.3** Project-scope management.
- [ ] **T-5.4** `schema_version = 2`: hooks, commands, subagents, plugins.
- [ ] **T-5.5** Marketplace browsing — separate crate, the only component allowed network access. *NF-4.2*
- [ ] **T-5.6** Bidirectional sync. Deliberately last; the hardest to get right.

---

## Open questions

Tracked rather than guessed at. Each needs an answer before the milestone that depends on it.

| # | Question | Blocks | Notes |
|---|---|---|---|
| Q-1 | Schema of Gemini's post-migration `config/mcp_config.json` | T-2.4 | File is 0 bytes on the dev machine. Do not write to it until confirmed. |
| Q-2 | Lockfile format — TOML vs SQLite | T-3.9 | Start with TOML; revisit only if it gets slow, which it won't. |
| Q-3 | Does any harness besides Claude Code namespace project config inside the user file? | T-1.10 | Only Claude Code found so far. If it stays unique, the dynamic-pointer machinery is one provider's tax. |
| Q-4 | Is `~/.agents/skills` genuinely stable, or still a moving community proposal? | T-3.4 | The whole minimum-write-set advantage rests on it. Re-check before M3. |
| Q-5 | Project-scope UX — explicit `--project` flag, or infer from cwd? | M5 | Inferring is friendlier and more dangerous. |
| Q-6 | Should `deploy` offer to migrate deprecated paths, or only `doctor` report them? | T-5.2 | Migration mutates state the user didn't ask about. Lean report-only. |
| Q-11 | Is Gemini CLI sunset for individual users, with Antigravity as successor? | T-6.3 | A search result says 2026-06-18 and that skills migrate to `.agents/skills`; the primary docs say nothing. The dev machine has `~/.gemini/antigravity/`. If true, the `gemini-cli` provider needs renaming or a second spec. |
| Q-10 | Should instruction files (`AGENTS.md`, `CLAUDE.md`) be managed? | — | **Decided 2026-09-22: no.** One filename, one format, nothing to translate — the provider-spec machinery has no work to do. Detected and reported only, via T-6.8. |
| ~~Q-9~~ | **Decided 2026-09-21: overwrite.** Resolving drift replaces a deprecated copy rather than deleting it; `doctor` keeps suggesting migration separately. Deletion stays the one thing the app only does when explicitly asked. | — | — |
| Q-8 | Should the GUI follow the OS theme, or stay committed to the slate-and-paper world? | M4 polish | It currently commits. Theme-following would mean designing a light chassis that keeps "paper = file content" meaningful. |
| ~~Q-7~~ | **Decided 2026-10-10: a managed library at `~/.inception/library`, switchable off.** Built in M7. The library is a *location*, not a mode — turning it off removes a directory from the scan rather than selecting different rules. The "stateless translator" argument lost on a fact rather than on taste: `~/.inception/backups/` already held non-derivable data, so the purity being defended was gone before the question was asked. | — | — |
| Q-12 | Should an **MCP server** be a library artifact too, and in what form? | T-3.13, MVP #6 | Skills map onto a library directory unchanged. A server does not: `McpServer.extra` is deliberately per-provider ("restored only to its own harness"), so a canonical library entry needs either per-provider extras or a rule for dropping them. That is a schema decision, which is why M7 stopped at skills. Until it is answered, promoting a literal secret on import has nowhere to live. |

---

## Risks

| Risk | Impact | Mitigation |
|---|---|---|
| A harness can't be expressed declaratively | Kills the core premise | All five fit v1 with no escape hatch. Re-test the moment a sixth is added; extend the schema with a named strategy rather than adding scripting. |
| `~/.agents/skills` doesn't stick | Loses the minimum-write-set edge | Q-4. Falls back to per-provider copies, which is just parity. |
| Harness formats change under us | Specs go stale | Specs are data — a fix is a text edit, not a release. This is the architecture paying off. |
| Owner can't maintain Rust unaided | Bus factor | Keep code plain and commented (*NF-5.4*); domain knowledge in specs, not code. |
| Scope creep toward "nicer dashboard" | Competes head-on with 4.7k★ | M2 before M4, always. |
