use crate::core::{
    DistanceMetric, Document, Micropuffer, MiniStore, Namespace, PATCH_BY_FILTER_LIMIT,
    ScalarEqKey, bm25_stats_build_count, default_created_at, default_encryption, id_key,
    namespace_metadata, parse_fts_config, query_namespace, query_store,
    reset_bm25_stats_build_count, tokenize, write_store,
};
use base64::{Engine, engine::general_purpose::STANDARD};
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

#[test]
fn ann_ranks_by_squared_distance_and_projects_attributes() {
    let response = query_namespace(
        &namespace(),
        &json!({
            "rank_by": ["vector", "ANN", [0.0, 0.0]],
            "limit": 2,
            "include_attributes": ["title"]
        }),
    )
    .unwrap();
    let rows = rows(&response);
    assert_eq!(rows[0]["id"], 1);
    assert_eq!(rows[0]["$dist"], 0.0);
    assert_eq!(rows[0]["title"], "Rust vector search guide");
    assert_eq!(rows[1]["id"], 2);
}

#[test]
fn top_k_selection_preserves_exact_tie_order() {
    let namespace: Namespace = serde_json::from_value(json!({
        "name": "ties",
        "documents": [
            {"id": "d", "vector": [1.0, 1.0]},
            {"id": "b", "vector": [1.0, 1.0]},
            {"id": "c", "vector": [1.0, 1.0]},
            {"id": "a", "vector": [1.0, 1.0]}
        ]
    }))
    .unwrap();
    let response = query_namespace(
        &namespace,
        &json!({
            "rank_by": ["vector", "ANN", [1.0, 1.0]],
            "limit": 2
        }),
    )
    .unwrap();

    assert_eq!(
        rows(&response)
            .iter()
            .map(|row| row["id"].clone())
            .collect::<Vec<_>>(),
        vec![json!("a"), json!("b")]
    );
}

#[test]
fn knn_requires_filters() {
    let error = query_namespace(
        &namespace(),
        &json!({
            "rank_by": ["vector", "kNN", [0.0, 0.0]],
            "limit": 2
        }),
    )
    .unwrap_err();
    assert!(error.to_string().contains("kNN requires filters"));
}

#[test]
fn filters_cover_boolean_array_glob_token_and_fuzzy_cases() {
    let response = query_namespace(
        &namespace(),
        &json!({
            "rank_by": ["score", "desc"],
            "filters": ["And", [
                ["public", "Eq", true],
                ["tags", "ContainsAny", ["bm25", "search"]],
                ["title", "IGlob", "*search*"],
                ["body", "ContainsAllTokens", "rust rank", {"last_as_prefix": true}],
                ["title", "Fuzzy", "hybryd", {"max_edits": [[1, 1], [5, 2]]}]
            ]],
            "limit": 5,
            "include_attributes": true
        }),
    )
    .unwrap();
    let rows = rows(&response);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["id"], 3);
}

#[test]
fn filters_support_documented_fuzzy_options_token_arrays_and_null_comparisons() {
    let fuzzy = query_namespace(
        &namespace(),
        &json!({
            "rank_by": ["id", "asc"],
            "filters": ["title", "Fuzzy", "hybryd", { "max_edit_distance": [
                {"min_query_chars": 3, "distance": 0},
                {"min_query_chars": 6, "distance": 1}
            ]}],
            "limit": 10
        }),
    )
    .unwrap();
    assert_eq!(rows(&fuzzy)[0]["id"], 3);

    let short_fuzzy = query_namespace(
        &namespace(),
        &json!({
            "rank_by": ["id", "asc"],
            "filters": ["title", "Fuzzy", "hy", { "max_edit_distance": [
                {"min_query_chars": 3, "distance": 0}
            ]}],
            "limit": 10
        }),
    )
    .unwrap();
    assert!(rows(&short_fuzzy).is_empty());

    let token_array = query_namespace(
        &namespace(),
        &json!({
            "rank_by": ["id", "asc"],
            "filters": ["body", "ContainsAllTokens", ["rust", "ranking"]],
            "limit": 10
        }),
    )
    .unwrap();
    assert_eq!(rows(&token_array)[0]["id"], 3);

    let null_lt = query_namespace(
        &namespace(),
        &json!({
            "rank_by": ["id", "asc"],
            "filters": ["missing_optional", "Lt", 5],
            "limit": 10
        }),
    )
    .unwrap();
    assert_eq!(rows(&null_lt).len(), 3);

    let null_lt_null = query_namespace(
        &namespace(),
        &json!({
            "rank_by": ["id", "asc"],
            "filters": ["missing_optional", "Lt", null],
            "limit": 10
        }),
    )
    .unwrap();
    assert!(rows(&null_lt_null).is_empty());
}

#[test]
fn non_bm25_rank_plans_do_not_build_bm25_stats() {
    let namespace = namespace();
    let non_bm25_queries = [
        json!({
            "rank_by": ["vector", "ANN", [0.0, 0.0]],
            "limit": 2
        }),
        json!({
            "rank_by": ["sparse_vector", "SparseKNN", {"a": 1.0}],
            "limit": 2
        }),
        json!({
            "rank_by": ["score", "desc"],
            "limit": 2
        }),
        json!({
            "rank_by": [["tenant_id", "asc"], ["score", "desc"]],
            "limit": 2
        }),
        json!({
            "rank_by": ["Decay", ["Dist", ["Attribute", "timestamp"], "2026-05-31T00:00:00Z"], { "midpoint": "1d" }],
            "limit": 2
        }),
    ];

    for query in non_bm25_queries {
        reset_bm25_stats_build_count();
        query_namespace(&namespace, &query).unwrap();
        assert_eq!(bm25_stats_build_count(), 0, "query: {query}");
    }
}

#[test]
fn bm25_and_rank_operators_score_higher_matches_first() {
    reset_bm25_stats_build_count();

    let response = query_namespace(
        &namespace(),
        &json!({
            "rank_by": ["Sum", [
                ["Product", 2, ["title", "BM25", "rust search"]],
                ["body", "BM25", "rust vector"]
            ]],
            "limit": 3,
            "include_attributes": ["title"]
        }),
    )
    .unwrap();
    let response_rows = rows(&response);
    assert_eq!(response_rows[0]["id"], 3);
    assert!(
        response_rows[0]["$dist"].as_f64().unwrap() > response_rows[1]["$dist"].as_f64().unwrap()
    );
    assert_eq!(bm25_stats_build_count(), 1);

    let token_array = query_namespace(
        &namespace(),
        &json!({
            "rank_by": ["body", "BM25", ["rust", "hybrid"]],
            "limit": 3
        }),
    )
    .unwrap();
    assert_eq!(rows(&token_array)[0]["id"], 3);
    assert_eq!(bm25_stats_build_count(), 2);
}

