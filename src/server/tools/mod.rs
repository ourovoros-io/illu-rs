pub mod batch_context;
pub mod callpath;
pub mod context;
pub mod crate_graph;
pub mod diff_impact;
pub mod freshness;
pub mod docs;
pub mod impact;
pub mod overview;
pub mod query;
pub mod tree;
pub mod unused;

pub(crate) use crate::truncate_at as truncate_snippet;
