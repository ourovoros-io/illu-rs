//! Pure selection logic: given detection results and flags, return which agents to configure.

use super::{Agent, DetectionLevel, SetupFlags};

/// How the selection should be resolved based on flags and TTY state.
pub enum Mode {
    /// `--agent X [--agent Y ...]`: caller named exact agent ids.
    Explicit,
    /// `--all`: configure every known agent.
    All,
    /// `--yes` or no TTY: auto-select agents that detection found.
    AutoDetect,
    /// Interactive terminal with no resolving flag: prompt the user.
    Interactive,
}

/// Decide which [`Mode`] applies for the given flag bundle and TTY state.
#[must_use]
pub fn mode(flags: &SetupFlags, has_tty: bool) -> Mode {
    if !flags.explicit_agents.is_empty() {
        Mode::Explicit
    } else if flags.all {
        Mode::All
    } else if flags.yes || !has_tty {
        Mode::AutoDetect
    } else {
        Mode::Interactive
    }
}

/// Select the agents to configure from [`SetupFlags`] and detection results.
///
/// Returns [`SelectionError::NeedsPrompt`] when the caller must defer to an
/// interactive prompt (i.e. we're in [`Mode::Interactive`]).
///
/// # Errors
///
/// - [`SelectionError::UnknownId`] if `flags.explicit_agents` names an id not
///   present in `agents`.
/// - [`SelectionError::NeedsPrompt`] if the mode resolves to
///   [`Mode::Interactive`].
pub fn select_from_flags<'a>(
    agents: &'a [Agent],
    flags: &SetupFlags,
    detection: &[(&'a Agent, DetectionLevel)],
    has_tty: bool,
) -> Result<Vec<&'a Agent>, SelectionError> {
    match mode(flags, has_tty) {
        Mode::Explicit => select_explicit(agents, &flags.explicit_agents),
        Mode::All => Ok(agents.iter().collect()),
        Mode::AutoDetect => Ok(detected(detection)),
        Mode::Interactive => Err(SelectionError::NeedsPrompt),
    }
}

fn select_explicit<'a>(
    agents: &'a [Agent],
    ids: &[String],
) -> Result<Vec<&'a Agent>, SelectionError> {
    let mut out = Vec::with_capacity(ids.len());
    for id in ids {
        match agents.iter().find(|a| a.id == id) {
            Some(a) => out.push(a),
            None => return Err(SelectionError::UnknownId(id.clone())),
        }
    }
    Ok(out)
}

fn detected<'a>(results: &[(&'a Agent, DetectionLevel)]) -> Vec<&'a Agent> {
    results
        .iter()
        .filter(|(_, lvl)| matches!(lvl, DetectionLevel::Active | DetectionLevel::Installed))
        .map(|(a, _)| *a)
        .collect()
}

/// Errors that can arise when resolving the agent selection.
#[derive(Debug, thiserror::Error)]
pub enum SelectionError {
    /// The caller passed `--agent <id>` with an id that isn't in the registry.
    #[error("unknown agent id: {0}")]
    UnknownId(String),
    /// Resolution requires an interactive prompt; the orchestrator must handle it.
    #[error("selection requires an interactive prompt")]
    NeedsPrompt,
}

#[cfg(test)]
#[expect(clippy::unwrap_used, reason = "tests")]
mod tests {
    use super::*;
    use crate::agents::{Detection, McpFormat, RepoConfig};

    fn agent(id: &'static str) -> Agent {
        Agent {
            id,
            display_name: id,
            detection: Detection {
                env_vars: &[],
                binaries: &[],
                config_dirs: &[],
                app_bundles: &[],
            },
            repo_config: Some(RepoConfig {
                mcp_config_path: "x.json",
                mcp_format: McpFormat::ClaudeCodeJson,
                instruction_file: None,
                agents_dir: None,
                allow_list_path: None,
            }),
            global_config: None,
            tool_prefix: "mcp__x__",
        }
    }

    #[test]
    fn explicit_single_agent() {
        let a = agent("x");
        let b = agent("y");
        let agents = &[a, b];
        let flags = SetupFlags {
            explicit_agents: vec!["x".into()],
            ..SetupFlags::default()
        };
        let picked = select_from_flags(agents, &flags, &[], false).unwrap();
        assert_eq!(picked.len(), 1);
        assert_eq!(picked[0].id, "x");
    }

    #[test]
    fn explicit_unknown_errors() {
        let a = agent("x");
        let flags = SetupFlags {
            explicit_agents: vec!["z".into()],
            ..SetupFlags::default()
        };
        let Err(err) = select_from_flags(&[a], &flags, &[], false) else {
            unreachable!("expected error for unknown agent id")
        };
        assert!(matches!(err, SelectionError::UnknownId(_)));
    }

    #[test]
    fn all_selects_every_agent() {
        let a = agent("x");
        let b = agent("y");
        let agents = &[a, b];
        let flags = SetupFlags {
            all: true,
            ..SetupFlags::default()
        };
        let picked = select_from_flags(agents, &flags, &[], false).unwrap();
        assert_eq!(picked.len(), 2);
    }

    #[test]
    fn yes_uses_detection() {
        let a = agent("x");
        let b = agent("y");
        let agents = &[a, b];
        let detection = vec![
            (&agents[0], DetectionLevel::Installed),
            (&agents[1], DetectionLevel::Unknown),
        ];
        let flags = SetupFlags {
            yes: true,
            ..SetupFlags::default()
        };
        let picked = select_from_flags(agents, &flags, &detection, false).unwrap();
        assert_eq!(picked.len(), 1);
        assert_eq!(picked[0].id, "x");
    }

    #[test]
    fn no_tty_behaves_like_yes() {
        let a = agent("x");
        let b = agent("y");
        let agents = &[a, b];
        let detection = vec![
            (&agents[0], DetectionLevel::Active),
            (&agents[1], DetectionLevel::Unknown),
        ];
        let flags = SetupFlags::default();
        let picked = select_from_flags(agents, &flags, &detection, false).unwrap();
        assert_eq!(picked.len(), 1);
        assert_eq!(picked[0].id, "x");
    }

    #[test]
    fn interactive_returns_needs_prompt() {
        let a = agent("x");
        let flags = SetupFlags::default();
        let Err(err) = select_from_flags(&[a], &flags, &[], true) else {
            unreachable!("expected needs-prompt error in interactive mode")
        };
        assert!(matches!(err, SelectionError::NeedsPrompt));
    }
}