#[test]
fn rank_operators_reject_negative_product_and_decay_uses_duration_midpoints() {
    let error = query_namespace(
        &namespace(),
        &json!({
            "rank_by": ["Product", -1, ["body", "BM25", "rust"]],
            "limit": 3
        }),
    )
    .unwrap_err();
    assert!(error.to_string().contains("non-negative"));

    let response = query_namespace(
        &namespace(),
        &json!({
            "rank_by": ["Decay", ["Dist", ["Attribute", "timestamp"], "2026-05-31T00:00:00Z"], { "midpoint": "1d" }],
            "limit": 3
        }),
    )
    .unwrap();
    let rows = rows(&response);
    assert_eq!(rows[0]["id"], 1);
    assert_eq!(rows[0]["$dist"], 0.5);
}

#[test]
fn vector_queries_support_cosine_distance_and_base64_float32_vectors() {
    let mut clone = Micropuffer::new();
    clone
        .write(
            "metric-demo",
            &json!({
                "distance_metric": "cosine_distance",
                "upsert_rows": [
                    {"id": "same-direction", "vector": [10.0, 0.0], "title": "same direction"},
                    {"id": "near-euclidean", "vector": [1.0, 1.0], "title": "near euclidean"}
                ]
            }),
        )
        .unwrap();
    let response = clone
        .query(
            "metric-demo",
            &json!({
                "rank_by": ["vector", "ANN", [1.0, 0.0]],
                "limit": 2
            }),
        )
        .unwrap();
    assert_eq!(rows(&response)[0]["id"], "same-direction");
    assert_eq!(rows(&response)[0]["$dist"], 0.0);

    let encoded_query = STANDARD.encode([1.0_f32.to_le_bytes(), 0.0_f32.to_le_bytes()].concat());
    let encoded_doc = STANDARD.encode([0.0_f32.to_le_bytes(), 1.0_f32.to_le_bytes()].concat());
    clone
        .write(
            "base64-demo",
            &json!({
                "distance_metric": "euclidean_squared",
                "upsert_rows": [
                    {"id": "base64-doc", "vector": encoded_doc},
                    {"id": "array-doc", "vector": [1.0, 0.0]}
                ]
            }),
        )
        .unwrap();
    let base64_response = clone
        .query(
            "base64-demo",
            &json!({
                "rank_by": ["vector", "ANN", encoded_query],
                "limit": 2
            }),
        )
        .unwrap();
    assert_eq!(rows(&base64_response)[0]["id"], "array-doc");
}

#[test]
fn sparse_knn_uses_dot_product_descending_and_excludes_zero_scores() {
    let response = query_namespace(
        &namespace(),
        &json!({
            "rank_by": ["sparse_vector", "SparseKNN", {"a": 1.0}],
            "limit": 3
        }),
    )
    .unwrap();
    assert_eq!(
        rows(&response)
            .iter()
            .map(|row| row["id"].clone())
            .collect::<Vec<_>>(),
        vec![json!(1), json!(2)]
    );
    assert_eq!(rows(&response)[0]["$dist"], 1.0);
    assert_eq!(rows(&response)[1]["$dist"], 0.2);
}

#[test]
fn order_by_multiple_attributes_uses_stable_tie_breaks() {
    let response = query_namespace(
        &namespace(),
        &json!({
            "rank_by": [["tenant_id", "asc"], ["score", "desc"]],
            "limit": 3,
            "include_attributes": ["tenant_id", "score"]
        }),
    )
    .unwrap();
    assert_eq!(
        rows(&response)
            .iter()
            .map(|row| row["id"].clone())
            .collect::<Vec<_>>(),
        vec![json!(1), json!(3), json!(2)]
    );
    assert!(rows(&response)[0].get("$dist").is_none());
}

#[test]
fn indexed_eq_filter_uses_scalar_postings() {
    let namespace = namespace();
    let response = query_namespace(
        &namespace,
        &json!({
            "rank_by": ["id", "asc"],
            "filters": ["tenant_id", "Eq", "alpha"],
            "limit": 10
        }),
    )
    .unwrap();
    assert_eq!(
        rows(&response)
            .iter()
            .map(|row| row["id"].clone())
            .collect::<Vec<_>>(),
        vec![json!(1), json!(3)]
    );
    assert!(
        namespace
            .query_indexes
            .borrow()
            .equality
            .contains_key("tenant_id")
    );
}

#[test]
fn indexed_order_by_preserves_multi_attribute_and_stable_id_order() {
    let namespace: Namespace = serde_json::from_value(json!({
        "name": "ordered",
        "documents": [
            {"id": "z", "tenant": "a", "score": 1},
            {"id": "b", "tenant": "a", "score": 2},
            {"id": "a", "tenant": "a", "score": 1},
            {"id": "c", "tenant": "b", "score": 9}
        ]
    }))
    .unwrap();
    let response = query_namespace(
        &namespace,
        &json!({
            "rank_by": [["tenant", "asc"], ["score", "desc"]],
            "limit": 10
        }),
    )
    .unwrap();
    assert_eq!(
        rows(&response)
            .iter()
            .map(|row| row["id"].clone())
            .collect::<Vec<_>>(),
        vec![json!("b"), json!("a"), json!("z"), json!("c")]
    );
    assert_eq!(namespace.query_indexes.borrow().order.len(), 1);
}

#[test]
fn fts_schema_options_affect_bm25_and_token_filters() {
    let mut clone = Micropuffer::new();
    clone
        .write(
            "fts-options",
            &json!({
                "schema": {
                    "body": {
                        "type": "string",
                        "full_text_search": {
                            "ascii_folding": true,
                            "stemming": true,
                            "remove_stopwords": true,
                            "language": "english",
                            "max_token_length": 12,
                            "tokenizer": "word_v3"
                        }
                    },
                    "exact_body": {
                        "type": "string",
                        "full_text_search": {
                            "case_sensitive": true
                        }
                    }
                },
                "upsert_rows": [
                    {"id": 1, "vector": [0.0, 0.0], "body": "The café runner runs quickly", "exact_body": "Case Token"},
                    {"id": 2, "vector": [1.0, 1.0], "body": "fish reef", "exact_body": "case token"}
                ]
            }),
        )
        .unwrap();

    let folded_and_stemmed = clone
        .query(
            "fts-options",
            &json!({
                "rank_by": ["body", "BM25", "cafe running"],
                "limit": 10
            }),
        )
        .unwrap();
    assert_eq!(rows(&folded_and_stemmed)[0]["id"], 1);

    let stopword = clone
        .query(
            "fts-options",
            &json!({
                "rank_by": ["body", "BM25", "the"],
                "limit": 10
            }),
        )
        .unwrap();
    assert!(rows(&stopword).is_empty());

    let case_sensitive = clone
        .query(
            "fts-options",
            &json!({
                "rank_by": ["id", "asc"],
                "filters": ["exact_body", "ContainsAllTokens", "case"],
                "limit": 10
            }),
        )
        .unwrap();
    assert_eq!(rows(&case_sensitive)[0]["id"], 2);
}

