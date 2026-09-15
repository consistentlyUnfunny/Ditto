# Provider Spec v1

A **provider spec** describes one agent harness as data. The engine reads specs at
runtime; adding a harness means shipping a `.provider.toml` file, not writing Rust
and cutting a release.

This is the project's central bet. Every competitor hardcodes harnesses in compiled
code — HarnessKit carries ~11.4k lines of hand-written adapters across 14 files. If
a spec can express every harness in [RESEARCH.md](RESEARCH.md), that entire category
of work becomes a data file a contributor can write in twenty minutes.

Specs are TOML. They live in `providers/` (built-in) and `~/.inception/providers/`
(user-supplied; user specs override built-ins by `id`).

---

## 1. The canonical model

Everything a spec does is map a harness's file format onto these two types, and back.

```rust
struct McpServer {
    name:      String,
    transport: Transport,               // Stdio | Http | Sse | Ws
    command:   String,                  // stdio only
    args:      Vec<String>,             // stdio only
    env:       BTreeMap<String, Secret>,
    url:       Option<String>,          // remote only
    headers:   BTreeMap<String, Secret>,
    enabled:   bool,                    // default true
    extra:     Map<String, Value>,      // see §6 — harness fields we don't model
}

struct Skill {
    name:        String,
    description: String,
    frontmatter: Map<String, Value>,    // everything else, preserved verbatim
    body:        String,                // markdown below the frontmatter
    files:       Vec<RelPath>,          // scripts/, references/, assets/
}
```

`Secret` is `Literal(String)` or `Ref(name)`, resolved from the OS keychain at emit
time. See [REQUIREMENTS.md](REQUIREMENTS.md) §NF-4.

Five harnesses means five readers and five writers — not twenty-five pairwise
converters. Same reason a compiler has an IR.

---

## 2. Skeleton

```toml
schema_version = 1

id       = "codex"                      # stable slug, unique, used in CLI args
name     = "Codex CLI"                  # display name
homepage = "https://developers.openai.com/codex"

[home]
env     = "CODEX_HOME"                  # optional: env var that overrides the default
default = "~/.codex"

[[files]]  # ... see §3
[mcp]      # ... see §4
[skills]   # ... see §7
```

A spec declaring neither `[mcp]` nor `[skills]` is valid but inert — useful for
registering a harness so its presence is *detected* before support is written.

### Path placeholders

Usable in any path field:

| Placeholder | Expands to |
|---|---|
| `{home}` | the resolved `[home]` directory for this provider |
| `{user_home}` | the OS user home (`C:\Users\User`, `/home/user`) |
| `{config}` | XDG config dir — `~/.config` on Unix, `%APPDATA%` on Windows |
| `{project}` | the project root, when operating in project scope |

Paths always use `/` in specs; the engine normalises per-OS. A leading `~/` is
equivalent to `{user_home}/`.

---

## 3. `[[files]]` — where config lives

```toml
[[files]]
id     = "global"
scope  = "user"                         # user | project
format = "toml"                         # toml | json | jsonc | yaml
path   = "{home}/config.toml"
```

### Candidate paths

A harness may accept several filenames, or have moved between versions. `path` may
be a list, tried in order:

```toml
[[files]]
id     = "global"
scope  = "user"
format = "jsonc"
path   = ["{config}/opencode/opencode.jsonc", "{config}/opencode/opencode.json"]
```

- **Read:** every candidate that exists is read and merged, first-wins on conflict.
- **Write:** the first candidate that exists; if none exist, the first listed.

This is not hypothetical — Gemini CLI on the dev machine uses a post-migration
layout (`~/.gemini/config/config.json`) rather than the documented
`~/.gemini/settings.json`. See [RESEARCH.md](RESEARCH.md) §4.

### Format note

`format = "json"` is **an alias for `jsonc`**. Real `.json` config files contain
`//` comments in the wild — `~/.copilot/config.json` does. The engine never uses a
comment-stripping JSON parser.

---

## 4. `[mcp]` — MCP server mapping

```toml
[mcp]
supported = true

[[mcp.locations]]
file    = "global"                      # references a [[files]] id
pointer = "/mcp_servers"                # JSON Pointer (RFC 6901) to the server map
```

The pointer addresses a **map of name → server object**. It applies to TOML and YAML
too; the engine models all three as a common tree.

### Dynamic pointers

Claude Code namespaces project-local servers inside the *user* file:

```toml
[[mcp.locations]]
file    = "user_json"
scope   = "local"
pointer = "/projects/{project_path}/mcpServers"
```

`{project_path}` expands to the absolute project path **with JSON Pointer escaping
applied** (`~` → `~0`, `/` → `~1`). Windows paths contain `/` after normalisation,
so `C:/Users/User` becomes `C:~1Users~1User`. Getting this wrong silently creates a
nested object instead of addressing the right key — it is the single most
error-prone part of the format.

### 4.1 Transport discrimination

Three strategies, because harnesses genuinely do this three different ways.

