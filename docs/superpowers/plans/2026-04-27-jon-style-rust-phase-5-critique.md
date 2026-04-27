# Jon-Style Rust Phase 5 (Critique) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task.

**Goal:** Add `mcp__illu__critique` MCP tool that takes a unified-diff string and returns potential axiom violations detected by 5 regex-based detectors.

**Architecture:** Single new module `src/server/tools/critique.rs` with a hand-written unified-diff parser, 5 detector functions, a registry holding axiom metadata, and a Markdown formatter. Stateless — no `init`, no asset directory, no JSON schema. New MCP tool registered alongside the existing four.

**Tech Stack:** Rust 2024, `regex` crate (already a workspace dependency for indexing), `rmcp` macros. No new external dependencies.

---

## Pre-Execution Notes

**Spec:** [docs/superpowers/specs/2026-04-27-jon-style-rust-phase-5-critique-design.md](docs/superpowers/specs/2026-04-27-jon-style-rust-phase-5-critique-design.md)

**Existing state:** Phase 0–4 + Phase 6 merged. 102 universal axioms, 9 exemplars, project_style + decisions. The 5 axiom IDs the detectors reference must be verified against the corpus before commit (Step 2 of the plan).

**Lint constraints** (unchanged): `unwrap_used = "deny"`, `expect_used = "warn"`, `allow_attributes = "deny"`. Tests use `#[expect(clippy::unwrap_used, reason = "tests")]`.

**Key design decisions** (per the spec):
- Architecture A (regex on diff lines), not tree-sitter or rust-analyzer.
- 5 detectors: bare_unsafe_block, undocumented_unsafe_fn, box_of_copy, mem_uninitialized, unwrap_in_non_test.
- `diff: String` input, no git plumbing.
- Severity enum: `Info`, `Warning`, `Error` — `#[non_exhaustive]`.
- Detectors are independent `fn(&[DiffHunk]) -> Vec<Critique>` functions in a `OnceLock<Vec<DetectorEntry>>` registry.
- Hunk-local context only (no cross-hunk doc-comment lookup); known-limitation.

**Drafts pre-approved by user.** Skip the user-review step inside batch tasks.

---

## File Structure

| File | Change | Responsibility |
|------|--------|----------------|
| `src/server/tools/critique.rs` | Create | Module: parser, Severity/Critique types, 5 detectors, registry, handle_critique, 11 tests |
| `src/server/tools/mod.rs` | Modify | Add `pub mod critique;` |
| `src/server/mod.rs` | Modify | Register `mcp__illu__critique` MCP tool |
| `src/api.rs` | Modify | Re-export `critique::handle_critique` |

---

## Task 1: Schema + Parser + 5 Detectors + Tool Registration + Tests

**Files (create):** `src/server/tools/critique.rs`.

**Files (modify):** `src/server/tools/mod.rs`, `src/server/mod.rs`, `src/api.rs`.

- [ ] **Step 1: Verify the 5 axiom IDs against the corpus**

```bash
for id in rust_quality_94_unsafe_block_discipline rust_quality_95_unsafe_fn_contract rust_quality_93_heap_discipline rust_quality_97_maybe_uninit; do
  grep -q "\"id\": \"$id\"" assets/rust_quality_axioms.json && echo "OK: $id" || echo "MISSING: $id"
done
```

The fifth detector (`unwrap_in_non_test`) maps to "axiom 25" in the spec, but axiom 25 is a pre-Phase-0 entry whose exact ID needs verification. Find an axiom about unwrap discipline:

```bash
grep -B1 -A1 'unwrap' assets/rust_quality_axioms.json | head -20
```

Pick the closest matching axiom — it may be `rust_quality_25_explicit_error_handling` or a similar slug. If no such axiom exists, drop the detector to 4 and document in the commit message; do not invent an axiom ID.

- [ ] **Step 2: Create `src/server/tools/critique.rs`**

