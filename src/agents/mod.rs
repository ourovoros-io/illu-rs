//! Per-agent config registry and orchestration.

pub mod agent_files;
pub mod allow_list;
pub mod detect;
pub mod formats;
pub mod instruction_md;
pub mod paths;
pub mod prompt;
pub mod selection;

use std::path::{Path, PathBuf};

use crate::agents::detect::DetectionContext as _;

/// Static metadata for one supported agent.
pub struct Agent {
    pub id: &'static str,
    pub display_name: &'static str,
    pub detection: Detection,
    pub repo_config: Option<RepoConfig>,
    pub global_config: Option<GlobalConfig>,
    pub tool_prefix: &'static str,
}

/// Heuristics used to detect whether an agent is installed or active.
pub struct Detection {
    pub env_vars: &'static [&'static str],
    pub binaries: &'static [&'static str],
    pub config_dirs: &'static [&'static str],
    pub app_bundles: &'static [&'static str],
}

/// Per-repo config targets.
pub struct RepoConfig {
    pub mcp_config_path: &'static str,
    pub mcp_format: McpFormat,
    pub instruction_file: Option<&'static str>,
    pub agents_dir: Option<&'static str>,
    pub allow_list_path: Option<&'static str>,
}

/// Global config targets.
pub struct GlobalConfig {
    pub mcp_config_path: GlobalPath,
    pub mcp_format: McpFormat,
    pub instruction_file: Option<GlobalPath>,
    pub agents_dir: Option<GlobalPath>,
    pub allow_list_path: Option<GlobalPath>,
}

/// Platform-sensitive path kind used in global configs.
#[derive(Clone, Copy)]
#[non_exhaustive]
pub enum GlobalPath {
    /// Path relative to `$HOME` on all platforms.
    Home(&'static str),
    /// `~/Library/Application Support/<vendor>/<file>` on macOS,
    /// `~/AppData/Roaming/<vendor>/<file>` on Windows,
    /// `~/.config/<vendor>/<file>` on Linux. Use when the agent follows
    /// platform-native config conventions (e.g. Claude Desktop).
    AppSupport(&'static str, &'static str),
    /// Always `~/.config/<rel>` on all platforms. `XDG_CONFIG_HOME` is the
    /// caller's responsibility, not this resolver's.
    XdgConfig(&'static str),
}

/// One entry per distinct on-disk config file schema.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub enum McpFormat {
    ClaudeCodeJson,
    GeminiJson,
    ClaudeDesktopJson,
    CursorJson,
    VsCodeJson,
    CodexToml,
    AntigravityJson,
}

/// Detection strength for a single agent.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub enum DetectionLevel {
    Active,
    Installed,
    Unknown,
}

/// Command invoked by the MCP client to run illu.
#[derive(Clone, Debug)]
pub struct IlluCommand {
    pub command: String,
    pub args: Vec<String>,
}

impl IlluCommand {
    /// Per-repo invocation with an explicit `--repo` flag.
    ///
    /// MCP clients do not guarantee a specific current working directory when
    /// they spawn servers. Without `--repo`, illu-rs falls back to CWD-based
    /// detection in `main.rs`, which fails whenever the client launches from
    /// outside the project (e.g. `/` or `$HOME`). The resulting process opens
    /// an empty in-memory index and exposes only cross-repo tools — a
    /// confusing failure mode that looks like a broken index.
    ///
    /// Embedding the canonical absolute path pins the server to this repo
    /// regardless of spawn CWD. `.mcp.json` is per-checkout anyway (the
    /// PATH-relative `illu-rs` binary name already assumes per-machine
    /// configuration), so baking in an absolute path is consistent with the
    /// file's existing portability model.
    ///
    /// Falls back to bare `["serve"]` when canonicalization or UTF-8
    /// conversion fails, matching pre-patch behavior instead of panicking,
    /// and logs a `warn!` so the regression is observable.
    #[must_use]
    pub fn serve(repo_path: &Path) -> Self {
        if let Ok(canonical) = dunce::canonicalize(repo_path)
            && let Ok(s) = canonical.into_os_string().into_string()
        {
            return Self {
                command: "illu-rs".to_string(),
                args: vec!["--repo".to_string(), s, "serve".to_string()],
            };
        }
        tracing::warn!(
            path = %repo_path.display(),
            "could not canonicalize repo path for MCP config; falling back \
             to bare `serve` — MCP clients that spawn with CWD outside the \
             repo will land in cross-repo-only mode"
        );
        Self {
            command: "illu-rs".to_string(),
            args: vec!["serve".to_string()],
        }
    }

