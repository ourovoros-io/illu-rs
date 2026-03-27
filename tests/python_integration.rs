#![expect(clippy::unwrap_used, reason = "integration tests")]

use illu_rs::db::Database;
use illu_rs::indexer::parser::{SymbolKind, Visibility};
use illu_rs::indexer::{IndexConfig, index_repo, refresh_index};
use illu_rs::server::tools::{context, impact, overview, query};

fn write_python_project(dir: &std::path::Path) {
    std::fs::write(
        dir.join("pyproject.toml"),
        r#"[project]
name = "test-app"
version = "1.0.0"
dependencies = [
    "requests>=2.28",
    "click",
]

[project.optional-dependencies]
dev = ["pytest>=7.0"]
"#,
    )
    .unwrap();
}

fn write_python_sources(src: &std::path::Path) {
    std::fs::write(src.join("__init__.py"), "").unwrap();

    std::fs::write(
        src.join("app.py"),
        r#"
from .service import UserService
from .types import Config

def create_app(config: Config) -> None:
    """Initialize and run the application."""
    service = UserService(config.api_url)
    service.fetch_users()

def main() -> None:
    config = Config(api_url="http://localhost", debug=False)
    create_app(config)
"#,
    )
    .unwrap();

    std::fs::write(
        src.join("service.py"),
        r#"
class UserService:
    """Service for managing users."""

    def __init__(self, base_url: str):
        self._base_url = base_url

    def fetch_users(self) -> list:
        """Fetch all users from the API."""
        return []

    def get_user(self, user_id: int):
        """Get a single user by ID."""
        return None

    def _internal_method(self):
        pass
"#,
    )
    .unwrap();

    std::fs::write(
        src.join("types.py"),
        r#"
from dataclasses import dataclass
from enum import Enum

@dataclass
class Config:
    """Application configuration."""
    api_url: str
    debug: bool = False

class LogLevel(Enum):
    """Log levels."""
    DEBUG = "debug"
    INFO = "info"
    ERROR = "error"

class ApiResponse:
    status: int
    data: dict
"#,
    )
    .unwrap();

    std::fs::write(
        src.join("utils.py"),
        r#"
MAX_RETRIES = 3
_INTERNAL_TIMEOUT = 30

def format_date(date) -> str:
    """Format a date for display."""
    return str(date)

async def fetch_data(url: str) -> bytes:
    """Async data fetcher."""
    return b""
"#,
    )
    .unwrap();

    let test_dir = src.join("tests");
    std::fs::create_dir_all(&test_dir).unwrap();
    std::fs::write(test_dir.join("__init__.py"), "").unwrap();
    std::fs::write(
        test_dir.join("test_service.py"),
        r#"
from ..service import UserService

def test_create_service():
    service = UserService("http://test")
    assert service is not None

def test_fetch_users():
    service = UserService("http://test")
    users = service.fetch_users()
    assert users == []

class TestUserService:
    def test_get_user(self):
        service = UserService("http://test")
        assert service.get_user(1) is None
"#,
    )
    .unwrap();
}

fn setup_python_project() -> (tempfile::TempDir, Database) {
    let dir = tempfile::TempDir::new().unwrap();
    let src = dir.path().join("src");
    std::fs::create_dir_all(&src).unwrap();

    write_python_project(dir.path());
    write_python_sources(&src);

    let db = Database::open_in_memory().unwrap();
    let config = IndexConfig {
        repo_path: dir.path().to_path_buf(),
    };
    index_repo(&db, &config).unwrap();

    (dir, db)
}

#[test]
fn test_py_symbols_indexed() {
    let (_dir, db) = setup_python_project();

    // Functions
    let results = db.search_symbols("create_app").unwrap();
    assert!(!results.is_empty(), "create_app should be indexed");
    assert_eq!(results[0].kind, SymbolKind::Function);

    // Classes
    let results = db.search_symbols("UserService").unwrap();
    assert!(!results.is_empty(), "UserService should be indexed");
    assert_eq!(results[0].kind, SymbolKind::Class);

    // Enums
    let results = db.search_symbols("LogLevel").unwrap();
    assert!(!results.is_empty(), "LogLevel should be indexed");
    assert_eq!(results[0].kind, SymbolKind::Enum);

    // Constants
    let results = db.search_symbols("MAX_RETRIES").unwrap();
    assert!(!results.is_empty(), "MAX_RETRIES should be indexed");
    assert_eq!(results[0].kind, SymbolKind::Const);
}

#[test]
fn test_py_class_methods_have_impl_type() {
    let (_dir, db) = setup_python_project();

    let results = db.search_symbols("fetch_users").unwrap();
    assert!(!results.is_empty());
    assert_eq!(results[0].impl_type.as_deref(), Some("UserService"));
}

#[test]
fn test_py_query_tool() {
    let (_dir, db) = setup_python_project();

    let result =
        query::handle_query(&db, "UserService", None, None, None, None, None, None).unwrap();
    assert!(result.contains("UserService"));
    assert!(
        result.contains("class"),
        "query result should show class kind: {result}"
    );
}

#[test]
fn test_py_context_tool() {
    let (_dir, db) = setup_python_project();

    let result =
        context::handle_context(&db, "create_app", false, None, None, None, false).unwrap();
    assert!(
        result.contains("create_app"),
        "context should show create_app"
    );
}

#[test]
fn test_py_overview_tool() {
    let (_dir, db) = setup_python_project();

    let result = overview::handle_overview(&db, "src/", false, None).unwrap();
    assert!(
        result.contains("UserService"),
        "overview should show UserService"
    );
    assert!(result.contains("Config"), "overview should show Config");
}

