use std::path::{Path, PathBuf};

/// Resolve a Python import path to a filesystem-relative path.
///
/// Returns `None` for unresolvable imports (stdlib, third-party).
#[must_use]
pub fn resolve_py_import(
    import_path: &str,
    current_file: &str,
    repo_path: &Path,
) -> Option<String> {
    let relative_level = import_path.chars().take_while(|&c| c == '.').count();
    if relative_level > 0 {
        resolve_relative_import(import_path, current_file, repo_path, relative_level)
    } else {
        resolve_absolute_import(import_path, repo_path)
    }
}

fn resolve_relative_import(
    import_path: &str,
    current_file: &str,
    repo_path: &Path,
    level: usize,
) -> Option<String> {
    let current_dir = Path::new(current_file).parent()?;

    // Walk up `level` directories (level=1 is current package, level=2 is parent, etc.)
    let mut base = current_dir.to_path_buf();
    for _ in 1..level {
        base.pop();
    }

    // Strip leading dots from the path
    let module_part = import_path.trim_start_matches('.');
    if module_part.is_empty() {
        // `from . import bar` — the module is the current package's __init__.py
        probe_py_module(repo_path, &base)
    } else {
        let module_path = module_part.replace('.', "/");
        let target = base.join(&module_path);
        probe_py_module(repo_path, &target)
    }
}

fn resolve_absolute_import(import_path: &str, repo_path: &Path) -> Option<String> {
    let module_path = import_path.replace('.', "/");
    let target = PathBuf::from(&module_path);

    // Try directly from repo root
    if let Some(found) = probe_py_module(repo_path, &target) {
        return Some(found);
    }

    // Try from src/ directory
    let src_target = Path::new("src").join(&module_path);
    if let Some(found) = probe_py_module(repo_path, &src_target) {
        return Some(found);
    }

    // Try package roots from pyproject.toml
    for root in find_python_package_roots(repo_path) {
        let rel_root = root.strip_prefix(repo_path).unwrap_or(&root);
        let rooted_target = rel_root.join(&module_path);
        if let Some(found) = probe_py_module(repo_path, &rooted_target) {
            return Some(found);
        }
    }

    None
}

/// Probe for a Python module given a path stem.
/// Given `foo/bar`, tries: `foo/bar.py`, `foo/bar/__init__.py`.
fn probe_py_module(repo_path: &Path, target: &Path) -> Option<String> {
    // If target is absolute, make relative
    let target = if target.is_absolute() {
        target
            .strip_prefix(repo_path)
            .unwrap_or(target)
            .to_path_buf()
    } else {
        target.to_path_buf()
    };

    // Already a .py file?
    if target.extension().is_some_and(|e| e == "py") && repo_path.join(&target).exists() {
        return Some(target.to_string_lossy().to_string());
    }

    // Try as module file
    let as_file = target.with_extension("py");
    if repo_path.join(&as_file).exists() {
        return Some(as_file.to_string_lossy().to_string());
    }

    // Try as package
    let as_package = target.join("__init__.py");
    if repo_path.join(&as_package).exists() {
        return Some(as_package.to_string_lossy().to_string());
    }

    None
}

/// Find Python package root directories from project configuration.
#[must_use]
pub fn find_python_package_roots(repo_path: &Path) -> Vec<PathBuf> {
    let mut roots = Vec::new();

    // Try pyproject.toml
    let pyproject = repo_path.join("pyproject.toml");
    if let Ok(content) = std::fs::read_to_string(&pyproject)
        && let Ok(parsed) = toml::from_str::<toml::Value>(&content)
    {
        // [tool.setuptools.packages.find] where = ["src"]
        if let Some(where_dirs) = parsed
            .get("tool")
            .and_then(|t| t.get("setuptools"))
            .and_then(|s| s.get("packages"))
            .and_then(|p| p.get("find"))
            .and_then(|f| f.get("where"))
            .and_then(toml::Value::as_array)
        {
            for dir in where_dirs {
                if let Some(s) = dir.as_str() {
                    let path = repo_path.join(s);
                    if path.is_dir() {
                        roots.push(path);
                    }
                }
            }
        }

        // [tool.poetry.packages] = [{include = "mypackage", from = "src"}]
        if let Some(packages) = parsed
            .get("tool")
            .and_then(|t| t.get("poetry"))
            .and_then(|p| p.get("packages"))
            .and_then(toml::Value::as_array)
        {
            for pkg in packages {
                if let Some(from) = pkg.get("from").and_then(toml::Value::as_str) {
                    let path = repo_path.join(from);
                    if path.is_dir() {
                        roots.push(path);
                    }
                }
            }
        }
    }

    // Fallback: look for top-level dirs with __init__.py
    if roots.is_empty()
        && let Ok(entries) = std::fs::read_dir(repo_path)
    {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir()
                && path.join("__init__.py").exists()
                && !super::is_excluded_dir(&path.file_name().unwrap_or_default().to_string_lossy())
            {
                roots.push(path);
            }
        }
    }

    roots
}

