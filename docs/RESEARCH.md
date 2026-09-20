# Harness research — verified ground truth

Compiled 2026-09-15. Sources: official docs (linked inline), HarnessKit's adapter
source (read directly — it carries tests), and this machine's real config files.

Anything marked **[verified: machine]** was read off disk, not from docs.

---

## 1. MCP server configuration

### Where each harness stores MCP servers

| Harness | File | Format | Key path | Notes |
|---|---|---|---|---|
| Claude Code (user) | `~/.claude.json` | JSON | `/mcpServers` | top level = all projects |
| Claude Code (local) | `~/.claude.json` | JSON | `/projects/<abs-path>/mcpServers` | **dynamic pointer** — namespaced per project |
| Claude Code (project) | `<repo>/.mcp.json` | JSON | `/mcpServers` | committed to git |
| Codex | `~/.codex/config.toml` | TOML | `[mcp_servers.<name>]` | project: `<repo>/.codex/config.toml` |
| OpenCode | `~/.config/opencode/opencode.jsonc` **or** `.json` | JSONC | `/mcp` | jsonc wins if both exist |
| Gemini CLI | `~/.gemini/settings.json` | JSON | `/mcpServers` | project: `<repo>/.gemini/settings.json` |
| Copilot CLI | `~/.copilot/mcp-config.json` | JSONC | `/servers` | **NOT `mcpServers`** — easy to get wrong |

`COPILOT_HOME` overrides `~/.copilot`; `CODEX_HOME` overrides `~/.codex`.

### Field-level differences — this is the translation problem

| Concept | Claude Code | Codex | OpenCode | Gemini CLI | Copilot CLI |
|---|---|---|---|---|---|
| command | `command` (string) | `command` (string) | `command` (**argv array**, cmd+args merged) | `command` (string) | `command` (string) |
| args | `args` | `args` | *(inside `command`)* | `args` | `args` |
| env vars | `env` | `env` (**sub-table**) | `environment` | `env` | `env` |
| remote url | `url` | `url` | `url` | `url` / `httpUrl` | `url` |
| headers | `headers` | `http_headers` | `headers` | `headers` | `headers` |
| enable flag | — | — | `enabled` | — | — |
| transport | `type` value | implicit | `type` value | **key name** | `type` value |

### Transport discrimination — three different mechanisms

1. **By `type` value** — Claude Code (`http` / `streamable-http` / `sse` / `ws` / absent ⇒ stdio),
   OpenCode (`local` / `remote`), Copilot (`local` / `http` / `sse`).
2. **By key presence** — Gemini CLI: `url` means SSE, `httpUrl` means streamable HTTP.
   The *key name itself* is the discriminator. A value-mapping table cannot express this.
3. **Implicit** — Codex: presence of `url` ⇒ remote, else stdio. Plus `bearer_token_env_var`.

Claude Code rejects a `url` with no `type`, so emission must always write `type` for
remote entries.

### Harness-specific fields that MUST survive a round-trip

Not representable in any canonical model. Dropping these corrupts the user's setup.

- **Codex** — `startup_timeout_sec` (default 10), `tool_timeout_sec`, `bearer_token_env_var`
- **Gemini** — `timeout` (ms, default 600000), `trust`, `includeTools`, `excludeTools`
- **OpenCode** — `enabled`
- **Claude Code** — `ws` transport, which has no analogue anywhere else

**[verified: machine]** `~/.codex/config.toml` has a real example combining these:

```toml
[mcp_servers.node_repl]
args = []
command = 'C:\Users\User\AppData\...\node_repl.exe'   # single-quoted LITERAL string
startup_timeout_sec = 120

[mcp_servers.node_repl.env]                            # env is a SUB-TABLE, not inline
NODE_REPL_NODE_PATH = 'C:\Users\User\...\node.exe'
```

Note the single-quoted TOML literal strings for Windows paths, and that `env` is a
separate sub-table rather than inline. Both are formatting choices a plain
parse→serialize cannot preserve — see §5 for what measurement showed.

---

## 2. Skills

### The single most important finding

**`~/.agents/skills` is an emerging cross-agent standard — and Claude Code refuses it.**

> "Claude Code does **not** read `~/.agents/skills/`. Use `~/.claude/skills/` instead."
> — <https://code.claude.com/docs/en/skills>