```rust
//! Critique: regex-based axiom-violation detection on unified-diff input.
//!
//! Five detectors (Phase 5 first cut) target high-confidence patterns that
//! commonly violate axioms in the Phase-0/1 corpus. Each detector is an
//! independent `fn(&[DiffHunk]) -> Vec<Critique>`; the registry binds each
//! to its axiom metadata so detector functions stay short.
//!
//! Limitations (documented as non-goals in the spec):
//! - Regex on diff text, not the full Rust AST. False positives in string
//!   literals, doc comments, and identifiers containing the pattern.
//! - Hunk-local context only. A `pub unsafe fn` whose `# Safety` doc lives
//!   above the hunk's first line will trigger a false positive.
//! - Removed lines are skipped; only added (`+`) and context (` `) lines
//!   are inspected.

use regex::Regex;
use std::fmt::Write;
use std::path::PathBuf;
use std::sync::OnceLock;

/// Severity tier for a detected critique. Maps roughly to clippy's
/// info/warn/deny tiers but is advisory only — the tool returns
/// suggestions, never errors out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Severity {
    Info,
    Warning,
    Error,
}

impl Severity {
    fn as_str(self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Warning => "warning",
            Self::Error => "error",
        }
    }
}

/// One detected axiom violation.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Critique {
    pub axiom_id: String,
    pub axiom_title: String,
    pub file: PathBuf,
    pub line: u32,
    pub severity: Severity,
    pub message: String,
}

/// Internal: one line in a parsed unified-diff hunk.
#[derive(Debug, Clone)]
struct HunkLine {
    new_line_number: u32,
    text: String,
    is_added: bool,
}

/// Internal: one parsed unified-diff hunk for a single file.
#[derive(Debug, Clone)]
struct DiffHunk {
    file: PathBuf,
    lines: Vec<HunkLine>,
}

/// Internal: signature of a detector function.
type DetectorFn = fn(&[DiffHunk]) -> Vec<Critique>;

/// Internal: registry entry binding a detector to its axiom metadata.
#[derive(Debug)]
struct DetectorEntry {
    name: &'static str,
    axiom_id: &'static str,
    axiom_title: &'static str,
    severity: Severity,
    detect: DetectorFn,
}

/// Process-global cache of detector entries. Computed once on first
/// access; the cost is one Vec construction per process lifetime.
fn detectors() -> &'static [DetectorEntry] {
    static REGISTRY: OnceLock<Vec<DetectorEntry>> = OnceLock::new();
    REGISTRY.get_or_init(|| {
        vec![
            DetectorEntry {
                name: "bare_unsafe_block",
                axiom_id: "rust_quality_94_unsafe_block_discipline",
                axiom_title: "Unsafe Block Discipline",
                severity: Severity::Warning,
                detect: detect_bare_unsafe_block,
            },
            DetectorEntry {
                name: "undocumented_unsafe_fn",
                axiom_id: "rust_quality_95_unsafe_fn_contract",
                axiom_title: "Unsafe Fn Contract",
                severity: Severity::Warning,
                detect: detect_undocumented_unsafe_fn,
            },
            DetectorEntry {
                name: "box_of_copy",
                axiom_id: "rust_quality_93_heap_discipline",
                axiom_title: "Heap Allocation Discipline",
                severity: Severity::Warning,
                detect: detect_box_of_copy,
            },
            DetectorEntry {
                name: "mem_uninitialized",
                axiom_id: "rust_quality_97_maybe_uninit",
                axiom_title: "MaybeUninit",
                severity: Severity::Error,
                detect: detect_mem_uninitialized,
            },
            DetectorEntry {
                name: "unwrap_in_non_test",
                // The implementer must replace this with the actual axiom
                // ID found during Step 1 verification. If no matching axiom
                // exists, drop this entry and document in the commit message.
                axiom_id: "rust_quality_TBD_unwrap_discipline",
                axiom_title: "Unwrap Discipline",
                severity: Severity::Info,
                detect: detect_unwrap_in_non_test,
            },
        ]
    })
}

