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

pub fn detect_level(agent: &Agent, ctx: &dyn DetectionContext) -> DetectionLevel {
    // Active: any env var present
    for var in agent.detection.env_vars {
        if ctx.env_var(var).is_some() {
            return DetectionLevel::Active;
        }
    }
    // Installed: binary on PATH, or config dir present, or (macOS) app bundle
    for bin in agent.detection.binaries {
        if ctx.binary_on_path(bin) {
            return DetectionLevel::Installed;
        }
    }
    for rel in agent.detection.config_dirs {
        if ctx.path_exists(&ctx.home().join(rel)) {
            return DetectionLevel::Installed;
        }
    }
    if ctx.os() == TargetOs::MacOs {
        for bundle in agent.detection.app_bundles {
            if ctx.path_exists(Path::new(bundle)) {
                return DetectionLevel::Installed;
            }
        }
    }
    DetectionLevel::Unknown
}

/// Real detection context backed by the process environment and filesystem.
pub struct RealContext {
    home: PathBuf,
    os: TargetOs,
}

impl RealContext {
    pub fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let home = std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .map(PathBuf::from)
            .map_err(|_| "neither HOME nor USERPROFILE set")?;
        let os = match std::env::consts::OS {
            "macos" => TargetOs::MacOs,
            "windows" => TargetOs::Windows,
            _ => TargetOs::Linux,
        };
        Ok(Self { home, os })
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
        std::env::split_paths(&path).any(|dir| {
            let candidate = dir.join(name);
            candidate.is_file() || candidate.with_extension("exe").is_file()
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
