# Jon-Style Rust Phase 0 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Land 5–10 new error-handling axioms in `assets/rust_quality_axioms.json`, each user-reviewed, source-cited, surfaced via `mcp__illu__axioms` and `rust_preflight`. End state: a representative axioms query returns the new entries; `rust_preflight` evidence packets include them; one end-to-end sample function reflects the guidance.

**Architecture:** Pure content enrichment of an existing pipeline. No new code paths, no new files, no new MCP tools. Append entries to a JSON array; existing parse-once-cache layer at [src/server/tools/axioms.rs:67-87](src/server/tools/axioms.rs:67) picks them up. One existing test ([test_rust_quality_axioms_are_loaded_with_sources](src/server/tools/axioms.rs:211)) is supplemented with per-batch coverage tests.

**Tech Stack:** Rust 2024, `serde_json` for axiom parsing, `cargo test` / `cargo clippy --all-targets --all-features -- -D warnings` / `cargo fmt --all -- --check` for verification, `mcp__illu__axioms` and `mcp__illu__rust_preflight` for end-to-end demo.

---

## Pre-Execution Notes

**Spec:** [docs/superpowers/specs/2026-04-26-jon-style-rust-phase-0-design.md](docs/superpowers/specs/2026-04-26-jon-style-rust-phase-0-design.md)

**Existing state:** 56 axioms already in `assets/rust_quality_axioms.json` (IDs `rust_quality_01_*` through `rust_quality_56_*`). New entries use IDs `rust_quality_57_*` through `rust_quality_66_*`. **Verify max existing ID at the start of Task 1** in case the file has changed; bump new IDs accordingly.

**Existing assertion to bump:** [test_axiom_assets_have_unique_ids_and_required_fields](src/server/tools/axioms.rs:229) at the end of the file asserts `rust_quality_axiom_count == 56` exactly. Each batch task includes a step to bump this number by the count of axioms added in that batch (so it tracks the new total: 56 → 59 → 62 → 65 → 66, or less if some drafts are rejected). The test also enforces unique IDs and non-empty `source`/`category`/`triggers`/`rule_summary`/`prompt_injection`/`anti_pattern`/`good_pattern` for every entry — all drafts below satisfy these requirements.

**Drafted axiom content is included in each batch task.** Per the brainstorming pipeline, you (the user) review each draft before integration. Edit any inline before the JSON-append step. Reject by removing from the batch — fewer than 10 new axioms total is acceptable per spec exit criteria.

**Execution mode recommendation:** Inline (executing-plans), not subagent-driven. Each batch task includes a user-review step that requires the user, not a subagent.

**Source citation policy:** Drafts use placeholder citations like `"Rust for Rustaceans, ch. 4 §<topic>"`. Tighten to exact section names or page numbers during review if you have the book at hand. Citations to Jon's repos use placeholder symbol references; replace with `repo:path:line` if you have the repos cloned locally.

**Triggers field weight:** Recall from [handle_axioms()](src/server/tools/axioms.rs:89) scoring: `triggers` matches add +10, exact match +30, `category` contains +5, exact category +20, `rule_summary` contains +2. Drafts use 5–6 specific triggers per axiom; review for collisions with existing axioms (run `mcp__illu__axioms` with each candidate trigger phrase to spot dilution before integration).

**Commit policy per CLAUDE.md:** No `Co-Authored-By` trailer. Use the user's git identity only.

---

## File Structure

| File | Change | Responsibility |
|------|--------|----------------|
| `assets/rust_quality_axioms.json` | Append entries | New axiom data; same JSON-array shape as existing |
| `src/server/tools/axioms.rs` | Add 4 test fns + extend 1 | Per-batch coverage tests + end-to-end query test |

No other files touched. No schema changes, no new modules, no new dependencies.

---

## Task 1: Batch 1 — Error Source Chain, Wrap vs Propagate, Variant Naming

**Files:**
- Modify: `assets/rust_quality_axioms.json` (append 3 entries before the closing `]`)
- Modify: `src/server/tools/axioms.rs` (add `test_error_handling_axioms_batch_1_present` in the test module at the bottom)

- [ ] **Step 1: Confirm starting ID**