| Harness | Global skill dirs (scan order) | Project skill dir |
|---|---|---|
| Claude Code | `~/.claude/skills` **only** | `.claude/skills` |
| Codex | `~/.agents/skills` (canonical), `~/.codex/skills` (deprecated) | `.agents/skills` (cwd → repo root) |
| OpenCode | `~/.config/opencode/skills`, `~/.agents/skills` | `.opencode/skills` |
| Gemini CLI | `~/.gemini/skills`, `~/.agents/skills` | `.gemini/skills` |
| Copilot CLI | `~/.copilot/skills`, `~/.agents/skills` | `.github/skills` |

Codex also reads `/etc/codex/skills` (system scope). Claude Code additionally loads
skills from plugins (`<plugin>/skills/`) and `--add-dir` paths.

**Product consequence:** for 4 of 5 harnesses, "install everywhere" collapses to
*one write* to `~/.agents/skills`. Only Claude Code needs its own copy. Two writes,
not five. No competitor appears to exploit this.

### SKILL.md frontmatter

Layout is universal — `<skill-name>/SKILL.md` with YAML frontmatter, plus optional
`scripts/`, `references/`, `assets/`.

- **Codex** requires exactly `name` + `description`.
- **Claude Code** technically requires none, but supports ~20 optional fields:
  `when_to_use`, `allowed-tools`, `disallowed-tools`, `disable-model-invocation`,
  `user-invocable`, `argument-hint`, `arguments`, `model`, `effort`, `context`,
  `agent`, `background`, `paths`, `shell`, `hooks`, `metadata`, `license`,
  `compatibility`.

Codex→Claude is lossless. Claude→Codex carries fields Codex ignores but which
**must be preserved verbatim** so the skill still works when copied back.

**[verified: machine]** `~/.codex/skills/.system/skill-creator/SKILL.md` uses nested
YAML in frontmatter (`metadata: { short-description: ... }`). The preservation
requirement is not hypothetical.

---

## 3. This machine's actual state (2026-09-15)

Installed: Claude Code, Codex, OpenCode, Gemini CLI, Copilot CLI.

**Skills**

| Location | Contents |
|---|---|
| `~/.codex/skills` | `design-taste-frontend`, `full-output-enforcement`, `gpt-taste`, `karpathy-coder`, `minimalist-ui` — plus `codex-primary-runtime/`, an **empty directory with no `SKILL.md`**, and a `.system/` dir of 6 Codex-managed skills |
| `~/.config/opencode/skills` | `karpathy-coder`, `minimalist-ui` |
| `~/.claude/skills`, `~/.agents/skills`, `~/.gemini/skills`, `~/.copilot/skills` | absent |

So: **7 real user skills**, 1 empty directory, 6 harness-managed. A scanner that
counts directories reports 12; one that counts `SKILL.md` files silently reports 7
and loses the empty dir. Neither is right — the empty directory must be *reported*
(F-3.3), because it is more likely a half-finished install than an intentional
state.

Two live problems, both perfect acceptance tests:

1. **Real drift.** `karpathy-coder` exists in Codex and OpenCode with *different*
   `description` text — same name, diverged silently. `minimalist-ui` is
   byte-identical in both.
2. **Deprecated path.** Every skill sits in `~/.codex/skills`, which current Codex
   docs no longer list. Migrating to `~/.agents/skills` would expose all six to
   OpenCode, Gemini and Copilot at once.

**MCP servers**

| Where | Server |
|---|---|
| `~/.claude.json` → `projects["C:/Users/User"].mcpServers` | `notion` — `{type: "http", url: "https://mcp.notion.com/mcp"}` (local scope) |
| `~/.codex/config.toml` | `node_repl` — stdio, 12 env vars, `startup_timeout_sec = 120` |
| `~/.config/opencode/opencode.jsonc` | `obsidian` — `{type: "local", command: ["uvx","mcp-obsidian"], environment: {...}}` incl. a plaintext API key |
| Gemini, Copilot | none |

Global `mcpServers` in `~/.claude.json` is `null`, not `{}`.

---

## 4. Parsing hazards found in real files