#[test]
fn aggregates_and_grouped_aggregates_apply_filters() {
    let response = query_namespace(
        &namespace(),
        &json!({
            "aggregate_by": {"count": ["Count"], "score_sum": ["Sum", "score"]},
            "filters": ["public", "Eq", true],
            "limit": 10
        }),
    )
    .unwrap();
    assert_eq!(response["aggregations"]["count"], 2);
    assert_eq!(response["aggregations"]["score_sum"], 17.0);

    let grouped = query_namespace(
        &namespace(),
        &json!({
            "aggregate_by": {"count": ["Count"]},
            "group_by": ["tenant_id", {"tag": ["ForEachUnique", "tags"]}],
            "limit": {"total": 10}
        }),
    )
    .unwrap();
    let groups = grouped
        .get("aggregation_groups")
        .and_then(Value::as_array)
        .unwrap();
    assert!(groups.iter().any(|group| group["tenant_id"] == "alpha"
        && group["tag"] == "rust"
        && group["count"] == 2));
}

#[test]
fn multi_query_preserves_result_order() {
    let response = query_namespace(
        &namespace(),
        &json!({
            "queries": [
                {"rank_by": ["vector", "ANN", [0.0, 0.0]], "limit": 1},
                {"aggregate_by": {"count": ["Count"]}, "limit": 1}
            ]
        }),
    )
    .unwrap();
    let results = response.get("results").and_then(Value::as_array).unwrap();
    assert_eq!(results[0]["rows"][0]["id"], 1);
    assert_eq!(results[1]["aggregations"]["count"], 3);
}

#[test]
fn base64_vector_encoding_applies_to_included_vectors() {
    let response = query_namespace(
        &namespace(),
        &json!({
            "rank_by": ["vector", "ANN", [0.0, 0.0]],
            "limit": 1,
            "include_attributes": true,
            "vector_encoding": "base64"
        }),
    )
    .unwrap();
    assert_eq!(rows(&response)[0]["vector"], "AAAAAAAAAAA=");
}

#[test]
fn query_store_finds_namespace_by_name() {
    let store = MiniStore {
        namespaces: vec![namespace()],
    };
    let response = query_store(
        &store,
        "demo",
        &json!({
            "rank_by": ["score", "desc"],
            "limit": 1
        }),
    )
    .unwrap();
    assert_eq!(rows(&response)[0]["id"], 1);
}

#[test]
fn documents_flatten_unknown_attributes() {
    let document: Document = serde_json::from_value(json!({"id": "a", "custom": 42})).unwrap();
    assert_eq!(document.id, "a");
    assert_eq!(document.attributes["custom"], 42);
}

#[test]
fn writes_upsert_patch_delete_and_query_in_memory() {
    let mut store = MiniStore::default();
    write_store(
        &mut store,
        "local-test",
        &json!({
            "upsert_rows": [
                {"id": 1, "vector": [0.0, 0.0], "title": "first", "score": 10, "tenant_id": "a"},
                {"id": 2, "vector": [1.0, 1.0], "title": "second", "score": 2, "tenant_id": "b"}
            ]
        }),
    )
    .unwrap();
    let write = write_store(
        &mut store,
        "local-test",
        &json!({
            "patch_rows": [
                {"id": 1, "score": 11}
            ],
            "deletes": [2],
            "return_affected_ids": true
        }),
    )
    .unwrap();
    assert_eq!(write["rows_affected"], 2);
    let response = query_store(
        &store,
        "local-test",
        &json!({
            "rank_by": ["id", "asc"],
            "filters": ["id", "Gte", 1],
            "limit": 10,
            "include_attributes": true
        }),
    )
    .unwrap();
    let rows = rows(&response);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["id"], 1);
    assert_eq!(rows[0]["score"], 11);
}

#[test]
fn column_writes_conditions_ref_new_and_by_filter_mutations_work() {
    let mut store = MiniStore {
        namespaces: vec![namespace()],
    };
    write_store(
        &mut store,
        "demo",
        &json!({
            "upsert_columns": {
                "id": [4, 5],
                "vector": [[3.0, 3.0], [4.0, 4.0]],
                "title": ["condition pass", "condition fail"],
                "score": [12, 1],
                "tenant_id": ["alpha", "alpha"]
            }
        }),
    )
    .unwrap();
    let conditional = write_store(
        &mut store,
        "demo",
        &json!({
            "upsert_rows": [
                {"id": 4, "vector": [3.0, 3.0], "title": "updated", "score": 13, "tenant_id": "alpha"},
                {"id": 5, "vector": [4.0, 4.0], "title": "blocked", "score": 0, "tenant_id": "alpha"}
            ],
            "upsert_condition": ["score", "Lt", {"$ref_new": "score"}],
            "patch_by_filter": {
                "filters": ["tenant_id", "Eq", "beta"],
                "patch": {"tenant_id": "patched"}
            },
            "delete_by_filter": ["id", "Eq", 2]
        }),
    )
    .unwrap();
    assert_eq!(conditional["rows_affected"], 2);
    let response = query_store(
        &store,
        "demo",
        &json!({
            "rank_by": ["id", "asc"],
            "limit": 10,
            "include_attributes": ["title", "tenant_id", "score"]
        }),
    )
    .unwrap();
    let rows = rows(&response);
    assert_eq!(rows.len(), 4);
    assert_eq!(rows[2]["id"], 4);
    assert_eq!(rows[2]["title"], "updated");
    assert_eq!(rows[3]["id"], 5);
    assert_eq!(rows[3]["score"], 1);
}