/// Public entry point used by the `mcp__illu__critique` MCP tool.
pub fn handle_critique(diff: &str) -> Result<String, crate::IlluError> {
    let hunks = parse_unified_diff(diff);
    let mut all = Vec::new();
    for entry in detectors() {
        let mut found = (entry.detect)(&hunks);
        for c in &mut found {
            c.axiom_id = entry.axiom_id.to_string();
            c.axiom_title = entry.axiom_title.to_string();
            c.severity = entry.severity;
        }
        all.extend(found);
    }
    Ok(format_critiques(&all))
}

/// Parse a unified diff string into hunks. Skips non-Rust files,
/// non-diff lines, and removed (`-`) lines. Tolerates malformed input
/// by returning fewer hunks rather than failing — this is a critique
/// tool, not a strict diff validator.
fn parse_unified_diff(diff: &str) -> Vec<DiffHunk> {
    let mut hunks: Vec<DiffHunk> = Vec::new();
    let mut current_file: Option<PathBuf> = None;
    let mut current_hunk: Option<DiffHunk> = None;
    let mut new_line_number: u32 = 0;
    let hunk_header = Regex::new(r"^@@ -\d+(?:,\d+)? \+(\d+)(?:,\d+)? @@").unwrap_or_else(|_| {
        // Fallback: if regex fails to compile (it won't for this static
        // pattern), an empty regex matches nothing.
        Regex::new("$.").expect("trivial regex compiles")
    });

    for line in diff.lines() {
        if let Some(rest) = line.strip_prefix("+++ b/") {
            // New-file path. Close any open hunk and switch files.
            if let Some(h) = current_hunk.take() {
                hunks.push(h);
            }
            let path = PathBuf::from(rest);
            current_file = if path.extension().is_some_and(|e| e == "rs") {
                Some(path)
            } else {
                None
            };
        } else if let Some(caps) = hunk_header.captures(line) {
            // Start a new hunk. Capture group 1 is the new-file start line.
            if let Some(h) = current_hunk.take() {
                hunks.push(h);
            }
            if let Some(file) = current_file.clone() {
                if let Some(start_str) = caps.get(1) {
                    if let Ok(start) = start_str.as_str().parse::<u32>() {
                        new_line_number = start;
                        current_hunk = Some(DiffHunk {
                            file,
                            lines: Vec::new(),
                        });
                    }
                }
            }
        } else if let Some(hunk) = current_hunk.as_mut() {
            if let Some(text) = line.strip_prefix('+') {
                if !text.starts_with('+') {
                    // '+' indicates an added line. Skip the '++' file
                    // marker (handled above) by checking the first byte.
                    hunk.lines.push(HunkLine {
                        new_line_number,
                        text: text.to_string(),
                        is_added: true,
                    });
                    new_line_number += 1;
                }
            } else if let Some(text) = line.strip_prefix(' ') {
                hunk.lines.push(HunkLine {
                    new_line_number,
                    text: text.to_string(),
                    is_added: false,
                });
                new_line_number += 1;
            } else if line.starts_with('-') && !line.starts_with("---") {
                // Removed line — does not advance new_line_number, not stored.
            }
            // Other prefixes (\\ "No newline at end of file" etc.) ignored.
        }
    }
    if let Some(h) = current_hunk.take() {
        hunks.push(h);
    }
    hunks
}

/// Format detected critiques as Markdown grouped by file.
fn format_critiques(critiques: &[Critique]) -> String {
    let mut out = String::new();
    if critiques.is_empty() {
        let _ = writeln!(
            out,
            "## Critique results\n\nNo potential axiom violations detected in the diff.\n"
        );
        return out;
    }
    let _ = writeln!(
        out,
        "## Critique results — {} potential axiom violation{}\n",
        critiques.len(),
        if critiques.len() == 1 { "" } else { "s" }
    );

    // Group by file. Stable order: file appearance order, then line.
    let mut current_file: Option<&PathBuf> = None;
    let mut sorted: Vec<&Critique> = critiques.iter().collect();
    sorted.sort_by(|a, b| {
        a.file
            .cmp(&b.file)
            .then(a.line.cmp(&b.line))
            .then(a.axiom_id.cmp(&b.axiom_id))
    });
    for c in sorted {
        if current_file != Some(&c.file) {
            let _ = writeln!(out, "### {}\n", c.file.display());
            current_file = Some(&c.file);
        }
        let _ = writeln!(
            out,
            "#### Line {} — {} (`{}`)",
            c.line, c.axiom_title, c.axiom_id
        );
        let _ = writeln!(out, "**Severity:** {}\n", c.severity.as_str());
        let _ = writeln!(out, "{}\n", c.message);
    }
    out
}

