# Multi-agent setup: detect-or-prompt configuration

**Status:** design
**Date:** 2026-04-20

## Summary

Replace illu's current "write every agent's config every time" install flow with a
detect-or-prompt model that configures only the agents the user actually uses. Expand
supported agents from two (Claude Code, Gemini CLI) to eight (add Codex CLI, Codex
Desktop, Antigravity IDE, Claude Desktop, Cursor, VS Code + Copilot). Structure the
agent list as a single data-driven registry so adding a ninth agent is one row.

## Motivation

Today, `illu init`, `illu install`, and every `illu serve` unconditionally write
Claude Code and Gemini CLI configuration — MCP server configs, markdown instruction
sections, agent-definition files, and tool allow-lists — regardless of which agents
the user actually uses. Two problems:

1. **Wrong files for non-users.** A user who only runs Gemini CLI still gets
   `.claude/` scaffolding written into their repo, and vice-versa.
2. **New agents don't fit.** Users of Codex, Antigravity, Claude Desktop, Cursor,
   and VS Code + Copilot are unsupported. Extending the current pattern would
   mean unconditionally writing eight agents' worth of files into every repo,
   which is strictly worse.

The right model is: detect which agents the user has, prompt to confirm, and write
only for the confirmed set. Desktop apps and IDEs are first-class citizens alongside
CLIs.

## Non-goals

- Reliably configuring agents the user doesn't have installed. Detection is
  best-effort. If we can't detect and the user doesn't explicitly pick it, we
  don't write anything.
- Auto-generating illu slash commands or custom prompts for each agent. We only
  write MCP server configuration, instruction-file sections, and (for agents
  that support it) agent-definition files.
- Managing agent auth, login, or per-user preferences. Out of scope.
- Supporting every MCP-capable tool in existence. This spec names eight targets;
  adding more happens in follow-ups.

## Supported agents

| Agent | Type | Scope(s) | MCP transport |
|---|---|---|---|
| Claude Code | CLI | per-repo + global | stdio |
| Gemini CLI | CLI | per-repo + global | stdio |
| Codex CLI | CLI | global | stdio |
| Codex Desktop | Desktop | global | stdio |
| Antigravity IDE | IDE | global | stdio |
| Claude Desktop | Desktop | global (platform-specific path) | stdio |
| Cursor | IDE | per-repo + global | stdio |
| VS Code + Copilot | IDE | per-repo | stdio |

Exact env-var names, binary names, config paths, and config schemas for any row
marked "verify" below are confirmed against vendor documentation during
implementation, not in this spec.

## Architecture

### New module: `src/agents/`

```
src/agents/
  mod.rs            # Agent struct, static AGENTS registry, public entry points
  detect.rs         # Detection heuristics (env vars, binaries, dirs, bundles)
  formats.rs        # McpFormat variants and per-format writers
  agent_files.rs    # Moved from src/main.rs (generate_agent_files + AGENT_DEFS)
  instruction_md.rs # Moved from src/main.rs (illu_agent_section + write_md_section)
  allow_list.rs     # Moved from src/main.rs (ensure_tools_allowed)
  prompt.rs         # dialoguer-based multi-select prompt + no-TTY fallback
```

### Core types (`src/agents/mod.rs`)