```bash
grep -oE '"id": "rust_quality_[0-9]+_' assets/rust_quality_axioms.json | grep -oE '[0-9]+' | sort -n | tail -1
```

Expected: `56`. If different, update the IDs in the drafts below to start at `<max>+1`.

- [ ] **Step 2: Write the failing batch-1 coverage test**

Open `src/server/tools/axioms.rs`. Find the `#[cfg(test)]` module at the bottom (after `handle_axioms`). Add this test function inside the existing `mod tests` block, alongside the other `test_*` functions:

```rust
    #[test]
    fn test_error_handling_axioms_batch_1_present() {
        let result = handle_axioms(
            "error source chain wrap propagate variant naming Error::source map_err InvalidUtf8 Display",
        )
        .unwrap();
        assert!(
            result.contains("Error Source Chain"),
            "missing category Error Source Chain"
        );
        assert!(
            result.contains("Error Boundary Discipline"),
            "missing category Error Boundary Discipline"
        );
        assert!(
            result.contains("Error API Surface"),
            "missing category Error API Surface"
        );
    }
```

- [ ] **Step 3: Run the new test, expect failure**

Run: `cargo test --lib -- test_error_handling_axioms_batch_1_present`
Expected: FAIL — assertions panic because the three categories are not yet in the JSON.

- [ ] **Step 4: User reviews drafted axioms**

Three drafts below. For each: keep / edit / reject. Edits can be applied directly in the JSON-append step or here in this plan.

**Draft 1.1 — Error Source Chain:**

```json
{
  "id": "rust_quality_57_error_source_chain",
  "category": "Error Source Chain",
  "source": "Rust for Rustaceans, ch. 4 §Error Source",
  "triggers": ["error source", "Error::source", "error chain", "wrapped error", "underlying cause", "source method"],
  "rule_summary": "Library error variants that wrap an upstream error must implement `Error::source()` so callers can walk the cause chain. Do not flatten the underlying cause into a `Display` string.",
  "prompt_injection": "MANDATORY RULE: When an error variant wraps another error, return the wrapped error from `source()`. Never collapse the cause into a formatted string in `Display`; the message describes this layer's failure, the chain carries the rest.",
  "anti_pattern": "impl fmt::Display for Error {\n    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {\n        match self {\n            Error::Read(io) => write!(f, \"failed to read: {io}\"),\n        }\n    }\n}\nimpl std::error::Error for Error {}",
  "good_pattern": "impl fmt::Display for Error {\n    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {\n        match self {\n            Error::Read(_) => f.write_str(\"failed to read configuration\"),\n        }\n    }\n}\nimpl std::error::Error for Error {\n    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {\n        match self {\n            Error::Read(io) => Some(io),\n        }\n    }\n}"
}
```

**Draft 1.2 — Wrap vs Propagate:**

```json
{
  "id": "rust_quality_58_wrap_vs_propagate",
  "category": "Error Boundary Discipline",
  "source": "Rust for Rustaceans, ch. 4 §Error Conversion",
  "triggers": ["wrap error", "propagate error", "map_err", "boundary context", "preserve original", "domain boundary"],
  "rule_summary": "Use `?` (with `From`) to propagate errors through layers that share a domain. Use `map_err` only at module or domain boundaries where the calling layer needs different context. Do not apply both at the same call site without justification.",
  "prompt_injection": "MANDATORY RULE: `?` and `map_err` answer different questions. Use `?` when the caller's domain matches yours. Use `map_err` only when crossing into a layer that owns a different error type. If you find yourself adding context strings inside one domain, extend the underlying error variant instead.",
  "anti_pattern": "fn read_config(path: &Path) -> Result<Config, ConfigError> {\n    let raw = std::fs::read_to_string(path)\n        .map_err(|e| ConfigError::Other(format!(\"read failed: {e}\")))?;\n    parse(&raw).map_err(|e| ConfigError::Other(format!(\"parse failed: {e}\")))\n}",
  "good_pattern": "fn read_config(path: &Path) -> Result<Config, ConfigError> {\n    let raw = std::fs::read_to_string(path).map_err(ConfigError::Read)?;\n    parse(&raw).map_err(ConfigError::Parse)\n}"
}
```

**Draft 1.3 — Variant Naming and Display Conventions:**