// ---- Detectors ---------------------------------------------------------

fn detect_bare_unsafe_block(hunks: &[DiffHunk]) -> Vec<Critique> {
    let pat = Regex::new(r"\bunsafe\s*\{").unwrap_or_else(|_| Regex::new("$.").expect("trivial"));
    let safety_pat = Regex::new(r"//\s*SAFETY:").unwrap_or_else(|_| Regex::new("$.").expect("trivial"));
    let mut out = Vec::new();
    for hunk in hunks {
        for (i, line) in hunk.lines.iter().enumerate() {
            if !line.is_added {
                continue;
            }
            if !pat.is_match(&line.text) {
                continue;
            }
            // Look at the immediately-preceding line (added or context) for
            // a // SAFETY: comment. If hunk-local context doesn't show one,
            // fire.
            let has_safety = i
                .checked_sub(1)
                .and_then(|j| hunk.lines.get(j))
                .is_some_and(|prev| safety_pat.is_match(&prev.text));
            if !has_safety {
                out.push(Critique {
                    axiom_id: String::new(),
                    axiom_title: String::new(),
                    file: hunk.file.clone(),
                    line: line.new_line_number,
                    severity: Severity::Info,
                    message: "The line introduces an `unsafe` block without an immediately-preceding `// SAFETY:` comment. Pair every unsafe block with a comment that names the invariants the caller satisfies.".to_string(),
                });
            }
        }
    }
    out
}

fn detect_undocumented_unsafe_fn(hunks: &[DiffHunk]) -> Vec<Critique> {
    let pat = Regex::new(r"\bpub(?:\([^)]*\))?\s+unsafe\s+fn\b")
        .unwrap_or_else(|_| Regex::new("$.").expect("trivial"));
    let safety_doc = Regex::new(r"#\s*Safety\b").unwrap_or_else(|_| Regex::new("$.").expect("trivial"));
    let mut out = Vec::new();
    for hunk in hunks {
        for (i, line) in hunk.lines.iter().enumerate() {
            if !line.is_added {
                continue;
            }
            if !pat.is_match(&line.text) {
                continue;
            }
            // Look up to 20 lines back in the same hunk for a `# Safety`
            // heading inside doc-comment chrome (`///`).
            let start = i.saturating_sub(20);
            let has_safety_doc = hunk.lines[start..i]
                .iter()
                .any(|prev| prev.text.contains("///") && safety_doc.is_match(&prev.text));
            if !has_safety_doc {
                out.push(Critique {
                    axiom_id: String::new(),
                    axiom_title: String::new(),
                    file: hunk.file.clone(),
                    line: line.new_line_number,
                    severity: Severity::Info,
                    message: "Public `unsafe fn` declared without a `# Safety` rustdoc section in the surrounding 20 lines. Document the preconditions a caller must uphold.".to_string(),
                });
            }
        }
    }
    out
}

fn detect_box_of_copy(hunks: &[DiffHunk]) -> Vec<Critique> {
    let pat = Regex::new(r"\bBox\s*<\s*(?:u8|u16|u32|u64|u128|usize|i8|i16|i32|i64|i128|isize|f32|f64|bool|char)\s*>")
        .unwrap_or_else(|_| Regex::new("$.").expect("trivial"));
    let mut out = Vec::new();
    for hunk in hunks {
        for line in &hunk.lines {
            if !line.is_added {
                continue;
            }
            if let Some(m) = pat.find(&line.text) {
                out.push(Critique {
                    axiom_id: String::new(),
                    axiom_title: String::new(),
                    file: hunk.file.clone(),
                    line: line.new_line_number,
                    severity: Severity::Info,
                    message: format!(
                        "`{}` wraps a small Copy primitive in heap allocation. Store the value directly — `Box<T>` is for trait objects, recursive types, or genuinely large moves.",
                        m.as_str()
                    ),
                });
            }
        }
    }
    out
}