| Hazard | Evidence |
|---|---|
| `.json` files containing `//` comments | **[verified: machine]** `~/.copilot/config.json` opens with two `//` lines. Parse *all* JSON as JSONC. |
| Zero-byte config files | **[verified: machine]** `~/.gemini/config/mcp_config.json` is 0 bytes. Must not error. |
| Hidden system skill dirs | **[verified: machine]** `~/.codex/skills/.system/` holds Codex-managed skills. Exclude dot-prefixed dirs from user inventory. |
| Harness-managed buckets that are **not** dot-prefixed | **[verified: machine, 2026-09-20]** `~/.claude/skills/synced/<uuid>/` is Claude Code's own sync target, holding its built-in skills (pdf, docx, xlsx, pptx, skill-creator, mcp-builder). A `.*` exclusion does not catch it, and a scanner that misses this reports `synced` as a broken skill with no `SKILL.md`. Named explicitly in `claude-code.provider.toml`. Real skills sit two levels down, so this is a container, not a skill. |
| Layout drift between versions | **[verified: machine]** This machine has `~/.gemini/config/config.json` + `mcp_config.json` (post-migration, `.migrated` marker), *not* the documented `~/.gemini/settings.json`. Specs need ordered candidate paths. |
| TOML literal vs basic strings | **[verified: machine]** `~/.codex/config.toml` uses `'...'` for Windows paths throughout. |
| Missing vs empty vs null | Gemini's mcp file exists but is empty; Claude's global `mcpServers` is `null`. Three distinct "nothing here" states. |

---

## 5. Competitor implementation notes

Read from source: `HarnessKit` (431★), `bridle` (437★), `SkillDock` (565★).

**HarnessKit** — `crates/hk-core/src/deployer.rs:451`, `upsert_mcp_server_toml()`:

```rust
let mut doc: toml::Table = existing.parse::<toml::Table>()?;   // comments dropped here
// ... insert entry ...
atomic_write(config_path, &toml::to_string_pretty(&doc)?)?;    // whole file rewritten
```

`toml::Table` is a `BTreeMap` with no comment representation. They *did* solve this
for JSON — `jsonc-parser` with the `cst` feature at `deployer.rs:2784` — TOML just
never got the same treatment. Confirmed: no `toml_edit` anywhere in their workspace.

**Measured, not assumed.** Running that exact approach (`toml` 1.1) over the corpus
files produces:

| Property | naive round-trip | format-preserving |
|---|---|---|
| comments | **all lost** (2 → 0) | kept |
| key order | alphabetised — the file no longer starts where it started | kept |
| value quoting | normalised, flipping *both* ways: `"C:\\x"` → `'C:\x'` and `'literal'` → `"literal"` | kept |
| array layout | single-line arrays re-wrapped multi-line | kept |
| quoted table keys | **survive** — `[projects.'D:\Downloads\blog-agents']` is not rewritten | kept |

That last row corrects an earlier assumption recorded here: quoted *table keys* do
survive the naive path. The damage is to comments, ordering and value quoting.
Still disqualifying for a tool pointed at a file you hand-maintain, but the claim is
now the measured one. Pinned by
`tests/round_trip.rs::naive_serialize_mangles_the_file_and_our_editor_does_not`,
which fails if either side's behaviour changes.

HarnessKit pins `toml = "0.8"` while this was measured on 1.1; the comment loss is a
type-level certainty in both, the exact quoting behaviour may differ.

They also inject a `_hk_name` key into entries whose names need sanitising: tool
metadata written into the user's config file.

**bridle** — `HarnessConfig::parse_mcp_servers()` returns `Vec<(String, bool)>`,
name + enabled only. It can detect a server, not translate one.

**Worth stealing:** HarnessKit's `McpTransport` modelling and its documented
reasoning about which harnesses can and cannot yield `sse`. That analysis is correct
and hard-won.

---

## Sources

- [Claude Code — Skills](https://code.claude.com/docs/en/skills)
- [Claude Code — MCP](https://code.claude.com/docs/en/mcp)
- [Codex — Build skills](https://learn.chatgpt.com/docs/build-skills)
- [Codex — MCP](https://developers.openai.com/codex/mcp)
- [OpenCode — MCP servers](https://opencode.ai/docs/mcp-servers/)
- [Gemini CLI — MCP servers](https://github.com/google-gemini/gemini-cli/blob/main/docs/tools/mcp-server.md)
- [Copilot CLI — config directory](https://docs.github.com/en/copilot/reference/copilot-cli-reference/cli-config-dir-reference)
- [HarnessKit](https://github.com/RealZST/HarnessKit) · [bridle](https://github.com/neiii/bridle) · [SkillDock](https://github.com/wanghuan9/skilldock)