    /// Absolute-path invocation, suitable for user-global configs.
    /// GUI-launched agents (Claude Desktop, Codex Desktop, Cursor, VS Code,
    /// Antigravity) do not inherit shell PATH on macOS, so a bare name yields
    /// `spawn illu-rs ENOENT`.
    ///
    /// Resolution prefers, in order: canonicalized `current_exe` via
    /// `dunce::canonicalize` (avoids Windows `\\?\` extended-length paths that
    /// some MCP clients mishandle); raw `current_exe` if canonicalization
    /// fails; the bare name `"illu-rs"` as a last resort (preserves pre-patch
    /// behavior rather than panicking). A fallback to the bare name is
    /// logged at warn level so operators can diagnose the `ENOENT` case.
    #[must_use]
    pub fn serve_resolved() -> Self {
        Self {
            command: resolved_binary_path(),
            args: vec!["serve".to_string()],
        }
    }
}

/// User-facing flags shared by the `init` and `install` subcommands.
#[derive(Clone, Debug, Default)]
pub struct SetupFlags {
    pub explicit_agents: Vec<String>,
    pub all: bool,
    pub yes: bool,
    pub dry_run: bool,
}

/// One entry per agent we wrote to (or skipped).
#[derive(Debug)]
#[non_exhaustive]
pub struct AgentWriteReport {
    pub agent_id: &'static str,
    pub written_paths: Vec<PathBuf>,
    pub skipped: bool,
}