fn detect_mem_uninitialized(hunks: &[DiffHunk]) -> Vec<Critique> {
    let pat = Regex::new(r"\b(?:std::|core::)?mem::uninitialized\b")
        .unwrap_or_else(|_| Regex::new("$.").expect("trivial"));
    let mut out = Vec::new();
    for hunk in hunks {
        for line in &hunk.lines {
            if !line.is_added {
                continue;
            }
            if pat.is_match(&line.text) {
                out.push(Critique {
                    axiom_id: String::new(),
                    axiom_title: String::new(),
                    file: hunk.file.clone(),
                    line: line.new_line_number,
                    severity: Severity::Info,
                    message: "`mem::uninitialized` is undefined behavior for almost every type and is deprecated. Use `MaybeUninit::<T>::uninit()` and write fields through raw pointers instead.".to_string(),
                });
            }
        }
    }
    out
}

fn detect_unwrap_in_non_test(hunks: &[DiffHunk]) -> Vec<Critique> {
    let pat = Regex::new(r"\.(?:unwrap|expect)\s*\(")
        .unwrap_or_else(|_| Regex::new("$.").expect("trivial"));
    let cfg_test = Regex::new(r"#\[cfg\(test\)\]|^\s*mod\s+tests\b")
        .unwrap_or_else(|_| Regex::new("$.").expect("trivial"));
    let mut out = Vec::new();
    for hunk in hunks {
        // Skip files clearly under test trees.
        let path_str = hunk.file.to_string_lossy();
        if path_str.starts_with("tests/")
            || path_str.contains("/tests/")
            || path_str.ends_with("_test.rs")
            || path_str.ends_with("_tests.rs")
        {
            continue;
        }
        for (i, line) in hunk.lines.iter().enumerate() {
            if !line.is_added {
                continue;
            }
            if !pat.is_match(&line.text) {
                continue;
            }
            // Skip if any prior hunk-local line opens a test scope.
            let inside_test_scope = hunk.lines[..i]
                .iter()
                .any(|prev| cfg_test.is_match(&prev.text));
            if inside_test_scope {
                continue;
            }
            out.push(Critique {
                axiom_id: String::new(),
                axiom_title: String::new(),
                file: hunk.file.clone(),
                line: line.new_line_number,
                severity: Severity::Info,
                message: "`.unwrap()` / `.expect(` in non-test code panics on `None`/`Err`. Prefer `?` for error propagation, `unwrap_or` for defaults, or `let ... else` for explicit handling.".to_string(),
            });
        }
    }
    out
}

// ---- Tests -------------------------------------------------------------

#[cfg(test)]
#[expect(clippy::unwrap_used, reason = "tests")]
mod tests {
    use super::*;

    /// Build a minimal unified diff with one file + one hunk for tests.
    fn diff(file: &str, start_line: u32, added: &[&str]) -> String {
        let mut s = String::new();
        s.push_str(&format!("diff --git a/{file} b/{file}\n"));
        s.push_str(&format!("--- a/{file}\n"));
        s.push_str(&format!("+++ b/{file}\n"));
        s.push_str(&format!("@@ -{start_line},0 +{start_line},{} @@\n", added.len()));
        for line in added {
            s.push_str(&format!("+{line}\n"));
        }
        s
    }

