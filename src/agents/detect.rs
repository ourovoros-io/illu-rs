//! Detection of installed and active agents.

use super::{Agent, DetectionLevel};
use std::path::{Path, PathBuf};

/// Environment the detector sees. Abstracted for testability.
pub trait DetectionContext {
    fn env_var(&self, name: &str) -> Option<String>;
    fn binary_on_path(&self, name: &str) -> bool;
    fn path_exists(&self, path: &Path) -> bool;
    fn home(&self) -> &Path;
    fn os(&self) -> TargetOs;
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TargetOs {
    MacOs,
    Linux,
    Windows,
}

pub(crate) fn detect_level(agent: &Agent, ctx: &dyn DetectionContext) -> DetectionLevel {
    detect_with_reason(agent, ctx).0
}

/// Classify an agent and return both the detection level and a human-readable
/// reason string in a single pass over the detection heuristics.
///
/// The reason is empty for `DetectionLevel::Unknown` and otherwise names
/// the first signal that matched (e.g. `"env:CLAUDECODE"`, `"binary:claude"`,
/// `"~/.cursor"`, or `"/Applications/Claude.app"`).
pub(crate) fn detect_with_reason(
    agent: &Agent,
    ctx: &dyn DetectionContext,
) -> (DetectionLevel, String) {
    // Active: any env var present
    for var in agent.detection.env_vars {
        if ctx.env_var(var).is_some() {
            return (DetectionLevel::Active, format!("env:{var}"));
        }
    }
    // Installed: binary on PATH, or config dir present, or (macOS) app bundle
    for bin in agent.detection.binaries {
        if ctx.binary_on_path(bin) {
            return (DetectionLevel::Installed, format!("binary:{bin}"));
        }
    }
    for rel in agent.detection.config_dirs {
        if ctx.path_exists(&ctx.home().join(rel)) {
            return (DetectionLevel::Installed, format!("~/{rel}"));
        }
    }
    if ctx.os() == TargetOs::MacOs {
        for bundle in agent.detection.app_bundles {
            if ctx.path_exists(Path::new(bundle)) {
                return (DetectionLevel::Installed, (*bundle).to_string());
            }
        }
    }
    (DetectionLevel::Unknown, String::new())
}

/// Real detection context backed by the process environment and filesystem.
pub(crate) struct RealContext {
    home: PathBuf,
    os: TargetOs,
}

/// Derive the target-OS enum from `std::env::consts::OS`, collapsing
/// everything that isn't macOS or Windows to Linux.
fn current_os() -> TargetOs {
    match std::env::consts::OS {
        "macos" => TargetOs::MacOs,
        "windows" => TargetOs::Windows,
        _ => TargetOs::Linux,
    }
}

impl RealContext {
    pub(crate) fn new() -> Result<Self, crate::IlluError> {
        let home = std::env::var("HOME")
            .ok()
            .or_else(|| std::env::var("USERPROFILE").ok())
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
            .ok_or_else(|| {
                crate::IlluError::Agent("neither HOME nor USERPROFILE set".to_string())
            })?;
        Ok(Self {
            home,
            os: current_os(),
        })
    }

    /// Create a `RealContext` with a caller-supplied `home`, picking up
    /// the OS from the current compilation target. Useful for tests and
    /// for callers that already know HOME (e.g. `configure_global`).
    #[must_use]
    pub(crate) fn with_home(home: PathBuf) -> Self {
        Self {
            home,
            os: current_os(),
        }
    }
}

impl DetectionContext for RealContext {
    fn env_var(&self, name: &str) -> Option<String> {
        std::env::var(name).ok()
    }

