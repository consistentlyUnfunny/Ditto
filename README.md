# Ditto

One place to manage the skills and MCP servers scattered across every AI coding
agent on your machine — without mangling the files they live in.

If you use more than one agent, the same skill ends up copied into three or four
directories, and the copies drift. The same MCP server gets declared in TOML for
one harness, JSON for another, JSONC with comments for a third. Nothing tells you
which copy is current, and most tools that try to help rewrite your config files
on the way through.

This reads all of it, shows you one table, and writes back **byte-identically**
except for the lines it was asked to change.

```
SKILL                      STATUS      AVAILABLE TO
drift-demo                 DRIFTED     codex, copilot-cli, gemini-cli, opencode  (+ library)
    ~/.ditto/library/drift-demo           (your library)  changed 2 weeks ago
    ~/.agents/skills/drift-demo           (shared dir)    changed 2 weeks ago
    ~/.config/opencode/skills/drift-demo                  changed 2 weeks ago
```

## The name

The project is **Ditto** — a ditto mark means "same as above", which is what
this does to a skill. The command is **`dit`**, because macOS already ships a
`/usr/bin/ditto` for copying app bundles, and shadowing a system tool that
packaging scripts depend on is a rude way to introduce yourself.

## Status

Working, and not released. The engine, the CLI and the desktop window all run;
there is no installer yet and the version is `0.1.0`. Build it from source.

## What makes it different

**Harness support is data, not code.** Every harness is a
[`providers/*.provider.toml`](providers/) file describing where its config lives,
what format it is in and what it supports. The engine contains no harness names
at all. Adding one is a file, not a pull request and a release.

**Format preservation is an invariant, not an aspiration.** Writes go through
`toml_edit` and `jsonc-parser`'s comment-preserving CST, and the test suite reads
every file in a corpus, writes it back unchanged and asserts the bytes are equal.
Your comments, key order and string quoting survive. (The nearest competitor
builds a fresh `toml::Table` and writes it out — which alphabetises your keys and
drops every comment. See [docs/RESEARCH.md](docs/RESEARCH.md) §5.)

**It writes as few files as it can.** `~/.agents/skills` is read by four of the
five harnesses, so "install this everywhere" is **two writes, not five** — and
the preview tells you which harnesses each write serves, and why.

**Nothing happens without a preview.** Every command plans first and shows you
the diff, the destination and what will be overwritten. `--yes` skips the
question, never the diff.

### Harnesses

All five support both MCP servers and skills. The skill directories overlap,
which is where the write count comes from:

| Harness | Skill directories it reads, at user scope |
|---|---|
| Claude Code | `~/.claude/skills` |
| Codex CLI | `~/.agents/skills`, `~/.codex/skills` *(deprecated)* |
| GitHub Copilot CLI | `~/.copilot/skills`, `~/.agents/skills` |
| Gemini CLI | `~/.agents/skills`, `~/.gemini/skills` |
| OpenCode | `~/.config/opencode/skills`, `~/.claude/skills`, `~/.agents/skills` |

Four of the five read `~/.agents/skills`; Claude Code refuses it and reads
`~/.claude/skills`, which OpenCode also reads. So two files reach all five, and
`dit status` will tell you which of these actually exist on your machine.

## Running it

### You need

