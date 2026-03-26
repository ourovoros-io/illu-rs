use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Parsed tsconfig paths configuration.
#[derive(Debug, Default)]
pub struct TsConfigPaths {
    base_url: Option<PathBuf>,
    paths: HashMap<String, Vec<String>>,
}

/// Parse `tsconfig.json` and extract `compilerOptions.paths` + `baseUrl`.
pub fn parse_tsconfig_paths(
    repo_path: &Path,
) -> TsConfigPaths {
    let tsconfig_path = repo_path.join("tsconfig.json");
    let Ok(content) = std::fs::read_to_string(&tsconfig_path) else {
        return TsConfigPaths::default();
    };

    // Strip single-line comments (tsconfig allows them)
    let stripped = strip_json_comments(&content);
    let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&stripped) else {
        return TsConfigPaths::default();
    };

    let Some(compiler) = parsed.get("compilerOptions") else {
        return TsConfigPaths::default();
    };

    let base_url = compiler
        .get("baseUrl")
        .and_then(serde_json::Value::as_str)
        .map(|b| repo_path.join(b));

    let mut paths = HashMap::new();
    if let Some(obj) = compiler.get("paths").and_then(serde_json::Value::as_object) {
            for (pattern, targets) in obj {
                if let Some(arr) = targets.as_array() {
                    let resolved: Vec<String> = arr
                        .iter()
                        .filter_map(serde_json::Value::as_str)
                        .map(String::from)
                        .collect();
                    paths.insert(pattern.clone(), resolved);
                }
            }
    }

    TsConfigPaths { base_url, paths }
}

/// Resolve a TS import path to a filesystem-relative path.
///
/// Returns `None` for bare specifiers (npm packages like `react`).
#[must_use]
pub fn resolve_ts_import(
    import_path: &str,
    current_file: &str,
    repo_path: &Path,
    tsconfig: &TsConfigPaths,
) -> Option<String> {
    // Relative imports: ./foo, ../bar
    if import_path.starts_with("./")
        || import_path.starts_with("../")
    {
        let current_dir =
            Path::new(current_file).parent()?;
        let target = normalize_path(
            &current_dir.join(import_path),
        );
        return probe_ts_file(repo_path, &target);
    }

    // Try tsconfig paths aliases
    for (pattern, targets) in &tsconfig.paths {
        if let Some(matched) = match_ts_path_pattern(
            pattern,
            import_path,
        ) {
            for target_pattern in targets {
                let resolved = target_pattern
                    .replace('*', &matched);
                let base = tsconfig
                    .base_url
                    .as_deref()
                    .unwrap_or(repo_path);
                let target = base.join(&resolved);
                if let Some(path) =
                    probe_ts_file(repo_path, &target)
                {
                    return Some(path);
                }
            }
        }
    }

    // Bare specifier (npm package) — not resolvable
    None
}

/// Normalize a path by resolving `.` and `..` components
/// without requiring the path to exist on disk.
fn normalize_path(path: &Path) -> PathBuf {
    let mut result = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                result.pop();
            }
            other => result.push(other),
        }
    }
    result
}

/// Match a tsconfig path pattern like `@/*` against an import.
/// Returns the captured wildcard portion if matched.
fn match_ts_path_pattern(
    pattern: &str,
    import: &str,
) -> Option<String> {
    if let Some(prefix) = pattern.strip_suffix('*') {
        if let Some(rest) = import.strip_prefix(prefix) {
            return Some(rest.to_string());
        }
    } else if pattern == import {
        return Some(String::new());
    }
    None
}

/// Probe for a TypeScript file with various extensions.
/// Given `src/foo`, tries: `src/foo.ts`, `src/foo.tsx`,
/// `src/foo/index.ts`, `src/foo/index.tsx`.
fn probe_ts_file(
    repo_path: &Path,
    target: &Path,
) -> Option<String> {
    let extensions = ["ts", "tsx", "js", "jsx"];

    // If target is already absolute, make it relative to repo
    let target = if target.is_absolute() {
        target
            .strip_prefix(repo_path)
            .unwrap_or(target)
            .to_path_buf()
    } else {
        target.to_path_buf()
    };

    // Already has a known extension? Check as-is first.
    if target
        .extension()
        .is_some_and(|e| extensions.contains(&e.to_str().unwrap_or("")))
        && repo_path.join(&target).exists()
    {
        return Some(
            target.to_string_lossy().to_string(),
        );
    }

    // Try appending extensions (use OsString to avoid
    // `with_extension` which replaces dotted basenames
    // like `vite.config` → `vite.ts` instead of
    // `vite.config.ts`)
    let target_str = target.to_string_lossy();
    for ext in &extensions {
        let appended =
            PathBuf::from(format!("{target_str}.{ext}"));
        if repo_path.join(&appended).exists() {
            return Some(
                appended.to_string_lossy().to_string(),
            );
        }
    }

    // Try as directory with index file
    for ext in &extensions {
        let index = target.join(format!("index.{ext}"));
        if repo_path.join(&index).exists() {
            return Some(
                index.to_string_lossy().to_string(),
            );
        }
    }

    None
}

