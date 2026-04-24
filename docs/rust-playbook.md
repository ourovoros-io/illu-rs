# Rust Playbook

This playbook is the local Rust quality contract for illu-rs. Agents must use it
with `rust_preflight`, `std_docs`, `axioms`, and `quality_gate` before changing
Rust code.

## Design First

- Start with the data flow, not the syntax. Name the inputs, outputs, ownership
  boundaries, failure modes, and invariants before writing code.
- Choose structs, enums, newtypes, and collections deliberately. Explain lookup
  needs, ordering needs, mutation points, lifetimes, and invalid states.
- Prefer representations that make illegal states unrepresentable. Do not carry
  loosely related booleans or stringly state when an enum or newtype would encode
  the rule.
- Read documentation before relying on behavior, including the standard library.
  Memory, naming, or "this probably works like..." is not evidence.

## Error Handling

- Use `IlluError` and `crate::Result<T>` across public crate boundaries.
- Prefer existing `IlluError` variants before adding a new one. Use
  `IlluError::Invalid` for rejected user/tool input, `IlluError::Git` for git
  subprocess state, `IlluError::Docs` for documentation lookup failures, and
  `IlluError::Other` only for true escape hatches.
- MCP handlers should return user-facing tool text for expected misses
  ("symbol not found", "docs unavailable") and reserve protocol errors for
  actual tool failure.
- Never add production `unwrap`, `expect`, `panic`, `todo`, or `unimplemented`
  to bypass modeling or error propagation.

## MCP Tool Shape

- Keep protocol parameter structs in `src/server/mod.rs` and handler logic in
  `src/server/tools/*.rs`.
- Handlers should take typed inputs and return `Result<String, IlluError>`.
- Use `run_blocking` when the handler shells out, reads files, or performs long
  work while holding the database lock.
- Build output with clear Markdown sections and concrete next steps. Do not hide
  missing evidence behind a successful-looking response.

## Database And Transactions

- `Database` owns one SQLite connection. Treat it as an indexed project view,
  not as a generic connection pool.
- Keep write sequences explicit and transactional when several tables must stay
  in sync.
- Preserve deterministic ordering in user-visible output with `ORDER BY`,
  `BTreeMap`, or explicit sorting.
- Schema changes must be paired with migration tests or focused database tests.

## Parser And Indexer

- Tree-sitter is the fast offline index. Use it for broad symbol discovery,
  structural scans, and language-agnostic indexing.
- rust-analyzer is the compiler-accurate authority. Use it for macro expansion,
  trait resolution, renames, diagnostics, and position-based reference checks.
- Parser changes need fixture-style tests that cover the exact syntax being
  added or fixed. Include negative cases when a pattern could overmatch.
- Store references with honest confidence. Do not label heuristic matches as
  compiler-accurate.

## Comments And Docs

- Comments are first-class design artifacts. Add them when they explain
  invariants, safety, concurrency, ownership rationale, performance constraints,
  or why a non-obvious design exists.
- Delete comments that narrate syntax or exist only to satisfy a lint.
- Public load-bearing types need docs that state their invariants and their role
  in the architecture.

## Performance Claims

- A performance claim needs evidence: benchmark output, complexity analysis,
  profiling data, or a before/after measurement.
- Avoid "faster" wording unless the task includes `performance_evidence`.
- Prefer deterministic, bounded work in MCP handlers. Be explicit when a tool may
  shell out, scan local docs, or walk a large graph.