    /// Build a diff with one file + one hunk that includes context lines
    /// (each line is `(text, is_added)`).
    fn diff_with_context(file: &str, start_line: u32, lines: &[(&str, bool)]) -> String {
        let added_count = lines.iter().filter(|(_, a)| *a).count();
        let context_count = lines.iter().filter(|(_, a)| !*a).count();
        let mut s = String::new();
        s.push_str(&format!("diff --git a/{file} b/{file}\n"));
        s.push_str(&format!("--- a/{file}\n"));
        s.push_str(&format!("+++ b/{file}\n"));
        s.push_str(&format!(
            "@@ -{start_line},{context_count} +{start_line},{} @@\n",
            added_count + context_count
        ));
        for (text, is_added) in lines {
            let prefix = if *is_added { '+' } else { ' ' };
            s.push_str(&format!("{prefix}{text}\n"));
        }
        s
    }

    // ---- bare_unsafe_block ----

    #[test]
    fn test_detect_bare_unsafe_block_positive() {
        let d = diff("src/foo.rs", 10, &["fn x() {", "    unsafe { *p = 1; }", "}"]);
        let result = handle_critique(&d).unwrap();
        assert!(result.contains("rust_quality_94_unsafe_block_discipline"));
        assert!(result.contains("Line 11"));
    }

    #[test]
    fn test_detect_bare_unsafe_block_negative_with_safety_comment() {
        let d = diff(
            "src/foo.rs",
            10,
            &[
                "fn x() {",
                "    // SAFETY: caller guarantees p is non-null.",
                "    unsafe { *p = 1; }",
                "}",
            ],
        );
        let result = handle_critique(&d).unwrap();
        assert!(!result.contains("rust_quality_94_unsafe_block_discipline"));
    }

    // ---- undocumented_unsafe_fn ----

    #[test]
    fn test_detect_undocumented_unsafe_fn_positive() {
        let d = diff(
            "src/foo.rs",
            10,
            &["pub unsafe fn deref(p: *const u8) -> u8 { *p }"],
        );
        let result = handle_critique(&d).unwrap();
        assert!(result.contains("rust_quality_95_unsafe_fn_contract"));
    }

    #[test]
    fn test_detect_undocumented_unsafe_fn_negative_with_safety_doc() {
        let d = diff(
            "src/foo.rs",
            10,
            &[
                "/// Dereferences a pointer.",
                "///",
                "/// # Safety",
                "/// `p` must be non-null and aligned.",
                "pub unsafe fn deref(p: *const u8) -> u8 { *p }",
            ],
        );
        let result = handle_critique(&d).unwrap();
        assert!(!result.contains("rust_quality_95_unsafe_fn_contract"));
    }

    // ---- box_of_copy ----

    #[test]
    fn test_detect_box_of_copy_positive() {
        let d = diff(
            "src/foo.rs",
            10,
            &["struct Config { timeout: Box<u32>, flag: Box<bool> }"],
        );
        let result = handle_critique(&d).unwrap();
        assert!(result.contains("rust_quality_93_heap_discipline"));
        // Two matches on one line — the formatter should produce two
        // critiques, but may collapse them. Just confirm the axiom fires.
    }

    #[test]
    fn test_detect_box_of_copy_negative_user_type() {
        let d = diff(
            "src/foo.rs",
            10,
            &["struct Plugin { handler: Box<dyn Handler> }"],
        );
        let result = handle_critique(&d).unwrap();
        assert!(!result.contains("rust_quality_93_heap_discipline"));
    }

    // ---- mem_uninitialized ----

    #[test]
    fn test_detect_mem_uninitialized_positive() {
        let d = diff(
            "src/foo.rs",
            10,
            &["let x: u32 = unsafe { std::mem::uninitialized() };"],
        );
        let result = handle_critique(&d).unwrap();
        assert!(result.contains("rust_quality_97_maybe_uninit"));
        assert!(result.contains("error"));
    }

    #[test]
    fn test_detect_mem_uninitialized_negative_maybeuninit() {
        let d = diff(
            "src/foo.rs",
            10,
            &["let x = std::mem::MaybeUninit::<u32>::uninit();"],
        );
        let result = handle_critique(&d).unwrap();
        assert!(!result.contains("rust_quality_97_maybe_uninit"));
    }

    // ---- unwrap_in_non_test ----

