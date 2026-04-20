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
    #[must_use]
    pub fn serve() -> Self {
        Self {
            command: "illu-rs".to_string(),
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
            mcp_config_path: GlobalPath::Home(".claude/settings.json"),
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
            config_dirs: &["Library/Application Support/Claude"],
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
            config_dirs: &[".antigravity"],
            app_bundles: &["/Applications/Antigravity.app"],
        },
        repo_config: None,
        global_config: Some(GlobalConfig {
            mcp_config_path: GlobalPath::Home(".antigravity/mcp.json"),
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
) -> Result<Vec<AgentWriteReport>, Box<dyn std::error::Error>> {
    let ctx = detect::RealContext::new()?;
    let scoped: Vec<&Agent> = AGENTS.iter().filter(|a| a.repo_config.is_some()).collect();
    let detection = detect_scoped(&ctx, |a| a.repo_config.is_some());
    let chosen = resolve_selection(&scoped, &detection, flags)?;
    let cmd = IlluCommand::serve();
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
) -> Result<Vec<AgentWriteReport>, Box<dyn std::error::Error>> {
    let ctx = detect::RealContext::with_home(home.to_path_buf());
    let scoped: Vec<&Agent> = AGENTS
        .iter()
        .filter(|a| a.global_config.is_some())
        .collect();
    let detection = detect_scoped(&ctx, |a| a.global_config.is_some());
    let chosen = resolve_selection(&scoped, &detection, flags)?;
    let cmd = IlluCommand::serve();
    let mut reports = Vec::with_capacity(chosen.len());
    for agent in chosen {
        let report = write_global_for(agent, home, ctx.os(), &cmd, flags.dry_run)?;
        reports.push(report);
    }
    Ok(reports)
}

/// Detect via env vars only and write configs for any `Active` agent.
/// Called by `illu serve` on startup.
pub fn self_heal_on_serve(
    repo_path: Option<&Path>,
    home: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let ctx = detect::RealContext::new()?;
    let cmd = IlluCommand::serve();
    for agent in AGENTS {
        if detect::detect_level(agent, &ctx) != DetectionLevel::Active {
            continue;
        }
        if let (Some(repo), Some(_)) = (repo_path, &agent.repo_config)
            && let Err(e) = write_repo_for(agent, repo, &cmd, false)
        {
            tracing::warn!(agent = agent.id, "self-heal repo write failed: {e}");
        }
        if agent.global_config.is_some()
            && let Err(e) = write_global_for(agent, home, ctx.os(), &cmd, false)
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
) -> Result<Vec<&'a Agent>, Box<dyn std::error::Error>> {
    let pairs: Vec<(&Agent, DetectionLevel)> = detection.iter().map(|(a, l, _)| (*a, *l)).collect();
    match selection::select_from_flags(scoped_agents, flags, &pairs, prompt::has_tty()) {
        Ok(picked) => Ok(picked),
        Err(selection::SelectionError::UnknownId(id)) => {
            Err(format!("unknown agent id for this scope: {id}").into())
        }
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
    ) -> Result<Vec<PathBuf>, Box<dyn std::error::Error>> {
        let mut written = Vec::new();
        let (mcp_path, fmt) = self.mcp;
        if !dry_run {
            formats::write(&mcp_path, fmt, cmd)?;
        }
        written.push(mcp_path);
        if let Some((md_path, heading)) = self.instruction {
            if !dry_run {
                let parent = md_path.parent().ok_or("instruction file has no parent")?;
                std::fs::create_dir_all(parent)?;
                let file_name = md_path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .ok_or("instruction file has no name")?;
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
) -> Result<AgentWriteReport, Box<dyn std::error::Error>> {
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
) -> Result<AgentWriteReport, Box<dyn std::error::Error>> {
    let Some(targets) = ResolvedTargets::from_global(agent, home, os) else {
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
