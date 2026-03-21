use crate::indexer::dependencies::DirectDep;
use std::collections::HashMap;

#[derive(Debug)]
pub struct WorkspaceInfo {
    pub is_workspace: bool,
    pub members: Vec<String>,
    pub workspace_deps: Vec<DirectDep>,
}

#[derive(Debug)]
pub struct PathDep {
    pub name: String,
    pub path: String,
}

/// Parse a root `Cargo.toml` to detect whether it defines a workspace.
/// Returns workspace members and workspace-level dependencies.
pub fn parse_workspace_toml(content: &str) -> Result<WorkspaceInfo, toml::de::Error> {
    let parsed: toml::Value = toml::from_str(content)?;

    let Some(workspace) = parsed.get("workspace") else {
        return Ok(WorkspaceInfo {
            is_workspace: false,
            members: vec![],
            workspace_deps: vec![],
        });
    };

    let members = workspace
        .get("members")
        .and_then(toml::Value::as_array)
        .map(|arr| {
            let mut result = Vec::new();
            for v in arr {
                if let Some(s) = v.as_str() {
                    result.push(s.to_string());
                }
            }
            result
        })
        .unwrap_or_default();

    let workspace_deps = parse_deps_table(workspace.get("dependencies"));

    Ok(WorkspaceInfo {
        is_workspace: true,
        members,
        workspace_deps,
    })
}

/// Iterate over all `(name, value)` pairs from dependencies,
/// dev-dependencies, and build-dependencies tables.
fn iter_dep_entries(parsed: &toml::Value) -> Vec<(&String, &toml::Value)> {
    let tables = [
        parsed.get("dependencies"),
        parsed.get("dev-dependencies"),
        parsed.get("build-dependencies"),
    ];
    let mut entries = Vec::new();
    for table in tables.into_iter().flatten() {
        let Some(deps) = table.as_table() else {
            continue;
        };
        for (name, value) in deps {
            entries.push((name, value));
        }
    }
    entries
}

/// Resolve a member crate's dependencies, substituting `workspace = true`
/// entries with definitions from the workspace root.
#[must_use]
pub fn resolve_member_deps(parsed: &toml::Value, ws_deps: &[DirectDep]) -> Vec<DirectDep> {
    let ws_lookup: HashMap<&str, &DirectDep> =
        ws_deps.iter().map(|d| (d.name.as_str(), d)).collect();

    let mut result = Vec::new();
    for (name, value) in iter_dep_entries(parsed) {
        if get_path_value(value).is_some() {
            continue;
        }

        if is_workspace_dep(value) {
            if let Some(ws_dep) = ws_lookup.get(name.as_str()) {
                let mut resolved = (*ws_dep).clone();
                if let Some(member_features) = value.get("features").and_then(toml::Value::as_array)
                {
                    for feat in member_features {
                        if let Some(f) = feat.as_str()
                            && !resolved.features.contains(&f.to_string())
                        {
                            resolved.features.push(f.to_string());
                        }
                    }
                }
                result.push(resolved);
            }
            continue;
        }

        let (version_req, features) = extract_version_features(value);
        result.push(DirectDep {
            name: name.clone(),
            version_req,
            features,
        });
    }

    result
}

/// Extract path-based dependencies from a member's `Cargo.toml`.
/// These represent inter-crate dependencies within the workspace.
#[must_use]
pub fn extract_path_deps(parsed: &toml::Value) -> Vec<PathDep> {
    let mut result = Vec::new();
    for (name, value) in iter_dep_entries(parsed) {
        if let Some(path) = get_path_value(value) {
            result.push(PathDep {
                name: name.clone(),
                path,
            });
        }
    }

    result
}

fn is_workspace_dep(value: &toml::Value) -> bool {
    value
        .as_table()
        .and_then(|t| t.get("workspace"))
        .and_then(toml::Value::as_bool)
        .unwrap_or(false)
}

fn get_path_value(value: &toml::Value) -> Option<String> {
    value
        .as_table()
        .and_then(|t| t.get("path"))
        .and_then(toml::Value::as_str)
        .map(String::from)
}

pub(crate) fn extract_version_features(value: &toml::Value) -> (String, Vec<String>) {
    match value {
        toml::Value::String(v) => (v.clone(), vec![]),
        toml::Value::Table(t) => {
            let version = t
                .get("version")
                .and_then(toml::Value::as_str)
                .unwrap_or("")
                .to_string();
            let features = t
                .get("features")
                .and_then(toml::Value::as_array)
                .map(|arr| {
                    let mut feats = Vec::new();
                    for v in arr {
                        if let Some(s) = v.as_str() {
                            feats.push(s.to_string());
                        }
                    }
                    feats
                })
                .unwrap_or_default();
            (version, features)
        }
        _ => (String::new(), vec![]),
    }
}

fn parse_deps_table(table: Option<&toml::Value>) -> Vec<DirectDep> {
    let Some(deps) = table.and_then(toml::Value::as_table) else {
        return vec![];
    };
    let mut result = Vec::new();
    for (name, value) in deps {
        let (version_req, features) = extract_version_features(value);
        result.push(DirectDep {
            name: name.clone(),
            version_req,
            features,
        });
    }
    result
}

