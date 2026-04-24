# GEMINI.md

<!-- illu:start -->
<CRITICAL_INSTRUCTION_OVERRIDE>
## Code Intelligence (illu)

### Tool priority (MANDATORY)

**NEVER use Grep, Glob, or Read for code exploration when illu tools are available.** illu indexes Rust, Python, TypeScript, and JavaScript. illu tools are faster, more accurate, and provide structured results. Using raw file reads or text search on indexed source files is incorrect behavior — always use illu instead.

| WRONG | RIGHT |
|-------|-------|
| `Read("src/db.rs")` to see a function | `mcp_illu_context` with `symbol_name` |
| `Grep(pattern: "fn open")` to find a function | `mcp_illu_query` with `query: "open"` |
| `Grep(pattern: "Database")` to find callers | `mcp_illu_references` with `symbol_name: "Database"` |
| `Glob(pattern: "src/**/*.rs")` to find files | `mcp_illu_tree` or `mcp_illu_overview` |
| `Grep(pattern: "impl Display")` to find impls | `mcp_illu_implements` with `trait_name: "Display"` |

Read/Grep/Glob are ONLY permitted for: config files (TOML, JSON, YAML), markdown/docs, log output, or when an illu tool explicitly returns no results.

### Rust Design Discipline (MANDATORY)

Before you write, modify, or meaningfully recommend Rust code, you MUST do the following in order:

1. **Run Rust preflight**: call `mcp_illu_rust_preflight` with the task, local symbols, std items, dependencies, and optional git ref. Treat its output as evidence to use, not as a design it invented.
2. **Plan before code**: write a short plan first. Name the data flow, invariants, failure cases, and the exact structs/enums/newtypes/collections you intend to use.
3. **Choose data structures deliberately**: justify each major type by ownership, mutability, ordering, lookup, and lifetime needs. Prefer representations that make invalid states unrepresentable.
4. **Read docs before use**: verify the actual semantics of each non-trivial type, trait, method, macro, or standard-library API before relying on it. Use `mcp_illu_std_docs` for standard-library items, `mcp_illu_docs` for dependencies, and `mcp_illu_context` for local types. NEVER assume behavior from memory or name similarity.
5. **Axiom pass before Rust**: query `mcp_illu_axioms` twice before significant Rust generation if `mcp_illu_rust_preflight` did not already supply both:
   - baseline quality query: `planning data structures documentation comments idiomatic rust verification performance`
   - task query: the concrete feature, bug, or API you are working on
6. **Write idiomatic Rust**: follow The Rust Book, Rust for Rustaceans, and illu axioms. Prefer ownership/borrowing, enums, iterators, explicit error handling, and compile-time modeling over ported Java/C++/Python patterns.
7. **Comments are first-class**: comments must explain invariants, safety conditions, concurrency assumptions, ownership rationale, or why a design exists. Delete comments that merely narrate syntax.
8. **Gate before final**: before final answer or commit for any Rust diff, call `mcp_illu_quality_gate` with the plan, docs verified, impact checked, tests run, and safety/performance evidence when relevant. Treat `BLOCKED` as not ready.

### Subagent instructions (MANDATORY)

When spawning subagents for code tasks, ALWAYS include this instruction in the prompt:

"MANDATORY: Use mcp_illu_* tools instead of Grep/Glob/Read for ALL code exploration (Rust, Python, TypeScript/JavaScript). NEVER use Read to view source files — use mcp_illu_context instead. NEVER use Grep to search code — use mcp_illu_query instead. Only use Read/Grep/Glob for non-code content (config, docs, logs). Before giving Rust implementation advice, first call mcp_illu_rust_preflight, make a short plan, choose data structures deliberately, verify docs for every non-trivial API with mcp_illu_std_docs/mcp_illu_docs/mcp_illu_context, and run mcp_illu_quality_gate before final answer or commit."

Prefer `illu-explore`, `illu-review`, `illu-refactor` agents when available.

### Workflow

1. **Locate before you read**: `mcp_illu_query` or `mcp_illu_context` first, then Read only what you need
2. **Rust preflight before you generate Rust**: use `mcp_illu_rust_preflight` to gather axioms, local context, impact hints, std docs, dependency docs, and model-failure reminders
3. **Plan before you generate Rust**: produce a short design plan naming data structures, ownership/mutability, invariants, and error handling
4. **Impact before you change**: always run `mcp_illu_impact` before modifying any public symbol
5. **Save tokens**: use `sections` param on context/batch_context to fetch only what you need
6. **Production focus**: use `exclude_tests: true` to filter out test functions
7. **Cross-repo**: use `mcp_illu_cross_query`/`mcp_illu_cross_impact`/`mcp_illu_cross_deps`/`mcp_illu_cross_callpath` — NEVER navigate to or read files from other repositories directly
8. **Axiom pass**: use `mcp_illu_axioms` with `planning data structures documentation comments idiomatic rust verification performance` first, then with the concrete task context before meaningful Rust design/code
9. **Documentation pass**: use `mcp_illu_context` with `sections: ["docs", "source"]` for local types, `mcp_illu_docs` for dependency types, and `mcp_illu_std_docs` for standard-library behavior
10. **Quality gate**: use `mcp_illu_quality_gate` before final answer or commit; `BLOCKED` means keep working

### Hard Constraints (Middleware Interception Simulation)
You MUST act as if a middleware intercepts your native tool calls. If you ever attempt to use `view_file`, `grep_search`, `Read`, or `Glob` on source code files, you must IMMEDIATELY halt that thought process. Instead of throwing an error and stopping, you must **automatically self-correct and invoke `mcp_illu_context` or `mcp_illu_query` instead.**

If you are about to write or modify Rust code, or propose a Rust architectural plan, and you have NOT yet called `mcp_illu_rust_preflight` for the task, you must IMMEDIATELY halt your generation. You must call `mcp_illu_rust_preflight` first.

If you are about to write or modify Rust code, or propose a Rust architectural plan, and you have NOT yet produced a short plan naming the data structures, ownership/mutability model, and error strategy, you must IMMEDIATELY halt your generation. You must write that plan first.

If you have NOT yet queried `mcp_illu_axioms` using both the baseline quality query `planning data structures documentation comments idiomatic rust verification performance` and the task-specific context in this session, you must IMMEDIATELY halt your generation. You must **invoke `mcp_illu_axioms` first, and only then proceed with Rust generation.**

If you are about to rely on the behavior of a type, trait, method, macro, or standard-library API whose semantics you have not verified from documentation or authoritative code context, you must IMMEDIATELY halt and read the docs first. Standard library items require `mcp_illu_std_docs` and are NOT exempt.

If you are about to final-answer or commit a Rust diff and have NOT called `mcp_illu_quality_gate` with your plan, docs verified, impact checked, and tests run, you must IMMEDIATELY halt and run `mcp_illu_quality_gate`. If it returns `BLOCKED`, do not present the work as complete.

If you are about to add comments that merely restate what the code already says, you must delete or rewrite them so they capture invariants, why, safety, or other non-obvious context.
</CRITICAL_INSTRUCTION_OVERRIDE>
<!-- illu:end -->
