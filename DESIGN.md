# project-inception — design notes

Status: M0 complete (toolchain + research + spec). No engine code yet.

## Documents

| Doc | What it holds |
|---|---|
| **DESIGN.md** (this file) | positioning, decisions, architecture |
| [docs/RESEARCH.md](docs/RESEARCH.md) | verified harness facts — paths, schemas, hazards |
| [docs/PROVIDER-SPEC.md](docs/PROVIDER-SPEC.md) | the provider spec format v1 + canonical model |
| [docs/REQUIREMENTS.md](docs/REQUIREMENTS.md) | numbered requirements + MVP acceptance |
| [TODO.md](TODO.md) | task breakdown, open questions, risks |
| [providers/](providers/) | the five built-in provider specs |

## Positioning

Not "a dashboard for agent extensions" — that space has real incumbents
(HarnessKit 431*, SkillDock 565*, skills-manager 4.7k*, bridle 437*).

We build **the translation engine they don't have**: harness support defined as
*data*, not compiled code, with format-preserving writes as a hard invariant.

### Why this is the gap (verified by reading their source)

- HarnessKit hardcodes 14 harnesses across ~11.4k lines of hand-written Rust
  adapters (`crates/hk-core/src/adapter/{claude,codex,opencode,...}.rs`).
  bridle uses a `HarnessKind` enum with per-harness match arms.
  **Adding a harness = writing Rust + a PR + a release.** New harnesses ship
  roughly monthly.
- HarnessKit uses `jsonc-parser` with the comment-preserving `cst` feature for
  JSON — correct — but plain `toml = "0.8"` for TOML. `deploy_mcp_server_toml`
  builds a fresh `toml::Table` and writes it back. `toml::Table` has no
  representation for comments. No `toml_edit` anywhere in their workspace.
  **Deploying an MCP server to Codex loses every comment in the file,
  alphabetises its keys and rewrites string quoting.** Measured, not assumed —
  [docs/RESEARCH.md](docs/RESEARCH.md) §5 has the table and names the test that
  pins it.

Their MCP domain model is good (transport discrimination, secret-bearing
headers flagged) — worth learning from, not beating.

### A third gap, found during M0 research

`~/.agents/skills` is an emerging cross-agent standard read by Codex, OpenCode,
Gemini CLI and Copilot CLI. Claude Code explicitly refuses it.

So "install this skill everywhere" is **two writes, not five** — one shared, one
for Claude Code. No competitor exploits this; they all copy per-harness. A tool
that writes twice and explains why is doing something visibly smarter than one
that writes five times. See [docs/RESEARCH.md](docs/RESEARCH.md) §2.

Caveat tracked as Q-4 in [TODO.md](TODO.md): the shared path is a community
convention, not a ratified standard. If it moves, we fall back to per-provider
copies and lose only the edge, not the product.

## Decisions taken

| Decision | Choice |
|---|---|
| Core | Rust |
| GUI | Tauri (deferred until engine is proven) |
| MVP scope | Skills + MCP servers |
| Install model | Copy with provenance tracking (no symlinks — Windows-clean) |

## Hard invariants

1. **Byte-identical round-trip.** Parse -> emit with no semantic change MUST
   produce the input bytes exactly. Enforced by property tests over a corpus of
   real configs. This is the thing incumbents fail.
2. **Surgical writes.** Adding one MCP server touches only the lines for that
   server. Comments, key order, quoting style, indentation all survive.
3. **No silent secret fan-out.** A deploy that writes a credential into a config
   says so, naming the variable and the file. (Avoiding the copy entirely is not
   possible — the harness needs the literal to start the server. See
   [REQUIREMENTS](docs/REQUIREMENTS.md) F-7.3.)
4. **Never silently overwrite.** Show the diff before any write.

## Architecture

```
provider specs (data)  ->  [ parse ]  ->  canonical model  ->  [ emit ]  ->  provider configs
     *.provider.toml                       Skill / McpServer                  surgical, format-preserving
```

### Canonical model

Authoritative definition: [docs/PROVIDER-SPEC.md](docs/PROVIDER-SPEC.md) §1.
In outline — `McpServer` and `Skill`, each carrying an untyped bag of
harness-specific fields that must survive a round-trip verbatim:

```rust
struct McpServer {
    name, transport,             // Stdio | Http | Sse | Ws  (ws is Claude Code only)
    command, args,               // stdio
    url, headers,                // remote
    env:   BTreeMap<String, Secret>,
    enabled: bool,
    extra: Map<String, Value>,   // startup_timeout_sec, trust, timeout, ...
}

struct Skill {
    name, description,
    frontmatter: Map<String, Value>,   // everything else, preserved verbatim
    body: String,
    files: Vec<RelPath>,               // scripts/, references/, assets/
}
```

`Secret` is `Literal(String)` or `Ref(name)`, resolved from the OS keychain at
emit time. Import offers to promote literals to refs.

The `extra` / `frontmatter` bags are not a convenience — they are a correctness
requirement. Real configs carry `startup_timeout_sec = 120` and Claude Code
defines ~20 optional frontmatter fields. Dropping any of them on a round-trip
corrupts the user's setup.

### Provider spec — the differentiator

A harness is a TOML manifest, not Rust. The format is specified in full in
[docs/PROVIDER-SPEC.md](docs/PROVIDER-SPEC.md); the five built-in specs live in
[providers/](providers/).

Custom providers are therefore a first-class feature, not an escape hatch: a user
writes (or the GUI generates) one of these files, and no release is needed.

**The format was validated by writing all five specs before any engine code.**
Every harness fits without a scripting escape hatch, including the three awkward
cases that motivated the design:

- OpenCode's merged `command: ["uvx","mcp-obsidian"]` → `form = "argv-array"`
- Gemini's key-name-as-discriminator (`url` = SSE, `httpUrl` = HTTP) →
  `strategy = "key-presence"`
- Claude Code's project-namespaced pointer `/projects/{project_path}/mcpServers`
  → placeholder substitution with JSON Pointer escaping

That last one is the sharpest edge in the whole format: Windows paths used as
object keys contain `/`, which must become `~1` or the pointer silently addresses
the wrong node.

## Proof case — the MVP must do exactly this

Real state on this machine, 2026-09-15:

- `obsidian` MCP server exists only in `opencode.jsonc`, with a plaintext API key.
- `karpathy-coder` + `minimalist-ui` are hand-duplicated in `~/.codex/skills/`
  and `~/.config/opencode/skills/` (drift risk, already).
- `design-taste-frontend`, `full-output-enforcement`, `gpt-taste`,
  `codex-primary-runtime` are Codex-only.
- Claude Code has no global skills dir at all.

MVP success = import `obsidian` from OpenCode, promote its key to a SecretRef,
deploy to Claude Code (`~/.claude.json` -> `mcpServers`) and Codex
(`~/.codex/config.toml` -> `[mcp_servers.obsidian]`), and have
`~/.codex/config.toml` diff show *only* the added table — every other line,
its ordering and its quoting byte-identical.

The byte-identity half is already enforced by `tests/round_trip.rs` as of M2.

## Milestones

Full task breakdown with IDs, acceptance criteria, open questions and risks lives
in [TODO.md](TODO.md).

- **M0 — toolchain + research. DONE (2026-09-15).** VS Build Tools 2022 (MSVC
  14.44.35207 + Windows 11 SDK), rustup 1.29.1 / rustc 1.98.1 / cargo 1.98.1 on
  `stable-x86_64-pc-windows-msvc`; compile+link verified. WebView2 152.0.4191.66
  and Node 22.18.0 already present. All five harnesses researched, spec format
  drafted and validated against them.
- **M1 — read-only inventory.** Discover every provider, parse skills + MCP, print
  a unified table. No writes. Proves the spec format survives real files.
- **M2 — round-trip fidelity.** Corpus of real configs, property test asserting
  byte-identical re-emit. `toml_edit` + `jsonc-parser` CST + YAML.
  **This milestone is the moat. Do not skip it to reach the GUI.**
- **M3 — deploy.** Copy with provenance, diff preview, minimum write set, secrets
  to the OS keychain, drift detection.
- **M4 — Tauri GUI** over the proven engine.
- **M5 — custom provider authoring, migration assistant, marketplace.**

## Open questions

Tracked as Q-1 … Q-6 in [TODO.md](TODO.md#open-questions). The one that matters
most: **Q-4 — is `~/.agents/skills` a stable standard or a moving proposal?** The
minimum-write-set advantage rests on it.