```json
{
  "id": "rust_quality_59_error_variant_naming",
  "category": "Error API Surface",
  "source": "Rust for Rustaceans, ch. 4 §Error Display; Rust API Guidelines C-WORD-ORDER.",
  "triggers": ["variant naming", "error name", "error display", "InvalidUtf8", "Display impl", "error message style"],
  "rule_summary": "Error variant names are nouns naming the failure (`InvalidUtf8`, `MissingField`), not verb phrases (`FailedToParseUtf8`). `Display` produces one short clause without trailing punctuation and without leaking internal state callers cannot use.",
  "prompt_injection": "MANDATORY RULE: Name error variants for the failure condition, not the operation that failed. Format `Display` output as a noun-phrase or short clause: lowercase, no trailing period, no struct dumps. Internal detail belongs in `Debug` or in `source()` chains.",
  "anti_pattern": "#[derive(Debug)]\npub enum Error {\n    FailedToParseUtf8 { bytes: Vec<u8>, position: usize },\n    CouldNotFindField,\n}\nimpl fmt::Display for Error {\n    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {\n        write!(f, \"Error: {self:?}.\")\n    }\n}",
  "good_pattern": "#[derive(Debug)]\n#[non_exhaustive]\npub enum Error {\n    InvalidUtf8 { position: usize },\n    MissingField(&'static str),\n}\nimpl fmt::Display for Error {\n    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {\n        match self {\n            Error::InvalidUtf8 { position } => write!(f, \"invalid UTF-8 at byte {position}\"),\n            Error::MissingField(name) => write!(f, \"missing required field `{name}`\"),\n        }\n    }\n}"
}
```

- [ ] **Step 5: Append approved drafts to `assets/rust_quality_axioms.json`**

The file is a top-level JSON array. Locate the closing `]` at the end. Insert the approved drafts immediately before it, with a comma after the previous last entry. Use the `Edit` tool to perform the insertion: `old_string` should be the last entry's closing `}` followed by the closing `]`; `new_string` should be the same `}` followed by `,\n  <new entries joined by commas>\n]`.

After insertion, validate JSON parses:

```bash
python3 -c "import json; json.load(open('assets/rust_quality_axioms.json')); print('valid')"
```

Expected: `valid`.

- [ ] **Step 6: Bump the count assertion in `axioms.rs`**

Find [test_axiom_assets_have_unique_ids_and_required_fields](src/server/tools/axioms.rs:229) at the bottom of `src/server/tools/axioms.rs`. The last line of that test reads:

```rust
assert_eq!(rust_quality_axiom_count, 56);
```

Update `56` to the new total: 56 + (number of accepted drafts in this batch). If all 3 batch-1 drafts were accepted, the new value is `59`. If only 2 were accepted, `58`. If 1, `57`.

- [ ] **Step 7: Run the batch-1 test, expect pass**

Run: `cargo test --lib -- test_error_handling_axioms_batch_1_present`
Expected: PASS (assuming all 3 drafts approved with the categories above; if the user renamed a category during review, update the assertion).

- [ ] **Step 8: Run the full test suite**

Run: `cargo test --lib`
Expected: all tests pass, including [test_rust_quality_axioms_are_loaded_with_sources](src/server/tools/axioms.rs:211) (asserts every entry has a non-empty `source` field) and [test_axiom_assets_have_unique_ids_and_required_fields](src/server/tools/axioms.rs:229) (now asserting the bumped count and that each new entry has unique ID + all required fields non-empty).

- [ ] **Step 9: Lint and format checks**

Run:
```bash
cargo clippy --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
```
Expected: clean. Only `axioms.rs` is rust-formatted; the JSON file is not touched by `cargo fmt`.

- [ ] **Step 10: Commit batch 1**

```bash
git add assets/rust_quality_axioms.json src/server/tools/axioms.rs
git commit -m "$(cat <<'EOF'
axioms: add error-handling batch 1 (source chain, wrap vs propagate, variant naming)

Adds three new entries from Rust for Rustaceans ch. 4 covering Error::source
discipline, the boundary distinction between ? and map_err, and Display/variant
naming conventions. Adds a coverage test asserting the three new categories
appear for the canonical query.
EOF
)"
```

---

## Task 2: Batch 2 — Error Categories, Backtrace Policy, Stable Semantics

