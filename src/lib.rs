mod core;
mod error;

#[cfg(target_arch = "wasm32")]
mod wasm_api;

pub use crate::core::{
    DistanceMetric, Document, Micropuffer, MiniStore, Namespace, explain_query, export_namespace,
    namespace_metadata, namespace_schema, patch_namespace_metadata, query_namespace, query_store,
    recall_namespace, update_namespace_schema, write_store,
};
pub use crate::error::QueryError;