/// Master registry of supported agents.
pub static AGENTS: &[Agent] = &[
    Agent {
        id: "claude-code",
        display_name: "Claude Code",
        detection: Detection {
            env_vars: &["CLAUDECODE"],
            binaries: &["claude"],
            config_dirs: &[".claude"],
            app_bundles: &[],
        },
        repo_config: Some(RepoConfig {
            mcp_config_path: ".mcp.json",
            mcp_format: McpFormat::ClaudeCodeJson,
            instruction_file: Some("CLAUDE.md"),
            agents_dir: Some(".claude/agents"),
            allow_list_path: Some(".claude/settings.local.json"),
        }),
        global_config: Some(GlobalConfig {
            // User-scope MCP servers live in `~/.claude.json` per Claude Code
            // docs. `~/.claude/settings.json` holds permissions/hooks/env and
            // does NOT accept `mcpServers` — writing MCP there is silently
            // ignored by Claude Code's loader.
            mcp_config_path: GlobalPath::Home(".claude.json"),
            mcp_format: McpFormat::ClaudeCodeJson,
            instruction_file: Some(GlobalPath::Home(".claude/CLAUDE.md")),
            agents_dir: Some(GlobalPath::Home(".claude/agents")),
            allow_list_path: Some(GlobalPath::Home(".claude/settings.json")),
        }),
        tool_prefix: "mcp__illu__",
    },
    Agent {
        id: "gemini-cli",
        display_name: "Gemini CLI",
        detection: Detection {
            env_vars: &[],
            binaries: &["gemini"],
            config_dirs: &[".gemini"],
            app_bundles: &[],
        },
        repo_config: Some(RepoConfig {
            mcp_config_path: ".gemini/settings.json",
            mcp_format: McpFormat::GeminiJson,
            instruction_file: Some("GEMINI.md"),
            agents_dir: Some(".gemini/agents"),
            allow_list_path: None,
        }),
        global_config: Some(GlobalConfig {
            mcp_config_path: GlobalPath::Home(".gemini/settings.json"),
            mcp_format: McpFormat::GeminiJson,
            instruction_file: Some(GlobalPath::Home(".gemini/GEMINI.md")),
            agents_dir: Some(GlobalPath::Home(".gemini/agents")),
            allow_list_path: None,
        }),
        tool_prefix: "mcp_illu_",
    },
    Agent {
        id: "codex-cli",
        display_name: "Codex CLI",
        detection: Detection {
            env_vars: &[],
            binaries: &["codex"],
            config_dirs: &[".codex"],
            app_bundles: &[],
        },
        repo_config: None,
        global_config: Some(GlobalConfig {
            mcp_config_path: GlobalPath::Home(".codex/config.toml"),
            mcp_format: McpFormat::CodexToml,
            instruction_file: None,
            agents_dir: None,
            allow_list_path: None,
        }),
        tool_prefix: "mcp__illu__",
    },
    Agent {
        id: "claude-desktop",
        display_name: "Claude Desktop",
        detection: Detection {
            env_vars: &[],
            binaries: &[],
            // No `config_dirs` entry: the only platform Claude Desktop ships on
            // uses `Library/Application Support/Claude` which is already covered
            // by the macOS-gated `app_bundles` check below. A literal
            // `~/Library/Application Support/Claude` lookup on Linux would be
            // a nonsensical false lead.
            config_dirs: &[],
            app_bundles: &["/Applications/Claude.app"],
        },
        repo_config: None,
        global_config: Some(GlobalConfig {
            mcp_config_path: GlobalPath::AppSupport("Claude", "claude_desktop_config.json"),
            mcp_format: McpFormat::ClaudeDesktopJson,
            instruction_file: None,
            agents_dir: None,
            allow_list_path: None,
        }),
        tool_prefix: "mcp__illu__",
    },
    Agent {
        id: "cursor",
        display_name: "Cursor",
        detection: Detection {
            env_vars: &[],
            binaries: &["cursor"],
            config_dirs: &[".cursor"],
            app_bundles: &["/Applications/Cursor.app"],
        },
        repo_config: Some(RepoConfig {
            mcp_config_path: ".cursor/mcp.json",
            mcp_format: McpFormat::CursorJson,
            instruction_file: None,
            agents_dir: None,
            allow_list_path: None,
        }),
        global_config: Some(GlobalConfig {
            mcp_config_path: GlobalPath::Home(".cursor/mcp.json"),
            mcp_format: McpFormat::CursorJson,
            instruction_file: None,
            agents_dir: None,
            allow_list_path: None,
        }),
        tool_prefix: "mcp__illu__",
    },
    Agent {
        id: "vscode-copilot",
        display_name: "VS Code + Copilot",
        detection: Detection {
            env_vars: &["VSCODE_PID"],
            binaries: &["code"],
            config_dirs: &[],
            app_bundles: &["/Applications/Visual Studio Code.app"],
        },
        repo_config: Some(RepoConfig {
            mcp_config_path: ".vscode/mcp.json",
            mcp_format: McpFormat::VsCodeJson,
            instruction_file: None,
            agents_dir: None,
            allow_list_path: None,
        }),
        global_config: None,
        tool_prefix: "mcp__illu__",
    },
    Agent {
        id: "codex-desktop",
        display_name: "Codex Desktop",
        detection: Detection {
            env_vars: &[],
            binaries: &[],
            config_dirs: &[".codex"],
            app_bundles: &["/Applications/Codex.app", "/Applications/ChatGPT.app"],
        },
        repo_config: None,
        global_config: Some(GlobalConfig {
            mcp_config_path: GlobalPath::Home(".codex/config.toml"),
            mcp_format: McpFormat::CodexToml,
            instruction_file: None,
            agents_dir: None,
            allow_list_path: None,
        }),
        tool_prefix: "mcp__illu__",
    },
    Agent {
        id: "antigravity",
        display_name: "Antigravity",
        detection: Detection {
            env_vars: &[],
            binaries: &["antigravity"],
            // Antigravity stores its MCP config under `.gemini/antigravity/`
            // rather than a dedicated top-level dir. Using the nested subdir
            // as the detection signal avoids falsely firing on `.gemini`,
            // which is shared with the Gemini CLI.
            config_dirs: &[".gemini/antigravity"],
            app_bundles: &["/Applications/Antigravity.app"],
        },
        repo_config: None,
        global_config: Some(GlobalConfig {
            mcp_config_path: GlobalPath::Home(".gemini/antigravity/mcp_config.json"),
            mcp_format: McpFormat::AntigravityJson,
            instruction_file: None,
            agents_dir: None,
            allow_list_path: None,
        }),
        tool_prefix: "mcp__illu__",
    },
];