#[test]
fn attribute_indexes_are_incrementally_maintained_after_upsert_patch_and_delete() {
    let mut store = MiniStore {
        namespaces: vec![
            serde_json::from_value(json!({
                "name": "indexed-writes",
                "documents": [
                    {"id": 1, "group": "keep", "score": 1},
                    {"id": 2, "group": "skip", "score": 2},
                    {"id": 3, "group": "keep", "score": 3}
                ]
            }))
            .unwrap(),
        ],
    };
    let first = query_store(
        &store,
        "indexed-writes",
        &json!({
            "rank_by": ["score", "asc"],
            "filters": ["group", "Eq", "keep"],
            "limit": 10
        }),
    )
    .unwrap();
    assert_eq!(
        rows(&first)
            .iter()
            .map(|row| row["id"].clone())
            .collect::<Vec<_>>(),
        vec![json!(1), json!(3)]
    );
    let namespace = store.namespace("indexed-writes").unwrap();
    assert_eq!(namespace.query_indexes.borrow().equality.len(), 1);
    assert_eq!(namespace.query_indexes.borrow().order.len(), 1);

    write_store(
        &mut store,
        "indexed-writes",
        &json!({
            "patch_rows": [
                {"id": 2, "group": "keep", "score": 0}
            ]
        }),
    )
    .unwrap();
    let patched = query_store(
        &store,
        "indexed-writes",
        &json!({
            "rank_by": ["score", "asc"],
            "filters": ["group", "Eq", "keep"],
            "limit": 10
        }),
    )
    .unwrap();
    assert_eq!(
        rows(&patched)
            .iter()
            .map(|row| row["id"].clone())
            .collect::<Vec<_>>(),
        vec![json!(2), json!(1), json!(3)]
    );
    let namespace = store.namespace("indexed-writes").unwrap();
    assert_eq!(namespace.query_indexes.borrow().equality.len(), 1);
    assert_eq!(namespace.query_indexes.borrow().order.len(), 1);

    write_store(
        &mut store,
        "indexed-writes",
        &json!({
            "upsert_rows": [
                {"id": 3, "group": "skip", "score": 9}
            ]
        }),
    )
    .unwrap();
    let replaced = query_store(
        &store,
        "indexed-writes",
        &json!({
            "rank_by": ["score", "asc"],
            "filters": ["group", "Eq", "keep"],
            "limit": 10
        }),
    )
    .unwrap();
    assert_eq!(
        rows(&replaced)
            .iter()
            .map(|row| row["id"].clone())
            .collect::<Vec<_>>(),
        vec![json!(2), json!(1)]
    );
    let namespace = store.namespace("indexed-writes").unwrap();
    assert_eq!(namespace.query_indexes.borrow().equality.len(), 1);
    assert_eq!(namespace.query_indexes.borrow().order.len(), 1);

    write_store(&mut store, "indexed-writes", &json!({"deletes": [1]})).unwrap();
    let deleted = query_store(
        &store,
        "indexed-writes",
        &json!({
            "rank_by": ["score", "asc"],
            "filters": ["group", "Eq", "keep"],
            "limit": 10
        }),
    )
    .unwrap();
    assert_eq!(
        rows(&deleted)
            .iter()
            .map(|row| row["id"].clone())
            .collect::<Vec<_>>(),
        vec![json!(2)]
    );
    let namespace = store.namespace("indexed-writes").unwrap();
    assert_eq!(namespace.query_indexes.borrow().equality.len(), 1);
    assert_eq!(namespace.query_indexes.borrow().order.len(), 1);

    write_store(
        &mut store,
        "indexed-writes",
        &json!({
            "upsert_rows": [
                {"id": 4, "group": "keep", "score": -1}
            ]
        }),
    )
    .unwrap();
    let upserted = query_store(
        &store,
        "indexed-writes",
        &json!({
            "rank_by": ["score", "asc"],
            "filters": ["group", "Eq", "keep"],
            "limit": 10
        }),
    )
    .unwrap();
    assert_eq!(
        rows(&upserted)
            .iter()
            .map(|row| row["id"].clone())
            .collect::<Vec<_>>(),
        vec![json!(4), json!(2)]
    );
    let namespace = store.namespace("indexed-writes").unwrap();
    assert_eq!(namespace.query_indexes.borrow().equality.len(), 1);
    assert_eq!(namespace.query_indexes.borrow().order.len(), 1);
}

#[test]
fn indexed_candidates_are_rechecked_by_filter_evaluator() {
    let namespace: Namespace = serde_json::from_value(json!({
        "name": "source-of-truth",
        "documents": [
            {"id": 1, "group": "keep", "score": 1},
            {"id": 2, "group": "skip", "score": 2}
        ]
    }))
    .unwrap();
    let first = query_namespace(
        &namespace,
        &json!({
            "rank_by": ["score", "asc"],
            "filters": ["group", "Eq", "keep"],
            "limit": 10
        }),
    )
    .unwrap();
    assert_eq!(rows(&first)[0]["id"], 1);

    namespace
        .query_indexes
        .borrow_mut()
        .equality
        .get_mut("group")
        .unwrap()
        .postings
        .entry(ScalarEqKey::String("keep".to_string()))
        .or_default()
        .insert(id_key(&json!(2)));

    let poisoned = query_namespace(
        &namespace,
        &json!({
            "rank_by": ["score", "asc"],
            "filters": ["group", "Eq", "keep"],
            "limit": 10
        }),
    )
    .unwrap();
    assert_eq!(
        rows(&poisoned)
            .iter()
            .map(|row| row["id"].clone())
            .collect::<Vec<_>>(),
        vec![json!(1)]
    );
}

#[test]
fn indexed_order_and_filters_track_by_filter_writes_and_edge_values() {
    let mut store = MiniStore {
        namespaces: vec![
            serde_json::from_value(json!({
                "name": "edge-indexes",
                "documents": [
                    {"id": 1, "group": "alpha", "score": 10, "tie": "b"},
                    {"id": 2, "group": "beta", "score": 2.0, "tie": "a", "optional": null},
                    {"id": "3", "group": "alpha", "score": 2, "tie": "c"},
                    {"id": "4", "group": "beta", "score": 4, "tie": "d", "optional": "present"}
                ]
            }))
            .unwrap(),
        ],
    };
    let first = query_store(
        &store,
        "edge-indexes",
        &json!({
            "rank_by": [["score", "asc"], ["tie", "asc"]],
            "filters": ["group", "Eq", "beta"],
            "limit": 10,
            "include_attributes": ["score", "tie", "optional"]
        }),
    )
    .unwrap();
    assert_eq!(
        rows(&first)
            .iter()
            .map(|row| row["id"].clone())
            .collect::<Vec<_>>(),
        vec![json!(2), json!("4")]
    );

    write_store(
        &mut store,
        "edge-indexes",
        &json!({
            "patch_by_filter": {
                "filters": ["optional", "Eq", null],
                "patch": {"group": "patched", "score": 1, "tie": "z"}
            }
        }),
    )
    .unwrap();
    let patched = query_store(
        &store,
        "edge-indexes",
        &json!({
            "rank_by": [["score", "asc"], ["tie", "asc"]],
            "filters": ["group", "Eq", "patched"],
            "limit": 10
        }),
    )
    .unwrap();
    assert_eq!(
        rows(&patched)
            .iter()
            .map(|row| row["id"].clone())
            .collect::<Vec<_>>(),
        vec![json!("3"), json!(1), json!(2)]
    );

    write_store(
        &mut store,
        "edge-indexes",
        &json!({"delete_by_filter": ["score", "Eq", 1.0]}),
    )
    .unwrap();
    let remaining = query_store(
        &store,
        "edge-indexes",
        &json!({
            "rank_by": [["score", "asc"], ["tie", "asc"]],
            "filters": ["group", "Eq", "patched"],
            "limit": 10
        }),
    )
    .unwrap();
    assert!(rows(&remaining).is_empty());
    let namespace = store.namespace("edge-indexes").unwrap();
    assert_eq!(namespace.query_indexes.borrow().equality.len(), 1);
    assert_eq!(namespace.query_indexes.borrow().order.len(), 1);
}