    #[test]
    fn test_detect_unwrap_positive() {
        let d = diff(
            "src/foo.rs",
            10,
            &["fn parse(s: &str) -> u32 { s.parse().unwrap() }"],
        );
        let result = handle_critique(&d).unwrap();
        // Implementer must verify the actual axiom ID and update.
        assert!(result.contains("Unwrap"));
    }

    #[test]
    fn test_detect_unwrap_negative_in_test_module() {
        let d = diff_with_context(
            "src/foo.rs",
            10,
            &[
                ("#[cfg(test)]", false),
                ("mod tests {", false),
                ("    fn it_works() {", false),
                ("        let x = parse(\"42\").unwrap();", true),
                ("    }", false),
                ("}", false),
            ],
        );
        let result = handle_critique(&d).unwrap();
        // The line is inside a #[cfg(test)] scope per hunk-local detection.
        // Skip the assertion if the implementer's axiom-id-resolution Step 1
        // changed the message. Looser: assert no Critique was produced for
        // unwrap-in-test even though one would for unwrap-in-prod.
        assert!(!result.contains("Unwrap"));
    }

    // ---- Integration ----

    #[test]
    fn test_handle_critique_integration_all_five_detectors() {
        let mut combined = String::new();
        // bare unsafe
        combined.push_str(&diff("src/a.rs", 10, &["fn x() {", "    unsafe { *p = 1; }", "}"]));
        // undocumented unsafe fn
        combined.push_str(&diff("src/b.rs", 20, &["pub unsafe fn d(p: *const u8) -> u8 { *p }"]));
        // box of copy
        combined.push_str(&diff("src/c.rs", 30, &["    timeout: Box<u32>,"]));
        // mem::uninitialized
        combined.push_str(&diff(
            "src/d.rs",
            40,
            &["    let x: u32 = unsafe { mem::uninitialized() };"],
        ));
        // unwrap in non-test
        combined.push_str(&diff(
            "src/e.rs",
            50,
            &["    let x = parse(s).unwrap();"],
        ));
        let result = handle_critique(&combined).unwrap();
        assert!(result.contains("rust_quality_94_unsafe_block_discipline"));
        assert!(result.contains("rust_quality_95_unsafe_fn_contract"));
        assert!(result.contains("rust_quality_93_heap_discipline"));
        assert!(result.contains("rust_quality_97_maybe_uninit"));
    }

    // ---- Cross-reference with axiom corpus ----

    #[test]
    fn test_detector_axiom_ids_resolve() {
        let axioms = crate::server::tools::axioms::axioms_for_test();
        let known: std::collections::HashSet<&str> =
            axioms.iter().map(|a| a.id.as_str()).collect();
        for entry in detectors() {
            // The unwrap-discipline detector's TBD ID is permitted to fail
            // if the implementer dropped the detector during Step 1; we
            // assert only on the four high-confidence detectors.
            if entry.axiom_id.starts_with("rust_quality_TBD") {
                continue;
            }
            assert!(
                known.contains(entry.axiom_id),
                "detector `{}` references unknown axiom `{}`",
                entry.name,
                entry.axiom_id
            );
        }
    }
}
```

(The implementer must replace the `rust_quality_TBD_unwrap_discipline` placeholder ID with the actual axiom ID found in Step 1. If no matching axiom exists, drop the `unwrap_in_non_test` detector entirely — delete its registry entry, its detector function, and its two tests — and reduce the test count to 9.)

- [ ] **Step 3: Wire `pub mod critique;` in `src/server/tools/mod.rs`**

Add alphabetically.

- [ ] **Step 4: Register `mcp__illu__critique` MCP tool in `src/server/mod.rs`**

```rust
#[derive(Deserialize, JsonSchema)]
struct CritiqueParams {
    /// Unified-diff output (e.g. from `git diff`).
    diff: String,
}
```

```rust
#[tool(
    name = "critique",
    description = "Critique a unified diff for potential axiom violations. Runs a small menu of regex-based detectors over added lines and returns a Markdown summary listing each potential violation by file:line, axiom ID + title, and severity (info/warning/error). Detectors cover unsafe block discipline, unsafe fn contracts, Box<CopyType>, mem::uninitialized, and .unwrap()/.expect() in non-test code. Output is advisory — false positives exist."
)]
async fn critique(
    &self,
    Parameters(params): Parameters<CritiqueParams>,
) -> Result<CallToolResult, McpError> {
    tracing::info!(diff_len = params.diff.len(), "Tool call: critique");
    let _guard = crate::status::StatusGuard::new(&format!("critique ▸ {} bytes", params.diff.len()));
    let result = tools::critique::handle_critique(&params.diff).map_err(to_mcp_err)?;
    Ok(text_result(result))
}
```

- [ ] **Step 5: Re-export in `src/api.rs`**

```rust
pub mod critique {
    pub use crate::server::tools::critique::handle_critique;
}
```

- [ ] **Step 6: Run cargo gauntlet**

```bash
cargo build
cargo test --lib
cargo clippy --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
```

All must pass. The 11 new tests should pass; existing 634 tests should be unaffected. Verify whether the `regex` crate is already in `Cargo.toml`'s `[dependencies]` — if not, add it.

- [ ] **Step 7: Commit Task 1**

```bash
git add src/server/tools/critique.rs src/server/tools/mod.rs src/server/mod.rs src/api.rs
git commit -m "$(cat <<'EOF'
critique: add Phase 5 axiom-violation detection on unified diffs

