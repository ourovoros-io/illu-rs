//! Critique: regex-based axiom-violation detection on unified-diff input.
//!
//! Four detectors (Phase 5 first cut) target high-confidence patterns that
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
//!
//! Phase 5 dropped a fifth proposed `unwrap_in_non_test` detector because
//! the universal corpus has no matching axiom — `rust_quality_42_todo_macros`
//! covers `todo!()`/`unimplemented!()`/`unreachable!()` rather than
//! `.unwrap()`/`.expect()`, and inventing an axiom ID would have left the
//! detector pointing at a non-existent rule.

use regex_lite::Regex;
use std::fmt::Write;
use std::path::PathBuf;
use std::sync::{LazyLock, OnceLock};
use thiserror::Error;

/// Failure categories for [`handle_critique`] input validation.
///
/// Detector execution itself is infallible (regex passes either match or
/// they don't); this enum carries the pre-detector input rejections.
/// `#[non_exhaustive]` because future input checks (e.g. unsupported
/// diff format detected) can be added without breaking downstream
/// `match` statements.
///
/// Wrapped as [`crate::IlluError::CritiqueInput`] when bubbled across
/// the module boundary; tests can match the precise variant directly
/// for [`Error Path Specificity`].
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum CritiqueInputError {
    /// `diff` length exceeds [`MAX_DIFF_BYTES`]. Carries `actual` and
    /// `limit_mib` as typed fields so callers can render a tailored
    /// message or decide programmatically whether to chunk and retry.
    #[error(
        "diff length {actual} bytes exceeds the {limit_mib} MiB limit; \
         pre-filter and resubmit smaller chunks"
    )]
    DiffTooLarge {
        /// Observed length of the rejected diff, in bytes.
        actual: usize,
        /// Cap in mebibytes; mirrors [`MAX_DIFF_BYTES`] / `1024 * 1024`.
        limit_mib: usize,
    },
}

/// Severity tier for a detected critique. Maps roughly to clippy's
/// info/warn/deny tiers but is advisory only — the tool returns
/// suggestions, never errors out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Severity {
    /// Suggestion-grade observation; usually a stylistic concern.
    Info,
    /// Likely smell; readers should justify the design or change it.
    Warning,
    /// Almost-certain bug or undefined-behavior risk; address before merge.
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

/// One detected axiom violation. Owned across all fields so detectors can
/// build values without juggling borrows of the diff input.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Critique {
    /// Stable axiom identifier (matches `assets/rust_quality_axioms.json`).
    pub axiom_id: String,
    /// Human-readable axiom title for display.
    pub axiom_title: String,
    /// File path as reported by the diff (`b/...` side).
    pub file: PathBuf,
    /// 1-based line number on the new-file side of the diff.
    pub line: u32,
    /// Advisory severity tier.
    pub severity: Severity,
    /// Message body shown to the user.
    pub message: String,
}

/// Internal: one line in a parsed unified-diff hunk. Stored lines are
/// either added (`+`) or context (` `); removed (`-`) lines are filtered
/// out at parse time so detectors never see them.
#[derive(Debug, Clone)]
struct HunkLine {
    new_line_number: u32,
    text: String,
    is_added: bool,
}

/// Internal: one parsed unified-diff hunk for a single file. The same
/// file may produce multiple hunks; each carries its own line range.
#[derive(Debug, Clone)]
struct DiffHunk {
    file: PathBuf,
    lines: Vec<HunkLine>,
}

/// Signature of a detector function. Each detector reads the parsed hunks
/// and emits its own critiques; the registry assigns the axiom metadata.
type DetectorFn = fn(&[DiffHunk]) -> Vec<Critique>;

/// Registry entry binding a detector to its axiom metadata. The registry
/// is the single source of truth for the (detector, axiom) mapping; new
/// detectors plug in by adding one row.
#[derive(Debug)]
struct DetectorEntry {
    /// Stable detector identifier used in tests and (future) telemetry.
    name: &'static str,
    /// Axiom ID this detector reports against; must resolve in the corpus.
    axiom_id: &'static str,
    /// Display title for the axiom.
    axiom_title: &'static str,
    /// Severity assigned to every critique this detector emits.
    severity: Severity,
    /// Pure function that scans hunks and emits critiques.
    detect: DetectorFn,
}