/// All agent IDs currently supported. Derived from `AGENTS`.
#[must_use]
pub fn known_agent_ids() -> Vec<&'static str> {
    AGENTS.iter().map(|a| a.id).collect()
}

/// Detect every agent with a per-repo config target.
#[must_use]
pub fn detect_repo_agents(
    ctx: &dyn detect::DetectionContext,
) -> Vec<(&'static Agent, DetectionLevel, String)> {
    detect_scoped(ctx, |a| a.repo_config.is_some())
}

/// Detect every agent with a global config target.
#[must_use]
pub fn detect_global_agents(
    ctx: &dyn detect::DetectionContext,
) -> Vec<(&'static Agent, DetectionLevel, String)> {
    detect_scoped(ctx, |a| a.global_config.is_some())
}

fn detect_scoped(
    ctx: &dyn detect::DetectionContext,
    filter: impl Fn(&&Agent) -> bool,
) -> Vec<(&'static Agent, DetectionLevel, String)> {
    AGENTS
        .iter()
        .filter(filter)
        .map(|a| {
            let (lvl, reason) = detect::detect_with_reason(a, ctx);
            (a, lvl, reason)
        })
        .collect()
}

/// Configure per-repo agents in `repo_path` according to `flags`.
pub fn configure_repo(
    repo_path: &Path,
    flags: &SetupFlags,
) -> Result<Vec<AgentWriteReport>, crate::IlluError> {
    let ctx = detect::RealContext::new()?;
    let scoped: Vec<&Agent> = AGENTS.iter().filter(|a| a.repo_config.is_some()).collect();
    let detection = detect_scoped(&ctx, |a| a.repo_config.is_some());
    let chosen = resolve_selection(&scoped, &detection, flags)?;
    let cmd = IlluCommand::serve(repo_path);
    let mut reports = Vec::with_capacity(chosen.len());
    for agent in chosen {
        let report = write_repo_for(agent, repo_path, &cmd, flags.dry_run)?;
        reports.push(report);
    }
    Ok(reports)
}

/// Configure global agents in `home` according to `flags`.
pub fn configure_global(
    home: &Path,
    flags: &SetupFlags,
) -> Result<Vec<AgentWriteReport>, crate::IlluError> {
    let ctx = detect::RealContext::with_home(home.to_path_buf());
    let scoped: Vec<&Agent> = AGENTS
        .iter()
        .filter(|a| a.global_config.is_some())
        .collect();
    let detection = detect_scoped(&ctx, |a| a.global_config.is_some());
    let chosen = resolve_selection(&scoped, &detection, flags)?;
    let cmd = IlluCommand::serve_resolved();
    let mut reports = Vec::with_capacity(chosen.len());
    for agent in chosen {
        let report = write_global_for(agent, home, ctx.os(), &cmd, flags.dry_run)?;
        reports.push(report);
    }
    Ok(reports)
}

/// Detect via env vars only and write configs for any `Active` agent.
/// Called by `illu serve` on startup.
///
/// When `repo_path` is `None` (no manifest detected at spawn CWD), the
/// per-repo self-heal leg is skipped and a `warn!` is emitted so operators
/// see the degraded mode explicitly instead of silently running without
/// per-repo config fixes. Global self-heal runs regardless because
/// `serve_resolved` needs no repo context.
pub fn self_heal_on_serve(repo_path: Option<&Path>, home: &Path) -> Result<(), crate::IlluError> {
    let ctx = detect::RealContext::new()?;
    // `repo_cmd` is `Some` iff `repo_path` is `Some` — bound by construction.
    let repo_cmd = repo_path.map(IlluCommand::serve);
    if repo_path.is_none() {
        tracing::warn!(
            "self-heal: no project manifest detected at spawn CWD; \
             skipping per-repo config writes. Per-repo .mcp.json files \
             will not be refreshed this run — run `illu init` inside a \
             project to regenerate them."
        );
    }
    let global_cmd = IlluCommand::serve_resolved();
    for agent in AGENTS {
        if detect::detect_level(agent, &ctx) != DetectionLevel::Active {
            continue;
        }
        if let Some(repo) = repo_path
            && let Some(cmd) = &repo_cmd
            && agent.repo_config.is_some()
            && let Err(e) = write_repo_for(agent, repo, cmd, false)
        {
            tracing::warn!(agent = agent.id, "self-heal repo write failed: {e}");
        }
        if agent.global_config.is_some()
            && let Err(e) = write_global_for(agent, home, ctx.os(), &global_cmd, false)
        {
            tracing::warn!(agent = agent.id, "self-heal global write failed: {e}");
        }
    }
    Ok(())
}

