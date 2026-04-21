use serde::Deserialize;
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct DirectDep {
    pub name: String,
    pub version_req: String,
    pub features: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct LockedDep {
    pub name: String,
    pub version: String,
    pub source: Option<String>,
}

#[derive(Debug, Clone)]
#[non_exhaustive]
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
    Ok(parsed
        .dependencies
        .unwrap_or_default()
        .iter()
        .filter(|(_, v)| matches!(v, toml::Value::String(_) | toml::Value::Table(_)))
        .map(|(name, value)| {
            let (version_req, features) =
                crate::indexer::workspace::extract_version_features(value);
            DirectDep {
                name: name.clone(),
                version_req,
                features,
            }
        })
        .collect())
}

pub(crate) fn parse_cargo_lock(content: &str) -> Result<Vec<LockedDep>, toml::de::Error> {
    let parsed: CargoLock = toml::from_str(content)?;
    Ok(parsed
        .package
        .unwrap_or_default()
        .into_iter()
        .map(|pkg| LockedDep {
            name: pkg.name,
            version: pkg.version,
            source: pkg.source,
        })
        .collect())
}

/// Extract a repository URL from a Cargo.lock `source` field.
/// Git sources look like `git+https://github.com/user/repo?branch=main#commit`.
fn repo_url_from_lock_source(source: Option<&String>) -> Option<String> {
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

    locked
        .iter()
        .map(|lock| {
            let direct_entry = direct_names.get(lock.name.as_str()).copied();
            let features = direct_entry.map(|d| d.features.clone()).unwrap_or_default();
            ResolvedDep {
                name: lock.name.clone(),
                version: lock.version.clone(),
                is_direct: direct_entry.is_some(),
                repository_url: repo_url_from_lock_source(lock.source.as_ref()),
                features,
            }
        })
        .collect()
}

/// Parse Python dependencies from `pyproject.toml` or `requirements.txt`.
pub fn parse_python_deps(
    repo_path: &std::path::Path,
) -> Result<Vec<DirectDep>, Box<dyn std::error::Error>> {
    // Try pyproject.toml first
    let pyproject = repo_path.join("pyproject.toml");
    if pyproject.exists() {
        let content = std::fs::read_to_string(&pyproject)?;
        let deps = parse_pyproject_deps(&content);
        if !deps.is_empty() {
            return Ok(deps);
        }
    }

    // Fall back to requirements.txt
    let req = repo_path.join("requirements.txt");
    if req.exists() {
        let content = std::fs::read_to_string(&req)?;
        return Ok(parse_requirements_txt(&content));
    }

    Ok(Vec::new())
}

fn parse_pyproject_deps(content: &str) -> Vec<DirectDep> {
    let Ok(parsed) = toml::from_str::<toml::Value>(content) else {
        return Vec::new();
    };
    let mut deps = Vec::new();

    // PEP 621: [project.dependencies]
    if let Some(arr) = parsed
        .get("project")
        .and_then(|p| p.get("dependencies"))
        .and_then(toml::Value::as_array)
    {
        for item in arr {
            if let Some(spec) = item.as_str()
                && let Some(dep) = parse_pep508_spec(spec)
            {
                deps.push(dep);
            }
        }
    }

    // PEP 621: [project.optional-dependencies]
    if let Some(table) = parsed
        .get("project")
        .and_then(|p| p.get("optional-dependencies"))
        .and_then(toml::Value::as_table)
    {
        for arr in table.values() {
            if let Some(arr) = arr.as_array() {
                for item in arr {
                    if let Some(spec) = item.as_str()
                        && let Some(dep) = parse_pep508_spec(spec)
                    {
                        deps.push(dep);
                    }
                }
            }
        }
    }

    // Poetry: [tool.poetry.dependencies]
    if let Some(table) = parsed
        .get("tool")
        .and_then(|t| t.get("poetry"))
        .and_then(|p| p.get("dependencies"))
        .and_then(toml::Value::as_table)
    {
        for (name, value) in table {
            if name == "python" {
                continue;
            }
            let version_req = match value {
                toml::Value::String(s) => s.clone(),
                toml::Value::Table(t) => t
                    .get("version")
                    .and_then(toml::Value::as_str)
                    .unwrap_or("*")
                    .to_string(),
                _ => "*".to_string(),
            };
            deps.push(DirectDep {
                name: name.clone(),
                version_req,
                features: Vec::new(),
            });
        }
    }

    deps
}

