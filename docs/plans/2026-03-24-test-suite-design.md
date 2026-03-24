# Test Suite Design: Semantic Correctness Guards

**Date:** 2026-03-24
**Status:** Approved
**Goal:** Add ~94 tests across 4 new test files that verify the semantic correctness of indexing, reference resolution, graph traversal, incremental refresh, and cross-repo tools.

## Problem

The existing test suite (~230 integration + ~130 unit tests) is strong on **output formatting** (does the markdown look right?) but weak on **semantic correctness** (did we index the right thing? did we resolve the reference to the correct target? did we leave stale data behind?). Silent wrong answers are the worst kind of bug for an AI code intelligence tool.

## Structure

| File | Tests | Focus |
|------|-------|-------|
| `tests/parser_correctness.rs` | ~30 | Symbol extraction, ref resolution, confidence scoring, is_test detection |
| `tests/graph_correctness.rs` | ~22 | Impact CTE, test discovery, callpath, confidence filtering, cross-tool consistency |
| `tests/incremental_correctness.rs` | ~20 | Refresh pipeline, stale data cleanup, hash-based change detection |
| `tests/cross_repo_correctness.rs` | ~22 | Registry, cross-query/impact/deps, error handling, readonly DB |

These complement (not replace) existing files. Existing `data_quality.rs` and `data_integrity.rs` test output contracts. These new files test semantic truth.

## Shared Test Helpers

All files reuse the existing pattern: `index_source(lib_rs) -> (TempDir, Database)` and `index_multi_file(files)`. New helpers needed:

- `index_source_with_refs(lib_rs)` — indexes and returns DB with refs already extracted (most tests need this)
- `setup_two_repos_with_registry()` — for cross-repo tests, creates two repos + registry
- `get_ref_confidence(db, source_name, target_name) -> &str` — helper to query confidence column directly
- `symbol_exists_with_impl_type(db, name, impl_type) -> bool` — for Type::method verification

## File 1: `parser_correctness.rs`

Tests the indexer in isolation — no tool layer. Verifies that tree-sitter extraction produces correct symbols and references.

### Group 1: Symbol Extraction Edge Cases (~10 tests)

| Test | Invariant |
|------|-----------|
| `async_fn_preserves_async_in_signature` | Signature contains "async" |
| `unsafe_fn_preserves_unsafe_in_signature` | Signature contains "unsafe" |
| `const_generic_parameter_captured` | Signature contains `<const N: usize>` |
| `where_clause_not_truncated` | Signature contains full where clause |
| `enum_variants_get_parent_as_impl_type` | Variant's impl_type = parent enum name |
| `impl_method_gets_type_as_impl_type` | Method's impl_type = impl target type |
| `nested_mod_symbols_extracted` | Symbols inside `mod tests { }` blocks indexed |
| `extern_c_functions_extracted` | Foreign fn declarations inside `extern "C"` indexed |
| `union_item_extracted` | `union` keyword parsed as SymbolKind::Union |
| `reexport_use_captured` | `pub use other::Symbol` captured as Use kind |

### Group 2: Reference Resolution Accuracy (~11 tests)

| Test | Invariant |
|------|-----------|
| `self_method_resolves_to_own_impl_type` | `self.process()` in `impl Foo` → ref targets Foo::process |
| `self_method_no_cross_type_leak` | Two types with same method → self.run() doesn't cross-pollinate |
| `qualified_call_sets_target_context` | `Database::open()` → ref has target_context = "Database" |
| `crate_path_resolves_target_file` | `crate::status::set()` → ref points to src/status.rs |
| `crate_path_type_detection_by_case` | `crate::status::StatusGuard::new()` → type context = StatusGuard |
| `import_map_resolves_use_declaration` | `use config::Config; Config::new()` → ref has target_file |
| `aliased_import_resolves_correctly` | `use Foo as Bar; Bar::new()` → ref targets Foo's origin |
| `noisy_symbol_filtered_when_bare` | Bare `clear()` → no ref. `self.clear()` → ref created |
| `qualified_call_bypasses_noisy_filter` | `Status::clear()` → ref created despite "clear" being noisy |
| `constructor_on_unknown_type_filtered` | `Vec::new()` → no ref. `MyType::new()` → ref created |
| `local_variable_shadow_prevents_false_ref` | `let Config = 42;` → no ref to Config struct |

### Group 3: Confidence Scoring (~4 tests)