#[test]
fn copy_and_branch_start_with_empty_independent_query_indexes() {
    let mut clone = Micropuffer::from_store(MiniStore {
        namespaces: vec![namespace()],
    });
    clone
        .query(
            "demo",
            &json!({
                "rank_by": [["tenant_id", "asc"], ["score", "desc"]],
                "filters": ["tenant_id", "Eq", "alpha"],
                "limit": 10
            }),
        )
        .unwrap();
    assert!(
        !clone
            .store()
            .namespace("demo")
            .unwrap()
            .query_indexes
            .borrow()
            .is_empty()
    );

    clone
        .write("demo-copy-indexes", &json!({"copy_from_namespace": "demo"}))
        .unwrap();
    clone
        .write(
            "demo-branch-indexes",
            &json!({"branch_from_namespace": "demo"}),
        )
        .unwrap();

    assert!(
        clone
            .store()
            .namespace("demo-copy-indexes")
            .unwrap()
            .query_indexes
            .borrow()
            .is_empty()
    );
    assert!(
        clone
            .store()
            .namespace("demo-branch-indexes")
            .unwrap()
            .query_indexes
            .borrow()
            .is_empty()
    );

    let copied = clone
        .query(
            "demo-copy-indexes",
            &json!({
                "rank_by": [["tenant_id", "asc"], ["score", "desc"]],
                "filters": ["tenant_id", "Eq", "alpha"],
                "limit": 10
            }),
        )
        .unwrap();
    assert_eq!(
        rows(&copied)
            .iter()
            .map(|row| row["id"].clone())
            .collect::<Vec<_>>(),
        vec![json!(1), json!(3)]
    );
}

#[test]
fn complex_filters_fall_back_to_full_evaluator() {
    let namespace: Namespace = serde_json::from_value(json!({
        "name": "fallback",
        "documents": [
            {"id": 1, "tenant_id": "beta", "title": "dashboard"},
            {"id": 2, "tenant_id": "alpha", "title": "hybrid search"},
            {"id": 3, "tenant_id": "alpha", "title": "plain search"}
        ]
    }))
    .unwrap();
    let response = query_namespace(
        &namespace,
        &json!({
            "rank_by": ["id", "asc"],
            "filters": ["Or", [
                ["tenant_id", "Eq", "beta"],
                ["title", "Fuzzy", "hybryd"]
            ]],
            "limit": 10
        }),
    )
    .unwrap();
    assert_eq!(
        rows(&response)
            .iter()
            .map(|row| row["id"].clone())
            .collect::<Vec<_>>(),
        vec![json!(1), json!(2)]
    );
}

#[test]
fn writes_enforce_schema_types_and_vector_invariants() {
    let mut clone = Micropuffer::new();
    clone
        .write(
            "schema-demo",
            &json!({
                "schema": {
                    "vector": "[2]f32",
                    "title": "string",
                    "score": "int"
                },
                "upsert_rows": [
                    {"id": 1, "vector": [0.0, 1.0], "title": "ok", "score": 1}
                ]
            }),
        )
        .unwrap();

    let missing_vector = clone
        .write(
            "schema-demo",
            &json!({
                "upsert_rows": [
                    {"id": 2, "title": "missing vector", "score": 2}
                ]
            }),
        )
        .unwrap_err();
    assert!(
        missing_vector
            .to_string()
            .contains("missing required vector")
    );

    let wrong_dimension = clone
        .write(
            "schema-demo",
            &json!({
                "upsert_rows": [
                    {"id": 2, "vector": [1.0, 2.0, 3.0], "title": "wrong dimension", "score": 2}
                ]
            }),
        )
        .unwrap_err();
    assert!(wrong_dimension.to_string().contains("schema type"));

    let wrong_type = clone
        .write(
            "schema-demo",
            &json!({
                "upsert_rows": [
                    {"id": 2, "vector": [1.0, 0.0], "title": "wrong score", "score": "high"}
                ]
            }),
        )
        .unwrap_err();
    assert!(wrong_type.to_string().contains("score"));

    let patch_vector = clone
        .write(
            "schema-demo",
            &json!({
                "patch_rows": [
                    {"id": 1, "vector": [1.0, 0.0]}
                ]
            }),
        )
        .unwrap_err();
    assert!(patch_vector.to_string().contains("cannot be patched"));

    let patch_by_filter_vector = clone
        .write(
            "schema-demo",
            &json!({
                "patch_by_filter": {
                    "filters": ["id", "Eq", 1],
                    "patch": {"vector": [1.0, 0.0]}
                }
            }),
        )
        .unwrap_err();
    assert!(
        patch_by_filter_vector
            .to_string()
            .contains("cannot be patched")
    );
}

#[test]
fn write_responses_include_requested_zero_counts_and_query_billing() {
    let mut clone = Micropuffer::new();
    clone
        .write(
            "response-demo",
            &json!({
                "upsert_rows": [
                    {"id": 1, "title": "first", "score": 1}
                ]
            }),
        )
        .unwrap();
    let skipped = clone
        .write(
            "response-demo",
            &json!({
                "upsert_rows": [
                    {"id": 1, "title": "blocked", "score": 2}
                ],
                "upsert_condition": ["id", "Eq", null],
                "return_affected_ids": true
            }),
        )
        .unwrap();
    assert_eq!(skipped["rows_affected"], 0);
    assert_eq!(skipped["rows_upserted"], 0);
    assert!(skipped.get("rows_patched").is_none());
    assert!(
        skipped["billing"]["billable_logical_bytes_written"]
            .as_u64()
            .unwrap()
            > 0
    );
    assert!(
        skipped["billing"]["query"]["billable_logical_bytes_queried"]
            .as_u64()
            .unwrap()
            > 0
    );
    assert!(skipped.get("upserted_ids").is_none());
    let current = clone
        .query(
            "response-demo",
            &json!({"rank_by": ["id", "asc"], "limit": 1, "include_attributes": true}),
        )
        .unwrap();
    assert_eq!(rows(&current)[0]["title"], "first");
}

