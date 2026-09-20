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

Outstanding: the **library/provenance** layer (T-3.9, T-3.10, T-3.13, T-3.15).
These hang together around one design question that has not been answered yet —
where an *imported* artifact lives once it is no longer just "the copy in
Codex's folder". Bolting a lockfile on without deciding that would be guesswork.

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
- [ ] **T-3.9** Lockfile `~/.inception/lock.toml`. *F-6.1, F-6.2* — blocked on the library-location decision (Q-7)
- [ ] **T-3.10** Detect edited-in-place managed artifacts; offer re-import / overwrite / leave. *F-6.3*
- [x] **T-3.11** Partial-failure reporting. *NF-2.4*
- [~] **T-3.12** Drift detection + diff. *F-4.1–F-4.3* — **skills only.** F-4.4
  (same-named MCP servers whose canonical form differs) was never implemented;
  this was ticked in error on 2026-09-16 and corrected on 2026-09-20.
- [~] **T-3.13** Detection, keychain store and `Ref` resolution at emit are **done** (`secrets.rs`); the `import` command that promotes a literal is blocked on Q-7. *F-7.1–F-7.3*
- [x] **T-3.14** Redaction everywhere. *F-7.4*
- [ ] **T-3.15** Prefer native indirection (`bearer_token_env_var`). *F-7.5* — needs a spec field to declare it; not expressible in schema v1
- [x] **T-3.16** `inception doctor`. *F-8, NF-4.4*

### M3 acceptance — the full MVP bar
- [x] **T-3.17** `drift` reports `karpathy-coder` drifted, `minimalist-ui` identical. *MVP #3, F-4.5*
- [x] **T-3.18** `doctor` flags deprecated `~/.codex/skills` and the plaintext Obsidian key. *MVP #4*
- [~] **T-3.19** `deploy obsidian --to codex,claude-code` writes both, and the `config.toml` diff contains **only** the new `[mcp_servers.obsidian]` block — verified on the dev machine. The keychain half waits on `import` (Q-7). *MVP #6*
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
| Q-8 | Should the GUI follow the OS theme, or stay committed to the slate-and-paper world? | M4 polish | It currently commits. Theme-following would mean designing a light chassis that keeps "paper = file content" meaningful. |
| Q-7 | Where does an **imported** artifact live? A managed library under `~/.inception/`, or does the tool stay stateless and always deploy harness-to-harness? | T-3.9, T-3.10, T-3.13 | Everything about provenance, drift resolution and `import` depends on this. A library makes "which copy is canonical?" answerable — `deploy` currently just picks the first readable one and says so. Stateless keeps the tool honest about being a translator, not a store. |

---

## Risks

| Risk | Impact | Mitigation |
|---|---|---|
| A harness can't be expressed declaratively | Kills the core premise | All five fit v1 with no escape hatch. Re-test the moment a sixth is added; extend the schema with a named strategy rather than adding scripting. |
| `~/.agents/skills` doesn't stick | Loses the minimum-write-set edge | Q-4. Falls back to per-provider copies, which is just parity. |
| Harness formats change under us | Specs go stale | Specs are data — a fix is a text edit, not a release. This is the architecture paying off. |
| Owner can't maintain Rust unaided | Bus factor | Keep code plain and commented (*NF-5.4*); domain knowledge in specs, not code. |
| Scope creep toward "nicer dashboard" | Competes head-on with 4.7k★ | M2 before M4, always. |