fn resolve_selection<'a>(
    scoped_agents: &[&'a Agent],
    detection: &[(&'a Agent, DetectionLevel, String)],
    flags: &SetupFlags,
) -> Result<Vec<&'a Agent>, crate::IlluError> {
    let pairs: Vec<(&Agent, DetectionLevel)> = detection.iter().map(|(a, l, _)| (*a, *l)).collect();
    match selection::select_from_flags(scoped_agents, flags, &pairs, prompt::has_tty()) {
        Ok(picked) => Ok(picked),
        Err(selection::SelectionError::UnknownId(id)) => Err(crate::IlluError::Agent(format!(
            "unknown agent id for this scope: {id}"
        ))),
        Err(selection::SelectionError::NeedsPrompt) => prompt::prompt_agents(detection),
    }
}

/// Fully-resolved on-disk targets for one agent (either repo- or global-scoped).
///
/// Collapses the shape of `RepoConfig` and `GlobalConfig` into the same
/// concrete `PathBuf` layout so the actual write logic lives in one place.
struct ResolvedTargets {
    mcp: (PathBuf, McpFormat),
    /// `(path, heading)` for the instruction markdown file.
    instruction: Option<(PathBuf, String)>,
    agents_dir: Option<PathBuf>,
    allow_list: Option<PathBuf>,
}

impl ResolvedTargets {
    fn from_repo(agent: &Agent, repo_path: &Path) -> Option<Self> {
        let cfg = agent.repo_config.as_ref()?;
        Some(Self {
            mcp: (repo_path.join(cfg.mcp_config_path), cfg.mcp_format),
            instruction: cfg
                .instruction_file
                .map(|md| (repo_path.join(md), format!("# {md}"))),
            agents_dir: cfg.agents_dir.map(|ad| repo_path.join(ad)),
            allow_list: cfg.allow_list_path.map(|al| repo_path.join(al)),
        })
    }

    fn from_global(agent: &Agent, home: &Path, os: detect::TargetOs) -> Option<Self> {
        let cfg = agent.global_config.as_ref()?;
        let instruction = if let Some(md_gp) = &cfg.instruction_file {
            let md_path = paths::resolve(md_gp, os, home);
            let file_name = md_path
                .file_name()
                .and_then(|n| n.to_str())
                .map(str::to_owned)?;
            let heading = format!("# {file_name}");
            Some((md_path, heading))
        } else {
            None
        };
        Some(Self {
            mcp: (
                paths::resolve(&cfg.mcp_config_path, os, home),
                cfg.mcp_format,
            ),
            instruction,
            agents_dir: cfg
                .agents_dir
                .as_ref()
                .map(|gp| paths::resolve(gp, os, home)),
            allow_list: cfg
                .allow_list_path
                .as_ref()
                .map(|gp| paths::resolve(gp, os, home)),
        })
    }

    fn apply(
        self,
        agent: &Agent,
        cmd: &IlluCommand,
        dry_run: bool,
    ) -> Result<Vec<PathBuf>, crate::IlluError> {
        let mut written = Vec::new();
        let (mcp_path, fmt) = self.mcp;
        if !dry_run {
            formats::write(&mcp_path, fmt, cmd)?;
        }
        written.push(mcp_path);
        if let Some((md_path, heading)) = self.instruction {
            if !dry_run {
                let parent = md_path.parent().ok_or_else(|| {
                    crate::IlluError::Agent("instruction file has no parent".to_string())
                })?;
                std::fs::create_dir_all(parent)?;
                let file_name = md_path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .ok_or_else(|| {
                        crate::IlluError::Agent("instruction file has no name".to_string())
                    })?;
                instruction_md::write_md_section(
                    parent,
                    file_name,
                    &heading,
                    &instruction_md::illu_agent_section(agent.tool_prefix),
                )?;
            }
            written.push(md_path);
        }
        if let Some(dir) = self.agents_dir {
            if !dry_run {
                agent_files::generate_agent_files(&dir, agent.tool_prefix)?;
            }
            written.push(dir);
        }
        if let Some(al) = self.allow_list {
            if !dry_run {
                allow_list::ensure_tools_allowed(&al)?;
            }
            written.push(al);
        }
        Ok(written)
    }
}

fn write_repo_for(
    agent: &Agent,
    repo_path: &Path,
    cmd: &IlluCommand,
    dry_run: bool,
) -> Result<AgentWriteReport, crate::IlluError> {
    let Some(targets) = ResolvedTargets::from_repo(agent, repo_path) else {
        return Ok(AgentWriteReport {
            agent_id: agent.id,
            written_paths: vec![],
            skipped: true,
        });
    };
    let written = targets.apply(agent, cmd, dry_run)?;
    Ok(AgentWriteReport {
        agent_id: agent.id,
        written_paths: written,
        skipped: false,
    })
}

fn write_global_for(
    agent: &Agent,
    home: &Path,
    os: detect::TargetOs,
    cmd: &IlluCommand,
    dry_run: bool,
) -> Result<AgentWriteReport, crate::IlluError> {
    let Some(targets) = ResolvedTargets::from_global(agent, home, os) else {
        return Ok(AgentWriteReport {
            agent_id: agent.id,
            written_paths: vec![],
            skipped: true,
        });
    };
    let written = targets.apply(agent, cmd, dry_run)?;
    if agent.id == "claude-code"
        && !dry_run
        && let Err(e) = migrate_claude_code_legacy_mcp(home)
    {
        tracing::warn!("claude-code legacy mcp migration failed: {e}");
    }
    Ok(AgentWriteReport {
        agent_id: agent.id,
        written_paths: written,
        skipped: false,
    })
}

/// Resolve the running binary's path for use as the `command` field in
/// user-global MCP configs. See `IlluCommand::serve_resolved` for the
/// resolution order and rationale.
fn resolved_binary_path() -> String {
    let raw = std::env::current_exe().ok();
    let canonical = raw.as_deref().and_then(|p| dunce::canonicalize(p).ok());
    if let Some(path) = canonical.or(raw)
        && let Ok(s) = path.into_os_string().into_string()
    {
        return s;
    }
    tracing::warn!(
        "could not resolve absolute path to illu-rs binary; \
         falling back to bare name `illu-rs` — GUI agents without shell PATH \
         will fail with spawn ENOENT"
    );
    "illu-rs".to_string()
}

/// Earlier illu-rs versions wrote the `mcpServers.illu` entry to
/// `~/.claude/settings.json`, where Claude Code's MCP loader silently
/// ignores it. User-scope MCP servers actually live in `~/.claude.json`.
///
/// When the orchestrator writes the correct file, scrub the legacy entry
/// from settings.json so it does not linger and confuse operators reading
/// their config. Preserves any sibling permissions/hooks/env keys and any
/// other `mcpServers.*` entries a third-party tool might have added.
fn migrate_claude_code_legacy_mcp(home: &Path) -> Result<(), crate::IlluError> {
    let legacy = home.join(".claude/settings.json");
    let content = match std::fs::read_to_string(&legacy) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e.into()),
    };
    let mut v: serde_json::Value = match serde_json::from_str(&content) {
        Ok(v) => v,
        // Don't clobber a hand-edited malformed settings.json.
        Err(_) => return Ok(()),
    };
    let Some(root) = v.as_object_mut() else {
        return Ok(());
    };
    let Some(servers) = root
        .get_mut("mcpServers")
        .and_then(serde_json::Value::as_object_mut)
    else {
        return Ok(());
    };
    if servers.remove("illu").is_none() {
        return Ok(());
    }
    if servers.is_empty() {
        root.remove("mcpServers");
    }
    let serialized = serde_json::to_string_pretty(&v)?;
    std::fs::write(&legacy, serialized)?;
    Ok(())
}