**Files:**
- Modify: `assets/rust_quality_axioms.json` (append 3 entries)
- Modify: `src/server/tools/axioms.rs` (add `test_error_handling_axioms_batch_2_present`)

- [ ] **Step 1: Write the failing batch-2 coverage test**

Add inside `mod tests` in `src/server/tools/axioms.rs`:

```rust
    #[test]
    fn test_error_handling_axioms_batch_2_present() {
        let result = handle_axioms(
            "error category io domain invariant Other variant catchall backtrace capture stable non_exhaustive contract",
        )
        .unwrap();
        assert!(
            result.contains("Error Category Structure"),
            "missing category Error Category Structure"
        );
        assert!(
            result.contains("Backtrace Policy"),
            "missing category Backtrace Policy"
        );
        assert!(
            result.contains("Error Stability"),
            "missing category Error Stability"
        );
    }
```

- [ ] **Step 2: Run the new test, expect failure**

Run: `cargo test --lib -- test_error_handling_axioms_batch_2_present`
Expected: FAIL — three categories not yet in JSON.

- [ ] **Step 3: User reviews drafted axioms**

**Draft 2.1 — Error Category Structure:**

```json
{
  "id": "rust_quality_60_error_categories",
  "category": "Error Category Structure",
  "source": "Rust for Rustaceans, ch. 4 §Error Hierarchies",
  "triggers": ["error category", "io vs domain", "invariant violation", "Other variant", "stringly typed error", "error catchall"],
  "rule_summary": "Distinguish failure categories — I/O, parse, domain, and invariant violations — as distinct variants or sub-enums. Never collapse them into a single `Other(String)` or `Internal(String)` catch-all.",
  "prompt_injection": "MANDATORY RULE: Each error variant should let a caller decide whether to retry, surface, or abort. Variants that erase that distinction (`Other(String)`, `Internal(String)`) push the work back to the caller through string parsing. Add a real variant or sub-enum instead.",
  "anti_pattern": "#[derive(Debug)]\npub enum Error {\n    Other(String),\n}\n// Caller has to grep `Display` to know if this was retry-able I/O or a parse bug.",
  "good_pattern": "#[derive(Debug)]\n#[non_exhaustive]\npub enum Error {\n    Io(std::io::Error),\n    Parse(ParseError),\n    Schema(SchemaError),\n    Invariant(&'static str),\n}\n// Each variant tells the caller what kind of failure it is."
}
```

**Draft 2.2 — Backtrace Policy:**

```json
{
  "id": "rust_quality_61_backtrace_capture",
  "category": "Backtrace Policy",
  "source": "Rust for Rustaceans, ch. 4 §Backtraces",
  "triggers": ["backtrace", "Backtrace::capture", "stack context", "RUST_BACKTRACE", "error origin", "stack walk"],
  "rule_summary": "Capture `std::backtrace::Backtrace` only at the boundary where stack context would otherwise be lost — typically the lowest-level error variant or the first conversion away from the originating frame. Avoid capturing on every wrap.",
  "prompt_injection": "MANDATORY RULE: Backtraces are expensive (a stack walk plus optional symbolication). Capture them once, at the deepest layer that owns the original failure, not at every wrapping site. Application code is more permissive than library code; libraries should let the caller opt in.",
  "anti_pattern": "#[derive(Debug)]\npub enum Error {\n    Io { source: io::Error, backtrace: Backtrace },\n    Parse { source: ParseError, backtrace: Backtrace },\n    Domain { source: DomainError, backtrace: Backtrace },\n}\n// A single failure walks the stack three times as it is wrapped.",
  "good_pattern": "#[derive(Debug)]\n#[non_exhaustive]\npub enum Error {\n    Io(io::Error),\n    Parse(ParseError),\n    Domain { source: DomainError, backtrace: Backtrace },\n}\n// Capture once, at the boundary that owns the originating failure."
}
```

**Draft 2.3 — Stable Error Semantics:**