#[test]
fn repeated_query_and_metadata_reuse_cached_logical_bytes() {
    let namespace = namespace();

    assert_eq!(namespace.logical_bytes_recompute_count(), 0);
    assert!(!namespace.has_cached_logical_bytes());

    let first_query =
        query_namespace(&namespace, &json!({"rank_by": ["id", "asc"], "limit": 1})).unwrap();
    let first_bytes = first_query["billing"]["billable_logical_bytes_queried"]
        .as_u64()
        .unwrap();
    assert!(first_bytes > 0);
    assert_eq!(namespace.logical_bytes_recompute_count(), 1);
    assert!(namespace.has_cached_logical_bytes());

    let metadata = namespace_metadata(&namespace).unwrap();
    assert_eq!(metadata["approx_logical_bytes"], first_bytes);

    let second_query =
        query_namespace(&namespace, &json!({"rank_by": ["id", "asc"], "limit": 1})).unwrap();
    assert_eq!(
        second_query["billing"]["billable_logical_bytes_queried"],
        first_query["billing"]["billable_logical_bytes_queried"]
    );
    assert_eq!(namespace.logical_bytes_recompute_count(), 1);
}

#[test]
fn logical_bytes_cache_is_not_serialized_with_namespace() {
    let namespace = namespace();
    namespace_metadata(&namespace).unwrap();

    let serialized = serde_json::to_value(&namespace).unwrap();

    assert!(serialized.get("logical_bytes_cache").is_none());
    assert!(serialized.get("documents").is_some());
}

#[test]
fn write_invalidates_cached_logical_bytes_and_next_metadata_recomputes() {
    let mut store = MiniStore::default();
    write_store(
        &mut store,
        "metrics-cache",
        &json!({"upsert_rows": [{"id": 1, "title": "first"}]}),
    )
    .unwrap();

    let first_query = query_store(
        &store,
        "metrics-cache",
        &json!({"rank_by": ["id", "asc"], "limit": 1}),
    )
    .unwrap();
    let first_bytes = first_query["billing"]["billable_logical_bytes_queried"]
        .as_u64()
        .unwrap();
    let namespace = store.namespace("metrics-cache").unwrap();
    assert_eq!(namespace.logical_bytes_recompute_count(), 1);
    assert!(namespace.has_cached_logical_bytes());

    write_store(
        &mut store,
        "metrics-cache",
        &json!({
            "patch_rows": [
                {"id": 1, "payload": "the cached byte count should not survive this write"}
            ]
        }),
    )
    .unwrap();
    let namespace = store.namespace("metrics-cache").unwrap();
    assert_eq!(namespace.logical_bytes_recompute_count(), 1);
    assert!(!namespace.has_cached_logical_bytes());

    let metadata = namespace_metadata(namespace).unwrap();
    let recomputed_bytes = metadata["approx_logical_bytes"].as_u64().unwrap();
    assert!(recomputed_bytes > first_bytes);
    let namespace = store.namespace("metrics-cache").unwrap();
    assert_eq!(namespace.logical_bytes_recompute_count(), 2);
    assert!(namespace.has_cached_logical_bytes());
}

#[test]
fn copy_replaces_existing_empty_namespace_logical_bytes_cache() {
    let mut store = MiniStore::default();
    write_store(
        &mut store,
        "source",
        &json!({"upsert_rows": [{"id": 1, "title": "copied"}]}),
    )
    .unwrap();
    write_store(
        &mut store,
        "destination",
        &json!({"schema": {"title": "string"}}),
    )
    .unwrap();

    let empty_metadata = namespace_metadata(store.namespace("destination").unwrap()).unwrap();
    assert_eq!(empty_metadata["approx_logical_bytes"], 0);
    assert!(
        store
            .namespace("destination")
            .unwrap()
            .has_cached_logical_bytes()
    );

    write_store(
        &mut store,
        "destination",
        &json!({"copy_from_namespace": "source"}),
    )
    .unwrap();

    let destination_metadata = namespace_metadata(store.namespace("destination").unwrap()).unwrap();
    assert_eq!(destination_metadata["approx_row_count"], 1);
    assert!(
        destination_metadata["approx_logical_bytes"]
            .as_u64()
            .unwrap()
            > 0
    );
}

#[test]
fn patch_by_filter_respects_partial_limit_and_rows_remaining() {
    let mut documents = Vec::with_capacity(PATCH_BY_FILTER_LIMIT + 1);
    for index in 0..=PATCH_BY_FILTER_LIMIT {
        documents.push(Document {
            id: Value::Number(Number::from(index as u64)),
            attributes: Map::from_iter([
                ("group".to_string(), Value::String("all".to_string())),
                ("patched".to_string(), Value::Bool(false)),
            ]),
        });
    }
    let mut clone = Micropuffer::from_store(MiniStore {
        namespaces: vec![Namespace {
            name: "partial".to_string(),
            distance_metric: DistanceMetric::default(),
            schema: Map::from_iter([
                ("group".to_string(), json!("string")),
                ("patched".to_string(), json!("bool")),
            ]),
            created_at: default_created_at(),
            last_write_at: None,
            updated_at: default_created_at(),
            encryption: default_encryption(),
            pinning: None,
            branching_parent: None,
            documents,
            query_indexes: Default::default(),
            logical_bytes_cache: Default::default(),
        }],
    });
    let too_many = clone
        .write(
            "partial",
            &json!({
                "patch_by_filter": {
                    "filters": ["group", "Eq", "all"],
                    "patch": {"patched": true}
                }
            }),
        )
        .unwrap_err();
    assert!(too_many.to_string().contains("more documents than allowed"));

    let partial = clone
        .write(
            "partial",
            &json!({
                "patch_by_filter_allow_partial": true,
                "patch_by_filter": {
                    "filters": ["group", "Eq", "all"],
                    "patch": {"patched": true}
                },
                "return_affected_ids": true
            }),
        )
        .unwrap();
    assert_eq!(partial["rows_affected"], PATCH_BY_FILTER_LIMIT);
    assert_eq!(partial["rows_patched"], PATCH_BY_FILTER_LIMIT);
    assert_eq!(partial["rows_remaining"], true);
    assert_eq!(
        partial["patched_ids"].as_array().unwrap().len(),
        PATCH_BY_FILTER_LIMIT
    );
    assert!(
        partial["billing"]["query"]["billable_logical_bytes_queried"]
            .as_u64()
            .unwrap()
            > 0
    );
}

#[test]
fn schema_rejects_type_changes_and_too_many_vector_columns() {
    let mut clone = Micropuffer::new();
    clone
        .write(
            "schema-change",
            &json!({
                "schema": {"title": "string"},
                "upsert_rows": [
                    {"id": 1, "title": "hello"}
                ]
            }),
        )
        .unwrap();
    let type_change = clone
        .write("schema-change", &json!({"schema": {"title": "int"}}))
        .unwrap_err();
    assert!(type_change.to_string().contains("Changing the type"));

    let vector_count = clone
        .write(
            "too-many-vectors",
            &json!({
                "schema": {
                    "vector": "[2]f32",
                    "image_vector": "[2]f32",
                    "audio_vector": "[2]f32"
                }
            }),
        )
        .unwrap_err();
    assert!(vector_count.to_string().contains("up to 2 vector columns"));
}