#[cfg(test)]
#[expect(clippy::unwrap_used, reason = "tests")]
mod tests {
    use super::*;

    #[test]
    fn serve_embeds_absolute_repo_path() {
        // `serve()` must pin an existing repo path into `--repo` so the MCP
        // server does not depend on the spawn CWD. See the doc comment on
        // `IlluCommand::serve` for the failure mode this guards against.
        let dir = tempfile::tempdir().unwrap();
        let cmd = IlluCommand::serve(dir.path());
        assert_eq!(cmd.command, "illu-rs");
        assert_eq!(cmd.args.len(), 3, "expected [--repo, <path>, serve]");
        assert_eq!(cmd.args[0], "--repo");
        assert!(
            Path::new(&cmd.args[1]).is_absolute(),
            "canonicalized repo path must be absolute, got {:?}",
            cmd.args[1]
        );
        assert_eq!(cmd.args[2], "serve");
    }

    #[test]
    fn serve_falls_back_when_repo_does_not_exist() {
        // `dunce::canonicalize` fails on non-existent paths. The fallback
        // preserves pre-patch behavior instead of panicking so callers that
        // hand in a stale path still get a working (if CWD-dependent)
        // invocation.
        let cmd = IlluCommand::serve(Path::new("/this/path/does/not/exist/illu-test"));
        assert_eq!(cmd.command, "illu-rs");
        assert_eq!(cmd.args, vec!["serve".to_string()]);
    }

