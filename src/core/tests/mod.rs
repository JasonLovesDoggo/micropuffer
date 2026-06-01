use crate::{
    QueryError,
    core::{
        DistanceMetric, Document, Micropuffer, MiniStore, Namespace, PATCH_BY_FILTER_LIMIT,
        ScalarEqKey, default_created_at, default_encryption, namespace_metadata, parse_fts_config,
        patch_by_filter_with_limit, query_namespace, query_store, tokenize, write_store,
    },
};
use base64::{
    Engine,
    engine::general_purpose::{STANDARD, STANDARD_NO_PAD},
};
use serde_json::{Map, Number, Value, json};

fn namespace() -> Namespace {
    serde_json::from_value(json!({
        "name": "demo",
        "documents": [
            {
                "id": 1,
                "vector": [0.0, 0.0],
                "sparse_vector": {"a": 1.0, "b": 0.0},
                "title": "Rust vector search guide",
                "body": "fast exact local search for dashboard tests",
                "tenant_id": "alpha",
                "tags": ["rust", "search"],
                "public": true,
                "score": 10,
                "timestamp": "2026-05-30T00:00:00Z"
            },
            {
                "id": 2,
                "vector": [1.0, 1.0],
                "sparse_vector": {"a": 0.2, "c": 0.5},
                "title": "Typescript dashboard mocks",
                "body": "query workbench mock data",
                "tenant_id": "beta",
                "tags": ["typescript", "dashboard"],
                "public": false,
                "score": 4,
                "timestamp": "2026-05-29T00:00:00Z"
            },
            {
                "id": 3,
                "vector": [2.0, 2.0],
                "sparse_vector": {"a": 0.0, "b": 0.8},
                "title": "Hybrid search tuning",
                "body": "rust bm25 vector hybrid ranking",
                "tenant_id": "alpha",
                "tags": ["rust", "bm25"],
                "public": true,
                "score": 7,
                "timestamp": "2026-05-28T00:00:00Z"
            }
        ]
    }))
    .unwrap()
}

fn rows(response: &Value) -> &Vec<Value> {
    response.get("rows").and_then(Value::as_array).unwrap()
}

mod benchmarks;
mod debug;
mod fts;
mod query;
mod validation;
mod workspace;
mod write;