#[test]
fn micropuffer_lists_copies_and_deletes_namespaces() {
    let mut clone = Micropuffer::from_store(MiniStore {
        namespaces: vec![namespace()],
    });
    clone
        .write("demo-copy", &json!({"copy_from_namespace": "demo"}))
        .unwrap();
    let listed = clone.list_namespaces(Some("demo"), None, 10).unwrap();
    let namespaces = listed.get("namespaces").and_then(Value::as_array).unwrap();
    assert_eq!(namespaces.len(), 2);
    let copied = clone
        .query(
            "demo-copy",
            &json!({"aggregate_by": {"count": ["Count"]}, "limit": 1}),
        )
        .unwrap();
    assert_eq!(copied["aggregations"]["count"], 3);
    clone.delete_namespace("demo-copy").unwrap();
    assert!(
        clone
            .query("demo-copy", &json!({"rank_by": ["id", "asc"], "limit": 1}))
            .is_err()
    );
}

#[test]
fn copy_can_override_encryption_but_not_mix_with_writes() {
    let mut clone = Micropuffer::from_store(MiniStore {
        namespaces: vec![namespace()],
    });
    clone
        .write(
            "encrypted-copy",
            &json!({
                "copy_from_namespace": {
                    "source_namespace": "demo",
                    "source_api_key": "tpuf_test",
                    "source_region": "aws-us-east-1"
                },
                "encryption": {"mode": "aws:cmk", "key": "test-key"}
            }),
        )
        .unwrap();
    let metadata = clone.metadata("encrypted-copy").unwrap();
    assert_eq!(metadata["encryption"]["mode"], "aws:cmk");
    assert_eq!(metadata["encryption"]["key"], "test-key");

    let invalid = clone
        .write(
            "bad-copy",
            &json!({
                "copy_from_namespace": "demo",
                "upsert_rows": [{"id": 9, "vector": [0.0, 0.0, 0.0]}]
            }),
        )
        .unwrap_err();
    assert!(
        invalid
            .to_string()
            .contains("copy_from_namespace cannot be combined")
    );
}

#[test]
fn metadata_patch_and_export_match_documented_workspace_shape() {
    let mut clone = Micropuffer::new();
    clone
        .write(
            "workspace",
            &json!({
                "distance_metric": "cosine_distance",
                "schema": {
                    "title": {"type": "string", "full_text_search": true},
                    "published_at": "datetime"
                },
                "upsert_rows": [
                    {"id": 1, "vector": [1.0, 0.0], "title": "alpha", "published_at": "2026-05-31T00:00:00Z", "score": 2},
                    {"id": 2, "vector": [0.0, 1.0], "title": "beta", "published_at": "2026-05-30T00:00:00Z", "score": 3}
                ]
            }),
        )
        .unwrap();
    let metadata = clone.metadata("workspace").unwrap();
    assert_eq!(metadata["approx_row_count"], 2);
    assert_eq!(metadata["schema"]["title"]["type"], "string");
    assert_eq!(metadata["schema"]["published_at"], "datetime");
    assert_eq!(metadata["schema"]["vector"], "[2]f32");
    assert_eq!(metadata["index"]["status"], "up-to-date");
    assert!(metadata.get("last_write_at").is_some());

    let pinned = clone
        .patch_metadata("workspace", &json!({"pinning": {"replicas": 2}}))
        .unwrap();
    assert_eq!(pinned["pinning"]["replicas"], 2);
    assert_eq!(pinned["pinning"]["status"]["ready_replicas"], 2);
    let unpinned = clone
        .patch_metadata("workspace", &json!({"pinning": null}))
        .unwrap();
    assert!(unpinned.get("pinning").is_none());

    let export = clone
        .export_namespace(
            "workspace",
            &json!({
                "filters": ["id", "Gt", 1],
                "limit": 10,
                "include_attributes": ["title", "score"]
            }),
        )
        .unwrap();
    assert_eq!(rows(&export).len(), 1);
    assert_eq!(rows(&export)[0]["id"], 2);
    assert!(rows(&export)[0].get("$dist").is_none());
}

#[test]
fn schema_update_and_warm_cache_match_workspace_shapes() {
    let mut clone = Micropuffer::new();
    clone
        .write(
            "schema-api",
            &json!({
                "upsert_rows": [
                    {"id": 1, "vector": [0.0, 0.0], "title": "hello"}
                ]
            }),
        )
        .unwrap();
    let schema = clone.schema("schema-api").unwrap();
    assert_eq!(schema["id"]["type"], "uint");
    assert_eq!(schema["vector"]["type"], "[2]f32");
    assert_eq!(schema["title"]["type"], "string");

    let updated = clone
        .update_schema(
            "schema-api",
            &json!({
                "title": {
                    "type": "string",
                    "full_text_search": {
                        "tokenizer": "word_v3",
                        "language": "english",
                        "stemming": true
                    },
                    "regex": true
                }
            }),
        )
        .unwrap();
    assert_eq!(updated["title"]["full_text_search"]["stemming"], true);
    assert_eq!(updated["title"]["regex"], true);

    let warmed = clone.warm_cache("schema-api").unwrap();
    assert_eq!(warmed["status"], "ACCEPTED");
}

#[test]
fn fts_options_apply_language_stopwords_and_tokenizer_modes() {
    let config = parse_fts_config(&json!({
        "language": "spanish",
        "stemming": true,
        "remove_stopwords": true
    }))
    .unwrap();
    let tokens = tokenize("los gatos rápidos corriendo", &config);
    assert!(!tokens.contains(&"los".to_string()));
    assert!(tokens.iter().any(|token| token.starts_with("gat")));

    let v0 = parse_fts_config(&json!({"tokenizer": "word_v0", "remove_stopwords": false})).unwrap();
    let v1 = parse_fts_config(&json!({"tokenizer": "word_v1", "remove_stopwords": false})).unwrap();
    let v2 = parse_fts_config(&json!({"tokenizer": "word_v2", "remove_stopwords": false})).unwrap();
    let v3 = parse_fts_config(&json!({"tokenizer": "word_v3", "remove_stopwords": false})).unwrap();
    assert_eq!(tokenize("東京abc🙂", &v0), vec!["東京abc"]);
    assert_eq!(tokenize("東京abc🙂", &v1), vec!["東京abc", "🙂"]);
    assert_eq!(tokenize("東京abc🙂", &v2), vec!["東", "京", "abc", "🙂"]);
    assert_eq!(tokenize("東京abc🙂", &v3), vec!["東", "京", "abc"]);
}