/// Parse a PEP 508 dependency specifier like `requests>=2.28,<3.0`.
fn parse_pep508_spec(spec: &str) -> Option<DirectDep> {
    let spec = spec.trim();
    if spec.is_empty() || spec.starts_with('#') {
        return None;
    }

    // Split on version operators to extract name and version
    let name_end = spec
        .find(['>', '<', '=', '!', '~', '[', ';', ' '])
        .unwrap_or(spec.len());
    let name = spec[..name_end].trim();
    if name.is_empty() {
        return None;
    }

    let version_part = spec[name_end..].trim();
    // Strip extras [foo,bar] and environment markers ; ...
    let no_markers = version_part.split(';').next().unwrap_or("").trim();
    // Remove extras section: [async] or [security,test]
    let version_req = if let Some(bracket_start) = no_markers.find('[') {
        let after_bracket = no_markers[bracket_start..]
            .find(']')
            .map_or("", |end| &no_markers[bracket_start + end + 1..]);
        format!("{}{}", &no_markers[..bracket_start], after_bracket)
            .trim()
            .to_string()
    } else {
        no_markers.to_string()
    };
    let version_req = version_req.trim();
    let version_req = if version_req.is_empty() {
        "*".to_string()
    } else {
        version_req.to_string()
    };

    Some(DirectDep {
        name: name.to_string(),
        version_req,
        features: Vec::new(),
    })
}

fn parse_requirements_txt(content: &str) -> Vec<DirectDep> {
    let mut deps = Vec::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with('-') {
            continue;
        }
        if let Some(dep) = parse_pep508_spec(line) {
            deps.push(dep);
        }
    }
    deps
}

/// Parse `package.json` to extract dependencies as `DirectDep` entries.
pub fn parse_package_json(content: &str) -> Result<Vec<DirectDep>, serde_json::Error> {
    let parsed: serde_json::Value = serde_json::from_str(content)?;

    let mut result = Vec::new();

    for section in &["dependencies", "devDependencies"] {
        if let Some(obj) = parsed.get(section).and_then(serde_json::Value::as_object) {
            for (name, value) in obj {
                let version_req = value.as_str().unwrap_or("*").to_string();
                result.push(DirectDep {
                    name: name.clone(),
                    version_req,
                    features: Vec::new(),
                });
            }
        }
    }

    Ok(result)
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
    fn test_parse_package_json_deps() {
        let content = r#"{
  "name": "my-app",
  "version": "1.0.0",
  "dependencies": {
    "react": "^18.0.0",
    "@tauri-apps/api": "^2.0.0"
  },
  "devDependencies": {
    "typescript": "^5.0.0",
    "vitest": "^1.0.0"
  }
}"#;
        let deps = parse_package_json(content).unwrap();
        assert_eq!(deps.len(), 4);
        assert!(deps.iter().any(|d| d.name == "react"));
        assert!(deps.iter().any(|d| d.name == "@tauri-apps/api"));
        assert!(deps.iter().any(|d| d.name == "typescript"));
        assert!(deps.iter().any(|d| d.name == "vitest"));
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

    #[test]
    fn test_parse_pyproject_deps_pep621() {
        let content = r#"
[project]
name = "my-app"
dependencies = [
    "requests>=2.28",
    "flask[async]>=2.0,<3.0",
    "click",
]

[project.optional-dependencies]
dev = ["pytest>=7.0", "ruff"]
"#;
        let deps = parse_pyproject_deps(content);
        assert!(deps.iter().any(|d| d.name == "requests"), "deps: {deps:?}");
        assert!(deps.iter().any(|d| d.name == "flask"), "deps: {deps:?}");
        assert!(deps.iter().any(|d| d.name == "click"), "deps: {deps:?}");
        assert!(deps.iter().any(|d| d.name == "pytest"), "deps: {deps:?}");
        assert!(deps.iter().any(|d| d.name == "ruff"), "deps: {deps:?}");
    }

    #[test]
    fn test_parse_pyproject_deps_poetry() {
        let content = r#"
[tool.poetry]
name = "poetry-app"

[tool.poetry.dependencies]
python = "^3.11"
requests = "^2.28"
flask = {version = "^2.0", extras = ["async"]}
"#;
        let deps = parse_pyproject_deps(content);
        // "python" should be skipped
        assert!(!deps.iter().any(|d| d.name == "python"));
        assert!(deps.iter().any(|d| d.name == "requests"));
        assert!(deps.iter().any(|d| d.name == "flask"));
    }

    #[test]
    fn test_parse_requirements_txt() {
        let content = r"
# This is a comment
requests>=2.28
flask==2.0.1
-r other-requirements.txt
click

# Dev tools
pytest>=7.0
";
        let deps = parse_requirements_txt(content);
        assert_eq!(deps.len(), 4);
        assert!(deps.iter().any(|d| d.name == "requests"));
        assert!(deps.iter().any(|d| d.name == "flask"));
        assert!(deps.iter().any(|d| d.name == "click"));
        assert!(deps.iter().any(|d| d.name == "pytest"));
    }
}