```json
{
  "id": "rust_quality_62_stable_error_semantics",
  "category": "Error Stability",
  "source": "Rust for Rustaceans, ch. 4 §Error Stability and ch. 8 §Versioning Surface",
  "triggers": ["non_exhaustive insufficient", "stable variant", "error contract", "semver error", "library error stability"],
  "rule_summary": "`#[non_exhaustive]` lets you add variants without a major bump, but does not document which existing variants callers may match on as a stable contract. Document each variant's stability explicitly.",
  "prompt_injection": "MANDATORY RULE: Treat each public error variant as either a stable contract (callers may match on it) or an internal-detail variant (rendered through `Display` only, may be split or removed). Document the distinction in rustdoc; do not let it default to ambiguity.",
  "anti_pattern": "#[non_exhaustive]\npub enum Error {\n    Io(io::Error),\n    Parse(ParseError),\n    Network(NetworkError),\n}\n// No documentation. Callers do not know whether matching `Network` is supported.",
  "good_pattern": "/// # Stability\n///\n/// `Io`, `Parse`, and `Network` are stable contract — callers may match on\n/// them. `Internal` (and any future variants gated by `#[non_exhaustive]`) is\n/// internal detail and may be split, merged, or removed in any minor release.\n#[non_exhaustive]\npub enum Error {\n    Io(io::Error),\n    Parse(ParseError),\n    Network(NetworkError),\n    #[doc(hidden)]\n    Internal(InternalKind),\n}"
}
```

- [ ] **Step 4: Append approved drafts to JSON**

Same Edit-tool insertion pattern as Task 1 Step 5. Validate JSON parses:

```bash
python3 -c "import json; json.load(open('assets/rust_quality_axioms.json')); print('valid')"
```

- [ ] **Step 5: Bump the count assertion**

Update the `assert_eq!(rust_quality_axiom_count, ...)` line at the end of [test_axiom_assets_have_unique_ids_and_required_fields](src/server/tools/axioms.rs:229) by the number of batch-2 drafts accepted. After Task 1 the value was 56 + (batch-1 accepted count); now add (batch-2 accepted count).

- [ ] **Step 6: Run the batch-2 test, expect pass**

Run: `cargo test --lib -- test_error_handling_axioms_batch_2_present`
Expected: PASS.

- [ ] **Step 7: Run the full test suite**

Run: `cargo test --lib`
Expected: all tests pass.

- [ ] **Step 8: Lint and format checks**

```bash
cargo clippy --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
```
Expected: clean.

- [ ] **Step 9: Commit batch 2**

```bash
git add assets/rust_quality_axioms.json src/server/tools/axioms.rs
git commit -m "$(cat <<'EOF'
axioms: add error-handling batch 2 (categories, backtrace policy, stable semantics)

Adds three entries on distinguishing failure categories rather than collapsing
into Other(String), backtrace capture discipline at one boundary not every
wrap, and documenting which variants of a non_exhaustive enum are stable
contract vs internal detail. Source: Rust for Rustaceans ch. 4 / ch. 8.
EOF
)"
```

---

## Task 3: Batch 3 — Context as Values, No Box dyn Error in Library Internals, From Impls Are Public API

**Files:**
- Modify: `assets/rust_quality_axioms.json` (append 3 entries)
- Modify: `src/server/tools/axioms.rs` (add `test_error_handling_axioms_batch_3_present`)

- [ ] **Step 1: Write the failing batch-3 coverage test**

Add inside `mod tests` in `src/server/tools/axioms.rs`:

```rust
    #[test]
    fn test_error_handling_axioms_batch_3_present() {
        let result = handle_axioms(
            "error context typed format string Box dyn Error library internal helper From impl conversion graph",
        )
        .unwrap();
        assert!(
            result.contains("Error Context"),
            "missing category Error Context"
        );
        assert!(
            result.contains("Error Type Discipline"),
            "missing category Error Type Discipline"
        );
        assert!(
            result.contains("Error Conversion Surface"),
            "missing category Error Conversion Surface"
        );
    }