    #[test]
    fn serve_resolved_returns_non_empty_command() {
        let cmd = IlluCommand::serve_resolved();
        assert!(!cmd.command.is_empty());
        assert_eq!(cmd.args, vec!["serve".to_string()]);
        // Under `cargo test`, `current_exe()` returns the test binary path,
        // which should resolve to an absolute path (not the bare fallback).
        // If resolution ever falls back to `illu-rs`, the warn!() log fires
        // and this test becomes a red flag on the host environment.
        assert_ne!(
            cmd.command, "illu-rs",
            "current_exe() should have resolved under cargo test",
        );
    }

    #[test]
    fn migrate_claude_code_legacy_mcp_is_noop_when_file_missing() {
        let dir = tempfile::tempdir().unwrap();
        migrate_claude_code_legacy_mcp(dir.path()).unwrap();
        assert!(!dir.path().join(".claude/settings.json").exists());
    }

    #[test]
    fn migrate_claude_code_legacy_mcp_strips_only_illu_entry() {
        let dir = tempfile::tempdir().unwrap();
        let settings = dir.path().join(".claude/settings.json");
        std::fs::create_dir_all(settings.parent().unwrap()).unwrap();
        let initial = serde_json::json!({
            "permissions": { "deny": ["X"] },
            "mcpServers": {
                "illu": { "command": "illu-rs", "args": ["serve"] },
                "other": { "command": "keep-me" },
            },
        });
        std::fs::write(&settings, serde_json::to_string_pretty(&initial).unwrap()).unwrap();

        migrate_claude_code_legacy_mcp(dir.path()).unwrap();

        let after: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&settings).unwrap()).unwrap();
        assert!(after["mcpServers"].get("illu").is_none());
        assert_eq!(after["mcpServers"]["other"]["command"], "keep-me");
        assert_eq!(after["permissions"]["deny"][0], "X");
    }

    #[test]
    fn migrate_claude_code_legacy_mcp_removes_empty_mcpservers() {
        let dir = tempfile::tempdir().unwrap();
        let settings = dir.path().join(".claude/settings.json");
        std::fs::create_dir_all(settings.parent().unwrap()).unwrap();
        let initial = serde_json::json!({
            "permissions": {},
            "mcpServers": { "illu": { "command": "illu-rs" } },
        });
        std::fs::write(&settings, serde_json::to_string_pretty(&initial).unwrap()).unwrap();

        migrate_claude_code_legacy_mcp(dir.path()).unwrap();

        let after: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&settings).unwrap()).unwrap();
        assert!(
            after.get("mcpServers").is_none(),
            "empty mcpServers object should be stripped, got: {after}",
        );
    }

    #[test]
    fn migrate_claude_code_legacy_mcp_leaves_malformed_json_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let settings = dir.path().join(".claude/settings.json");
        std::fs::create_dir_all(settings.parent().unwrap()).unwrap();
        let junk = "{this is not json";
        std::fs::write(&settings, junk).unwrap();

        migrate_claude_code_legacy_mcp(dir.path()).unwrap();

        let after = std::fs::read_to_string(&settings).unwrap();
        assert_eq!(after, junk, "malformed settings.json must not be clobbered");
    }
}