| Test | Invariant |
|------|-----------|
| `import_resolved_ref_is_high_confidence` | Use-resolved ref → confidence = "high" |
| `crate_path_ref_is_high_confidence` | crate:: path ref → confidence = "high" |
| `bare_name_ref_is_low_confidence` | Unresolved name → confidence = "low" |
| `self_method_ref_is_high_confidence` | self.method() → confidence = "high" |

### Group 4: Test Attribute Detection (~6 tests)

| Test | Invariant |
|------|-----------|
| `standard_test_attribute_detected` | `#[test]` → is_test = true |
| `tokio_test_detected` | `#[tokio::test]` → is_test = true |
| `rstest_detected` | `#[rstest]` → is_test = true |
| `test_case_with_args_detected` | `#[test_case(1, 2)]` → is_test = true |
| `non_test_attribute_not_detected` | `#[derive(Debug)]` → is_test = false |
| `tool_attribute_with_test_in_name_rejected` | `#[tool(name = "test_impact")]` → is_test = false |

## File 2: `graph_correctness.rs`

Builds known call graphs in the DB and verifies CTE queries return exactly the right results.

### Group 1: Impact CTE Correctness (~8 tests)

| Test | Invariant |
|------|-----------|
| `impact_linear_chain_correct_depth` | a→b→c→d: depth 1={b}, depth 2={b,c}, depth 3={b,c,d} |
| `impact_diamond_deduplicates` | a→b→d, a→c→d: d appears once at depth 2 |
| `impact_circular_terminates` | a→b→c→a: CTE terminates, correct results |
| `impact_respects_depth_limit` | Chain of 7: depth limit 5 stops at 5 |
| `impact_via_chain_accurate` | a→b→c: depth-2 entry shows "via b" |
| `impact_type_method_syntax_works` | Database::open finds only open with impl_type=Database |
| `impact_excludes_low_confidence_refs` | Low-confidence ref at depth 2+ not followed |
| `impact_limit_100_truncates` | >100 dependents → exactly 100 returned |

### Group 2: Test Discovery Accuracy (~5 tests)

| Test | Invariant |
|------|-----------|
| `related_tests_finds_direct_test_caller` | test_foo calls helper → found |
| `related_tests_finds_transitive_test` | test_foo→mid→helper → test_foo found for helper |
| `related_tests_excludes_non_test_callers` | Production caller not in test results |
| `related_tests_respects_impl_type` | Foo::process tests ≠ Bar::process tests |
| `related_tests_no_false_positives_from_low_confidence` | Low-confidence test→symbol ref not counted |

### Group 3: Caller/Callee Confidence Filtering (~4 tests)

| Test | Invariant |
|------|-----------|
| `get_callees_only_returns_high_confidence` | Mixed confidence → only high returned |
| `get_callers_with_high_filter_excludes_low` | context path: low-confidence callers excluded |
| `get_callers_with_no_filter_includes_all` | boundary path: all confidences included |
| `neighborhood_excludes_low_confidence` | BFS doesn't follow low-confidence edges |

### Group 4: Callpath Correctness (~4 tests)

| Test | Invariant |
|------|-----------|
| `callpath_finds_shortest_path` | Multiple paths → BFS returns shortest |
| `callpath_no_path_returns_clear_message` | Disconnected → "No call path found" |
| `callpath_all_paths_finds_multiple` | Diamond → both paths found |
| `callpath_exclude_tests_skips_test_nodes` | Path through test skipped when exclude_tests=true |

### Group 5: Cross-Tool Consistency (~3 tests)

| Test | Invariant |
|------|-----------|
| `impact_and_callers_agree` | context callers at depth 0 = impact depth 1 |
| `test_impact_and_impact_related_tests_agree` | Same test set from both tools |
| `neighborhood_down_matches_callees` | neighborhood hops=1 down = context callees |

## File 3: `incremental_correctness.rs`

Tests that incremental re-indexing doesn't leave stale data behind.

### Group 1: Symbol Lifecycle (~5 tests)

| Test | Invariant |
|------|-----------|
| `refresh_adds_new_symbol` | Add function → appears in query |
| `refresh_removes_deleted_symbol` | Remove function → gone from query |
| `refresh_updates_changed_signature` | Edit signature → updated in DB |
| `refresh_updates_moved_line_numbers` | Prepend lines → line numbers shift |
| `refresh_preserves_unchanged_symbols` | Edit file A → file B symbols untouched |

### Group 2: Reference Lifecycle (~5 tests)

| Test | Invariant |
|------|-----------|
| `refresh_removes_refs_from_deleted_file` | Delete caller file → refs from it gone |
| `refresh_removes_refs_to_deleted_symbol` | Rename target → old refs cleaned |
| `refresh_updates_ref_when_call_removed` | Remove call → ref gone from impact |
| `refresh_adds_ref_when_call_added` | Add call → ref appears in impact |
| `refresh_refs_from_untouched_files_survive` | Edit target file → caller refs still valid |

