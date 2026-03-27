#![expect(clippy::unwrap_used, reason = "integration tests")]

use illu_rs::db::Database;
use illu_rs::indexer::parser::{SymbolKind, Visibility};
use illu_rs::indexer::{IndexConfig, index_repo, refresh_index};
use illu_rs::server::tools::{context, impact, overview, query};

fn write_project_configs(dir: &std::path::Path) {
    std::fs::write(
        dir.join("package.json"),
        r#"{
  "name": "test-app",
  "version": "1.0.0",
  "dependencies": { "react": "^18.0.0" },
  "devDependencies": { "vitest": "^1.0.0" }
}"#,
    )
    .unwrap();

    std::fs::write(
        dir.join("tsconfig.json"),
        r#"{
  "compilerOptions": { "target": "ES2020", "module": "ESNext", "strict": true }
}"#,
    )
    .unwrap();
}

fn write_ts_sources(src: &std::path::Path) {
    std::fs::write(
        src.join("app.ts"),
        r"
import { UserService } from './service';
import { Config } from './types';

/** Initialize and run the application. */
export function createApp(config: Config): void {
    const service = new UserService(config.apiUrl);
    service.fetchUsers();
}

export function main(): void {
    const config: Config = { apiUrl: 'http://localhost', debug: false };
    createApp(config);
}
",
    )
    .unwrap();

    // Service class
    std::fs::write(
        src.join("service.ts"),
        r"
/** Service for managing users. */
export class UserService {
    private baseUrl: string;

    constructor(baseUrl: string) {
        this.baseUrl = baseUrl;
    }

    /** Fetch all users from the API. */
    async fetchUsers(): Promise<User[]> {
        return [];
    }

    /** Get a single user by ID. */
    async getUser(id: number): Promise<User | null> {
        return null;
    }
}

export interface User {
    id: number;
    name: string;
    email: string;
}
",
    )
    .unwrap();

    // Types file
    std::fs::write(
        src.join("types.ts"),
        r"
/** Application configuration. */
export interface Config {
    apiUrl: string;
    debug: boolean;
}

/** API response wrapper. */
export type ApiResponse<T> = {
    data: T;
    status: number;
};

export enum LogLevel {
    Debug = 'DEBUG',
    Info = 'INFO',
    Error = 'ERROR',
}
",
    )
    .unwrap();

    // Utility functions
    std::fs::write(
        src.join("utils.ts"),
        r"
/** Format a date for display. */
export function formatDate(date: Date): string {
    return date.toISOString();
}

/** Deep clone an object. */
export const deepClone = <T>(obj: T): T => {
    return JSON.parse(JSON.stringify(obj));
}

export const MAX_RETRIES = 3;
",
    )
    .unwrap();

    let test_dir = src.join("__tests__");
    std::fs::create_dir_all(&test_dir).unwrap();
    std::fs::write(
        test_dir.join("service.test.ts"),
        r"
import { UserService } from '../service';

describe('UserService', () => {
    it('should create service', () => {
        const service = new UserService('http://test');
        expect(service).toBeDefined();
    });
});
",
    )
    .unwrap();
}

fn setup_ts_project() -> (tempfile::TempDir, Database) {
    let dir = tempfile::TempDir::new().unwrap();
    let src = dir.path().join("src");
    std::fs::create_dir_all(&src).unwrap();

    write_project_configs(dir.path());
    write_ts_sources(&src);

    let db = Database::open_in_memory().unwrap();
    let config = IndexConfig {
        repo_path: dir.path().to_path_buf(),
    };
    index_repo(&db, &config).unwrap();

    (dir, db)
}