/// Process-global cache of detector entries. Computed once on first
/// access; the cost is one `Vec` construction per process lifetime.
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
        ]
    })
}

/// Defense-in-depth cap on the `diff` parameter the MCP tool accepts.
///
/// `parse_unified_diff` clones each added/context line into an owned
/// `String`, so a multi-GB diff allocates ~1.5–2× its size in resident
/// memory. The MCP transport imposes no per-tool size limit; this cap
/// prevents a misbehaving (or malicious) client from exhausting the
/// server's memory with a single oversized request. 16 MiB is several
/// orders of magnitude larger than any realistic `git diff` output.
const MAX_DIFF_BYTES: usize = 16 * 1024 * 1024;

/// Public entry point used by the `mcp__illu__critique` MCP tool.
///
/// # Errors
///
/// Returns [`crate::IlluError::CritiqueInput`] wrapping
/// [`CritiqueInputError::DiffTooLarge`] when `diff` exceeds
/// [`MAX_DIFF_BYTES`]. The detector pipeline itself is infallible;
/// below the cap, this function always returns `Ok`.
pub fn handle_critique(diff: &str) -> Result<String, crate::IlluError> {
    if diff.len() > MAX_DIFF_BYTES {
        // `into()` triggers `IlluError: From<CritiqueInputError>` so
        // the typed category survives the boundary and tests can match
        // on `CritiqueInputError::DiffTooLarge { .. }` directly.
        return Err(CritiqueInputError::DiffTooLarge {
            actual: diff.len(),
            limit_mib: MAX_DIFF_BYTES / (1024 * 1024),
        }
        .into());
    }
    let hunks = parse_unified_diff(diff);
    let mut all = Vec::new();
    for entry in detectors() {
        let mut found = (entry.detect)(&hunks);
        // Detectors emit critiques with empty axiom metadata; the registry
        // owns the binding so detector code stays focused on detection.
        for c in &mut found {
            c.axiom_id = entry.axiom_id.to_string();
            c.axiom_title = entry.axiom_title.to_string();
            c.severity = entry.severity;
        }
        all.extend(found);
    }
    Ok(format_critiques(&all))
}

/// Static regex matching the start of a unified-diff hunk. Capture group 1
/// is the new-file start line.
static HUNK_HEADER: LazyLock<Regex> = LazyLock::new(|| {
    #[expect(clippy::expect_used, reason = "static regex literal is always valid")]
    Regex::new(r"^@@ -\d+(?:,\d+)? \+(\d+)(?:,\d+)? @@").expect("hunk-header regex is valid")
});