```

- [ ] **Step 2: Run the new test, expect failure**

Run: `cargo test --lib -- test_error_handling_axioms_batch_3_present`
Expected: FAIL.

- [ ] **Step 3: User reviews drafted axioms**

**Draft 3.1 — Error Context as Values:**

```json
{
  "id": "rust_quality_63_error_context_typed",
  "category": "Error Context",
  "source": "Rust for Rustaceans, ch. 4 §Carrying Context",
  "triggers": ["error context", "format string error", "eyre context", "typed context", "structured error", "context fields"],
  "rule_summary": "Carry error context as typed fields on the variant, not as `format!`-built strings. Reserve `eyre`/`anyhow` chained-string context for application code where the caller never needs to inspect the parts.",
  "prompt_injection": "MANDATORY RULE: If a caller might want to programmatically inspect what failed (which path, which line, which key), put that information in fields on the error variant. `format!`-built context strings discard the data and force string parsing later.",
  "anti_pattern": "return Err(Error::Parse(format!(\n    \"failed at {path:?}:{line}: unexpected token {tok:?}\"\n)));",
  "good_pattern": "return Err(Error::Parse(ParseError {\n    path: path.to_owned(),\n    line,\n    kind: ParseErrorKind::UnexpectedToken(tok),\n}));"
}
```

**Draft 3.2 — No `Box<dyn Error>` in Library Internals:**

```json
{
  "id": "rust_quality_64_no_box_dyn_error_internal",
  "category": "Error Type Discipline",
  "source": "Rust for Rustaceans, ch. 4 §Library Error Types",
  "triggers": ["Box dyn Error", "internal helper", "library error", "private function error", "anyhow internally", "structured error"],
  "rule_summary": "Library code should not use `Box<dyn Error>` even in private helpers: it leaks anyhow-style erasure into the call graph and prevents callers (including your own code) from reasoning about variants. Build the structured type once and use it everywhere.",
  "prompt_injection": "MANDATORY RULE: If you have a structured `Error` enum for the public API, use it in private helpers too. `Box<dyn Error>` in a private signature is a tell that you have not finished designing the variant; finish the design.",
  "anti_pattern": "fn parse_record(input: &str) -> Result<Record, Box<dyn std::error::Error>> {\n    /* private helper inside a library */\n}",
  "good_pattern": "fn parse_record(input: &str) -> Result<Record, ParseError> {\n    /* same helper, structured error type used everywhere */\n}"
}
```

**Draft 3.3 — `From` Impls Are Public API:**

```json
{
  "id": "rust_quality_65_from_impls_public_api",
  "category": "Error Conversion Surface",
  "source": "Rust for Rustaceans, ch. 4 §From Conversions and ch. 8 §Public Surface",
  "triggers": ["From impl", "error conversion", "auto convert error", "public surface", "conversion graph", "error From"],
  "rule_summary": "Every `From<E>` for your error type widens the conversion graph and becomes part of the public API: callers' types now flow through `?` into yours. Review each one as a deliberate API decision, not as a convenience for `?`.",
  "prompt_injection": "MANDATORY RULE: Adding `impl From<UpstreamError> for MyError` is a public API change even when the wrapping variant is internal-detail. Decide whether the upstream error genuinely belongs in your domain before adding the conversion.",
  "anti_pattern": "// Added because `?` complained.\nimpl From<reqwest::Error> for ConfigError {\n    fn from(error: reqwest::Error) -> Self {\n        ConfigError::Other(error.to_string())\n    }\n}",
  "good_pattern": "// Decision: ConfigError owns network failures because the loader fetches\n// remote includes. Reviewed in PR #142.\nimpl From<reqwest::Error> for ConfigError {\n    fn from(error: reqwest::Error) -> Self {\n        ConfigError::Network(error)\n    }\n}"
}
```

- [ ] **Step 4: Append approved drafts to JSON**

Same Edit-tool pattern. Validate JSON parses.

- [ ] **Step 5: Bump the count assertion**

Update the `assert_eq!(rust_quality_axiom_count, ...)` line by the number of batch-3 drafts accepted.

- [ ] **Step 6: Run the batch-3 test, expect pass**

Run: `cargo test --lib -- test_error_handling_axioms_batch_3_present`
Expected: PASS.

- [ ] **Step 7: Run the full test suite**

Run: `cargo test --lib`
Expected: all tests pass.

- [ ] **Step 8: Lint and format checks**

```bash
cargo clippy --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
```

- [ ] **Step 9: Commit batch 3**

```bash
git add assets/rust_quality_axioms.json src/server/tools/axioms.rs
git commit -m "$(cat <<'EOF'
axioms: add error-handling batch 3 (typed context, no Box dyn in libs, From as API)