- **Rust 1.85+** (edition 2024) — [rustup.rs](https://rustup.rs)
- **Node 20+** — only for the desktop window; the CLI needs nothing but Rust
- **Linux only:** the usual WebKitGTK/Tauri system packages — see
  [tauri.app/start/prerequisites](https://tauri.app/start/prerequisites/)

Windows and macOS need nothing beyond Rust and Node.

### The CLI

```sh
git clone https://github.com/ongkanghao/ditto
cd ditto
cargo run -p ditto-cli -- status
```

That prints which harnesses it found and where their config lives — a good first
command, because it touches nothing.

For a binary you can call by name:

```sh
cargo build --release -p ditto-cli
./target/release/dit status      # Windows: target\release\dit.exe
```

`-p ditto-cli` matters here. A bare `cargo build` builds the whole
workspace, and the desktop window will not compile until its frontend has been
built — see below.

### The desktop window

The frontend is bundled into the binary at compile time, so **build it first**.
Skip this and `cargo build` fails with:

```
The `frontendDist` configuration is set to `"ui/dist"` but this path doesn't exist
```

```sh
cd crates/ditto-gui/ui
npm ci
npm run build
cd ../../..
cargo run -p ditto-gui
```

After that, you only need to repeat the `npm run build` step when you change
something under `ui/src`.

For a release build:

```sh
cargo build --release
./target/release/ditto-gui         # Windows: target\release\ditto-gui.exe
```

> On Windows, the **debug** build deliberately keeps a console window attached so
> panics are visible. The release build does not. If you see a stray `cmd`
> window, you are running the debug binary.

## Commands

| Command | What it does |
|---|---|
| `dit status` | Which harnesses are installed, and where their config lives |
| `dit list [skills\|mcp]` | One inventory across every harness |
| `dit drift` | Skills that exist in several places with differing content, with a diff |
| `dit doctor` | Deprecated paths, plaintext secrets, drift |
| `dit deploy <name> [--to <ids>] [--from <id>]` | Copy a skill or server to other harnesses |
| `dit import <name> [--from <id>]` | Adopt a skill into your library |
| `dit transfer <name> --to project\|account --project <dir>` | Copy a skill between your account and a project |
| `dit remove <name> [--from <ids>]` | Take a skill or server out of harnesses |
| `dit projects [--add <dir>] [--forget <dir>]` | Projects your harnesses recorded, plus any you added |
| `dit providers` | The loaded provider specs and what each supports |
| `dit validate [file]` | Check a provider spec |

Most commands take `--project <dir>` to work in project scope instead of your
account, and anything that writes takes `--yes`.

### The library

`dit import` copies a skill into `~/.ditto/library/`, which no harness
reads. That copy is the one a deploy uses when you don't name a source —
otherwise the winner is whichever path happened to sort first, which is no answer
at all.

It is a **location, not a mode**: switching it off (`use_library = false` in
`~/.ditto/config.toml`, or the switch in the window) stops that directory
being scanned and nothing else changes. Nothing imports on your behalf, and a
deploy never writes into it.

Note that the library is **data, not cache** — a skill you imported and never
deployed exists only there.

## Where it keeps things

| Path | What |
|---|---|
| `~/.ditto/config.toml` | This tool's own settings |
| `~/.ditto/library/` | Skills you imported |
| `~/.ditto/backups/` | Copies of anything a write replaced |

Nothing else on your machine is ours. Every other path it touches belongs to a
harness, and is only written when you ask.

## Repository layout

```
crates/ditto-core/    the engine: detect, inventory, plan, write
crates/ditto-cli/     a thin shell over the engine
crates/ditto-gui/     Tauri window; ui/ is Vite + vanilla TypeScript
providers/                one .provider.toml per harness — the data the engine reads
docs/                     research, the provider spec format, requirements
```

Both frontends are deliberately thin: every decision lives in `ditto-core`,
so the CLI and the window can never disagree about what a deploy will do.

## Development

```sh
cargo test                              # the suite, including byte-identical round-trips
cargo clippy --all-targets -- -D warnings
cd crates/ditto-gui/ui && npm run build   # typechecks, then bundles
```

The window's frontend has **zero runtime dependencies** — no framework, no icon
package, no fonts fetched at load. A tool that reads your machine's configuration
has no business making network requests.

### Adding a harness

Write a `providers/<id>.provider.toml`, run `dit validate <file>`, and it
appears everywhere — the matrix, the planner, drift detection, doctor. If you
find yourself editing Rust to add a harness, something has gone wrong; that is
the one rule the architecture exists to keep.

[docs/PROVIDER-SPEC.md](docs/PROVIDER-SPEC.md) documents the format.

## Documentation

| Doc | What it holds |
|---|---|
| [DESIGN.md](DESIGN.md) | positioning, decisions, architecture |
| [docs/RESEARCH.md](docs/RESEARCH.md) | verified harness facts — paths, schemas, hazards |
| [docs/PROVIDER-SPEC.md](docs/PROVIDER-SPEC.md) | the provider spec format and canonical model |
| [docs/REQUIREMENTS.md](docs/REQUIREMENTS.md) | numbered requirements and acceptance criteria |
| [docs/USABILITY.md](docs/USABILITY.md) | interface decisions and what they cost |
| [TODO.md](TODO.md) | task breakdown, open questions, risks |

## License

MIT.
