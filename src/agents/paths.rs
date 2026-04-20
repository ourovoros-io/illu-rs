//! Platform-aware resolution of `GlobalPath` into real filesystem paths.

use super::GlobalPath;
use super::detect::TargetOs;
use std::path::{Path, PathBuf};

#[must_use]
pub fn resolve(global: &GlobalPath, os: TargetOs, home: &Path) -> PathBuf {
    match global {
        GlobalPath::Home(rel) => home.join(rel),
        GlobalPath::AppSupport(vendor, file) => match os {
            TargetOs::MacOs => home
                .join("Library/Application Support")
                .join(vendor)
                .join(file),
            TargetOs::Windows => home.join("AppData/Roaming").join(vendor).join(file),
            TargetOs::Linux => home.join(".config").join(vendor).join(file),
        },
        GlobalPath::AppData(vendor, file) => match os {
            TargetOs::Windows => home.join("AppData/Roaming").join(vendor).join(file),
            _ => home.join(".config").join(vendor).join(file),
        },
        GlobalPath::XdgConfig(rel) => home.join(".config").join(rel),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn home_joins_relative() {
        let resolved = resolve(
            &GlobalPath::Home(".claude/settings.json"),
            TargetOs::Linux,
            Path::new("/h"),
        );
        assert_eq!(resolved, Path::new("/h/.claude/settings.json"));
    }

    #[test]
    fn app_support_macos() {
        let resolved = resolve(
            &GlobalPath::AppSupport("Claude", "claude_desktop_config.json"),
            TargetOs::MacOs,
            Path::new("/h"),
        );
        assert_eq!(
            resolved,
            Path::new("/h/Library/Application Support/Claude/claude_desktop_config.json")
        );
    }

    #[test]
    fn app_support_windows() {
        let resolved = resolve(
            &GlobalPath::AppSupport("Claude", "claude_desktop_config.json"),
            TargetOs::Windows,
            Path::new("/h"),
        );
        assert_eq!(
            resolved,
            Path::new("/h/AppData/Roaming/Claude/claude_desktop_config.json")
        );
    }

    #[test]
    fn app_support_linux_uses_xdg_like_path() {
        let resolved = resolve(
            &GlobalPath::AppSupport("Claude", "claude_desktop_config.json"),
            TargetOs::Linux,
            Path::new("/h"),
        );
        assert_eq!(
            resolved,
            Path::new("/h/.config/Claude/claude_desktop_config.json")
        );
    }

    #[test]
    fn xdg_config_uses_dot_config() {
        let resolved = resolve(
            &GlobalPath::XdgConfig("antigravity/mcp.json"),
            TargetOs::Linux,
            Path::new("/h"),
        );
        assert_eq!(resolved, Path::new("/h/.config/antigravity/mcp.json"));
    }

    #[test]
    fn app_data_windows() {
        let resolved = resolve(
            &GlobalPath::AppData("Claude", "config.json"),
            TargetOs::Windows,
            Path::new("/h"),
        );
        assert_eq!(resolved, Path::new("/h/AppData/Roaming/Claude/config.json"));
    }

    #[test]
    fn app_data_non_windows_falls_back_to_dot_config() {
        let resolved = resolve(
            &GlobalPath::AppData("Claude", "config.json"),
            TargetOs::MacOs,
            Path::new("/h"),
        );
        assert_eq!(resolved, Path::new("/h/.config/Claude/config.json"));
    }
}