```rust
pub struct Agent {
    pub id: &'static str,
    pub display_name: &'static str,
    pub detection: Detection,
    pub repo_config: Option<RepoConfig>,
    pub global_config: Option<GlobalConfig>,
    pub tool_prefix: &'static str,
}

pub struct Detection {
    pub env_vars: &'static [&'static str],
    pub binaries: &'static [&'static str],
    pub config_dirs: &'static [&'static str],
    pub app_bundles: &'static [&'static str],
}

pub struct RepoConfig {
    pub mcp_config_path: &'static str,
    pub mcp_format: McpFormat,
    pub instruction_file: Option<InstructionFile>,
    pub agents_dir: Option<&'static str>,
    pub allow_list_path: Option<&'static str>,
}

pub struct GlobalConfig {
    pub mcp_config_path: GlobalPath,
    pub mcp_format: McpFormat,
    pub instruction_file: Option<InstructionFile>,
    pub agents_dir: Option<GlobalPath>,
    pub allow_list_path: Option<GlobalPath>,
}

pub enum GlobalPath {
    Home(&'static str),                 // $HOME/<rel>
    AppSupport(&'static str, &'static str), // platform-resolved app-support dir
    AppData(&'static str, &'static str),    // Windows %APPDATA%\...
    XdgConfig(&'static str),            // Linux $XDG_CONFIG_HOME or ~/.config
}

pub enum McpFormat {
    ClaudeCodeJson,
    GeminiJson,
    ClaudeDesktopJson,
    CursorJson,
    VsCodeJson,
    CodexToml,
    AntigravityJson,
}

pub enum DetectionLevel {
    Active,     // agent is calling us right now (env var matched)
    Installed,  // evidence agent is on the machine
    Unknown,    // no signal
}

pub static AGENTS: &[Agent] = &[ /* 8 rows */ ];
```

### Public entry points

```rust
// Called by init_repo in main.rs
pub fn configure_repo(
    repo_path: &Path,
    flags: &SetupFlags,
) -> Result<Vec<AgentWriteReport>, Error>;

// Called by install_global in main.rs
pub fn configure_global(
    home: &Path,
    flags: &SetupFlags,
) -> Result<Vec<AgentWriteReport>, Error>;

// Called by the Serve branch in main.rs
pub fn self_heal_on_serve(
    repo_path: &Path,
    home: &Path,
    env: &Env,
) -> Result<(), Error>;

pub struct SetupFlags {
    pub explicit_agents: Vec<String>,
    pub all: bool,
    pub yes: bool,
    pub dry_run: bool,
}

pub struct AgentWriteReport {
    pub agent_id: &'static str,
    pub written_paths: Vec<PathBuf>,
    pub skipped: bool,
}
```

## Detection

Detection returns one of `Active`, `Installed`, `Unknown` per agent.

- **`Active`**: any entry in `detection.env_vars` is set in the environment.
  Only possible during `serve` (the MCP client passes its env through to us).
- **`Installed`**: any of `binaries` found on `PATH`; or any of `config_dirs`
  exists under `$HOME`; or on macOS, any of `app_bundles` exists.
- **`Unknown`**: none of the above.

Detection is abstracted behind a `DetectionContext` trait so tests can inject a
fake environment, a fake filesystem view, and a fake binary locator. No test
touches real `PATH` or `$HOME`.

Per-agent heuristic summary (vendor-specific identifiers to be confirmed during
implementation, marked "verify"):

| Agent | env_vars | binaries | config_dirs | app_bundles (macOS) |
|---|---|---|---|---|
| Claude Code | `CLAUDECODE` | `claude` | `.claude` | — |
| Gemini CLI | `GEMINI_CLI` (verify) | `gemini` | `.gemini` | — |
| Codex CLI | `CODEX_CLI` (verify) | `codex` | `.codex` | — |
| Codex Desktop | — | — | — | `ChatGPT.app` / `Codex.app` (verify) |
| Antigravity | — (verify) | `antigravity` (verify) | `.antigravity` (verify) | `Antigravity.app` (verify) |
| Claude Desktop | — | — | `Library/Application Support/Claude` | `Claude.app` |
| Cursor | `CURSOR_TRACE_ID` (verify) | `cursor` | `.cursor` | `Cursor.app` |
| VS Code + Copilot | `VSCODE_PID`, `TERM_PROGRAM=vscode` | `code` | — | `Visual Studio Code.app` |

## Orchestration

### `illu init` (per-repo)

1. Validate the directory is a supported project (Cargo, TS/JS, Python).
2. Take `AGENTS.iter().filter(|a| a.repo_config.is_some())`.
3. Run detection for each.
4. Decide selection:
   - `--all` → every filtered agent.
   - `--agent X [--agent Y ...]` → exactly those IDs (error on unknown ID).
   - `--yes` → detected agents only.
   - Neither + TTY → prompt (multi-select, detected pre-checked).
   - Neither + no TTY → act as `--yes`.
