# Requirements

Numbered so tests and commits can cite them. `F-` functional, `NF-` non-functional.
Every requirement is written to be checkable — if you cannot write a failing test
for it, it is not a requirement, it is a wish.

Context: personal tool first, portfolio piece second, public adoption a bonus.
Positioning and competitive rationale live in [DESIGN.md](../DESIGN.md); verified
harness facts in [RESEARCH.md](RESEARCH.md); the spec format in
[PROVIDER-SPEC.md](PROVIDER-SPEC.md).

---

## Scope

**In, for v1:** skills and MCP servers, across Claude Code, Codex, OpenCode,
Gemini CLI and Copilot CLI, at user and project scope, on Windows / macOS / Linux.

**Out, for v1:** hooks, slash commands, subagents, plugins, marketplace browsing,
team/multi-device sync, remote provider specs. All deferred behind
`schema_version`, none designed out.

---

## F-1 Provider specs

- **F-1.1** A harness is described entirely by a TOML provider spec. No harness name
  may appear in engine source. *Test: grep the engine crate for `codex`, `opencode`,
  `claude` — zero hits outside `providers/` and test fixtures.*
- **F-1.2** Built-in specs ship embedded in the binary; user specs load from
  `~/.inception/providers/*.provider.toml` and override built-ins by `id`.
- **F-1.3** `inception validate` enforces every rule in PROVIDER-SPEC.md §9.
- **F-1.4** An invalid user spec disables only that provider, with a clear error.
  It never aborts startup or affects other providers.
- **F-1.5** All built-in specs are validated in CI.

## F-2 Discovery

- **F-2.1** Detect which providers are present by probing declared paths, without
  running any harness binary.
- **F-2.2** Try `path` candidates in order; read every candidate that exists.
- **F-2.3** Resolve `[home].env` overrides (`CODEX_HOME`, `COPILOT_HOME`) before
  defaults.
- **F-2.4** Distinguish three empty states and never conflate them: file absent,
  file present but zero bytes, key present but `null`. *All three exist on the dev
  machine — see RESEARCH.md §4.*
- **F-2.5** Report a provider whose config exists at a `deprecated = true` path as
  present, with a migration hint.

## F-3 Inventory

- **F-3.1** List every skill and MCP server found, per provider, per scope, with its
  source file path.
- **F-3.2** Exclude skill directories matching `[skills].exclude` (default `.*`), so
  harness-managed skills like `~/.codex/skills/.system/` never appear as user skills.
- **F-3.3** Parse SKILL.md frontmatter into `name` + `description` + preserved
  remainder. A skill with malformed or absent frontmatter is listed with a warning,
  not skipped.
- **F-3.4** Read MCP servers through the spec's transport strategy and field map,
  capturing unclaimed keys into `extra`.
- **F-3.5** Never execute, source, or evaluate any discovered file.

## F-4 Drift detection

- **F-4.1** Group same-named skills across providers and classify each group:
  `identical` / `drifted` / `unique`.
- **F-4.2** Compare on normalised content — trailing whitespace and line endings
  ignored, everything else significant.
- **F-4.3** Show a readable diff for a drifted group.
- **F-4.4** Detect same-named MCP servers whose canonical form differs, ignoring
  provider-specific `extra` fields.
- **F-4.5** *Acceptance:* on the dev machine, report `karpathy-coder` as **drifted**
  between Codex and OpenCode (the `description` differs) and `minimalist-ui` as
  **identical**.

## F-5 Deployment

- **F-5.1** Deploy a skill or MCP server to one, several, or all detected providers.
- **F-5.2** Compute the **minimum write set** by grouping targets that share a
  `shared = true` directory. *Acceptance: deploying a skill to all five providers
  produces exactly two writes — `~/.agents/skills` and `~/.claude/skills` — and the
  preview states why.*
- **F-5.3** Always show a diff preview and require confirmation before writing.
  `--yes` may skip the prompt, never the diff.