/// Parse a unified diff string into hunks. Skips non-Rust files, non-diff
/// lines, and removed (`-`) lines. Tolerates malformed input by returning
/// fewer hunks rather than failing — this is a critique tool, not a strict
/// diff validator.
fn parse_unified_diff(diff: &str) -> Vec<DiffHunk> {
    let mut hunks: Vec<DiffHunk> = Vec::new();
    let mut current_file: Option<PathBuf> = None;
    let mut current_hunk: Option<DiffHunk> = None;
    let mut new_line_number: u32 = 0;

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
        } else if let Some(caps) = HUNK_HEADER.captures(line) {
            // Start a new hunk. Capture group 1 is the new-file start line.
            if let Some(h) = current_hunk.take() {
                hunks.push(h);
            }
            if let Some(file) = current_file.clone()
                && let Some(start_str) = caps.get(1)
                && let Ok(start) = start_str.as_str().parse::<u32>()
            {
                new_line_number = start;
                current_hunk = Some(DiffHunk {
                    file,
                    lines: Vec::new(),
                });
            }
        } else if let Some(hunk) = current_hunk.as_mut() {
            // Within an open hunk, classify each line by its prefix. The
            // `+++`/`---` file-marker lines are handled above and never
            // reach this branch because a new file begins a fresh hunk.
            if let Some(text) = line.strip_prefix('+') {
                hunk.lines.push(HunkLine {
                    new_line_number,
                    text: text.to_string(),
                    is_added: true,
                });
                new_line_number += 1;
            } else if let Some(text) = line.strip_prefix(' ') {
                hunk.lines.push(HunkLine {
                    new_line_number,
                    text: text.to_string(),
                    is_added: false,
                });
                new_line_number += 1;
            }
            // Removed lines (`-` but not `---`) and `\ No newline...`
            // markers are deliberately ignored: removed lines do not
            // advance the new-file line counter, and meta lines carry
            // no source content for detectors to inspect.
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

    // Stable order: file appearance order, then line, then axiom id.
    // Sorting on owned references keeps allocations to one Vec and
    // avoids cloning the `Critique` records themselves.
    let mut sorted: Vec<&Critique> = critiques.iter().collect();
    sorted.sort_by(|a, b| {
        a.file
            .cmp(&b.file)
            .then(a.line.cmp(&b.line))
            .then(a.axiom_id.cmp(&b.axiom_id))
    });
    let mut current_file: Option<&PathBuf> = None;
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

/// Cached pattern matching `unsafe {`. Word boundary on `unsafe` avoids
/// matching tokens like `transmute_unsafe`; the `\s*\{` allows one or
/// more spaces before the brace.
static UNSAFE_BLOCK_PAT: LazyLock<Regex> = LazyLock::new(|| {
    #[expect(clippy::expect_used, reason = "static regex literal is always valid")]
    Regex::new(r"\bunsafe\s*\{").expect("unsafe-block regex is valid")
});

/// Cached pattern matching a `// SAFETY:` comment marker.
static SAFETY_COMMENT_PAT: LazyLock<Regex> = LazyLock::new(|| {
    #[expect(clippy::expect_used, reason = "static regex literal is always valid")]
    Regex::new(r"//\s*SAFETY:").expect("SAFETY-comment regex is valid")
});

fn detect_bare_unsafe_block(hunks: &[DiffHunk]) -> Vec<Critique> {
    let mut out = Vec::new();
    for hunk in hunks {
        for (i, line) in hunk.lines.iter().enumerate() {
            if !line.is_added {
                continue;
            }
            if !UNSAFE_BLOCK_PAT.is_match(&line.text) {
                continue;
            }
            // Look at the immediately-preceding line (added or context) for
            // a `// SAFETY:` comment. If hunk-local context doesn't show
            // one, fire — the false-positive on cross-hunk context is a
            // documented limitation of the Phase 5 spec.
            let has_safety = i
                .checked_sub(1)
                .and_then(|j| hunk.lines.get(j))
                .is_some_and(|prev| SAFETY_COMMENT_PAT.is_match(&prev.text));
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

/// Cached pattern matching `pub unsafe fn` (with optional visibility
/// scope like `pub(crate)`).
static UNSAFE_FN_PAT: LazyLock<Regex> = LazyLock::new(|| {
    #[expect(clippy::expect_used, reason = "static regex literal is always valid")]
    Regex::new(r"\bpub(?:\([^)]*\))?\s+unsafe\s+fn\b").expect("unsafe-fn regex is valid")
});

/// Cached pattern matching a `# Safety` rustdoc heading.
static SAFETY_DOC_PAT: LazyLock<Regex> = LazyLock::new(|| {
    #[expect(clippy::expect_used, reason = "static regex literal is always valid")]
    Regex::new(r"#\s*Safety\b").expect("Safety-doc regex is valid")
});

fn detect_undocumented_unsafe_fn(hunks: &[DiffHunk]) -> Vec<Critique> {
    let mut out = Vec::new();
    for hunk in hunks {
        for (i, line) in hunk.lines.iter().enumerate() {
            if !line.is_added {
                continue;
            }
            if !UNSAFE_FN_PAT.is_match(&line.text) {
                continue;
            }
            // Look up to 20 lines back in the same hunk for a `# Safety`
            // heading inside doc-comment chrome (`///`).
            let start = i.saturating_sub(20);
            let has_safety_doc = hunk.lines[start..i]
                .iter()
                .any(|prev| prev.text.contains("///") && SAFETY_DOC_PAT.is_match(&prev.text));
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

/// Cached pattern matching `Box<T>` for any Copy primitive `T`. The list
/// is restricted to the standard primitives where boxing is almost never
/// the right call (heap pressure for a single word). User-defined types
/// and trait objects (`Box<dyn ...>`) are deliberately not flagged.
static BOX_OF_COPY_PAT: LazyLock<Regex> = LazyLock::new(|| {
    #[expect(
        clippy::expect_used,
        reason = "static regex literal is always valid"
    )]
    Regex::new(
        r"\bBox\s*<\s*(?:u8|u16|u32|u64|u128|usize|i8|i16|i32|i64|i128|isize|f32|f64|bool|char)\s*>",
    )
    .expect("Box-of-copy regex is valid")
});

fn detect_box_of_copy(hunks: &[DiffHunk]) -> Vec<Critique> {
    let mut out = Vec::new();
    for hunk in hunks {
        for line in &hunk.lines {
            if !line.is_added {
                continue;
            }
            // `find_iter` so multiple `Box<u32>`/`Box<bool>` on one line each
            // produce a critique. Single-match `find` would silently
            // undercount declarations like `Box<u32>, Box<bool>`.
            for m in BOX_OF_COPY_PAT.find_iter(&line.text) {
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

/// Cached pattern matching `mem::uninitialized` (deprecated since 1.39).
/// Accepts the bare path or `std::`/`core::` prefixes.
static MEM_UNINIT_PAT: LazyLock<Regex> = LazyLock::new(|| {
    #[expect(clippy::expect_used, reason = "static regex literal is always valid")]
    Regex::new(r"\b(?:std::|core::)?mem::uninitialized\b")
        .expect("mem::uninitialized regex is valid")
});

fn detect_mem_uninitialized(hunks: &[DiffHunk]) -> Vec<Critique> {
    let mut out = Vec::new();
    for hunk in hunks {
        for line in &hunk.lines {
            if !line.is_added {
                continue;
            }
            if MEM_UNINIT_PAT.is_match(&line.text) {
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

// ---- Tests -------------------------------------------------------------

#[cfg(test)]
#[expect(clippy::unwrap_used, reason = "tests")]
mod tests {
    use super::*;

    /// Build a minimal unified diff with one file + one hunk for tests.
    fn diff(file: &str, start_line: u32, added: &[&str]) -> String {
        let mut s = String::new();
        // `write!` on `String` appends without an intermediate allocation;
        // see std::fmt::Write impl for String. Tests can ignore the return
        // value because String's impl never errors.
        let _ = writeln!(s, "diff --git a/{file} b/{file}");
        let _ = writeln!(s, "--- a/{file}");
        let _ = writeln!(s, "+++ b/{file}");
        let _ = writeln!(s, "@@ -{start_line},0 +{start_line},{} @@", added.len());
        for line in added {
            let _ = writeln!(s, "+{line}");
        }
        s
    }

    /// Build a diff with one file + one hunk that includes context lines
    /// (each line is `(text, is_added)`).
    fn diff_with_context(file: &str, start_line: u32, lines: &[(&str, bool)]) -> String {
        let added_count = lines.iter().filter(|(_, a)| *a).count();
        let context_count = lines.iter().filter(|(_, a)| !*a).count();
        let mut s = String::new();
        let _ = writeln!(s, "diff --git a/{file} b/{file}");
        let _ = writeln!(s, "--- a/{file}");
        let _ = writeln!(s, "+++ b/{file}");
        let _ = writeln!(
            s,
            "@@ -{start_line},{context_count} +{start_line},{} @@",
            added_count + context_count
        );
        for (text, is_added) in lines {
            let prefix = if *is_added { '+' } else { ' ' };
            let _ = writeln!(s, "{prefix}{text}");
        }
        s
    }

    // ---- bare_unsafe_block ----

    #[test]
    fn test_detect_bare_unsafe_block_positive() {
        let d = diff(
            "src/foo.rs",
            10,
            &["fn x() {", "    unsafe { *p = 1; }", "}"],
        );
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
        // Two matches on one line — only the first is reported by `find`,
        // which is fine for the cross-axiom assertion.
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

    // ---- Integration ----

    #[test]
    fn test_handle_critique_integration_all_four_detectors() {
        let mut combined = String::new();
        // bare unsafe
        combined.push_str(&diff(
            "src/a.rs",
            10,
            &["fn x() {", "    unsafe { *p = 1; }", "}"],
        ));
        // undocumented unsafe fn
        combined.push_str(&diff(
            "src/b.rs",
            20,
            &["pub unsafe fn d(p: *const u8) -> u8 { *p }"],
        ));
        // box of copy
        combined.push_str(&diff("src/c.rs", 30, &["    timeout: Box<u32>,"]));
        // mem::uninitialized
        combined.push_str(&diff(
            "src/d.rs",
            40,
            &["    let x: u32 = unsafe { mem::uninitialized() };"],
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
        // Map id -> category so we can also verify the detector's
        // `axiom_title` matches the corpus's category. A title that
        // drifts from the corpus (typo, rename) would otherwise show
        // wrong text in critique output without any test failure.
        let by_id: std::collections::HashMap<&str, &str> = axioms
            .iter()
            .map(|a| (a.id.as_str(), a.category.as_str()))
            .collect();
        for entry in detectors() {
            let category = by_id.get(entry.axiom_id);
            assert!(
                category.is_some(),
                "detector `{}` references unknown axiom `{}`",
                entry.name,
                entry.axiom_id
            );
            assert_eq!(
                *category.unwrap(),
                entry.axiom_title,
                "detector `{}` axiom_title `{}` does not match corpus category for `{}`",
                entry.name,
                entry.axiom_title,
                entry.axiom_id
            );
        }
    }

    // ---- Negative parsing: non-Rust files are skipped ----

    #[test]
    fn test_non_rust_file_is_skipped() {
        let d = diff(
            "README.md",
            10,
            &[
                "    unsafe { *p = 1; }",
                "    let x: u32 = std::mem::uninitialized();",
            ],
        );
        let result = handle_critique(&d).unwrap();
        assert!(!result.contains("rust_quality_94_unsafe_block_discipline"));
        assert!(!result.contains("rust_quality_97_maybe_uninit"));
    }

    // ---- Hunk-local context: SAFETY comment as context line ----

    #[test]
    fn test_bare_unsafe_block_negative_with_context_safety_comment() {
        let d = diff_with_context(
            "src/foo.rs",
            10,
            &[
                ("    // SAFETY: caller guarantees p is non-null.", false),
                ("    unsafe { *p = 1; }", true),
            ],
        );
        let result = handle_critique(&d).unwrap();
        assert!(!result.contains("rust_quality_94_unsafe_block_discipline"));
    }

    // ---- Defense-in-depth: oversized diff is rejected ----

    #[test]
    fn test_handle_critique_rejects_oversized_diff() {
        // Construct a string just over the cap to verify the early return
        // path. We don't need to actually populate it with valid diff
        // content — the size check fires before parsing.
        let oversized = "x".repeat(MAX_DIFF_BYTES + 1);
        let err = handle_critique(&oversized).unwrap_err();
        // [Error Path Specificity]: assert the exact variant + payload.
        // A test that only checked `is_err()` or substring of `Display`
        // would silently pass if the cap path collapsed into a different
        // failure category during a refactor.
        assert!(
            matches!(
                err,
                crate::IlluError::CritiqueInput(CritiqueInputError::DiffTooLarge {
                    actual,
                    limit_mib,
                }) if actual == MAX_DIFF_BYTES + 1
                    && limit_mib == MAX_DIFF_BYTES / (1024 * 1024)
            ),
            "expected DiffTooLarge {{ actual: {}, limit_mib: {} }}, got: {err:?}",
            MAX_DIFF_BYTES + 1,
            MAX_DIFF_BYTES / (1024 * 1024)
        );
    }

    #[test]
    fn test_handle_critique_accepts_diff_at_cap() {
        // A diff exactly at the cap is allowed (the check is `>`, not `>=`).
        // Use whitespace so it parses to zero hunks rather than panicking
        // somewhere downstream.
        let at_cap = " ".repeat(MAX_DIFF_BYTES);
        let result = handle_critique(&at_cap).unwrap();
        assert!(result.contains("No potential axiom violations"));
    }
}