**`type-value`** — a field's *value* names the transport (Claude Code, OpenCode, Copilot):

```toml
[mcp.transport]
strategy = "type-value"
key      = "type"
absent   = "stdio"                      # what a missing key means

[mcp.transport.read]                    # harness value -> canonical
http              = "http"
"streamable-http" = "http"
sse               = "sse"
ws                = "ws"

[mcp.transport.write]                   # canonical -> harness value
http = "http"
sse  = "sse"
ws   = "ws"
stdio = { omit = true }                 # write no `type` key at all
```

`read` and `write` are separate maps because the relation is not one-to-one:
Claude accepts both `http` and `streamable-http` on read but should emit `http`.

**`key-presence`** — *which key exists* names the transport (Gemini CLI):

```toml
[mcp.transport]
strategy = "key-presence"
absent   = "stdio"

[mcp.transport.keys]
sse  = "url"                            # a `url` key means SSE
http = "httpUrl"                        # an `httpUrl` key means streamable HTTP
```

**`url-presence`** — remote if a url field is set, else stdio (Codex):

```toml
[mcp.transport]
strategy = "url-presence"
key      = "url"
remote   = "http"                       # which canonical transport a url implies
```

### 4.2 Field mapping

```toml
[mcp.fields]
command = { key = "command" }
args    = { key = "args" }
env     = { key = "env" }
url     = { key = "url" }
headers = { key = "http_headers" }      # Codex renames this
enabled = { key = "enabled", default = true }
```

Omit a canonical field entirely and the engine treats it as unsupported: it is
dropped on emit, and a warning is raised if a source server carried a value for it.

**Forms** handle structural differences, not just renames:

```toml
# OpenCode stores command and args merged as one argv array:
#   "command": ["uvx", "mcp-obsidian"]
command = { key = "command", form = "argv-array", absorbs = "args" }
```

| Form | Meaning |
|---|---|
| `string` | plain scalar (default for `command`, `url`) |
| `argv-array` | array where `[0]` is the command and the rest are args; requires `absorbs` |
| `list` | array of scalars (default for `args`) |
| `map` | string→string map (default for `env`, `headers`) |
| `bool` | boolean (default for `enabled`) |

A field named by `absorbs` must not also appear in `[mcp.fields]`.

**Sub-table placement** is not a spec concern. `env` being an inline table
(`env = { A = "b" }`) or a sub-table (`[mcp_servers.x.env]`) is a TOML *serialisation*
choice; the emitter preserves whatever the file already used and picks the harness's
conventional form for new entries.

---

## 5. Capabilities

```toml
[mcp.capabilities]
transports = ["stdio", "http", "sse"]   # `ws` unsupported -> refuse to deploy one here
headers    = true
env        = true
```

Deploying a server that needs a capability the target lacks is a **hard error with
an explanation**, never a silent downgrade. "Your `ws` server can't go to Codex,
which supports stdio/http/sse only" is a useful message; quietly writing a broken
entry is not.

---

## 6. Unknown-field preservation

**Non-negotiable.** Real configs carry fields no canonical model covers:
`startup_timeout_sec`, `tool_timeout_sec`, `trust`, `includeTools`, `timeout`.

On read, every key in a server object not claimed by `[mcp.fields]` or the transport
strategy is captured into `McpServer.extra`, tagged with the provider it came from.

On write:

1. Keys originating from **this** provider are restored verbatim.
2. Keys from **another** provider are dropped, and each drop is reported in the
   deploy preview.
3. Keys already present in the target file for an existing entry are **never**
   removed by an update that did not set them.

Rule 3 is what makes updates safe. Adding a server to Codex must not silently strip
a `startup_timeout_sec` the user set by hand.

```toml
[mcp.extra]
known = ["startup_timeout_sec", "tool_timeout_sec", "bearer_token_env_var"]
```

`known` is documentation and validation only — unlisted keys are still preserved. It
lets `inception doctor` flag a typo like `startup_timout_sec`.

---

## 7. `[skills]`

```toml
[skills]
supported = true
layout    = "dir-with-skill-md"         # <name>/SKILL.md
exclude   = [".*"]                      # glob; skips ~/.codex/skills/.system/

[[skills.dirs]]
scope     = "user"
path      = "~/.agents/skills"
shared    = true                        # other harnesses read this same dir
canonical = true                        # default write target

[[skills.dirs]]
scope      = "user"
path       = "{home}/skills"
deprecated = true                       # read and report, never write

[[skills.dirs]]
scope = "project"
path  = "{project}/.agents/skills"
```

### Shared directories change the deploy strategy

`~/.agents/skills` is read by Codex, OpenCode, Gemini CLI and Copilot CLI. Claude
Code explicitly does not read it.

So "install this skill everywhere" is **two writes, not five**: one to
`~/.agents/skills`, one to `~/.claude/skills`. The planner must compute the minimum
write set by grouping targets on `shared = true` directories, and show the user that
it did. This is a visible advantage no competitor currently takes.