#[test]
fn test_ts_symbols_indexed() {
    let (_dir, db) = setup_ts_project();

    // Functions
    let results = db.search_symbols("createApp").unwrap();
    assert!(!results.is_empty(), "createApp should be indexed");
    assert_eq!(results[0].kind, SymbolKind::Function);

    // Classes
    let results = db.search_symbols("UserService").unwrap();
    assert!(!results.is_empty(), "UserService should be indexed");
    assert_eq!(results[0].kind, SymbolKind::Class);

    // Interfaces
    let results = db.search_symbols("Config").unwrap();
    let config = results.iter().find(|s| s.kind == SymbolKind::Interface);
    assert!(config.is_some(), "Config interface should be indexed");

    // Type aliases
    let results = db.search_symbols("ApiResponse").unwrap();
    assert!(!results.is_empty(), "ApiResponse should be indexed");
    assert_eq!(results[0].kind, SymbolKind::TypeAlias);

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
fn test_ts_class_methods_have_impl_type() {
    let (_dir, db) = setup_ts_project();

    let results = db.search_symbols("fetchUsers").unwrap();
    assert!(!results.is_empty());
    assert_eq!(results[0].impl_type.as_deref(), Some("UserService"));
}

#[test]
fn test_ts_query_tool() {
    let (_dir, db) = setup_ts_project();

    let result =
        query::handle_query(&db, "UserService", None, None, None, None, None, None).unwrap();
    assert!(result.contains("UserService"));
    assert!(result.contains("class"));
}

#[test]
fn test_ts_context_tool() {
    let (_dir, db) = setup_ts_project();

    let result = context::handle_context(&db, "createApp", false, None, None, None, false).unwrap();
    assert!(
        result.contains("createApp"),
        "context should show createApp"
    );
}

#[test]
fn test_ts_overview_tool() {
    let (_dir, db) = setup_ts_project();

    let result = overview::handle_overview(&db, "src/", false, None).unwrap();
    assert!(
        result.contains("UserService"),
        "overview should show UserService"
    );
    assert!(
        result.contains("Config"),
        "overview should show Config interface"
    );
}

#[test]
fn test_ts_impact_tool() {
    let (_dir, db) = setup_ts_project();

    let result = impact::handle_impact(&db, "UserService", None, false, false).unwrap();
    assert!(
        result.contains("UserService"),
        "impact should mention UserService: {result}"
    );
}

#[test]
fn test_ts_test_detection() {
    let (_dir, db) = setup_ts_project();

    // Test files should produce test symbols — check via
    // the related tests query
    let tests = db.get_related_tests("UserService", None).unwrap();
    // The __tests__/service.test.ts uses UserService,
    // but we mainly verify test symbols are marked
    let test_file_syms = db.get_symbols_by_path_prefix("src/__tests__/").unwrap();
    assert!(
        test_file_syms.iter().all(|s| {
            // Check attributes contain "test"
            s.attributes.as_ref().is_some_and(|a| a.contains("test"))
        }),
        "all symbols in __tests__/ should have test attribute"
    );
    let _ = tests;
}

#[test]
fn test_ts_enum_variants() {
    let (_dir, db) = setup_ts_project();

    let results = db.search_symbols("Debug").unwrap();
    let variant = results.iter().find(|s| s.kind == SymbolKind::EnumVariant);
    assert!(variant.is_some(), "Debug enum variant should be indexed");
    assert_eq!(variant.unwrap().impl_type.as_deref(), Some("LogLevel"));
}

#[test]
fn test_ts_doc_comments() {
    let (_dir, db) = setup_ts_project();

    let results = db.search_symbols("formatDate").unwrap();
    assert!(!results.is_empty());
    assert!(
        results[0]
            .doc_comment
            .as_ref()
            .is_some_and(|d| d.contains("Format a date")),
        "formatDate should have JSDoc: {:?}",
        results[0].doc_comment
    );
}

#[test]
fn test_ts_npm_deps_stored() {
    let (_dir, db) = setup_ts_project();

    let deps = db.get_direct_dependencies().unwrap();
    let dep_names: Vec<&str> = deps.iter().map(|d| d.name.as_str()).collect();
    assert!(
        dep_names.contains(&"react"),
        "react should be a dep: {dep_names:?}"
    );
    assert!(
        dep_names.contains(&"vitest"),
        "vitest should be a dep: {dep_names:?}"
    );
}

#[test]
fn test_ts_arrow_function_indexed() {
    let (_dir, db) = setup_ts_project();

    let results = db.search_symbols("deepClone").unwrap();
    assert!(!results.is_empty(), "deepClone arrow fn should be indexed");
    assert_eq!(results[0].kind, SymbolKind::Function);
    assert_eq!(results[0].visibility, Visibility::Public);
}

#[test]
fn test_ts_user_interface() {
    let (_dir, db) = setup_ts_project();

    let results = db.search_symbols("User").unwrap();
    let user = results
        .iter()
        .find(|s| s.kind == SymbolKind::Interface && s.name == "User");
    assert!(user.is_some(), "User interface should be indexed");
}

#[test]
fn test_ts_refresh_index() {
    let dir = tempfile::TempDir::new().unwrap();
    let src = dir.path().join("src");
    std::fs::create_dir_all(&src).unwrap();

    // Init git so refresh_index can detect changes
    std::process::Command::new("git")
        .args(["init"])
        .current_dir(dir.path())
        .output()
        .unwrap();

    write_project_configs(dir.path());
    write_ts_sources(&src);

    // Initial commit
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

    // Modify a TS file after initial index
    std::fs::write(
        src.join("utils.ts"),
        r"
/** Format a date for display. */
export function formatDate(date: Date): string {
    return date.toISOString();
}

/** NEW: validate input. */
export function validateInput(input: string): boolean {
    return input.length > 0;
}

export const MAX_RETRIES = 3;
",
    )
    .unwrap();

    let count = refresh_index(&db, &config).unwrap();
    assert!(count > 0, "refresh should re-index changed files");

    let results = db.search_symbols("validateInput").unwrap();
    assert!(
        !results.is_empty(),
        "new function should be indexed after refresh"
    );
    assert_eq!(results[0].kind, SymbolKind::Function);
}

#[test]
fn test_mixed_rust_ts_project() {
    let dir = tempfile::TempDir::new().unwrap();
    let src_rs = dir.path().join("src");
    std::fs::create_dir_all(&src_rs).unwrap();

    // Rust side
    std::fs::write(
        dir.path().join("Cargo.toml"),
        r#"[package]
name = "mixed-app"
version = "0.1.0"
edition = "2024"
"#,
    )
    .unwrap();
    std::fs::write(
        src_rs.join("main.rs"),
        "
pub fn rust_greet(name: &str) -> String {
    format!(\"Hello, {name}!\")
}

fn main() {}
",
    )
    .unwrap();

    // TS side
    let src_ts = dir.path().join("frontend/src");
    std::fs::create_dir_all(&src_ts).unwrap();
    std::fs::write(
        dir.path().join("package.json"),
        r#"{"name": "mixed-frontend", "dependencies": {"react": "^18.0.0"}}"#,
    )
    .unwrap();
    std::fs::write(
        src_ts.join("app.ts"),
        r"
export function tsGreet(name: string): string {
    return `Hello, ${name}!`;
}
",
    )
    .unwrap();

    let db = Database::open_in_memory().unwrap();
    let config = IndexConfig {
        repo_path: dir.path().to_path_buf(),
    };
    index_repo(&db, &config).unwrap();

    // Both Rust and TS symbols should be indexed
    let rust_sym = db.search_symbols("rust_greet").unwrap();
    assert!(!rust_sym.is_empty(), "Rust symbols should be indexed");
    assert_eq!(rust_sym[0].kind, SymbolKind::Function);

    let ts_sym = db.search_symbols("tsGreet").unwrap();
    assert!(!ts_sym.is_empty(), "TS symbols should be indexed");
    assert_eq!(ts_sym[0].kind, SymbolKind::Function);

    // Both should show in file count
    let file_count = db.file_count().unwrap();
    assert!(
        file_count >= 2,
        "should have at least 2 files (Rust + TS): {file_count}"
    );
}
