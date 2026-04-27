//! Curated Rust API used by the `illu` binary, tests, and benchmarks.
//!
//! The CLI and MCP protocol are the long-term stable interface. This facade is
//! intentionally explicit: adding an item here is a conscious public-surface
//! decision, while implementation modules remain private to the crate.

/// Agent installation and configuration helpers.
pub mod agents {
    pub use crate::agents::{
        AGENTS, Agent, AgentWriteReport, DetectionLevel, GlobalConfig, GlobalPath, IlluCommand,
        McpFormat, RepoConfig, SetupFlags, configure_global, configure_repo, detect_global_agents,
        detect_repo_agents, known_agent_ids, self_heal_on_serve,
    };
}

/// `SQLite` index database types exposed for integration tests and tooling.
pub mod db {
    pub use crate::db::{
        CalleeInfo, CrateId, CrossRef, Database, DepId, DocResult, FileId, FileRecord,
        FileSymbolCount, ImpactEntry, LargestFunction, StoredCrate, StoredDep, StoredSymbol,
        StoredTraitImpl, SymbolId, SymbolIdMap, SymbolRefCount, TestEntry,
    };
}

/// Git repository discovery helpers.
pub mod git {
    pub use crate::git::{detect_cargo_root, detect_repo_root, git_common_dir, git_remote_url};
}

/// Source indexing entry points and selected parser result types.
pub mod indexer {
    pub use crate::indexer::{
        INDEX_VERSION, IndexConfig, has_python_project, index_repo, is_index_stale, refresh_index,
    };

    /// Rustdoc JSON parsing helpers.
    pub mod cargo_doc {
        pub use crate::indexer::cargo_doc::parse_rustdoc_json_modules;
    }

    /// Dependency documentation fetch/store helpers.
    pub mod docs {
        pub use crate::indexer::docs::{fetch_docs, pending_docs, store_fetched_docs};
    }

    /// Parser DTOs that tests assert against.
    pub mod parser {
        pub use crate::indexer::parser::{SymbolKind, Visibility};
    }
}

/// rust-analyzer client facade.
pub mod ra {
    pub use crate::ra::{PositionSpec, RaClient, RaError};
}

/// Persistent multi-repository registry types.
pub mod registry {
    pub use crate::registry::{Registry, RepoEntry};
}

/// MCP server facade and direct tool handler access for tests and benchmarks.
pub mod server {
    pub use crate::server::IlluServer;

    #[cfg(feature = "dashboard")]
    pub use crate::server::start_dashboard;

    /// Direct handlers for MCP tools.
    pub mod tools {
        /// Axioms search tool.
        pub mod axioms {
            pub use crate::server::tools::axioms::handle_axioms;
        }

        /// Batch symbol context tool.
        pub mod batch_context {
            pub use crate::server::tools::batch_context::handle_batch_context;
        }

        /// Git blame tool.
        pub mod blame {
            pub use crate::server::tools::blame::handle_blame;
        }

        /// Module-boundary analysis tool.
        pub mod boundary {
            pub use crate::server::tools::boundary::handle_boundary;
        }

        /// Call-path search tool.
        pub mod callpath {
            pub use crate::server::tools::callpath::handle_callpath;
        }

        /// Symbol context tool.
        pub mod context {
            pub use crate::server::tools::context::handle_context;
        }

        /// Workspace crate graph tool.
        pub mod crate_graph {
            pub use crate::server::tools::crate_graph::handle_crate_graph;
        }

        /// Cross-crate impact tool.
        pub mod crate_impact {
            pub use crate::server::tools::crate_impact::handle_crate_impact;
        }

        /// Cross-repository call-path tool.
        pub mod cross_callpath {
            pub use crate::server::tools::cross_callpath::handle_cross_callpath;
        }

        /// Cross-repository dependency tool.
        pub mod cross_deps {
            pub use crate::server::tools::cross_deps::handle_cross_deps;
        }

        /// Cross-repository impact tool.
        pub mod cross_impact {
            pub use crate::server::tools::cross_impact::handle_cross_impact;
        }