### Group 3: Stale Data Scenarios (~4 tests)

| Test | Invariant |
|------|-----------|
| `no_ghost_symbols_after_file_delete` | Zero symbols + zero refs for deleted file path |
| `no_ghost_refs_after_symbol_rename` | Zero refs to old name after rename |
| `crate_id_updates_when_file_moves_between_crates` | crate_id FK updates after file move |
| `version_mismatch_triggers_full_reindex` | INDEX_VERSION change → full re-index |

### Group 4: Concurrent-Edit Resilience (~3 tests)

| Test | Invariant |
|------|-----------|
| `refresh_handles_file_added_and_deleted_simultaneously` | Both operations in one refresh |
| `refresh_handles_empty_file_after_edit` | Empty content → symbols removed, no crash |
| `refresh_handles_syntax_error_gracefully` | Partial parse → valid symbols still indexed |

### Group 5: Hash-Based Change Detection (~3 tests)

| Test | Invariant |
|------|-----------|
| `identical_content_after_rewrite_skips_reindex` | Same hash → not re-indexed |
| `whitespace_only_change_triggers_reindex` | Different hash → re-indexed |
| `refresh_with_no_changes_is_noop` | Returns 0, DB unchanged |

## File 4: `cross_repo_correctness.rs`

Tests the multi-repo layer — registry, cross-repo tools, error handling.

### Group 1: Registry Correctness (~4 tests)

| Test | Invariant |
|------|-----------|
| `registry_save_load_roundtrip` | All fields intact after save/load |
| `registry_dedup_by_git_common_dir` | Worktree dedup → one entry |
| `registry_prune_removes_missing_repos` | Missing path → pruned |
| `registry_other_repos_excludes_primary` | Primary not in other_repos() |

### Group 2: Cross-Query Accuracy (~4 tests)

| Test | Invariant |
|------|-----------|
| `cross_query_finds_symbol_in_other_repo` | Symbol in repo B found from repo A |
| `cross_query_excludes_primary_repo_results` | Only repo B results shown |
| `cross_query_with_kind_filter_works` | kind:"struct" filters across repos |
| `cross_query_returns_clear_message_when_no_repos` | Empty registry → clear message |

### Group 3: Cross-Impact Accuracy (~3 tests)

| Test | Invariant |
|------|-----------|
| `cross_impact_finds_name_based_refs` | Repo B calls shared_helper → found |
| `cross_impact_respects_impl_type` | Foo::process refs ≠ Bar::process refs |
| `cross_impact_with_no_refs_returns_clear_message` | No refs → clear message |

### Group 4: Cross-Deps (~3 tests)

| Test | Invariant |
|------|-----------|
| `cross_deps_finds_shared_crate_dependencies` | Both repos use serde → detected |
| `cross_deps_detects_path_dependencies` | path = "../repo-a" → detected |
| `cross_deps_with_no_overlap_returns_clear_message` | No shared deps → clear message |

### Group 5: Error Handling (~4 tests)

| Test | Invariant |
|------|-----------|
| `cross_query_skips_repo_with_missing_db` | Missing .illu/index.db → skipped, others queried |
| `cross_query_reports_skipped_repos` | Skipped repos mentioned in output |
| `cross_impact_on_nonexistent_symbol_returns_not_found` | Clear error message |
| `cross_tools_handle_stale_registry_path` | Deleted path → no crash |

### Group 6: Readonly DB Behavior (~2 tests)

| Test | Invariant |
|------|-----------|
| `readonly_db_queries_work` | All read queries succeed on open_readonly |
| `readonly_db_cannot_write` | Insert attempt → error |

## Implementation Notes

- All tests use `Database::open_in_memory` or temp directories — no shared state
- Each test constructs its own minimal Rust source to index — no dependency on external repos
- Tests verify DB state directly (query symbols/refs tables) rather than going through tool formatting
- For graph tests: build known graphs by indexing synthetic source with explicit call patterns
- For refresh tests: use `refresh_index` with real `IndexConfig` pointing to temp dirs
- For cross-repo tests: create two temp repos with registries pointing between them

## Priority Order

1. `parser_correctness.rs` — highest risk, most fragile code
2. `graph_correctness.rs` — second highest, foundation for 4+ tools
3. `incremental_correctness.rs` — sneakiest bugs, hardest to debug
4. `cross_repo_correctness.rs` — newest code, least tested