### `canonical` vs `shared` — how the planner chooses

The two flags answer different questions, and a dir may carry both.

- `canonical = true` — the default write target for a **single-provider** deploy.
  It should name a *private* dir wherever one exists, because a shared dir cannot
  express "give this to OpenCode only."
- `shared = true` — other providers read this same dir. The planner may collapse
  several targets onto it.

Selection rule, in order:

1. If the target set has ≥2 providers that all list the same `shared` dir at the
   requested scope, write once to that dir and mark those targets satisfied.
2. Otherwise write to each remaining target's `canonical` dir.
3. Never write to a `deprecated` dir.

The preview always states which rule fired, so a single write covering four
harnesses never looks like three silently skipped ones.

**Codex is the exception:** its only non-deprecated user dir *is* the shared
`~/.agents/skills`, so it carries both flags. A Codex-only skill deploy therefore
becomes visible to OpenCode, Gemini and Copilot, and the preview must say so
rather than pretending the write was private.

`deprecated = true` dirs are scanned so existing skills are found — the dev machine
has six skills in `~/.codex/skills`, a path current Codex docs no longer list — and
surfaced as a migration suggestion.

### Frontmatter

`name` and `description` are canonical. **Every other key is preserved verbatim**,
including nested maps. Claude Code defines ~20 optional fields; Codex ignores them
but they must survive a Codex round-trip so the skill still works when copied back.

```toml
[skills.frontmatter]
required = ["name", "description"]      # emit a warning if missing, never rewrite
```

---

## 8. Worked example — OpenCode

The hardest of the five: JSONC, merged argv array, renamed env key, its own
`enabled` flag, and two candidate filenames.

```toml
schema_version = 1
id   = "opencode"
name = "OpenCode"

[home]
default = "{config}/opencode"

[[files]]
id     = "global"
scope  = "user"
format = "jsonc"
path   = ["{home}/opencode.jsonc", "{home}/opencode.json"]

[mcp]
supported = true

[[mcp.locations]]
file    = "global"
pointer = "/mcp"

[mcp.transport]
strategy = "type-value"
key      = "type"
absent   = "stdio"
[mcp.transport.read]
local  = "stdio"
remote = "http"
[mcp.transport.write]
stdio = "local"
http  = "remote"
sse   = "remote"

[mcp.fields]
command = { key = "command", form = "argv-array", absorbs = "args" }
env     = { key = "environment" }
url     = { key = "url" }
headers = { key = "headers" }
enabled = { key = "enabled", default = true }

[mcp.capabilities]
transports = ["stdio", "http", "sse"]

[skills]
supported = true
layout    = "dir-with-skill-md"
exclude   = [".*"]

[[skills.dirs]]
scope     = "user"
path      = "{home}/skills"
canonical = true

[[skills.dirs]]
scope  = "user"
path   = "~/.agents/skills"
shared = true

[[skills.dirs]]
scope = "project"
path  = "{project}/.opencode/skills"
```

Reading the dev machine's real `obsidian` entry through this spec yields:

```rust
McpServer {
    name: "obsidian",
    transport: Stdio,                   // type "local" -> stdio
    command: "uvx",                     // argv-array split
    args: ["mcp-obsidian"],
    env: { OBSIDIAN_API_KEY: Ref("obsidian_api_key"), OBSIDIAN_HOST: Literal("127.0.0.1"), ... },
    enabled: true,
    extra: {},
}
```

Emitting that through `codex.provider.toml` must produce exactly this diff, and
nothing else in the file may move:

```diff
+[mcp_servers.obsidian]
+command = "uvx"
+args = ["mcp-obsidian"]
+
+[mcp_servers.obsidian.env]
+OBSIDIAN_API_KEY = "..."
```

---

## 9. Validation

`inception validate <spec.toml>` must reject:

- unknown top-level keys (typo protection)
- a `[[mcp.locations]]` naming a `file` id that does not exist
- a pointer that is not valid RFC 6901
- `form = "argv-array"` without `absorbs`
- a field named in `absorbs` that also appears in `[mcp.fields]`
- a `transport.write` value for a transport absent from `capabilities.transports`
- `canonical = true` on more than one `[[skills.dirs]]` of the same scope

Validation runs in CI over all built-in specs, and on load for user specs — a broken
user spec disables that provider with a clear error rather than crashing the app.

---

## 10. Deliberate non-goals for v1

- **No expression language.** No conditionals, no arithmetic, no scripting. If a
  harness cannot be expressed declaratively, that is a signal to extend the schema
  with a named strategy — as `key-presence` was added for Gemini — not to add escape
  hatches. A spec must stay reviewable by someone who does not read Rust.
- **No hooks, commands, subagents or plugins.** Skills and MCP only. The schema is
  versioned (`schema_version`) so these can land in v2 without breaking specs.
- **No remote spec fetching.** Built-in plus local user specs. Downloading executable
  descriptions of where to write config files is a supply-chain problem, and it is
  not needed to prove the idea.