#[test]
fn pre_tokenized_fields_require_array_queries() {
    let mut clone = Micropuffer::new();
    clone
        .write(
            "pretokenized",
            &json!({
                "schema": {
                    "tokens": {
                        "type": "[]string",
                        "full_text_search": {"tokenizer": "pre_tokenized_array"}
                    }
                },
                "upsert_rows": [
                    {"id": 1, "tokens": ["Foo", "Bar"]}
                ]
            }),
        )
        .unwrap();
    let hit = clone
        .query(
            "pretokenized",
            &json!({"rank_by": ["tokens", "BM25", ["Foo"]], "limit": 10}),
        )
        .unwrap();
    assert_eq!(rows(&hit)[0]["id"], 1);
    let string_query = clone
        .query(
            "pretokenized",
            &json!({"rank_by": ["tokens", "BM25", "Foo"], "limit": 10}),
        )
        .unwrap_err();
    assert!(string_query.to_string().contains("array of strings"));

    let invalid = clone
        .write(
            "bad-pretokenized",
            &json!({
                "schema": {
                    "tokens": {
                        "type": "[]string",
                        "full_text_search": {
                            "tokenizer": "pre_tokenized_array",
                            "stemming": true
                        }
                    }
                }
            }),
        )
        .unwrap_err();
    assert!(invalid.to_string().contains("pre_tokenized_array"));
}

#[test]
fn recall_and_explain_query_match_debug_endpoint_shapes() {
    let mut clone = Micropuffer::new();
    clone
        .write(
            "debug",
            &json!({
                "distance_metric": "cosine_distance",
                "schema": {"text": {"type": "string", "full_text_search": true}},
                "upsert_rows": [
                    {"id": 1, "vector": [1.0, 0.0], "text": "walrus mammal", "public": 1},
                    {"id": 2, "vector": [0.0, 1.0], "text": "reef fish", "public": 0},
                    {"id": 3, "vector": [0.9, 0.1], "text": "arctic mammal", "public": 1}
                ]
            }),
        )
        .unwrap();

    let recall = clone
        .recall(
            "debug",
            &json!({
                "num": 2,
                "top_k": 2,
                "filters": ["public", "Eq", 1],
                "include_ground_truth": true
            }),
        )
        .unwrap();
    assert_eq!(recall["avg_recall"], 1.0);
    assert_eq!(recall["avg_exhaustive_count"], 2.0);
    assert_eq!(recall["avg_ann_count"], 2.0);
    assert_eq!(
        recall["ground_truth"]
            .as_array()
            .expect("ground truth array")
            .len(),
        2
    );

    let explained = clone
        .explain_query(
            "debug",
            &json!({
                "rank_by": ["text", "BM25", "mammal"],
                "filters": ["public", "Eq", 1],
                "limit": 10
            }),
        )
        .unwrap();
    assert!(
        explained["plan_text"]
            .as_str()
            .expect("plan text")
            .contains("operation=query")
    );
}

#[test]
fn branch_metadata_records_parent_namespace() {
    let mut clone = Micropuffer::from_store(MiniStore {
        namespaces: vec![namespace()],
    });
    clone
        .write("demo-branch", &json!({"branch_from_namespace": "demo"}))
        .unwrap();
    let metadata = clone.metadata("demo-branch").unwrap();
    assert_eq!(metadata["branching"]["parent"], "demo");

    let invalid_extra_field = clone
        .write(
            "bad-branch",
            &json!({
                "branch_from_namespace": "demo",
                "encryption": {"mode": "aws:cmk"}
            }),
        )
        .unwrap_err();
    assert!(
        invalid_extra_field
            .to_string()
            .contains("branch_from_namespace cannot be combined")
    );

    let invalid_source_shape = clone
        .write(
            "bad-branch",
            &json!({"branch_from_namespace": {"source_namespace": "demo"}}),
        )
        .unwrap_err();
    assert!(
        invalid_source_shape
            .to_string()
            .contains("branch_from_namespace must be a string")
    );
}

#[test]
fn write_id_index_preserves_numeric_id_matching_for_loaded_stores() {
    let namespace: Namespace =
        serde_json::from_str(r#"{"name":"loaded","documents":[{"id":1.0,"score":1}]}"#).unwrap();
    let mut store = MiniStore {
        namespaces: vec![namespace],
    };
    let response = write_store(
        &mut store,
        "loaded",
        &json!({
            "patch_rows": [{"id": 1, "score": 2}],
            "return_affected_ids": true
        }),
    )
    .unwrap();

    assert_eq!(response["rows_patched"], 1);
    assert_eq!(response["patched_ids"], json!([1]));
    assert_eq!(
        store.namespace("loaded").unwrap().documents[0].attributes["score"],
        2
    );
}

#[test]
fn write_id_index_keeps_large_u64_ids_distinct() {
    let first_id = 9_007_199_254_740_992_u64;
    let second_id = 9_007_199_254_740_993_u64;
    let mut store = MiniStore::default();
    let upsert = write_store(
        &mut store,
        "large-ids",
        &json!({
            "upsert_rows": [
                {"id": first_id, "score": 1},
                {"id": second_id, "score": 2}
            ]
        }),
    )
    .unwrap();

    assert_eq!(upsert["rows_upserted"], 2);
    assert_eq!(store.namespace("large-ids").unwrap().documents.len(), 2);

    let patch = write_store(
        &mut store,
        "large-ids",
        &json!({
            "patch_rows": [
                {"id": first_id, "score": 10},
                {"id": second_id, "score": 20}
            ]
        }),
    )
    .unwrap();
    assert_eq!(patch["rows_patched"], 2);

    let response = query_store(
        &store,
        "large-ids",
        &json!({
            "rank_by": ["score", "asc"],
            "limit": 2,
            "include_attributes": true
        }),
    )
    .unwrap();
    let rows = rows(&response);
    assert_eq!(rows[0]["id"], first_id);
    assert_eq!(rows[0]["score"], 10);
    assert_eq!(rows[1]["id"], second_id);
    assert_eq!(rows[1]["score"], 20);
}

#[test]
#[ignore = "performance evidence; run explicitly"]
fn write_100k_new_upserts_completes_in_one_batch() {
    let mut rows = Vec::with_capacity(100_000);
    for id in 0..100_000_u64 {
        let mut row = Map::new();
        row.insert("id".to_string(), Value::Number(Number::from(id)));
        row.insert("vector".to_string(), json!([0.0, 1.0]));
        row.insert("score".to_string(), Value::Number(Number::from(id)));
        rows.push(Value::Object(row));
    }
    let mut store = MiniStore::default();
    let started = std::time::Instant::now();
    let response = write_store(
        &mut store,
        "bulk-upsert",
        &json!({
            "upsert_rows": rows
        }),
    )
    .unwrap();
    let elapsed = started.elapsed();

    println!("100k new upserts elapsed: {elapsed:?}");
    assert_eq!(response["rows_upserted"], 100_000);
    assert_eq!(
        store.namespace("bulk-upsert").unwrap().documents.len(),
        100_000
    );
}
