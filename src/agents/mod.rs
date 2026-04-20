//! Per-agent config registry and orchestration.

use std::path::PathBuf;

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
    Home(&'static str),
    AppSupport(&'static str, &'static str),
    AppData(&'static str, &'static str),
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
