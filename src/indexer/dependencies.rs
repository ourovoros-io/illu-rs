use serde::Deserialize;
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectDep {
    pub name: String,
    pub version_req: String,
    pub features: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LockedDep {
    pub name: String,
    pub version: String,
    pub source: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedDep {
    pub name: String,
    pub version: String,
    pub is_direct: bool,
    pub repository_url: Option<String>,
    pub features: Vec<String>,
}

#[derive(Deserialize)]
struct CargoToml {
    dependencies: Option<HashMap<String, toml::Value>>,
}

#[derive(Deserialize)]
struct CargoLock {
    package: Option<Vec<LockPackage>>,
}

#[derive(Deserialize)]
struct LockPackage {
    name: String,
    version: String,
    source: Option<String>,
}

pub fn parse_cargo_toml(content: &str) -> Result<Vec<DirectDep>, toml::de::Error> {
    let parsed: CargoToml = toml::from_str(content)?;
    let Some(deps) = parsed.dependencies else {
        return Ok(vec![]);
    };
    let mut result = Vec::new();
    for (name, value) in &deps {
        match value {
            toml::Value::String(_) | toml::Value::Table(_) => {}
            toml::Value::Integer(_)
            | toml::Value::Float(_)
            | toml::Value::Boolean(_)
            | toml::Value::Datetime(_)
            | toml::Value::Array(_) => continue,
        }
        let (version_req, features) = crate::indexer::workspace::extract_version_features(value);
        result.push(DirectDep {
            name: name.clone(),
            version_req,
            features,
        });
    }
    Ok(result)
}

pub(crate) fn parse_cargo_lock(content: &str) -> Result<Vec<LockedDep>, toml::de::Error> {
    let parsed: CargoLock = toml::from_str(content)?;
    let Some(packages) = parsed.package else {
        return Ok(vec![]);
    };
    let mut result = Vec::new();
    for pkg in packages {
        result.push(LockedDep {
            name: pkg.name,
            version: pkg.version,
            source: pkg.source,
        });
    }
    Ok(result)
}

/// Extract a repository URL from a Cargo.lock `source` field.
/// Git sources look like `git+https://github.com/user/repo?branch=main#commit`.
fn repo_url_from_lock_source(source: Option<&str>) -> Option<String> {
    let source = source?;
    let url = source.strip_prefix("git+")?;
    // Strip fragment (#commit) and query (?branch=...)
    let url = url.split('#').next()?;
    let url = url.split('?').next()?;
    Some(url.to_string())
}

#[must_use]
pub fn resolve_dependencies(direct: &[DirectDep], locked: &[LockedDep]) -> Vec<ResolvedDep> {
    let direct_names: HashMap<&str, &DirectDep> =
        direct.iter().map(|d| (d.name.as_str(), d)).collect();

    let mut result = Vec::new();
    for lock in locked {
        let is_direct = direct_names.contains_key(lock.name.as_str());
        let features = direct_names
            .get(lock.name.as_str())
            .map(|d| d.features.clone())
            .unwrap_or_default();
        result.push(ResolvedDep {
            name: lock.name.clone(),
            version: lock.version.clone(),
            is_direct,
            repository_url: repo_url_from_lock_source(lock.source.as_deref()),
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
    fn test_parse_cargo_toml_direct_deps() {
        let toml_content = r#"
[package]
name = "test-project"
version = "0.1.0"

[dependencies]
serde = { version = "1.0", features = ["derive"] }
tokio = "1"
"#;
        let deps = parse_cargo_toml(toml_content).unwrap();
        assert_eq!(deps.len(), 2);
        assert!(deps.iter().any(|d| d.name == "serde"));
        assert!(deps.iter().any(|d| d.name == "tokio"));
    }

    #[test]
    fn test_parse_cargo_lock_versions() {
        let lock_content = r#"
[[package]]
name = "serde"
version = "1.0.210"
source = "registry+https://github.com/rust-lang/crates.io-index"

[[package]]
name = "serde_derive"
version = "1.0.210"
source = "registry+https://github.com/rust-lang/crates.io-index"
"#;
        let locked = parse_cargo_lock(lock_content).unwrap();
        assert_eq!(locked.len(), 2);
        assert_eq!(locked[0].version, "1.0.210");
    }

    #[test]
    fn test_classify_direct_vs_transitive() {
        let direct = vec![DirectDep {
            name: "serde".into(),
            version_req: "1.0".into(),
            features: vec!["derive".into()],
        }];
        let locked = vec![
            LockedDep {
                name: "serde".into(),
                version: "1.0.210".into(),
                source: None,
            },
            LockedDep {
                name: "serde_derive".into(),
                version: "1.0.210".into(),
                source: None,
            },
        ];
        let resolved = resolve_dependencies(&direct, &locked);
        assert!(
            resolved
                .iter()
                .find(|d| d.name == "serde")
                .unwrap()
                .is_direct
        );
        assert!(
            !resolved
                .iter()
                .find(|d| d.name == "serde_derive")
                .unwrap()
                .is_direct
        );
    }

    #[test]
    fn test_git_dep_repo_url_extracted() {
        let direct = vec![DirectDep {
            name: "my_sdk".into(),
            version_req: "*".into(),
            features: vec![],
        }];
        let locked = vec![LockedDep {
            name: "my_sdk".into(),
            version: "0.1.0".into(),
            source: Some("git+https://github.com/user/my-sdk?branch=main#abc123".into()),
        }];
        let resolved = resolve_dependencies(&direct, &locked);
        let dep = resolved.iter().find(|d| d.name == "my_sdk").unwrap();
        assert_eq!(
            dep.repository_url.as_deref(),
            Some("https://github.com/user/my-sdk")
        );
    }

    #[test]
    fn test_registry_dep_no_repo_url() {
        let direct = vec![];
        let locked = vec![LockedDep {
            name: "serde".into(),
            version: "1.0.210".into(),
            source: Some("registry+https://github.com/rust-lang/crates.io-index".into()),
        }];
        let resolved = resolve_dependencies(&direct, &locked);
        assert!(resolved[0].repository_url.is_none());
    }

    #[test]
    fn test_features_extracted() {
        let toml_content = r#"
[package]
name = "test"
version = "0.1.0"

[dependencies]
serde = { version = "1.0", features = ["derive", "rc"] }
"#;
        let deps = parse_cargo_toml(toml_content).unwrap();
        let serde = deps.iter().find(|d| d.name == "serde").unwrap();
        assert_eq!(serde.features, vec!["derive", "rc"]);
    }
}