Adds three entries on carrying error context as typed fields rather than
format-built strings, banning Box<dyn Error> in library helpers, and
treating every From impl on an error type as a deliberate public-API change.
EOF
)"
```

---

## Task 4: Batch 4 — Test the Failure Variant

**Files:**
- Modify: `assets/rust_quality_axioms.json` (append 1 entry)
- Modify: `src/server/tools/axioms.rs` (add `test_error_handling_axioms_batch_4_present` and `test_error_handling_demo_query_returns_new_axioms`)

- [ ] **Step 1: Write the failing batch-4 coverage test**

Add inside `mod tests`:

```rust
    #[test]
    fn test_error_handling_axioms_batch_4_present() {
        let result = handle_axioms(
            "test failure variant is_err assert matches error path coverage",
        )
        .unwrap();
        assert!(
            result.contains("Error Path Specificity"),
            "missing category Error Path Specificity"
        );
    }
```

- [ ] **Step 2: Run the new test, expect failure**

Run: `cargo test --lib -- test_error_handling_axioms_batch_4_present`
Expected: FAIL.

- [ ] **Step 3: User reviews drafted axiom**

**Draft 4.1 — Test the Failure Variant, Not Just `is_err`:**

```json
{
  "id": "rust_quality_66_test_failure_variant",
  "category": "Error Path Specificity",
  "source": "Rust for Rustaceans, ch. 6 §Testing Errors",
  "triggers": ["test error variant", "is_err", "assert matches", "failure path test", "error path coverage", "matches macro"],
  "rule_summary": "Tests of failure paths must assert on the specific error variant, not just `is_err()`. A test that passes for any failure cannot tell when a refactor silently flips one failure cause into another.",
  "prompt_injection": "MANDATORY RULE: For every fallible behavior under test, assert `matches!(result, Err(Error::ExpectedVariant(..)))` or destructure to inspect fields. `assert!(result.is_err())` lets two unrelated bugs satisfy the same test.",
  "anti_pattern": "#[test]\nfn rejects_missing_field() {\n    assert!(parse(\"{}\").is_err());\n}",
  "good_pattern": "#[test]\nfn rejects_missing_field() {\n    let err = parse(\"{}\").unwrap_err();\n    assert!(matches!(err, ParseError::MissingField(name) if name == \"id\"));\n}"
}
```

- [ ] **Step 4: Append approved draft to JSON**

Same Edit-tool pattern. Validate JSON parses.

- [ ] **Step 5: Bump the count assertion**

Update the `assert_eq!(rust_quality_axiom_count, ...)` line by the number of batch-4 drafts accepted (1 if approved as-is, 0 if rejected).

- [ ] **Step 6: Run the batch-4 test, expect pass**

Run: `cargo test --lib -- test_error_handling_axioms_batch_4_present`
Expected: PASS.

- [ ] **Step 7: Add the demo-query test**

This one test asserts the spec's demo-gate behavior end-to-end. Add to `mod tests`:

```rust
    #[test]
    fn test_error_handling_demo_query_returns_new_axioms() {
        let result =
            handle_axioms("error source chain wrap propagate variant naming").unwrap();
        // Expect at least 3 of the new categories to surface in the top results.
        let new_categories = [
            "Error Source Chain",
            "Error Boundary Discipline",
            "Error API Surface",
            "Error Category Structure",
            "Backtrace Policy",
            "Error Stability",
            "Error Context",
            "Error Type Discipline",
            "Error Conversion Surface",
            "Error Path Specificity",
        ];
        let surfaced = new_categories
            .iter()
            .filter(|cat| result.contains(*cat))
            .count();
        assert!(
            surfaced >= 3,
            "expected at least 3 new categories in demo query, got {surfaced}"
        );
    }
```

- [ ] **Step 8: Run the demo test, expect pass**

Run: `cargo test --lib -- test_error_handling_demo_query_returns_new_axioms`
Expected: PASS. If FAIL with surfaced count < 3, review trigger overlap with existing axioms — the new axioms may be ranked too low. Tighten triggers or use more specific words.

- [ ] **Step 9: Run the full test suite**

Run: `cargo test --lib`
Expected: all tests pass, including all four batch tests and the demo test.

- [ ] **Step 10: Lint and format checks**

```bash
cargo clippy --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
```

- [ ] **Step 11: Commit batch 4**

```bash
git add assets/rust_quality_axioms.json src/server/tools/axioms.rs
git commit -m "$(cat <<'EOF'
axioms: add error-handling batch 4 (failure-variant testing) + demo test

