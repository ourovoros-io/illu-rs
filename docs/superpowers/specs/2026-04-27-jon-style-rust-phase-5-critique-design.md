# Jon-Style Rust — Phase 5 (Critique) Design

**Date:** 2026-04-27
**Status:** Draft, pending user review
**Scope:** Phase 5 — `mcp__illu__critique` MCP tool that takes a unified diff and returns a list of potential axiom violations detected by a small menu of regex-based detectors.
**References:** [Phase 0 spec](2026-04-26-jon-style-rust-phase-0-design.md) for overall architecture; [Phase 4 spec](2026-04-27-jon-style-rust-phase-4-design-record-design.md) for the most-recent precedent on adding a new MCP tool that takes large free-form input.

## Motivation

Phases 0–4 added 102 axioms, 9 exemplars, project-style overrides, and design records. All of those are *retrieval* surfaces — the agent (or developer) queries them. None of them ask the inverse question: "given this diff, which axioms might it violate?" A diff that introduces a `Box<u32>` field, a `unsafe {` block without a `// SAFETY:` comment, or a `panic!` in library code violates one or more axioms — but neither the developer nor the agent has a tool that surfaces those concerns directly.

Phase 5 introduces that tool. The first cut is intentionally small: 5 detectors covering the most mechanically-detectable axiom violations, all regex-based. The design accepts brittleness in exchange for a small, testable surface that can be expanded incrementally as detectors prove themselves in practice.

## Goal

1. Add a new MCP tool `mcp__illu__critique` that takes a unified-diff string and returns a Markdown summary of potential axiom violations.
2. Implement 5 detectors targeting high-confidence patterns: bare `unsafe` blocks, undocumented `unsafe fn`, `Box<CopyPrimitive>`, `mem::uninitialized` calls, and `.unwrap()`/`.expect()` in non-test code.
3. Each detector is a `fn(&[DiffHunk]) -> Vec<Critique>`; detectors are independent and pluggable, so Phase 5.1+ can add more without touching the tool surface.
4. Tests: per-detector positive + negative + integration test that runs all 5 against a multi-violation fixture (~11 tests total).

## Scope

**In scope:**
- New module `src/server/tools/critique.rs`.
- New MCP tool `mcp__illu__critique` registered in `src/server/mod.rs`.
- 5 regex-based detectors implementing the patterns listed below.
- Lightweight unified-diff parser that extracts file path, hunk header, and added-line / context-line text. (No need for full diff semantics — we only consume added/modified lines.)
- A `Critique` struct: `{ axiom_id, axiom_title, file, line, severity, message }`.
- Markdown formatter that groups critiques by file and labels each with axiom ID and severity.
- ~11 tests in `critique::tests`.

**Explicit non-goals:**
- No tree-sitter or rust-analyzer integration. Phase 5.1+ if needed.
- No detection of "soft" axioms (e.g. error variant naming, API design choices) — those are not mechanically detectable.
- No git plumbing in the server. Caller pastes `git diff` output.
- No automatic fix suggestions; output is read-only diagnostics.
- No attempt to suppress false positives via comment markers (`// allow:` etc.). If the regex matches, the critique fires; the user reads and ignores false positives manually.
- No project-style integration on the critique tool itself. (Future Phase 5.x could let project_style suppress specific detectors per-codebase, but Phase 5 first cut is universal.)

## Architecture

```
src/server/tools/
└── critique.rs              (new)

src/server/mod.rs            (modified — registers mcp__illu__critique)
src/api.rs                   (modified — re-exports critique::handle_critique)
src/server/tools/mod.rs      (modified — pub mod critique;)
```

No new asset directory. No JSON schema. No `init` call from `IlluServer::new` — the critique tool is stateless (regexes are compiled once via `OnceLock<Vec<DetectorEntry>>`).

**Diff parser** is hand-written and minimal. The unified-diff format we consume:

```
diff --git a/src/foo.rs b/src/foo.rs
index abc..def 100644
--- a/src/foo.rs
+++ b/src/foo.rs
@@ -10,3 +10,5 @@ context
 unchanged line
+added line
+another added line
 unchanged line
```