#[test]
fn test_py_impact_tool() {
    let (_dir, db) = setup_python_project();

    let result = impact::handle_impact(&db, "UserService", None, false, false).unwrap();
    assert!(
        result.contains("UserService"),
        "impact should mention UserService: {result}"
    );
}

#[test]
fn test_py_enum_variants() {
    let (_dir, db) = setup_python_project();

    let results = db.search_symbols("DEBUG").unwrap();
    let variant = results.iter().find(|s| s.kind == SymbolKind::EnumVariant);
    assert!(variant.is_some(), "DEBUG enum variant should be indexed");
    assert_eq!(variant.unwrap().impl_type.as_deref(), Some("LogLevel"));
}

#[test]
fn test_py_doc_comments() {
    let (_dir, db) = setup_python_project();

    let results = db.search_symbols("format_date").unwrap();
    assert!(!results.is_empty());
    assert!(
        results[0]
            .doc_comment
            .as_ref()
            .is_some_and(|d| d.contains("Format a date")),
        "format_date should have docstring: {:?}",
        results[0].doc_comment
    );
}

#[test]
fn test_py_decorated_class() {
    let (_dir, db) = setup_python_project();

    let results = db.search_symbols("Config").unwrap();
    let config = results.iter().find(|s| s.kind == SymbolKind::Class);
    assert!(config.is_some(), "Config dataclass should be indexed");
    assert!(
        config
            .unwrap()
            .attributes
            .as_ref()
            .is_some_and(|a| a.contains("dataclass")),
        "Config should have @dataclass attribute: {:?}",
        config.unwrap().attributes
    );
}

#[test]
fn test_py_test_detection() {
    let (_dir, db) = setup_python_project();

    let test_syms = db.get_symbols_by_path_prefix("src/tests/").unwrap();
    let test_fns: Vec<_> = test_syms
        .iter()
        .filter(|s| s.name.starts_with("test_"))
        .collect();
    assert!(
        !test_fns.is_empty(),
        "test functions should be found in tests/"
    );
    assert!(
        test_fns
            .iter()
            .all(|s| { s.attributes.as_ref().is_some_and(|a| a.contains("test")) }),
        "test functions should have test attribute"
    );
}

#[test]
fn test_py_visibility() {
    let (_dir, db) = setup_python_project();

    let results = db.search_symbols("_internal_method").unwrap();
    assert!(!results.is_empty());
    assert_eq!(results[0].visibility, Visibility::Private);

    let results = db.search_symbols("_INTERNAL_TIMEOUT").unwrap();
    assert!(!results.is_empty());
    assert_eq!(results[0].visibility, Visibility::Private);
}

#[test]
fn test_py_async_def() {
    let (_dir, db) = setup_python_project();

    let results = db.search_symbols("fetch_data").unwrap();
    assert!(!results.is_empty(), "async function should be indexed");
    assert_eq!(results[0].kind, SymbolKind::Function);
    assert!(
        results[0].signature.contains("async"),
        "signature should contain async: {}",
        results[0].signature
    );
}

#[test]
fn test_py_deps_stored() {
    let (_dir, db) = setup_python_project();

    let deps = db.get_direct_dependencies().unwrap();
    let dep_names: Vec<&str> = deps.iter().map(|d| d.name.as_str()).collect();
    assert!(
        dep_names.contains(&"requests"),
        "requests should be a dep: {dep_names:?}"
    );
    assert!(
        dep_names.contains(&"click"),
        "click should be a dep: {dep_names:?}"
    );
    assert!(
        dep_names.contains(&"pytest"),
        "pytest should be a dep: {dep_names:?}"
    );
}

#[test]
fn test_py_refresh_index() {
    let dir = tempfile::TempDir::new().unwrap();
    let src = dir.path().join("src");
    std::fs::create_dir_all(&src).unwrap();

    // Init git for refresh_index
    std::process::Command::new("git")
        .args(["init"])
        .current_dir(dir.path())
        .output()
        .unwrap();

    write_python_project(dir.path());
    write_python_sources(&src);

    std::process::Command::new("git")
        .args(["add", "."])
        .current_dir(dir.path())
        .output()
        .unwrap();
    std::process::Command::new("git")
        .args(["commit", "-m", "init"])
        .current_dir(dir.path())
        .output()
        .unwrap();

    let db = Database::open_in_memory().unwrap();
    let config = IndexConfig {
        repo_path: dir.path().to_path_buf(),
    };
    index_repo(&db, &config).unwrap();

    // Modify a .py file
    std::fs::write(
        src.join("utils.py"),
        r#"
MAX_RETRIES = 3
_INTERNAL_TIMEOUT = 30

def format_date(date) -> str:
    """Format a date for display."""
    return str(date)

def validate_input(data: str) -> bool:
    """NEW: validate user input."""
    return len(data) > 0
"#,
    )
    .unwrap();

    let count = refresh_index(&db, &config).unwrap();
    assert!(count > 0, "refresh should re-index changed files");

    let results = db.search_symbols("validate_input").unwrap();
    assert!(
        !results.is_empty(),
        "new function should be indexed after refresh"
    );
}

#[test]
fn test_py_inheritance_trait_impl() {
    let (_dir, db) = setup_python_project();

    // LogLevel(Enum) should generate a TraitImpl
    let impls = db.get_trait_impls_for_type("LogLevel").unwrap();
    assert!(
        impls.iter().any(|t| t.trait_name == "Enum"),
        "LogLevel should implement Enum"
    );
}