/// Strip single-line `//` and multi-line `/* */` comments
/// from JSON (tsconfig allows JSONC).
fn strip_json_comments(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    let mut in_string = false;

    while let Some(&c) = chars.peek() {
        if in_string {
            result.push(c);
            chars.next();
            if c == '\\' {
                // Escaped char in string — push next too
                if let Some(&next) = chars.peek() {
                    result.push(next);
                    chars.next();
                }
            } else if c == '"' {
                in_string = false;
            }
        } else if c == '"' {
            in_string = true;
            result.push(c);
            chars.next();
        } else if c == '/' {
            chars.next();
            match chars.peek() {
                Some('/') => {
                    // Line comment — skip to end of line
                    for ch in chars.by_ref() {
                        if ch == '\n' {
                            result.push('\n');
                            break;
                        }
                    }
                }
                Some('*') => {
                    // Block comment — skip to */
                    chars.next();
                    let mut prev = ' ';
                    for ch in chars.by_ref() {
                        if prev == '*' && ch == '/' {
                            break;
                        }
                        prev = ch;
                    }
                }
                _ => {
                    result.push('/');
                }
            }
        } else {
            result.push(c);
            chars.next();
        }
    }
    result
}

/// Parse npm workspaces from `package.json`.
/// Returns list of workspace member glob patterns.
pub fn parse_npm_workspaces(
    repo_path: &Path,
) -> Vec<String> {
    let pkg_path = repo_path.join("package.json");
    let Ok(content) = std::fs::read_to_string(&pkg_path) else {
        return Vec::new();
    };
    let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&content) else {
        return Vec::new();
    };

    // npm/yarn workspaces field
    match parsed.get("workspaces") {
        Some(serde_json::Value::Array(arr)) => arr
            .iter()
            .filter_map(serde_json::Value::as_str)
            .map(String::from)
            .collect(),
        Some(serde_json::Value::Object(obj)) => {
            // Yarn workspaces object: { "packages": [...] }
            obj.get("packages")
                .and_then(serde_json::Value::as_array)
                .map(|arr| {
                    arr.iter()
                        .filter_map(serde_json::Value::as_str)
                        .map(String::from)
                        .collect()
                })
                .unwrap_or_default()
        }
        _ => Vec::new(),
    }
}

/// Resolve workspace glob patterns to actual directory paths.
#[must_use]
pub fn resolve_workspace_members(
    repo_path: &Path,
    patterns: &[String],
) -> Vec<PathBuf> {
    let mut members = Vec::new();
    for pattern in patterns {
        if pattern.contains('*') {
            // Glob pattern: packages/*
            let prefix = pattern
                .split('*')
                .next()
                .unwrap_or("");
            let parent = repo_path.join(prefix);
            if let Ok(entries) = std::fs::read_dir(&parent) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_dir()
                        && path.join("package.json").exists()
                    {
                        members.push(path);
                    }
                }
            }
        } else {
            // Exact path
            let path = repo_path.join(pattern);
            if path.is_dir()
                && path.join("package.json").exists()
            {
                members.push(path);
            }
        }
    }
    members
}

#[cfg(test)]
#[expect(clippy::unwrap_used, reason = "tests")]
mod tests {
    use super::*;