#[cfg(test)]
#[expect(clippy::unwrap_used, reason = "tests")]
mod tests {
    use super::*;

    #[test]
    fn test_detect_workspace() {
        let toml = r#"
[workspace]
members = ["crate-a", "crate-b"]
"#;
        let info = parse_workspace_toml(toml).unwrap();
        assert!(info.is_workspace);
        assert_eq!(info.members, vec!["crate-a", "crate-b"]);
    }

    #[test]
    fn test_detect_single_crate() {
        let toml = r#"
[package]
name = "my-crate"
version = "0.1.0"
"#;
        let info = parse_workspace_toml(toml).unwrap();
        assert!(!info.is_workspace);
        assert!(info.members.is_empty());
    }

    #[test]
    fn test_workspace_deps() {
        let toml = r#"
[workspace]
members = ["app"]

[workspace.dependencies]
serde = { version = "1.0", features = ["derive"] }
tokio = "1"
"#;
        let info = parse_workspace_toml(toml).unwrap();
        assert_eq!(info.workspace_deps.len(), 2);
        let serde = info
            .workspace_deps
            .iter()
            .find(|d| d.name == "serde")
            .unwrap();
        assert_eq!(serde.version_req, "1.0");
        assert_eq!(serde.features, vec!["derive"]);
    }

    #[test]
    fn test_resolve_workspace_dep() {
        let ws_deps = vec![DirectDep {
            name: "serde".into(),
            version_req: "1.0".into(),
            features: vec!["derive".into()],
        }];
        let member_toml = r#"
[package]
name = "my-app"
version = "0.1.0"

[dependencies]
serde = { workspace = true }
custom = "0.5"
"#;
        let resolved = resolve_member_deps(
            &toml::from_str::<toml::Value>(member_toml).unwrap(),
            &ws_deps,
        );
        let serde = resolved.iter().find(|d| d.name == "serde").unwrap();
        assert_eq!(serde.version_req, "1.0");
        let custom = resolved.iter().find(|d| d.name == "custom").unwrap();
        assert_eq!(custom.version_req, "0.5");
    }

    #[test]
    fn test_resolve_workspace_dep_merges_features() {
        let ws_deps = vec![DirectDep {
            name: "serde".into(),
            version_req: "1.0".into(),
            features: vec!["derive".into()],
        }];
        let member_toml = r#"
[package]
name = "my-app"
version = "0.1.0"

[dependencies]
serde = { workspace = true, features = ["json"] }
"#;
        let resolved = resolve_member_deps(
            &toml::from_str::<toml::Value>(member_toml).unwrap(),
            &ws_deps,
        );
        let serde = resolved.iter().find(|d| d.name == "serde").unwrap();
        assert_eq!(serde.version_req, "1.0");
        assert!(serde.features.contains(&"derive".to_string()));
        assert!(serde.features.contains(&"json".to_string()));
        assert_eq!(serde.features.len(), 2);
    }

    #[test]
    fn test_resolve_workspace_dep_no_duplicate_features() {
        let ws_deps = vec![DirectDep {
            name: "serde".into(),
            version_req: "1.0".into(),
            features: vec!["derive".into()],
        }];
        let member_toml = r#"
[package]
name = "my-app"
version = "0.1.0"

[dependencies]
serde = { workspace = true, features = ["derive"] }
"#;
        let resolved = resolve_member_deps(
            &toml::from_str::<toml::Value>(member_toml).unwrap(),
            &ws_deps,
        );
        let serde = resolved.iter().find(|d| d.name == "serde").unwrap();
        assert_eq!(serde.features, vec!["derive"]);
    }

    #[test]
    fn test_detect_inter_crate_deps() {
        let member_toml = r#"
[package]
name = "hcfs-server"
version = "0.1.0"

[dependencies]
hcfs-shared = { path = "../hcfs-shared" }
serde = { workspace = true }
"#;
        let path_deps = extract_path_deps(&toml::from_str::<toml::Value>(member_toml).unwrap());
        assert_eq!(path_deps.len(), 1);
        assert_eq!(path_deps[0].name, "hcfs-shared");
        assert_eq!(path_deps[0].path, "../hcfs-shared");
    }

    #[test]
    fn test_path_deps_excluded_from_resolve() {
        let ws_deps = vec![DirectDep {
            name: "serde".into(),
            version_req: "1.0".into(),
            features: vec![],
        }];
        let member_toml = r#"
[package]
name = "app"
version = "0.1.0"

[dependencies]
shared = { path = "../shared" }
serde = { workspace = true }
"#;
        let resolved = resolve_member_deps(
            &toml::from_str::<toml::Value>(member_toml).unwrap(),
            &ws_deps,
        );
        assert!(!resolved.iter().any(|d| d.name == "shared"));
        assert!(resolved.iter().any(|d| d.name == "serde"));
    }
}