Adds the final error-handling axiom on asserting the specific error variant
in failure-path tests rather than is_err(). Adds an end-to-end demo test
that exercises the canonical query and verifies the new categories surface.
EOF
)"
```

---

## Task 5: End-to-End Verification and Demo

**Files:** none modified directly; this task verifies the integration through MCP tools and runs the cargo gauntlet one last time.

- [ ] **Step 1: Full cargo verification**

Run all four checks:
```bash
cargo build
cargo test
cargo clippy --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
```
Expected: every command exits 0.

- [ ] **Step 2: MCP-level demo query**

Call the running illu MCP server with the canonical demo query. From a chat session with illu loaded:

```
mcp__illu__axioms(query: "error source chain wrap propagate variant naming")
```

Expected: response markdown contains at least 3 of the 10 new category names. If this fails (e.g., trigger collisions push them out of the top results), investigate via `mcp__illu__axioms` with single-trigger queries to see ranking.

- [ ] **Step 3: rust_preflight evidence packet check**

Call:
```
mcp__illu__rust_preflight(task: "Design a structured error type for a config loader that fetches local files and remote URLs", baseline_query: "planning data structures documentation comments idiomatic rust verification performance error handling")
```

Expected: the `axioms` section of the evidence packet contains at least one of the new category names (most likely `Error Source Chain`, `Error Category Structure`, or `Error API Surface` given the task content).

- [ ] **Step 4: End-to-end sample function**

Author a small fallible function as a usability check. Suggested prompt to give yourself: "Write a `load_config(path: &Path) -> Result<Config, ConfigError>` that reads a file, parses JSON, and validates a required `name` field. Use the new error-handling axioms."

The function should reflect: structured `ConfigError` enum with category variants (Io, Parse, MissingField), `Display` impls in noun-phrase form, `Error::source()` for variants that wrap, no `Box<dyn Error>`, no `Other(String)` catch-all, named variant tested with `matches!`.

Save the sample under `examples/jon_style_error_handling.rs` (gated as a regular example) **only if** you want a permanent demo; otherwise discard the file after eyeballing the output. The plan's exit criteria do not require this artifact be committed.

- [ ] **Step 5: Optional final commit**

If you saved a demo file:
```bash
git add examples/jon_style_error_handling.rs
git commit -m "examples: jon-style error handling demo"
```

If not, skip this step. The plan is complete after Step 4.

---

## Verification Summary

After all tasks, the following hold:

- 10 new axioms in `assets/rust_quality_axioms.json` (or fewer if some were rejected during review).
- 5 new tests in `src/server/tools/axioms.rs`: 4 batch tests + 1 demo test.
- All cargo checks clean.
- MCP-level query returns the new categories.
- `rust_preflight` evidence packet includes them.

## Risks Realized During Execution

If you hit any of these mid-task, here's the recovery:

- **JSON parse error after append.** Most likely a missing comma between entries or a trailing comma after the last. Run `python3 -c "import json; json.load(open('assets/rust_quality_axioms.json'))"` to get a line:column on the error and fix.
- **Batch test fails because category name was renamed during review.** Update the assertion in the test to match the renamed category.
- **Demo test fails — fewer than 3 new categories surface.** Examine which categories are losing the ranking. Likely a trigger collision with an existing axiom. Sharpen the new entry's triggers (more specific words) or drop one less-specific trigger that collides.
- **Source field rejected by `test_rust_quality_axioms_are_loaded_with_sources`.** That test asserts the result of a query *contains* "Source:". Since every drafted entry above has a non-empty `source`, this should never fire — but if it does, ensure the `source` field key is present and the value is non-empty.
- **Clippy warning on the new test code.** New tests use `unwrap()`. The `axioms.rs` test module already opts out of `clippy::unwrap_used` (per CLAUDE.md convention "Tests opt out via `#[expect(clippy::unwrap_used, reason = "tests")]`"). Confirm the existing module-level expect attribute covers the new tests; if not, mirror what the other test fns use.