    #[test]
    fn test_strip_json_comments() {
        let input = r#"{
  // This is a comment
  "foo": "bar", /* inline comment */
  "baz": "qux" // trailing
}"#;
        let result = strip_json_comments(input);
        let parsed: serde_json::Value =
            serde_json::from_str(&result).unwrap();
        assert_eq!(
            parsed.get("foo").unwrap().as_str(),
            Some("bar")
        );
        assert_eq!(
            parsed.get("baz").unwrap().as_str(),
            Some("qux")
        );
    }

    #[test]
    fn test_match_ts_path_pattern() {
        assert_eq!(
            match_ts_path_pattern(
                "@/*",
                "@/components/Foo"
            ),
            Some("components/Foo".to_string())
        );
        assert_eq!(
            match_ts_path_pattern("@utils/*", "@utils/bar"),
            Some("bar".to_string())
        );
        assert_eq!(
            match_ts_path_pattern("@/*", "react"),
            None
        );
        assert_eq!(
            match_ts_path_pattern("exact", "exact"),
            Some(String::new())
        );
    }

    #[test]
    fn test_probe_ts_file() {
        let dir = tempfile::TempDir::new().unwrap();
        let src = dir.path().join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(
            src.join("utils.ts"),
            "export function foo() {}",
        )
        .unwrap();

        let result = probe_ts_file(
            dir.path(),
            &PathBuf::from("src/utils"),
        );
        assert_eq!(
            result.as_deref(),
            Some("src/utils.ts")
        );
    }

    #[test]
    fn test_probe_dotted_basename() {
        let dir = tempfile::TempDir::new().unwrap();
        let src = dir.path().join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(
            src.join("vite.config.ts"),
            "export default {}",
        )
        .unwrap();

        let result = probe_ts_file(
            dir.path(),
            &PathBuf::from("src/vite.config"),
        );
        assert_eq!(
            result.as_deref(),
            Some("src/vite.config.ts"),
            "dotted basenames must not lose segments"
        );
    }

    #[test]
    fn test_probe_index_file() {
        let dir = tempfile::TempDir::new().unwrap();
        let comp = dir.path().join("src/components");
        std::fs::create_dir_all(&comp).unwrap();
        std::fs::write(
            comp.join("index.ts"),
            "export {}",
        )
        .unwrap();

        let result = probe_ts_file(
            dir.path(),
            &PathBuf::from("src/components"),
        );
        assert_eq!(
            result.as_deref(),
            Some("src/components/index.ts")
        );
    }

    #[test]
    fn test_resolve_relative_import() {
        let dir = tempfile::TempDir::new().unwrap();
        let src = dir.path().join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(
            src.join("utils.ts"),
            "export function foo() {}",
        )
        .unwrap();

        let tsconfig = TsConfigPaths::default();
        let result = resolve_ts_import(
            "./utils",
            "src/main.ts",
            dir.path(),
            &tsconfig,
        );
        assert_eq!(
            result.as_deref(),
            Some("src/utils.ts")
        );
    }

    #[test]
    fn test_resolve_alias_import() {
        let dir = tempfile::TempDir::new().unwrap();
        let comp = dir.path().join("src/components");
        std::fs::create_dir_all(&comp).unwrap();
        std::fs::write(
            comp.join("Button.tsx"),
            "export default function Button() {}",
        )
        .unwrap();

        let tsconfig = TsConfigPaths {
            base_url: Some(dir.path().to_path_buf()),
            paths: HashMap::from([(
                "@/*".to_string(),
                vec!["src/*".to_string()],
            )]),
        };
        let result = resolve_ts_import(
            "@/components/Button",
            "src/App.tsx",
            dir.path(),
            &tsconfig,
        );
        assert_eq!(
            result.as_deref(),
            Some("src/components/Button.tsx")
        );
    }

    #[test]
    fn test_bare_specifier_returns_none() {
        let dir = tempfile::TempDir::new().unwrap();
        let tsconfig = TsConfigPaths::default();
        let result = resolve_ts_import(
            "react",
            "src/App.tsx",
            dir.path(),
            &tsconfig,
        );
        assert!(result.is_none());
    }

    #[test]
    fn test_parse_npm_workspaces() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(
            dir.path().join("package.json"),
            r#"{"workspaces": ["packages/*", "apps/web"]}"#,
        )
        .unwrap();

        let result = parse_npm_workspaces(dir.path());
        assert_eq!(result, vec!["packages/*", "apps/web"]);
    }

    #[test]
    fn test_parse_npm_workspaces_yarn_format() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(
            dir.path().join("package.json"),
            r#"{"workspaces": {"packages": ["packages/*"]}}"#,
        )
        .unwrap();

        let result = parse_npm_workspaces(dir.path());
        assert_eq!(result, vec!["packages/*"]);
    }
}
