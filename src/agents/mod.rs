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
    /// Windows-style `AppData` path: `~/AppData/Roaming/<vendor>/<file>` on Windows,
    /// `~/.config/<vendor>/<file>` elsewhere. Use when the macOS target does NOT
    /// live under `Library/Application Support`.
    AppData(&'static str, &'static str),
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

/// Master registry. Populated in Phase 5 tasks.
pub static AGENTS: &[Agent] = &[];

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
            let lvl = detect::detect_level(a, ctx);
            let reason = detection_reason(a, ctx, lvl);
            (a, lvl, reason)
        })
        .collect()
}

fn detection_reason(
    agent: &Agent,
    ctx: &dyn detect::DetectionContext,
    lvl: DetectionLevel,
) -> String {
    match lvl {
        DetectionLevel::Active => agent
            .detection
            .env_vars
            .iter()
            .find(|v| ctx.env_var(v).is_some())
            .map_or_else(|| "env".to_string(), |v| format!("env:{v}")),
        DetectionLevel::Installed => {
            if let Some(b) = agent
                .detection
                .binaries
                .iter()
                .find(|b| ctx.binary_on_path(b))
            {
                format!("binary:{b}")
            } else if let Some(d) = agent
                .detection
                .config_dirs
                .iter()
                .find(|d| ctx.path_exists(&ctx.home().join(d)))
            {
                format!("~/{d}")
            } else if let Some(b) = agent
                .detection
                .app_bundles
                .iter()
                .find(|b| ctx.path_exists(Path::new(b)))
            {
                (*b).to_string()
            } else {
                "installed".to_string()
            }
        }
        DetectionLevel::Unknown => String::new(),
    }
}

/// Configure per-repo agents in `repo_path` according to `flags`.
pub fn configure_repo(
    repo_path: &Path,
    flags: &SetupFlags,
) -> Result<Vec<AgentWriteReport>, Box<dyn std::error::Error>> {
    let ctx = detect::RealContext::new()?;
    let detection = detect_repo_agents(&ctx);
    let chosen = resolve_selection(&detection, flags)?;
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
    let ctx = detect::RealContext::new()?;
    let detection = detect_global_agents(&ctx);
    let chosen = resolve_selection(&detection, flags)?;
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
        if let (Some(repo), Some(_)) = (repo_path, &agent.repo_config) {
            let _ = write_repo_for(agent, repo, &cmd, false);
        }
        if agent.global_config.is_some() {
            let _ = write_global_for(agent, home, ctx.os(), &cmd, false);
        }
    }
    Ok(())
}

fn resolve_selection<'a>(
    detection: &[(&'a Agent, DetectionLevel, String)],
    flags: &SetupFlags,
) -> Result<Vec<&'a Agent>, Box<dyn std::error::Error>> {
    let pairs: Vec<(&Agent, DetectionLevel)> = detection.iter().map(|(a, l, _)| (*a, *l)).collect();
    match selection::select_from_flags(AGENTS, flags, &pairs, prompt::has_tty()) {
        Ok(picked) => Ok(picked),
        Err(selection::SelectionError::UnknownId(id)) => {
            Err(format!("unknown agent id: {id}").into())
        }
        Err(selection::SelectionError::NeedsPrompt) => prompt::prompt_agents(detection),
    }
}

fn write_repo_for(
    agent: &Agent,
    repo_path: &Path,
    cmd: &IlluCommand,
    dry_run: bool,
) -> Result<AgentWriteReport, Box<dyn std::error::Error>> {
    let mut written = Vec::new();
    let Some(cfg) = &agent.repo_config else {
        return Ok(AgentWriteReport {
            agent_id: agent.id,
            written_paths: written,
            skipped: true,
        });
    };
    let mcp_path = repo_path.join(cfg.mcp_config_path);
    if !dry_run {
        formats::write(&mcp_path, cfg.mcp_format, cmd)?;
    }
    written.push(mcp_path);
    if let Some(md_rel) = cfg.instruction_file {
        let heading = format!("# {md_rel}");
        if !dry_run {
            instruction_md::write_md_section(
                repo_path,
                md_rel,
                &heading,
                &instruction_md::illu_agent_section(agent.tool_prefix),
            )?;
        }
        written.push(repo_path.join(md_rel));
    }
    if let Some(ad_rel) = cfg.agents_dir {
        let target = repo_path.join(ad_rel);
        if !dry_run {
            agent_files::generate_agent_files(&target, agent.tool_prefix)?;
        }
        written.push(target);
    }
    if let Some(al_rel) = cfg.allow_list_path {
        let target = repo_path.join(al_rel);
        if !dry_run {
            allow_list::ensure_tools_allowed(&target)?;
        }
        written.push(target);
    }
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
    let mut written = Vec::new();
    let Some(cfg) = &agent.global_config else {
        return Ok(AgentWriteReport {
            agent_id: agent.id,
            written_paths: written,
            skipped: true,
        });
    };
    let mcp_path = paths::resolve(&cfg.mcp_config_path, os, home);
    if !dry_run {
        formats::write(&mcp_path, cfg.mcp_format, cmd)?;
    }
    written.push(mcp_path);
    if let Some(md_gp) = &cfg.instruction_file {
        let md_path = paths::resolve(md_gp, os, home);
        let file_name = md_path
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or("instruction file has no name")?;
        let heading = format!("# {file_name}");
        if !dry_run {
            let parent = md_path.parent().ok_or("instruction file has no parent")?;
            std::fs::create_dir_all(parent)?;
            instruction_md::write_md_section(
                parent,
                file_name,
                &heading,
                &instruction_md::illu_agent_section(agent.tool_prefix),
            )?;
        }
        written.push(md_path);
    }
    if let Some(ad_gp) = &cfg.agents_dir {
        let target = paths::resolve(ad_gp, os, home);
        if !dry_run {
            agent_files::generate_agent_files(&target, agent.tool_prefix)?;
        }
        written.push(target);
    }
    if let Some(al_gp) = &cfg.allow_list_path {
        let target = paths::resolve(al_gp, os, home);
        if !dry_run {
            allow_list::ensure_tools_allowed(&target)?;
        }
        written.push(target);
    }
    Ok(AgentWriteReport {
        agent_id: agent.id,
        written_paths: written,
        skipped: false,
    })
}