We only need:
- File path (from `+++ b/<path>` line — skip files that don't end in `.rs`).
- Hunk start line (from `@@ -X,Y +A,B @@`; we want `A`, the new-file start line).
- Per-line text + line number for added (`+`) lines and context (` `) lines (some detectors need surrounding context, e.g. "is the line above this `unsafe {` a `// SAFETY:` comment?").

A `DiffHunk` struct holds:

```rust
struct DiffHunk {
    file: PathBuf,
    /// Lines in the new file, in order. Each entry is (line_number, text, is_added).
    /// Context lines are included so detectors can look at the line above an added one.
    lines: Vec<HunkLine>,
}

struct HunkLine {
    new_line_number: u32,
    text: String,
    is_added: bool,
}
```

Removed (`-`) lines are skipped — we don't critique deletions.

**Detectors** are independent functions taking the parsed hunk list:

```rust
type DetectorFn = fn(&[DiffHunk]) -> Vec<Critique>;

struct DetectorEntry {
    name: &'static str,
    axiom_id: &'static str,
    axiom_title: &'static str,
    severity: Severity,
    detect: DetectorFn,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Severity {
    Info,
    Warning,
    Error,
}
```

**The 5 detectors:**

1. **`bare_unsafe_block`** — axiom 94, severity `warning`.
   - Pattern: an added line whose text contains `unsafe {` (or starts with `unsafe ` followed by `{` after whitespace).
   - Negative check: the line directly above (whether added or context) must not be a `// SAFETY:` comment. If the unsafe block is on its own line, the comment must be on the immediately-prior line.
   - One critique per matching unsafe block.

2. **`undocumented_unsafe_fn`** — axiom 95, severity `warning`.
   - Pattern: an added line whose text contains `pub unsafe fn ` or `pub(crate) unsafe fn ` etc. (any visibility modifier followed by `unsafe fn`).
   - Negative check: walk up the added/context lines in the same hunk looking for a `# Safety` doc-comment heading within the prior 20 lines. If absent, fire.
   - Note: this is conservative — long doc comments split across hunks won't be seen, generating a false positive. Acceptable for the first cut.

3. **`box_of_copy`** — axiom 93, severity `warning`.
   - Pattern: an added line containing `Box<` followed by one of: `u8`, `u16`, `u32`, `u64`, `u128`, `usize`, `i8`, `i16`, `i32`, `i64`, `i128`, `isize`, `f32`, `f64`, `bool`, `char` immediately followed by `>` (no nested generics).
   - One critique per match.

4. **`mem_uninitialized`** — axiom 97, severity `error`.
   - Pattern: an added line containing `mem::uninitialized` or `std::mem::uninitialized` or `core::mem::uninitialized` (followed by `(` or `::`).
   - Severity `error` because this is unconditionally UB-prone.

5. **`unwrap_in_non_test`** — axiom 25 (Unwrap discipline — pre-Phase-0 axiom about idiomatic error handling), severity `info`.
   - Pattern: an added line containing `.unwrap()` or `.expect(`.
   - Negative check: skip files whose path matches `tests/` or contains `_test.rs` or `_tests.rs`. Within other files, skip if the line is inside a `#[cfg(test)]` block — implemented as: the most recent `mod tests` or `#[cfg(test)]` line in the same hunk before this line. Lower severity (`info`) because the false-positive rate is highest.

**Critique struct:**

```rust
#[non_exhaustive]
pub struct Critique {
    pub axiom_id: String,
    pub axiom_title: String,
    pub file: PathBuf,
    pub line: u32,
    pub severity: Severity,
    pub message: String,
}
```

**`handle_critique`:**

```rust
pub fn handle_critique(diff: &str) -> Result<String, crate::IlluError> {
    let hunks = parse_unified_diff(diff)?;
    let mut all = Vec::new();
    for entry in detectors() {
        all.extend((entry.detect)(&hunks).into_iter().map(|mut c| {
            c.axiom_id = entry.axiom_id.to_string();
            c.axiom_title = entry.axiom_title.to_string();
            c.severity = entry.severity;
            c
        }));
    }
    Ok(format_critiques(&all))
}
```

(The detector function fills the file/line/message; the dispatcher fills axiom_id/axiom_title/severity from the registry. Keeps detector functions short.)

**Output format** (Markdown):

```
## Critique results — 3 potential axiom violations

### src/foo.rs

#### Line 42 — Unsafe Block Discipline (`rust_quality_94_unsafe_block_discipline`)
**Severity:** warning
The line introduces an `unsafe` block without an immediately-preceding `// SAFETY:` comment. Pair every unsafe block with a comment that names the invariants the caller satisfies.

#### Line 88 — Heap Allocation Discipline (`rust_quality_93_heap_discipline`)
**Severity:** warning
`Box<u32>` wraps a small Copy type in heap allocation for nothing. Store `u32` directly.

### src/bar.rs

#### Line 17 — MaybeUninit (`rust_quality_97_maybe_uninit`)
**Severity:** error
`mem::uninitialized` is undefined behavior for almost every type and is deprecated. Use `MaybeUninit::<T>::uninit()` and write fields through raw pointers instead.
```

If no critiques: `## Critique results\n\nNo potential axiom violations detected in the diff.\n`.

## Schema (Rust types)

Defined inline above. No JSON schema; the tool consumes a string.

## MCP tool: `mcp__illu__critique`

**Parameters:** `diff: String` — unified-diff output, typically from `git diff` or `git diff <ref>`.

**Returns:** Markdown summary as described above.

**Use case:** an agent or developer pastes a diff after staging changes. The tool surfaces potential axiom violations by file:line. The output is advisory — false positives exist (the regex doesn't understand string literals, doc comments, or `cfg` gates beyond hunk-local detection). The user should read the violations and dismiss false alarms.

## Tests

11 tests in `critique::tests`:

- **Per-detector positive (5)**: 5 fixture diffs each containing exactly one violation that the corresponding detector should fire on.
- **Per-detector negative (5)**: 5 fixture diffs each containing a near-miss that the corresponding detector should NOT fire on (e.g. `unsafe {` with a `// SAFETY:` comment above; `pub unsafe fn` with `# Safety` doc; `Box<MyType>` not in the Copy primitive list; `mem::uninitialized_storage` not the call; `.unwrap()` in a `#[cfg(test)]` block).
- **Integration (1)**: one fixture diff with all 5 violations in different files; the integration test runs `handle_critique` and asserts the result contains all 5 axiom IDs.

Fixture diffs are small (5–15 lines each) and inline in the test bodies as `r#"..."#` strings rather than separate files. Diffs are well-formed enough to exercise the parser but minimal.

## Verification and Exit Criteria

Phase 5 is complete when:
- `src/server/tools/critique.rs` exists with the parser, 5 detectors, registry, and `handle_critique`.
- `mcp__illu__critique` registered in `src/server/mod.rs`; re-exported in `src/api.rs`.
- 11 tests pass.
- `cargo build`, `cargo test`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo fmt --all -- --check` all clean.
- Live MCP smoke test (after rebuild + restart): `mcp__illu__critique` accepts a diff and returns sensible Markdown.

## Risks and Mitigations

- **Regex false positives.** A `// SAFETY:` literal inside a doc comment, `Box<u32>` in a string literal, `unsafe {` inside a multi-line comment — all will be flagged. Mitigation: documented as a non-goal; severity tiers reflect confidence (`error` for `mem::uninitialized` which is very rarely a false positive; `info` for `.unwrap()` which often is).
- **Diff format quirks.** Unified diff has edge cases (binary files, mode-only changes, merges, file renames). Mitigation: simple parser only handles the common case; unrecognized hunks are skipped silently rather than failing the whole call.
- **Multi-line `unsafe fn` declarations.** A `pub unsafe fn` whose `# Safety` doc lives outside the diff hunk's context will trigger a false positive. Mitigation: documented in the detector comment; users read the warning and dismiss.
- **Axiom ID drift.** Detectors hard-code axiom IDs as `&'static str`; if the corpus renames an axiom, the detector returns a stale ID. Mitigation: a test asserts every detector's `axiom_id` resolves to a real axiom (using `axioms_for_test()`), parallel to the project_style/decisions cross-reference tests.
- **Plan drift between drafts and post-fix files** (recurring across all prior phases). Mitigation: explicit reconciliation pass before final review.

## Phase 5.1+ Continuation Outline

Out of scope; sketched only:
- Phase 5.1 — tree-sitter detectors for the most false-positive-prone patterns (e.g. `.unwrap()` correctly excluded from string-literal contexts).
- Phase 5.2 — project_style integration: detectors that an `axiom_overrides[].severity = "ignored"` axiom maps to are silently dropped; `noted` adds a project-specific message.
- Phase 5.3 — broader detector menu: `for i in 0..arr.len()` indexed access (axiom 87), `Box<dyn Trait>` for closed sets that should be enums (axiom 90), `format!` in a loop (axiom 85).
- Phase 5.4 — auto-suggested fixes (small inline rewrites for the simplest violations).

After Phase 5, all phases from the original Phase 0 outline are landed.

## Open Questions for User Review

- Confirm Architecture A (regex on diff lines) vs B (tree-sitter) vs C (rust-analyzer). Recommendation: A.
- Confirm the 5-detector menu. Alternative: drop the `unwrap_in_non_test` detector (highest false-positive rate) and replace with a less-noisy detector like `format_in_loop` (axiom 85) or keep at 4. My lean: keep all 5; the `info` severity tier signals lower confidence.
- Confirm `diff: String` input shape vs the alternative "git ref range, server runs git". Lean: string input — simpler, no git plumbing in the server.
- Confirm severity values (`Info`/`Warning`/`Error`). MADR/clippy-style is similar enough that this should slot in cleanly.