    fn binary_on_path(&self, name: &str) -> bool {
        let Some(path) = std::env::var_os("PATH") else {
            return false;
        };
        let extensions: Vec<String> = if self.os == TargetOs::Windows {
            std::env::var("PATHEXT")
                .unwrap_or_else(|_| ".EXE;.BAT;.CMD;.COM".to_string())
                .split(';')
                .filter(|s| !s.is_empty())
                .map(|s| s.trim_start_matches('.').to_ascii_lowercase())
                .collect()
        } else {
            Vec::new()
        };
        std::env::split_paths(&path).any(|dir| {
            let candidate = dir.join(name);
            if candidate.is_file() {
                return true;
            }
            extensions
                .iter()
                .any(|ext| candidate.with_extension(ext).is_file())
        })
    }

    fn path_exists(&self, path: &Path) -> bool {
        path.exists()
    }

    fn home(&self) -> &Path {
        &self.home
    }

    fn os(&self) -> TargetOs {
        self.os
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::{Agent, Detection};
    use std::collections::{HashMap, HashSet};

    struct MockCtx {
        env: HashMap<String, String>,
        path_bins: HashSet<String>,
        fs: HashSet<PathBuf>,
        home: PathBuf,
        os: TargetOs,
    }

    impl DetectionContext for MockCtx {
        fn env_var(&self, name: &str) -> Option<String> {
            self.env.get(name).cloned()
        }
        fn binary_on_path(&self, name: &str) -> bool {
            self.path_bins.contains(name)
        }
        fn path_exists(&self, path: &Path) -> bool {
            self.fs.contains(path)
        }
        fn home(&self) -> &Path {
            &self.home
        }
        fn os(&self) -> TargetOs {
            self.os
        }
    }

    fn sample_agent() -> Agent {
        Agent {
            id: "x",
            display_name: "X",
            detection: Detection {
                env_vars: &["XCODE"],
                binaries: &["x"],
                config_dirs: &[".x"],
                app_bundles: &["/Applications/X.app"],
            },
            repo_config: None,
            global_config: None,
            tool_prefix: "mcp__x__",
        }
    }

    fn empty_ctx() -> MockCtx {
        MockCtx {
            env: HashMap::new(),
            path_bins: HashSet::new(),
            fs: HashSet::new(),
            home: PathBuf::from("/home/test"),
            os: TargetOs::Linux,
        }
    }

    #[test]
    fn active_when_env_var_set() {
        let mut ctx = empty_ctx();
        ctx.env.insert("XCODE".into(), "1".into());
        assert_eq!(detect_level(&sample_agent(), &ctx), DetectionLevel::Active);
    }

    #[test]
    fn installed_when_binary_on_path() {
        let mut ctx = empty_ctx();
        ctx.path_bins.insert("x".into());
        assert_eq!(
            detect_level(&sample_agent(), &ctx),
            DetectionLevel::Installed
        );
    }

    #[test]
    fn installed_when_config_dir_exists() {
        let mut ctx = empty_ctx();
        ctx.fs.insert(ctx.home.join(".x"));
        assert_eq!(
            detect_level(&sample_agent(), &ctx),
            DetectionLevel::Installed
        );
    }

    #[test]
    fn installed_when_app_bundle_exists_on_macos() {
        let mut ctx = empty_ctx();
        ctx.os = TargetOs::MacOs;
        ctx.fs.insert(PathBuf::from("/Applications/X.app"));
        assert_eq!(
            detect_level(&sample_agent(), &ctx),
            DetectionLevel::Installed
        );
    }

    #[test]
    fn app_bundle_ignored_on_linux() {
        let mut ctx = empty_ctx();
        ctx.fs.insert(PathBuf::from("/Applications/X.app"));
        assert_eq!(detect_level(&sample_agent(), &ctx), DetectionLevel::Unknown);
    }

    #[test]
    fn unknown_when_no_signal() {
        assert_eq!(
            detect_level(&sample_agent(), &empty_ctx()),
            DetectionLevel::Unknown
        );
    }

    #[test]
    fn env_var_wins_over_installed_signal() {
        let mut ctx = empty_ctx();
        ctx.env.insert("XCODE".into(), "1".into());
        ctx.path_bins.insert("x".into());
        assert_eq!(detect_level(&sample_agent(), &ctx), DetectionLevel::Active);
    }
}