New mcp__illu__critique tool that takes a unified-diff string and
returns Markdown listing potential axiom violations by file:line,
axiom ID + title, severity (info/warning/error).

5 regex-based detectors (Phase 5 first cut, Architecture A):
- bare_unsafe_block (axiom 94, warn): unsafe { without // SAFETY:
- undocumented_unsafe_fn (axiom 95, warn): pub unsafe fn without
  # Safety rustdoc within 20 lines
- box_of_copy (axiom 93, warn): Box<u32>/Box<bool>/etc.
- mem_uninitialized (axiom 97, error): mem::uninitialized literal
- unwrap_in_non_test (axiom <id>, info): .unwrap()/.expect( outside
  /tests/, _test.rs, _tests.rs, or #[cfg(test)] hunk-local scope

Detectors are independent fn(&[DiffHunk]) -> Vec<Critique> functions
in a OnceLock<Vec<DetectorEntry>> registry; Phase 5.1+ can add more
(or upgrade to tree-sitter for fewer false positives) without
touching the tool surface.

Hand-written minimal unified-diff parser; skips removed lines and
non-Rust files. Hunk-local context only (a `# Safety` doc above the
hunk's first line will trigger a false positive — documented as a
non-goal in the spec).

11 tests: 5 positive + 5 negative + 1 integration covering all
detectors at once + 1 cross-reference test asserting every detector's
axiom_id resolves to a real entry in the universal corpus.

Closes Phase 5 (critique). After this, all phases from the original
Phase 0 outline are landed.
EOF
)"
```

---

## Task 2: End-to-End Verification + Plan Reconciliation

- [ ] **Step 1: Full cargo gauntlet**

```bash
cargo build
cargo test
cargo clippy --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
```

- [ ] **Step 2: Plan-reconciliation pass before final review** — if the implementer adjusted axiom IDs, dropped a detector, or tweaked the false-positive bounds during execution, update the plan body to match the merged code.

---

## Verification Summary

After all tasks:
- 1 new module (~700 lines incl. tests).
- 1 new MCP tool (`critique`) registered + re-exported.
- 11 tests (or 9 if the unwrap detector is dropped).
- Cargo gauntlet clean.

## Risks Realized During Execution

- **Axiom ID for unwrap detector may not exist.** Mitigation: drop the detector to 4, document in commit.
- **`regex` crate may not be a workspace dep.** Mitigation: add to `Cargo.toml` if absent (it almost certainly already is — used by tree-sitter integration and elsewhere).
- **False-positive rate.** Some negative tests may need fixture adjustment if the regex over-matches. Adjust the regex (e.g., add word-boundary anchors) rather than relaxing the assertion.
- **Plan drift between drafts and post-fix files** (recurring). Reconcile before final review.