5. For each selected agent: write MCP config, instruction md, agent-defs dir,
   allow-list — whichever fields the agent's `RepoConfig` defines.
6. Build initial index; update `.gitignore`.
7. Print summary.

`--dry-run` short-circuits step 5: instead of writing, print what would be written.

### `illu install` (global)

Same as above, but filter to `global_config.is_some()` and resolve `GlobalPath`
values against `$HOME` and the current OS. Also installs statusline and updates
the global `.gitignore`, as today.

### `illu serve`

1. Detect via env-vars only.
2. Zero `Active` agents → log info, skip all writes, start server.
3. One or more `Active` agents, and the CWD is a supported project → write that
   agent's repo config files; also write its global config files. Start server.
4. One or more `Active` agents, but CWD is not a supported project → skip repo
   writes, still perform global writes, start server in index-less mode (same
   gating as today's code).
5. Never prompt.

### CLI changes

```rust
enum Command {
    // ...
    Init {
        #[arg(long)] agent: Vec<String>,
        #[arg(long)] all: bool,
        #[arg(long, short = 'y')] yes: bool,
        #[arg(long)] dry_run: bool,
    },
    Install {
        #[arg(long)] agent: Vec<String>,
        #[arg(long)] all: bool,
        #[arg(long, short = 'y')] yes: bool,
        #[arg(long)] dry_run: bool,
    },
    // Serve unchanged externally
}
```

## Prompt UX (dialoguer)

New dependency: `dialoguer` (CLI prompts). Used for the multi-select.

```
illu can configure the following agents found on your system.
Use space to toggle, enter to confirm.

[x] Claude Code          (detected: binary + ~/.claude)
[x] Cursor               (detected: /Applications/Cursor.app)
[ ] Gemini CLI           (not detected)
[ ] Codex CLI            (not detected)
[ ] Codex Desktop        (not detected)
[ ] Claude Desktop       (not detected)
[ ] Antigravity          (not detected)
[ ] VS Code + Copilot    (detected: /Applications/Visual Studio Code.app)
```

No-TTY (`!stdin().is_terminal()`) fallback: behave as if `--yes` was passed.
Zero detected agents and no explicit `--agent`/`--all` → print a helpful message
listing all supported agents with example flags, exit non-zero.

## Config formats

Each `McpFormat` variant is one writer function with a uniform signature:

```rust
pub struct IlluCommand {
    pub command: String,     // "illu"
    pub args: Vec<String>,   // e.g. ["serve"]
}

fn write_<format>(path: &Path, illu_command: &IlluCommand) -> Result<()>;
```

All writers perform **read-modify-write**: load existing JSON/TOML if present,
merge in the `illu` entry, write back. Unrelated entries are preserved.
Malformed files error cleanly with a user-actionable message — no silent
overwrites.

### Shapes

- **ClaudeCodeJson / GeminiJson / ClaudeDesktopJson / CursorJson / AntigravityJson**

  ```json
  { "mcpServers": { "illu": { "command": "illu", "args": ["serve"] } } }
  ```

- **VsCodeJson** (note: `servers`, not `mcpServers`; requires `type`):

  ```json
  { "servers": { "illu": { "type": "stdio", "command": "illu", "args": ["serve"] } } }
  ```

- **CodexToml** (`toml_edit` preserves comments/formatting):

  ```toml
  [mcp_servers.illu]
  command = "illu"
  args = ["serve"]
  ```

### Platform-aware paths

`resolve_global_path(GlobalPath, os: TargetOs, home: &Path) -> PathBuf` maps:

- `Home(rel)` → `$HOME/<rel>`.
- `AppSupport(vendor, file)` →
  - macOS: `$HOME/Library/Application Support/<vendor>/<file>`
  - Windows: `%APPDATA%\<vendor>\<file>`
  - Linux: `$XDG_CONFIG_HOME/<vendor>/<file>` (fallback `$HOME/.config/<vendor>/<file>`)
- `AppData(vendor, file)` → Windows-first variant (same as `AppSupport` but named for clarity where apps use `%APPDATA%` on Windows).
- `XdgConfig(rel)` → `$XDG_CONFIG_HOME/<rel>` (fallback `$HOME/.config/<rel>`).

`os` is a parameter (not `std::env::consts::OS`) so unit tests can exercise all three branches.

## New dependencies

- **`dialoguer`** — multi-select prompt.
- **`toml_edit`** — in-place TOML editing for Codex config.

Both are scoped to `src/agents/`.

## Migration from current code

### Removed from `src/main.rs`

- `write_mcp_config`, `write_gemini_config`, `write_global_mcp_config`,
  `write_mcp_server_config`, `write_mcp_config_to`.
- `write_claude_md_section`, `write_gemini_md_section`.
- The per-agent block inside `init_repo` (currently ~15 lines of sequential
  calls for Claude then Gemini).
- The matching block inside `install_global`.
- The matching block inside `main`'s `Serve` branch.

Each of the three call sites collapses to a single call into `src/agents/`.

### Moved to `src/agents/` (no behavior change)

- `generate_agent_files` + `AGENT_DEFS` + `BUILTIN_TOOLS` → `agent_files.rs`.
- `illu_agent_section` + `write_md_section` → `instruction_md.rs`.
- `ensure_tools_allowed` → `allow_list.rs`.

Their existing tests (`test_generate_agent_files_creates_three_files`,
`test_ensure_tools_allowed`) move with them unchanged.

### Kept in `src/main.rs`

Index orchestration (`ensure_indexed`, `head_watcher`, `open_or_index`),
`.gitignore` handling, statusline install, CLI parsing, command dispatch.

### Not kept

No backwards-compatibility shim. Users who want the old "write everything"
behavior pass `--all`. Existing on-disk files from prior `illu init` runs are
left in place; nothing is deleted or migrated.

## Testing

### Unit tests (in `src/agents/`)

- **Detection**: each heuristic in isolation; `Active` / `Installed` / `Unknown`
  outcomes; no real `PATH` or `$HOME`.
- **Format writers**: one test per `McpFormat` variant covering fresh file,
  existing-unrelated-entries-preserved, existing-illu-entry-updated,
  malformed-file-errors.
- **Selection logic**: pure `select_agents(flags, detection, user_choices)`
  table-driven tests covering every flag combination.
- **Path resolution**: `resolve_global_path` for macOS, Windows, Linux
  (parameterized `os`).

### Integration tests (in `tests/`)

- **`init` end-to-end**: tempdir repo, `--yes --agent claude-code`, assert
  exact files and contents.
- **`install` end-to-end**: tempdir `$HOME`, `--yes --agent codex-cli`, assert
  `.codex/config.toml` contents.
- **Idempotence**: run `init` twice; run with `--agent A` then `--agent B`,
  assert both coexist.
- **Read-modify-write preservation**: seed config with an unrelated MCP server,
  run `init`, assert the unrelated server is still present.

### Manual smoke tests (documented, not automated)

- TUI rendering in a real terminal.
- Each of the eight real agents loading its generated config and connecting to
  `illu serve` successfully. This is the only check that validates the vendor
  identifiers flagged "verify" in the detection table.

### Lint gates

All new code respects `Cargo.toml [lints.clippy]`: no `unwrap_used`, no
`print_stdout` outside existing `#[expect]`-annotated CLI entry points, no
`panic`/`todo`/`unimplemented`. Test modules opt out with the existing
`#[expect(clippy::unwrap_used, reason = "tests")]` pattern.

## Open items confirmed during implementation

These are flagged here so the implementation plan accounts for them. They do
not change the architecture:

- Exact env-var names for Gemini CLI, Codex CLI, Cursor (currently "verify").
- Exact binary name, config dir, and app-bundle name for Antigravity.
- Whether Codex Desktop reuses `~/.codex/config.toml` or has its own config.
- Claude Desktop config path on Linux (confirm vendor convention).
- VS Code + Copilot's exact MCP config schema (`servers` key is assumed; verify).

Each is verified against vendor documentation before the corresponding
`AGENTS` row is committed.