        /// Cross-repository search tool.
        pub mod cross_query {
            pub use crate::server::tools::cross_query::{CrossQueryOpts, handle_cross_query};
        }

        /// Diff impact analysis tool.
        pub mod diff_impact {
            pub use crate::server::tools::diff_impact::{
                DiffHunk, handle_diff_impact, parse_diff, run_git_diff,
            };
        }

        /// Documentation coverage tool.
        pub mod doc_coverage {
            pub use crate::server::tools::doc_coverage::handle_doc_coverage;
        }

        /// Dependency documentation lookup tool.
        pub mod docs {
            pub use crate::server::tools::docs::handle_docs;
        }

        /// Curated Rust exemplars search tool.
        pub mod exemplars {
            pub use crate::server::tools::exemplars::handle_exemplars;
        }

        /// File dependency graph tool.
        pub mod file_graph {
            pub use crate::server::tools::file_graph::handle_file_graph;
        }

        /// Index freshness tool.
        pub mod freshness {
            pub use crate::server::tools::freshness::handle_freshness;
        }

        /// Graph export tool.
        pub mod graph_export {
            pub use crate::server::tools::graph_export::handle_graph_export;
        }

        /// Index health tool.
        pub mod health {
            pub use crate::server::tools::health::handle_health;
        }

        /// Git history tool.
        pub mod history {
            pub use crate::server::tools::history::handle_history;
        }

        /// Hotspot analysis tool.
        pub mod hotspots {
            pub use crate::server::tools::hotspots::handle_hotspots;
        }

        /// Symbol impact analysis tool.
        pub mod impact {
            pub use crate::server::tools::impact::handle_impact;
        }

        /// Trait implementation lookup tool.
        pub mod implements {
            pub use crate::server::tools::implements::handle_implements;
        }

        /// Local call-graph neighborhood tool.
        pub mod neighborhood {
            pub use crate::server::tools::neighborhood::handle_neighborhood;
        }

        /// Orphaned symbol discovery tool.
        pub mod orphaned {
            pub use crate::server::tools::orphaned::handle_orphaned;
        }

        /// Directory overview tool.
        pub mod overview {
            pub use crate::server::tools::overview::handle_overview;
        }

        /// Symbol query tool.
        pub mod query {
            pub use crate::server::tools::query::handle_query;
        }

        /// Symbol references tool.
        pub mod references {
            pub use crate::server::tools::references::handle_references;
        }

        /// Rename planning tool.
        pub mod rename_plan {
            pub use crate::server::tools::rename_plan::handle_rename_plan;
        }

        /// Registered repositories tool.
        pub mod repos {
            pub use crate::server::tools::repos::handle_repos;
        }

        /// Rust preflight evidence tool.
        pub mod rust_preflight {
            pub use crate::server::tools::rust_preflight::handle_rust_preflight;
        }

        /// Similar-symbol search tool.
        pub mod similar {
            pub use crate::server::tools::similar::handle_similar;
        }

        /// Codebase statistics tool.
        pub mod stats {
            pub use crate::server::tools::stats::handle_stats;
        }

        /// Local standard-library documentation tool.
        pub mod std_docs {
            pub use crate::server::tools::std_docs::handle_std_docs;
        }

        /// Symbol-at-line lookup tool.
        pub mod symbols_at {
            pub use crate::server::tools::symbols_at::handle_symbols_at;
        }

        /// Test impact tool.
        pub mod test_impact {
            pub use crate::server::tools::test_impact::handle_test_impact;
        }

        /// File tree tool.
        pub mod tree {
            pub use crate::server::tools::tree::handle_tree;
        }

        /// Type usage tool.
        pub mod type_usage {
            pub use crate::server::tools::type_usage::handle_type_usage;
        }

        /// Unused symbol tool.
        pub mod unused {
            pub use crate::server::tools::unused::handle_unused;
        }
    }
}

/// Status-file helpers used by the CLI.
pub mod status {
    pub use crate::status::{READY, StatusGuard, clear, init, set};
}