/// Extract project name from `pyproject.toml` content.
#[must_use]
pub fn extract_project_name(content: &str) -> Option<String> {
    let parsed: toml::Value = toml::from_str(content).ok()?;
    // PEP 621: [project] name = "..."
    parsed
        .get("project")
        .and_then(|p| p.get("name"))
        .and_then(toml::Value::as_str)
        .map(String::from)
        .or_else(|| {
            // Poetry: [tool.poetry] name = "..."
            parsed
                .get("tool")
                .and_then(|t| t.get("poetry"))
                .and_then(|p| p.get("name"))
                .and_then(toml::Value::as_str)
                .map(String::from)
        })
}

#[cfg(test)]
#[expect(clippy::unwrap_used, reason = "tests")]
mod tests {
    use super::*;

    #[test]
    fn test_probe_py_module_file() {
        let dir = tempfile::TempDir::new().unwrap();
        let src = dir.path().join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("utils.py"), "def foo(): pass").unwrap();

        let result = probe_py_module(dir.path(), &PathBuf::from("src/utils"));
        assert_eq!(result.as_deref(), Some("src/utils.py"));
    }

    #[test]
    fn test_probe_py_package() {
        let dir = tempfile::TempDir::new().unwrap();
        let pkg = dir.path().join("mypackage");
        std::fs::create_dir_all(&pkg).unwrap();
        std::fs::write(pkg.join("__init__.py"), "").unwrap();

        let result = probe_py_module(dir.path(), &PathBuf::from("mypackage"));
        assert_eq!(result.as_deref(), Some("mypackage/__init__.py"));
    }

    #[test]
    fn test_resolve_relative_import() {
        let dir = tempfile::TempDir::new().unwrap();
        let src = dir.path().join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("utils.py"), "def foo(): pass").unwrap();

        let result = resolve_py_import(".utils", "src/main.py", dir.path());
        assert_eq!(result.as_deref(), Some("src/utils.py"));
    }

    #[test]
    fn test_resolve_absolute_import() {
        let dir = tempfile::TempDir::new().unwrap();
        let pkg = dir.path().join("mypackage");
        std::fs::create_dir_all(&pkg).unwrap();
        std::fs::write(pkg.join("__init__.py"), "").unwrap();
        std::fs::write(pkg.join("core.py"), "def bar(): pass").unwrap();

        let result = resolve_py_import("mypackage.core", "main.py", dir.path());
        assert_eq!(result.as_deref(), Some("mypackage/core.py"));
    }

    #[test]
    fn test_bare_specifier_returns_none() {
        let dir = tempfile::TempDir::new().unwrap();
        let result = resolve_py_import("requests", "src/main.py", dir.path());
        assert!(result.is_none());
    }

    #[test]
    fn test_extract_project_name_pep621() {
        let content = r#"
[project]
name = "my-cool-app"
version = "1.0.0"
"#;
        assert_eq!(
            extract_project_name(content).as_deref(),
            Some("my-cool-app")
        );
    }

    #[test]
    fn test_extract_project_name_poetry() {
        let content = r#"
[tool.poetry]
name = "poetry-app"
version = "0.1.0"
"#;
        assert_eq!(extract_project_name(content).as_deref(), Some("poetry-app"));
    }

    #[test]
    fn test_find_package_roots_setuptools() {
        let dir = tempfile::TempDir::new().unwrap();
        let src = dir.path().join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(
            dir.path().join("pyproject.toml"),
            r#"
[tool.setuptools.packages.find]
where = ["src"]
"#,
        )
        .unwrap();

        let roots = find_python_package_roots(dir.path());
        assert_eq!(roots.len(), 1);
        assert!(roots[0].ends_with("src"));
    }

    #[test]
    fn test_find_package_roots_fallback() {
        let dir = tempfile::TempDir::new().unwrap();
        let pkg = dir.path().join("mypackage");
        std::fs::create_dir_all(&pkg).unwrap();
        std::fs::write(pkg.join("__init__.py"), "").unwrap();

        let roots = find_python_package_roots(dir.path());
        assert!(roots.iter().any(|r| r.ends_with("mypackage")));
    }
}