- **F-5.4** Refuse, with an explanation, any deploy needing a capability the target
  lacks. Never downgrade silently.
- **F-5.5** Report every `extra` field dropped in translation, per target.
- **F-5.6** Copy semantics only — no symlinks, no junctions. (Windows symlinks need
  Developer Mode or elevation; junctions don't work for files.)
- **F-5.7** Removal is supported and equally surgical.

## F-6 Provenance

- **F-6.1** Record every managed artifact in `~/.inception/lock.toml`: source,
  content hash, target provider, target path, timestamp.
- **F-6.2** Distinguish **managed** artifacts from ones that were already there.
- **F-6.3** Detect a managed artifact edited in place (hash mismatch) and offer
  re-import, overwrite, or leave-alone. Never silently overwrite a user edit.
- **F-6.4** The lockfile is disposable — deleting it loses management metadata but
  never breaks a harness. Everything is re-derivable by rescanning.

## F-7 Secrets

- **F-7.1** Detect credential-shaped values in `env` and `headers` on import and
  offer to promote them to a `Ref`.
- **F-7.2** Store secret values in the OS keychain (Windows Credential Manager,
  macOS Keychain, Secret Service), never in `~/.inception/`.
- **F-7.3** A `Ref` resolves at emit time, and every deploy that writes a
  credential into a config says so in its preview, naming the variable and the
  file.

  *Revised 2026-09-16.* This originally read "a single secret is never copied
  into multiple provider configs as a literal by a fan-out deploy." **That is not
  achievable.** An MCP server reads its credentials from the harness's own config
  file; a harness with no indirection mechanism must have the literal there or
  the server will not start. Deploying to three harnesses means three plaintext
  copies, unavoidably.

  What the keychain genuinely buys is that *this tool* never becomes another
  place the secret is stored, and that the user is told each time the count on
  disk goes up. Enforced by
  `emit::tests::deploying_a_credential_elsewhere_warns_about_the_extra_copy`.
- **F-7.4** Redact secret values in all output — CLI, logs, diff previews, GUI.
- **F-7.5** If a harness supports indirection natively (Codex
  `bearer_token_env_var`), prefer it over writing a literal.

## F-8 CLI

Engine-first: the GUI is a client of this, and everything is testable headlessly.

| Command | Does |
|---|---|
| `inception status` | which providers are installed, where, what version of layout |
| `inception list [skills\|mcp]` | unified inventory |
| `inception drift` | drift report |
| `inception show <name>` | one artifact, canonical form, per-provider rendering |
| `inception deploy <name> --to <ids>` | diff preview then write |
| `inception remove <name> --from <ids>` | surgical removal |
| `inception import <name> --from <id>` | adopt into the managed library |
| `inception validate [spec]` | spec validation |
| `inception doctor` | config health: deprecated paths, unknown keys, plaintext secrets |

- **F-8.1** `--json` on every read command, for the GUI and for scripting.
- **F-8.2** Exit non-zero on error; never a partial write with exit 0.

## F-9 GUI (M4+)

- **F-9.1** Tauri desktop app over the same engine crate — no logic duplicated.
- **F-9.2** Inventory, drift and deploy views reach parity with the CLI.
- **F-9.3** File viewer/editor for skills and configs, with the diff preview shown
  before any save.
- **F-9.4** Provider spec editor with live validation, so a user can add a custom
  harness without a text editor.

---

## NF-1 Round-trip fidelity — **the moat**

- **NF-1.1** Parse → emit with no semantic change **must** reproduce the input
  bytes exactly, for every supported format.
- **NF-1.2** Enforced by property tests over a corpus of real-world configs, plus
  generated cases.
- **NF-1.3** Writing one entry changes only the lines of that entry. Comments, key
  order, indentation, blank lines and quoting style all survive.
- **NF-1.4** TOML string style is preserved in **both** directions — a literal
  `'...'` stays literal and a basic `"..."` stays basic.
  *Measured: the naive path normalises quoting both ways. Either direction is a
  fail. See RESEARCH §5.*
- **NF-1.5** All JSON is parsed as JSONC. Comments in a `.json` file survive a write.
- **NF-1.6** A config that fails to parse is never written to. Report and abort.

This is where HarnessKit fails (RESEARCH.md §5) and it is the whole reason the
project is differentiated. **Do not defer NF-1 to get to the GUI sooner.**

## NF-2 Write safety

- **NF-2.1** Atomic writes — temp file in the same directory, then rename.
- **NF-2.2** Timestamped backup of every file before first modification, in
  `~/.inception/backups/`.
- **NF-2.3** Never write outside paths declared by a provider spec.
- **NF-2.4** A failed multi-target deploy reports exactly which targets were written
  and which were not. No silent partial success.
- **NF-2.5** Tool-specific metadata is never injected into a harness's config file.
  *HarnessKit writes `_hk_name` keys into user configs; we don't.*

## NF-3 Platform

- **NF-3.1** Windows is a first-class target, developed and tested on it.
- **NF-3.2** No symlink or junction dependency anywhere.
- **NF-3.3** Handle Windows paths in config keys, including JSON Pointer escaping of
  `/` and `~` (PROVIDER-SPEC.md §4).
- **NF-3.4** Preserve each file's existing line endings; never normalise CRLF↔LF.
- **NF-3.5** Handle paths with spaces and non-ASCII characters.

## NF-4 Security

- **NF-4.1** No secret ever written to `~/.inception/` or to logs.
- **NF-4.2** No network access in the engine. (Marketplace is a later, separate,
  clearly-scoped component.)
- **NF-4.3** Never execute anything found in a config or skill directory.
- **NF-4.4** `inception doctor` flags plaintext credentials in harness configs.
  *The dev machine has one: a plaintext API key in `opencode.jsonc`.*

## NF-5 Code quality

- **NF-5.1** Engine logic in a library crate with no Tauri dependency; CLI and GUI
  are thin shells.
- **NF-5.2** `cargo clippy -- -D warnings` clean.
- **NF-5.3** Every parsing hazard in RESEARCH.md §4 has a named regression test.
- **NF-5.4** Rust kept deliberately plain and commented — the owner does not write
  Rust. Domain knowledge belongs in provider specs, not in code.
- **NF-5.5** No `unsafe`.

## NF-6 Performance

- **NF-6.1** Full scan of all providers under 500 ms on the dev machine. Generous —
  this is a few dozen small files, and it is not a hot path.
- **NF-6.2** No background watchers or daemons in v1. Scan on demand.

---

## MVP acceptance — M1 to M3

Done when all of these pass on the dev machine, unattended:

1. `inception status` finds all five installed harnesses, including Gemini's
   non-standard post-migration layout.
2. `inception list skills` shows **7** user skills (5 in Codex, 2 in OpenCode),
   **excludes** the 6 in `~/.codex/skills/.system/`, and **reports** — rather than
   silently dropping — `~/.codex/skills/codex-primary-runtime/`, an empty
   directory with no `SKILL.md`.
   *Corrected 2026-09-15: this criterion originally said 8, counting the empty
   directory as a skill. The scanner found the discrepancy.*
3. `inception drift` reports `karpathy-coder` drifted, `minimalist-ui` identical.
4. `inception doctor` flags the deprecated `~/.codex/skills` path and the plaintext
   Obsidian API key.
5. Round-trip tests pass byte-identically over all real configs on the machine,
   including `~/.codex/config.toml` with its literal strings and
   `~/.copilot/config.json` with its `//` comments.
6. `inception deploy obsidian --to codex,claude-code` writes the server to both,
   promotes the API key to the keychain, and the resulting `config.toml` diff
   contains **only** the added `[mcp_servers.obsidian]` block.
7. `inception deploy karpathy-coder --to all` produces exactly two writes and says
   why.
8. Every write is reversible from `~/.inception/backups/`.
