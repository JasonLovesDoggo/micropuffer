use crate::{
    QueryError,
    core::{
        DistanceMetric, Document, Micropuffer, MiniStore, Namespace, PATCH_BY_FILTER_LIMIT,
        ScalarEqKey, default_created_at, default_encryption, namespace_metadata, parse_fts_config,
        query_namespace, query_store, tokenize, write_store,
    },
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
fn limit_per_diversifies_attribute_order_and_rejects_other_rankers() {
    let response = query_namespace(
        &namespace(),
        &json!({
            "rank_by": ["id", "asc"],
            "limit": {
                "total": 3,
                "per": {"attributes": ["tenant_id"], "limit": 1}
            },
            "include_attributes": ["tenant_id"]
        }),
    )
    .unwrap();

    assert_eq!(
        rows(&response)
            .iter()
            .map(|row| json!({"id": row["id"], "tenant_id": row["tenant_id"]}))
            .collect::<Vec<_>>(),
        vec![
            json!({"id": 1, "tenant_id": "alpha"}),
            json!({"id": 2, "tenant_id": "beta"})
        ]
    );

    let error = query_namespace(
        &namespace(),
        &json!({
            "rank_by": ["body", "BM25", "rust"],
            "limit": {
                "total": 3,
                "per": {"attributes": ["tenant_id"], "limit": 1}
            }
        }),
    )
    .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("💔 `limit.per` is only supported when ranking by an attribute")
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
fn non_bm25_rank_plans_execute_without_fts_indexes() {
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
        query_namespace(&namespace, &query).unwrap();
    }
}

#[test]
fn bm25_and_rank_operators_score_higher_matches_first() {
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

    let token_array = query_namespace(
        &namespace(),
        &json!({
            "rank_by": ["body", "BM25", ["rust", "hybrid"]],
            "limit": 3
        }),
    )
    .unwrap_err();
    assert_eq!(
        token_array.to_string(),
        "💔 invalid input '[rust, hybrid]' for rank_by field \"body\", expecting string"
    );
}

#[test]
fn nested_bm25_rank_operators_use_indexed_scores() {
    let namespace: Namespace = serde_json::from_value(json!({
        "name": "nested-bm25",
        "schema": {
            "text": {"type": "string", "full_text_search": true},
            "title": {"type": "string", "full_text_search": true}
        },
        "documents": [
            {"id": 1, "title": "walrus guide", "text": "walrus walrus arctic mammal", "boost": 1},
            {"id": 2, "title": "reef notes", "text": "reef coral fish", "boost": 100},
            {"id": 3, "title": "arctic field report", "text": "arctic mammal migration", "boost": 2}
        ]
    }))
    .unwrap();

    let product = query_namespace(
        &namespace,
        &json!({
            "rank_by": ["Product", 2, ["text", "BM25", "walrus arctic"]],
            "limit": 3
        }),
    )
    .unwrap();
    assert_eq!(rows(&product)[0]["id"], 1);

    let sum = query_namespace(
        &namespace,
        &json!({
            "rank_by": ["Sum", [
                ["text", "BM25", "walrus"],
                ["title", "BM25", "field"]
            ]],
            "limit": 3
        }),
    )
    .unwrap();
    assert_eq!(rows(&sum)[0]["id"], 1);
    assert_eq!(rows(&sum)[1]["id"], 3);

    let max = query_namespace(
        &namespace,
        &json!({
            "rank_by": ["Max",
                ["text", "BM25", "reef"],
                ["title", "BM25", "guide"]
            ],
            "limit": 3
        }),
    )
    .unwrap();
    assert_eq!(rows(&max)[0]["id"], 1);
    assert_eq!(rows(&max)[1]["id"], 2);

    let saturate = query_namespace(
        &namespace,
        &json!({
            "rank_by": ["Saturate", ["text", "BM25", "walrus"], {"midpoint": 0.1}],
            "limit": 3
        }),
    )
    .unwrap();
    assert_eq!(rows(&saturate)[0]["id"], 1);

    let simple = query_namespace(
        &namespace,
        &json!({
            "rank_by": ["text", "BM25", "walrus arctic"],
            "limit": 1
        }),
    )
    .unwrap();
    let origin = rows(&simple)[0]["$dist"].as_f64().unwrap();
    let decay = query_namespace(
        &namespace,
        &json!({
            "rank_by": ["Decay", ["Dist", ["text", "BM25", "walrus arctic"], origin], {"midpoint": 0.01}],
            "limit": 3
        }),
    )
    .unwrap();
    assert_eq!(rows(&decay)[0]["id"], 1);
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
fn sparse_knn_index_updates_after_writes() {
    let mut clone = Micropuffer::new();
    clone
        .write(
            "sparse-index",
            &json!({
                "upsert_rows": [
                    {"id": 1, "sparse_vector": {"a": 1.0}},
                    {"id": 2, "sparse_vector": {"a": 0.5}},
                    {"id": 3, "sparse_vector": {"b": 1.0}}
                ]
            }),
        )
        .unwrap();

    let first = clone
        .query(
            "sparse-index",
            &json!({
                "rank_by": ["sparse_vector", "SparseKNN", {"a": 1.0}],
                "limit": 10
            }),
        )
        .unwrap();
    assert_eq!(
        rows(&first)
            .iter()
            .map(|row| row["id"].clone())
            .collect::<Vec<_>>(),
        vec![json!(1), json!(2)]
    );

    clone
        .write(
            "sparse-index",
            &json!({
                "upsert_rows": [
                    {"id": 1, "sparse_vector": {"b": 1.0}},
                    {"id": 4, "sparse_vector": {"a": 2.0}}
                ]
            }),
        )
        .unwrap();
    let updated = clone
        .query(
            "sparse-index",
            &json!({
                "rank_by": ["sparse_vector", "SparseKNN", {"a": 1.0}],
                "limit": 10
            }),
        )
        .unwrap();
    assert_eq!(
        rows(&updated)
            .iter()
            .map(|row| row["id"].clone())
            .collect::<Vec<_>>(),
        vec![json!(4), json!(2)]
    );
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
            .guard()
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
    assert_eq!(namespace.query_indexes.guard().order.len(), 1);
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
            "distance_metric": "cosine_distance",
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
fn indexed_bm25_updates_after_writes_and_supports_prefix_queries() {
    let mut clone = Micropuffer::new();
    clone
        .write(
            "fts-index",
            &json!({
                "schema": {
                    "text": {
                        "type": "string",
                        "full_text_search": true
                    }
                },
                "upsert_rows": [
                    {"id": 1, "text": "walrus arctic mammal"},
                    {"id": 2, "text": "reef coral fish"}
                ]
            }),
        )
        .unwrap();

    let first = clone
        .query(
            "fts-index",
            &json!({
                "rank_by": ["text", "BM25", "walrus"],
                "limit": 10
            }),
        )
        .unwrap();
    assert_eq!(rows(&first)[0]["id"], 1);

    clone
        .write(
            "fts-index",
            &json!({
                "patch_rows": [
                    {"id": 1, "text": "reef coral fish"},
                    {"id": 2, "text": "walrus arctic mammal"}
                ]
            }),
        )
        .unwrap();

    let updated = clone
        .query(
            "fts-index",
            &json!({
                "rank_by": ["text", "BM25", "walrus"],
                "limit": 10
            }),
        )
        .unwrap();
    assert_eq!(rows(&updated)[0]["id"], 2);

    let prefix = clone
        .query(
            "fts-index",
            &json!({
                "rank_by": ["Product", 2, ["text", "BM25", "arct", {"last_as_prefix": true}]],
                "limit": 10
            }),
        )
        .unwrap();
    assert_eq!(rows(&prefix)[0]["id"], 2);

    let nested_after_write = clone
        .query(
            "fts-index",
            &json!({
                "rank_by": ["Sum", [
                    ["text", "BM25", "walrus"],
                    ["text", "BM25", "arctic"]
                ]],
                "limit": 10
            }),
        )
        .unwrap();
    assert_eq!(rows(&nested_after_write)[0]["id"], 2);
}

#[test]
fn aggregates_and_grouped_aggregates_apply_filters() {
    let response = query_namespace(
        &namespace(),
        &json!({
            "aggregate_by": {"count": ["Count"]},
            "filters": ["public", "Eq", true]
        }),
    )
    .unwrap();
    assert_eq!(response["aggregations"]["count"], 2);

    let sum = query_namespace(
        &namespace(),
        &json!({
            "aggregate_by": {"score_sum": ["Sum", "score"]},
            "filters": ["public", "Eq", true]
        }),
    )
    .unwrap();
    assert_eq!(sum["aggregations"]["score_sum"], 17.0);

    let multi_error = query_namespace(
        &namespace(),
        &json!({
            "aggregate_by": {"count": ["Count"], "score_sum": ["Sum", "score"]}
        }),
    )
    .unwrap_err();
    assert!(
        multi_error
            .to_string()
            .contains("💔 aggregate_by currently requires exactly one function")
    );

    let top_k_error = query_namespace(
        &namespace(),
        &json!({
            "aggregate_by": {"count": ["Count"]},
            "top_k": 10
        }),
    )
    .unwrap_err();
    assert!(
        top_k_error
            .to_string()
            .contains("💔 top_k is not supported in aggregation queries without group_by")
    );

    let limit_error = query_namespace(
        &namespace(),
        &json!({
            "aggregate_by": {"count": ["Count"]},
            "limit": 10
        }),
    )
    .unwrap_err();
    assert!(limit_error.to_string().contains("unknown field `limit`"));

    let grouped = query_namespace(
        &namespace(),
        &json!({
            "aggregate_by": {"count": ["Count"]},
            "group_by": ["tenant_id", {"tag": ["ForEachUnique", "tags"]}]
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

    let grouped_sum = query_namespace(
        &namespace(),
        &json!({
            "aggregate_by": {"score_sum": ["Sum", "score"]},
            "group_by": ["tenant_id"],
            "top_k": 10
        }),
    )
    .unwrap();
    let sum_groups = grouped_sum
        .get("aggregation_groups")
        .and_then(Value::as_array)
        .unwrap();
    assert!(
        sum_groups
            .iter()
            .any(|group| group["tenant_id"] == "alpha" && group["score_sum"] == 17.0)
    );
}

#[test]
fn scalar_grouped_count_preserves_sorted_group_order() {
    let grouped = query_namespace(
        &namespace(),
        &json!({
            "aggregate_by": {"count": ["Count"]},
            "group_by": ["tenant_id"],
            "top_k": 10
        }),
    )
    .unwrap();
    let groups = grouped
        .get("aggregation_groups")
        .and_then(Value::as_array)
        .unwrap();
    assert_eq!(
        groups
            .iter()
            .map(|group| json!({"tenant_id": group["tenant_id"], "count": group["count"]}))
            .collect::<Vec<_>>(),
        vec![
            json!({"tenant_id": "alpha", "count": 2}),
            json!({"tenant_id": "beta", "count": 1})
        ]
    );
}

#[test]
fn count_id_aggregate_matches_live_deprecated_variant() {
    let counted = query_namespace(
        &namespace(),
        &json!({
            "aggregate_by": {"count": ["Count", "id"]}
        }),
    )
    .unwrap();
    assert_eq!(counted["aggregations"]["count"], 3);

    let grouped = query_namespace(
        &namespace(),
        &json!({
            "aggregate_by": {"count": ["Count", "id"]},
            "group_by": ["tenant_id"],
            "top_k": 10
        }),
    )
    .unwrap();
    let groups = grouped
        .get("aggregation_groups")
        .and_then(Value::as_array)
        .unwrap();
    assert!(
        groups
            .iter()
            .any(|group| group["tenant_id"] == "alpha" && group["count"] == 2)
    );

    let unsupported = query_namespace(
        &namespace(),
        &json!({
            "aggregate_by": {"count": ["Count", "score"]}
        }),
    )
    .unwrap_err();
    assert_eq!(
        unsupported.to_string(),
        "💔 aggregate_by with attributes other than \"id\" not yet supported"
    );
}

#[test]
fn multi_query_preserves_result_order() {
    let response = query_namespace(
        &namespace(),
        &json!({
            "queries": [
                {"rank_by": ["vector", "ANN", [0.0, 0.0]], "limit": 1},
                {"aggregate_by": {"count": ["Count"]}}
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
fn vector_attributes_remain_visible_to_generic_filters() {
    let cases = [
        (json!(["vector", "Eq", [1.0, 1.0]]), vec![2]),
        (json!(["vector", "NotEq", [0.0, 0.0]]), vec![2, 3]),
        (
            json!(["vector", "In", [[0.0, 0.0], [1.0, 1.0]]]),
            vec![1, 2],
        ),
        (
            json!(["vector", "NotIn", [[0.0, 0.0], [2.0, 2.0]]]),
            vec![2],
        ),
        (json!(["vector", "Contains", 1.0]), vec![2]),
        (json!(["vector", "NotContains", 9.0]), vec![1, 2, 3]),
        (json!(["vector", "ContainsAny", [9.0, 1.0]]), vec![2]),
        (
            json!(["vector", "NotContainsAny", [9.0, 8.0]]),
            vec![1, 2, 3],
        ),
    ];

    for (filter, expected_ids) in cases {
        let response = query_namespace(
            &namespace(),
            &json!({
                "rank_by": ["id", "asc"],
                "filters": filter,
                "limit": 10,
                "include_attributes": ["vector"]
            }),
        )
        .unwrap();
        assert_eq!(
            rows(&response)
                .iter()
                .map(|row| row["id"].as_i64().unwrap())
                .collect::<Vec<_>>(),
            expected_ids
        );
        for row in rows(&response) {
            assert!(row.get("vector").is_some());
        }
    }
}

#[test]
fn vector_attributes_remain_visible_to_write_conditions() {
    let mut clone = Micropuffer::new();
    clone
        .write(
            "vector-conditions",
            &json!({
                "distance_metric": "cosine_distance",
                "upsert_rows": [
                    {"id": 1, "vector": [1.0, 0.0], "title": "original"},
                    {"id": 2, "vector": [2.0, 0.0], "title": "delete-me"}
                ]
            }),
        )
        .unwrap();

    let blocked_same_vector = clone
        .write(
            "vector-conditions",
            &json!({
                "upsert_rows": [
                    {"id": 1, "vector": [1.0, 0.0], "title": "should not update"}
                ],
                "upsert_condition": ["vector", "NotEq", {"$ref_new": "vector"}]
            }),
        )
        .unwrap();
    assert_eq!(blocked_same_vector["rows_affected"], 0);

    let changed_vector = clone
        .write(
            "vector-conditions",
            &json!({
                "upsert_rows": [
                    {"id": 1, "vector": [0.0, 1.0], "title": "updated"}
                ],
                "upsert_condition": ["vector", "NotEq", {"$ref_new": "vector"}]
            }),
        )
        .unwrap();
    assert_eq!(changed_vector["rows_affected"], 1);

    let deleted = clone
        .write(
            "vector-conditions",
            &json!({
                "deletes": [2],
                "delete_condition": ["vector", "Eq", [2.0, 0.0]]
            }),
        )
        .unwrap();
    assert_eq!(deleted["rows_affected"], 1);

    let response = clone
        .query(
            "vector-conditions",
            &json!({
                "rank_by": ["id", "asc"],
                "limit": 10,
                "include_attributes": true
            }),
        )
        .unwrap();
    assert_eq!(
        rows(&response),
        &[json!({"id": 1, "vector": [0.0, 1.0], "title": "updated"})]
    );
}

#[test]
fn vector_projection_export_and_roundtrip_keep_base64_and_typed_cache() {
    let mut clone = Micropuffer::new();
    clone
        .write(
            "vector-projection",
            &json!({
                "distance_metric": "cosine_distance",
                "upsert_rows": [
                    {"id": 1, "vector": [1.0, 0.0], "title": "one"},
                    {"id": 2, "vector": [0.0, 1.0], "title": "two"}
                ]
            }),
        )
        .unwrap();
    let expected_vector = STANDARD.encode([1.0_f32.to_le_bytes(), 0.0_f32.to_le_bytes()].concat());

    let query_include = clone
        .query(
            "vector-projection",
            &json!({
                "rank_by": ["vector", "ANN", [1.0, 0.0]],
                "limit": 1,
                "include_attributes": ["vector", "title"],
                "vector_encoding": "base64"
            }),
        )
        .unwrap();
    assert_eq!(rows(&query_include)[0]["vector"], expected_vector);
    assert_eq!(rows(&query_include)[0]["title"], "one");

    let query_include_false = clone
        .query(
            "vector-projection",
            &json!({
                "rank_by": ["id", "asc"],
                "limit": 1,
                "include_attributes": false
            }),
        )
        .unwrap();
    assert_eq!(rows(&query_include_false), &[json!({"id": 1})]);

    let query_exclude = clone
        .query(
            "vector-projection",
            &json!({
                "rank_by": ["id", "asc"],
                "limit": 1,
                "exclude_attributes": ["title"],
                "vector_encoding": "base64"
            }),
        )
        .unwrap();
    assert_eq!(rows(&query_exclude)[0]["vector"], expected_vector);
    assert!(rows(&query_exclude)[0].get("title").is_none());

    let export = clone
        .export_namespace(
            "vector-projection",
            &json!({
                "filters": ["vector", "Eq", [1.0, 0.0]],
                "include_attributes": ["vector"],
                "limit": 10,
                "vector_encoding": "base64"
            }),
        )
        .unwrap();
    assert_eq!(export["ids"], json!([1]));
    assert_eq!(export["vectors"], json!([expected_vector]));
    assert_eq!(export["attributes"], json!({}));
    assert_eq!(export["next_cursor"], Value::Null);
    assert_eq!(export["message"], Value::Null);

    let serialized = serde_json::to_value(clone.store()).unwrap();
    let imported: MiniStore = serde_json::from_value(serialized).unwrap();
    let imported_doc = &imported.namespace("vector-projection").unwrap().documents[0];
    assert_eq!(imported_doc.attributes["vector"], json!([1.0, 0.0]));
    assert_eq!(
        imported_doc.typed.dense_vector("vector").unwrap(),
        &[1.0, 0.0]
    );
    assert_eq!(
        query_store(
            &imported,
            "vector-projection",
            &json!({
                "rank_by": ["vector", "ANN", [1.0, 0.0]],
                "limit": 1,
                "include_attributes": ["title"]
            }),
        )
        .unwrap()["rows"][0]["id"],
        1
    );
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
fn document_typed_dense_vectors_are_cached_as_a_sidecar() {
    let document: Document = serde_json::from_value(json!({
        "id": "typed",
        "vector": [1.0, 2.0, 3.0],
        "sparse_vector": {"a": 0.5, "b": 1.5},
        "score": 7
    }))
    .unwrap();

    assert_eq!(
        document.typed.dense_vector("vector").unwrap(),
        &[1.0, 2.0, 3.0]
    );
    assert_eq!(document.attributes["vector"], json!([1.0, 2.0, 3.0]));

    let serialized = serde_json::to_value(&document).unwrap();
    assert_eq!(serialized["vector"], json!([1.0, 2.0, 3.0]));
    assert_eq!(serialized["sparse_vector"], json!({"a": 0.5, "b": 1.5}));
    assert!(serialized.get("typed").is_none());
}

#[test]
fn typed_dense_vectors_refresh_on_upsert_and_imported_stores() {
    let mut clone = Micropuffer::new();
    clone
        .write(
            "typed-refresh",
            &json!({
                "distance_metric": "cosine_distance",
                "schema": {
                    "vector": "[2]f32",
                    "sparse_vector": {
                        "type": "{}f16",
                        "sparse_knn": {"distance_metric": "dot_product"}
                    },
                    "score": "uint"
                },
                "upsert_rows": [
                    {"id": 1, "vector": [1.0, 0.0], "sparse_vector": {"a": 0.1}, "score": 1}
                ]
            }),
        )
        .unwrap();
    clone
        .write(
            "typed-refresh",
            &json!({
                "upsert_rows": [
                    {"id": 1, "vector": [0.0, 1.0], "sparse_vector": {"a": 2.0, "b": 3.0}, "score": 9}
                ]
            }),
        )
        .unwrap();

    let namespace = clone.store().namespace("typed-refresh").unwrap();
    let document = &namespace.documents[0];
    assert_eq!(document.typed.dense_vector("vector").unwrap(), &[0.0, 1.0]);

    let serialized = serde_json::to_value(clone.store()).unwrap();
    let imported: MiniStore = serde_json::from_value(serialized).unwrap();
    let imported_doc = &imported.namespace("typed-refresh").unwrap().documents[0];
    assert_eq!(
        imported_doc
            .typed
            .dense_vector("vector")
            .expect("cached imported vector"),
        &[0.0, 1.0]
    );
    assert_eq!(
        query_store(
            &imported,
            "typed-refresh",
            &json!({
                "rank_by": ["vector", "ANN", [0.0, 1.0]],
                "limit": 1
            }),
        )
        .unwrap()["rows"][0]["id"],
        1
    );
}

#[test]
fn writes_upsert_patch_delete_and_query_in_memory() {
    let mut store = MiniStore::default();
    write_store(
        &mut store,
        "local-test",
        &json!({
            "distance_metric": "cosine_distance",
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
    assert_eq!(write["status"], "OK");
    assert_eq!(write["message"], "documents committed successfully");
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
    assert_eq!(namespace.query_indexes.guard().equality.len(), 1);
    assert_eq!(namespace.query_indexes.guard().order.len(), 1);

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
    assert_eq!(namespace.query_indexes.guard().equality.len(), 1);
    assert_eq!(namespace.query_indexes.guard().order.len(), 1);

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
    assert_eq!(namespace.query_indexes.guard().equality.len(), 1);
    assert_eq!(namespace.query_indexes.guard().order.len(), 1);

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
    assert_eq!(namespace.query_indexes.guard().equality.len(), 1);
    assert_eq!(namespace.query_indexes.guard().order.len(), 1);

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
    assert_eq!(namespace.query_indexes.guard().equality.len(), 1);
    assert_eq!(namespace.query_indexes.guard().order.len(), 1);
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
        .guard()
        .equality
        .get_mut("group")
        .unwrap()
        .postings
        .entry(ScalarEqKey::String("keep".to_string()))
        .or_default()
        .insert(1);

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
fn empty_and_filter_falls_back_to_full_evaluator() {
    let namespace: Namespace = serde_json::from_value(json!({
        "name": "empty-and",
        "documents": [
            {"id": 1, "score": 3},
            {"id": 2, "score": 1},
            {"id": 3, "score": 2}
        ]
    }))
    .unwrap();
    let response = query_namespace(
        &namespace,
        &json!({
            "rank_by": ["score", "asc"],
            "filters": ["And", []],
            "limit": 10
        }),
    )
    .unwrap();
    assert_eq!(
        rows(&response)
            .iter()
            .map(|row| row["id"].clone())
            .collect::<Vec<_>>(),
        vec![json!(2), json!(3), json!(1)]
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
    assert_eq!(namespace.query_indexes.guard().equality.len(), 1);
    assert_eq!(namespace.query_indexes.guard().order.len(), 0);
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
            .guard()
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
            .guard()
            .is_empty()
    );
    assert!(
        clone
            .store()
            .namespace("demo-branch-indexes")
            .unwrap()
            .query_indexes
            .guard()
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
                "distance_metric": "cosine_distance",
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
        documents.push(Document::new(
            Value::Number(Number::from(index as u64)),
            Map::from_iter([
                ("group".to_string(), Value::String("all".to_string())),
                ("patched".to_string(), Value::Bool(false)),
            ]),
        ));
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
            fts_index_cache: Default::default(),
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
    assert_eq!(type_change.status_code(), 400);
    assert_eq!(
        type_change.to_string(),
        "🙅 invalid schema update for attribute 'title': cannot change attribute type from string to int"
    );
    assert_eq!(type_change.body_fields()["attribute"], "title");

    let vector_count = clone
        .write(
            "too-many-vectors",
            &json!({
                "distance_metric": "cosine_distance",
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
    let copied_namespace = clone
        .write("demo-copy", &json!({"copy_from_namespace": "demo"}))
        .unwrap();
    assert_eq!(copied_namespace["status"], "OK");
    assert_eq!(copied_namespace["message"], "namespace cloned successfully");
    let listed = clone.list_namespaces(Some("demo"), None, 10).unwrap();
    let namespaces = listed.get("namespaces").and_then(Value::as_array).unwrap();
    assert_eq!(namespaces.len(), 2);
    let copied = clone
        .query("demo-copy", &json!({"aggregate_by": {"count": ["Count"]}}))
        .unwrap();
    assert_eq!(copied["aggregations"]["count"], 3);
    let deleted = clone.delete_namespace("demo-copy").unwrap();
    assert_eq!(deleted, json!({"status": "OK"}));
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
    assert_eq!(
        invalid.to_string(),
        "💔 copy_from_namespace cannot be used with other write request fields"
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
    assert_eq!(
        metadata["schema"]["title"]["full_text_search"],
        json!({
            "k1": 1.2,
            "b": 0.75,
            "k3": 8.0,
            "language": "english",
            "stemming": false,
            "remove_stopwords": false,
            "ascii_folding": false,
            "case_sensitive": false,
            "max_token_length": 39,
            "tokenizer": "word_v3"
        })
    );
    assert_eq!(metadata["schema"]["published_at"]["type"], "datetime");
    assert_eq!(metadata["schema"]["vector"]["type"], "[2]f32");
    assert_eq!(
        metadata["schema"]["vector"]["ann"]["distance_metric"],
        "cosine_distance"
    );
    assert_eq!(metadata["encryption"], json!({"sse": true}));
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
    assert_eq!(export["ids"], json!([2]));
    assert_eq!(export["vectors"], json!([Value::Null]));
    assert_eq!(
        export["attributes"],
        json!({
            "score": [3],
            "title": ["beta"]
        })
    );
}

#[test]
fn export_namespace_uses_live_columnar_shape_with_missing_attribute_nulls() {
    let mut clone = Micropuffer::new();
    clone
        .write(
            "columnar-export",
            &json!({
                "distance_metric": "cosine_distance",
                "upsert_rows": [
                    {"id": 1, "vector": [1.0, 0.0], "title": "alpha", "score": 10},
                    {"id": 2, "vector": [0.0, 1.0], "title": "beta"}
                ]
            }),
        )
        .unwrap();

    let export = clone
        .export_namespace("columnar-export", &json!({}))
        .unwrap();

    assert_eq!(export["ids"], json!([1, 2]));
    assert_eq!(export["vectors"], json!([[1.0, 0.0], [0.0, 1.0]]));
    assert_eq!(
        export["attributes"],
        json!({
            "score": [10, null],
            "title": ["alpha", "beta"]
        })
    );
    assert_eq!(export["next_cursor"], Value::Null);
    assert_eq!(export["message"], Value::Null);
}

#[test]
fn query_validation_errors_carry_http_status_codes() {
    let namespace = namespace();

    let semantic_conflict = query_namespace(
        &namespace,
        &json!({
            "rank_by": ["id", "asc"],
            "limit": 1,
            "include_attributes": ["title"],
            "exclude_attributes": ["body"]
        }),
    )
    .unwrap_err();
    assert_eq!(semantic_conflict.status_code(), 400);
    assert_eq!(
        semantic_conflict.to_string(),
        "💔 cannot specify both include_attributes and exclude_attributes"
    );

    let invalid_projection = query_namespace(
        &namespace,
        &json!({
            "rank_by": ["id", "asc"],
            "limit": 1,
            "exclude_attributes": true
        }),
    )
    .unwrap_err();
    assert_eq!(invalid_projection.status_code(), 422);
    assert_eq!(
        invalid_projection.to_string(),
        "Failed to deserialize the JSON body into the target type: invalid type: boolean `true`, expected a sequence"
    );

    let invalid_vector_encoding = query_namespace(
        &namespace,
        &json!({
            "rank_by": ["id", "asc"],
            "limit": 1,
            "vector_encoding": "bad"
        }),
    )
    .unwrap_err();
    assert_eq!(invalid_vector_encoding.status_code(), 422);
    assert_eq!(
        invalid_vector_encoding.to_string(),
        "Failed to deserialize the JSON body into the target type: vector_encoding: unknown variant `bad`, expected `float` or `base64`"
    );
}

#[test]
fn missing_namespace_errors_match_live_status_and_text() {
    let mut clone = Micropuffer::new();
    let missing = "missing-namespace";
    let expected = "🤷 namespace 'missing-namespace' was not found";

    let query_error = clone
        .query(missing, &json!({"rank_by": ["id", "asc"], "limit": 1}))
        .unwrap_err();
    assert_eq!(query_error.status_code(), 404);
    assert_eq!(query_error.to_string(), expected);

    let metadata_error = clone.metadata(missing).unwrap_err();
    assert_eq!(metadata_error.status_code(), 404);
    assert_eq!(metadata_error.to_string(), expected);

    let delete_error = clone.delete_namespace(missing).unwrap_err();
    assert_eq!(delete_error.status_code(), 404);
    assert_eq!(delete_error.to_string(), expected);
}

#[test]
fn path_namespace_validation_matches_live_url_errors() {
    let mut clone = Micropuffer::new();
    let invalid_namespace = "bad namespace";
    const INVALID_MESSAGE: &str =
        "Invalid URL: Namespace contains invalid characters, must be [A-Za-z0-9-_.]";

    assert_invalid_url_error(clone.write(invalid_namespace, &json!({"upsert_rows": []})));
    assert_invalid_url_error(clone.query(
        invalid_namespace,
        &json!({"rank_by": ["id", "asc"], "limit": 1}),
    ));
    assert_invalid_url_error(clone.metadata(invalid_namespace));
    assert_invalid_url_error(clone.schema(invalid_namespace));
    assert_invalid_url_error(clone.update_schema(invalid_namespace, &json!({"title": "string"})));
    assert_invalid_url_error(clone.patch_metadata(invalid_namespace, &json!({"pinning": null})));
    assert_invalid_url_error(clone.export_namespace(invalid_namespace, &json!({})));
    assert_invalid_url_error(clone.warm_cache(invalid_namespace));
    assert_invalid_url_error(clone.recall(invalid_namespace, &json!({"num": 1, "top_k": 1})));
    assert_invalid_url_error(clone.explain_query(
        invalid_namespace,
        &json!({"rank_by": ["id", "asc"], "limit": 1}),
    ));
    assert_invalid_url_error(clone.delete_namespace(invalid_namespace));

    let long_namespace = "a".repeat(129);
    let too_long = clone
        .write(&long_namespace, &json!({"upsert_rows": []}))
        .unwrap_err();
    assert_eq!(too_long.status_code(), 400);
    assert!(too_long.has_plain_text_body());
    assert_eq!(
        too_long.to_string(),
        format!(
            "Invalid URL: Namespace `{long_namespace}` is too long, limit is currently 128 characters"
        )
    );

    fn assert_invalid_url_error(result: Result<Value, QueryError>) {
        let error = result.unwrap_err();
        assert_eq!(error.status_code(), 400);
        assert!(error.has_plain_text_body());
        assert_eq!(error.to_string(), INVALID_MESSAGE);
    }
}

#[test]
fn vector_writes_require_live_distance_metric_rules() {
    let mut clone = Micropuffer::new();

    let missing_metric = clone
        .write(
            "metric-required",
            &json!({"upsert_rows": [{"id": 1, "vector": [1.0, 0.0]}]}),
        )
        .unwrap_err();
    assert_eq!(missing_metric.status_code(), 400);
    assert_eq!(
        missing_metric.to_string(),
        "💔 distance_metric must be specified for write to namespace with a vector"
    );

    let invalid_metric = clone
        .write(
            "metric-invalid",
            &json!({
                "distance_metric": "bad",
                "upsert_rows": [{"id": 1, "vector": [1.0, 0.0]}]
            }),
        )
        .unwrap_err();
    assert_eq!(invalid_metric.status_code(), 422);
    assert_eq!(
        invalid_metric.to_string(),
        "Failed to deserialize the JSON body into the target type: distance_metric: unknown variant `bad`, expected one of `Unknown`, `euclidean_squared`, `cosine_distance`, `euclidean`, `Query`"
    );

    clone
        .write(
            "metric-mismatch",
            &json!({
                "distance_metric": "cosine_distance",
                "upsert_rows": [{"id": 1, "vector": [1.0, 0.0]}]
            }),
        )
        .unwrap();
    let mismatch = clone
        .write(
            "metric-mismatch",
            &json!({
                "distance_metric": "euclidean_squared",
                "upsert_rows": [{"id": 2, "vector": [0.0, 1.0]}]
            }),
        )
        .unwrap_err();
    assert_eq!(mismatch.status_code(), 400);
    assert_eq!(
        mismatch.to_string(),
        "💔 distance metric mismatch, expected cosine_distance, got euclidean_squared"
    );

    clone
        .write(
            "scalar-then-vector",
            &json!({"upsert_rows": [{"id": 1, "title": "scalar"}]}),
        )
        .unwrap();
    let added_vector = clone
        .write(
            "scalar-then-vector",
            &json!({"upsert_rows": [{"id": 2, "vector": [1.0, 0.0]}]}),
        )
        .unwrap_err();
    assert_eq!(added_vector.status_code(), 400);
    assert_eq!(
        added_vector.to_string(),
        "💔 Vector provided for namespace without vector attribute"
    );
}

#[test]
fn malformed_write_requests_use_live_style_error_shapes() {
    let mut clone = Micropuffer::new();

    let row_shape = clone
        .write("bad-write", &json!({"upsert_rows": {"id": 1}}))
        .unwrap_err();
    assert_eq!(row_shape.status_code(), 422);
    assert_eq!(
        row_shape.to_string(),
        "Failed to deserialize the JSON body into the target type: upsert_rows: invalid type: map, expected a sequence"
    );

    let missing_id = clone
        .write(
            "bad-write",
            &json!({"upsert_rows": [{"vector": [1.0, 0.0]}]}),
        )
        .unwrap_err();
    assert_eq!(missing_id.status_code(), 422);
    assert_eq!(
        missing_id.to_string(),
        "Failed to deserialize the JSON body into the target type: upsert_rows[0]: missing field `id`"
    );

    let deletes_shape = clone
        .write("bad-write", &json!({"deletes": true}))
        .unwrap_err();
    assert_eq!(deletes_shape.status_code(), 422);
    assert_eq!(
        deletes_shape.to_string(),
        "Failed to deserialize the JSON body into the target type: deletes: data did not match any variant of untagged enum IdVec"
    );
}

#[test]
fn schema_update_and_warm_cache_match_workspace_shapes() {
    let mut clone = Micropuffer::new();
    clone
        .write(
            "schema-api",
            &json!({
                "distance_metric": "cosine_distance",
                "upsert_rows": [
                    {"id": 1, "vector": [0.0, 0.0], "title": "hello"}
                ]
            }),
        )
        .unwrap();
    let schema = clone.schema("schema-api").unwrap();
    assert_eq!(schema["id"]["type"], "uint");
    assert!(schema["id"]["filterable"].is_null());
    assert!(schema["id"]["full_text_search"].is_null());
    assert_eq!(schema["vector"]["type"], "[2]f32");
    assert_eq!(schema["vector"]["ann"], true);
    assert_eq!(schema["title"]["type"], "string");
    assert_eq!(schema["title"]["filterable"], true);
    assert!(schema["title"]["full_text_search"].is_null());

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
    assert_eq!(updated["title"]["full_text_search"]["k1"], 1.2);
    assert_eq!(updated["title"]["full_text_search"]["tokenizer"], "word_v3");
    assert_eq!(updated["title"]["regex"], true);

    let unknown_option = clone
        .update_schema(
            "schema-api",
            &json!({
                "title": {
                    "type": "string",
                    "full_text_search": true,
                    "not_a_real_schema_option": true
                }
            }),
        )
        .unwrap();
    assert!(
        unknown_option["title"]
            .get("not_a_real_schema_option")
            .is_none()
    );
    assert!(
        clone.store().namespace("schema-api").unwrap().schema["title"]
            .get("not_a_real_schema_option")
            .is_none()
    );

    let missing_type = clone
        .update_schema(
            "schema-api",
            &json!({
                "title": {
                    "regex": true
                }
            }),
        )
        .unwrap_err();
    assert_eq!(missing_type.status_code(), 422);
    assert_eq!(
        missing_type.to_string(),
        "Failed to deserialize the JSON body into the target type: title: data did not match any variant of untagged enum AttributeSchemaInput"
    );

    let wrapped_schema = clone
        .update_schema(
            "schema-api",
            &json!({
                "schema": {
                    "title": "string"
                }
            }),
        )
        .unwrap_err();
    assert_eq!(wrapped_schema.status_code(), 422);
    assert_eq!(
        wrapped_schema.to_string(),
        "Failed to deserialize the JSON body into the target type: schema: data did not match any variant of untagged enum AttributeSchemaInput"
    );

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
    assert_eq!(
        invalid_extra_field.to_string(),
        "💔 branch_from_namespace cannot be used with other write request fields"
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

#[test]
#[ignore = "performance evidence; run explicitly"]
fn stateful_100k_dense_sparse_query_benchmark() {
    let row_count = benchmark_env_usize("MICROPUFFER_BENCH_ROWS").unwrap_or(100_000);
    let dimensions = benchmark_env_usize("MICROPUFFER_BENCH_DIMS").unwrap_or(32);
    let runs = benchmark_env_usize("MICROPUFFER_BENCH_QUERY_RUNS").unwrap_or(5);
    let rows = (0..row_count)
        .map(|index| {
            json!({
                "id": index as u64,
                "vector": benchmark_vector(index, dimensions),
                "sparse_vector": {
                    format!("{}", index % 128): ((index % 10) + 1) as f64 / 10.0,
                    format!("{}", (index * 7) % 128): ((index % 7) + 1) as f64 / 10.0
                },
                "category": format!("category_{}", index % 10),
                "score": index % 1_000
            })
        })
        .collect::<Vec<_>>();
    let mut clone = Micropuffer::new();
    let write_started = std::time::Instant::now();
    clone
        .write(
            "stateful-100k",
            &json!({
                "schema": {
                    "vector": format!("[{dimensions}]f32"),
                    "sparse_vector": {
                        "type": "{}f16",
                        "sparse_knn": {"distance_metric": "dot_product"}
                    },
                    "category": "string",
                    "score": "uint"
                },
                "upsert_rows": rows
            }),
        )
        .unwrap();
    let write_elapsed = write_started.elapsed();

    let dense_query = json!({
        "rank_by": ["vector", "ANN", benchmark_vector(42, dimensions)],
        "limit": 10,
        "include_attributes": ["category", "score"]
    });
    let sparse_query = json!({
        "rank_by": ["sparse_vector", "SparseKNN", {"7": 0.7, "12": 0.2}],
        "limit": 10,
        "include_attributes": ["category"]
    });
    let filter_order_query = json!({
        "rank_by": ["score", "desc"],
        "filters": ["category", "Eq", "category_7"],
        "limit": 100,
        "include_attributes": ["category", "score"]
    });
    let aggregate_count_query = json!({
        "aggregate_by": {"count": ["Count"]},
        "filters": ["category", "Eq", "category_7"]
    });
    let group_count_query = json!({
        "aggregate_by": {"count": ["Count"]},
        "group_by": ["category"],
        "top_k": 10
    });
    let dense_elapsed = benchmark_query_runs(&clone, "stateful-100k", &dense_query, runs);
    let sparse_elapsed = benchmark_query_runs(&clone, "stateful-100k", &sparse_query, runs);
    let filter_order_elapsed =
        benchmark_query_runs(&clone, "stateful-100k", &filter_order_query, runs);
    let aggregate_count_elapsed =
        benchmark_query_runs(&clone, "stateful-100k", &aggregate_count_query, runs);
    let group_count_elapsed =
        benchmark_query_runs(&clone, "stateful-100k", &group_count_query, runs);

    println!(
        "stateful_100k_dense_sparse_query_benchmark rows={row_count} dimensions={dimensions} runs={runs} write_ms={:.2} dense_total_ms={:.2} dense_mean_ms={:.2} sparse_total_ms={:.2} sparse_mean_ms={:.2} filter_order_total_ms={:.2} filter_order_mean_ms={:.2} aggregate_count_total_ms={:.2} aggregate_count_mean_ms={:.2} group_count_total_ms={:.2} group_count_mean_ms={:.2}",
        write_elapsed.as_secs_f64() * 1_000.0,
        dense_elapsed.as_secs_f64() * 1_000.0,
        dense_elapsed.as_secs_f64() * 1_000.0 / runs as f64,
        sparse_elapsed.as_secs_f64() * 1_000.0,
        sparse_elapsed.as_secs_f64() * 1_000.0 / runs as f64,
        filter_order_elapsed.as_secs_f64() * 1_000.0,
        filter_order_elapsed.as_secs_f64() * 1_000.0 / runs as f64,
        aggregate_count_elapsed.as_secs_f64() * 1_000.0,
        aggregate_count_elapsed.as_secs_f64() * 1_000.0 / runs as f64,
        group_count_elapsed.as_secs_f64() * 1_000.0,
        group_count_elapsed.as_secs_f64() * 1_000.0 / runs as f64
    );
}

fn benchmark_query_runs(
    clone: &Micropuffer,
    namespace_name: &str,
    query: &Value,
    runs: usize,
) -> std::time::Duration {
    let started = std::time::Instant::now();
    for _ in 0..runs {
        std::hint::black_box(clone.query(namespace_name, query).unwrap());
    }
    started.elapsed()
}

fn benchmark_env_usize(key: &str) -> Option<usize> {
    std::env::var(key).ok()?.parse().ok()
}

fn benchmark_vector(seed: usize, dimensions: usize) -> Vec<f64> {
    let mut values = (0..dimensions)
        .map(|index| {
            let raw = ((seed * 31 + index * 17) % 1_000) as f64 / 1_000.0;
            raw * 2.0 - 1.0
        })
        .collect::<Vec<_>>();
    let norm = values.iter().map(|value| value * value).sum::<f64>().sqrt();
    for value in &mut values {
        *value /= norm;
    }
    values
}

#[test]
#[ignore = "performance evidence; run explicitly"]
fn bm25_100k_indexed_query_benchmark() {
    let text_buckets = [
        "walrus arctic mammal",
        "reef coral fish",
        "falcon sky bird",
        "forest fox mammal",
    ];
    let rows = (1..=100_000_u64)
        .map(|id| {
            json!({
                "id": id,
                "text": format!("{} document {id}", text_buckets[id as usize % text_buckets.len()]),
                "category": format!("category_{}", id % 10)
            })
        })
        .collect::<Vec<_>>();
    let mut store = MiniStore::default();
    write_store(
        &mut store,
        "bm25-bench",
        &json!({
            "schema": {
                "text": {
                    "type": "string",
                    "full_text_search": true
                }
            },
            "upsert_rows": rows
        }),
    )
    .unwrap();
    let request = json!({
        "rank_by": ["text", "BM25", "walrus mammal"],
        "limit": 10
    });

    let cold_started = std::time::Instant::now();
    query_store(&store, "bm25-bench", &request).unwrap();
    let cold_elapsed = cold_started.elapsed();

    let warm_runs = 10;
    let warm_started = std::time::Instant::now();
    for _ in 0..warm_runs {
        std::hint::black_box(query_store(&store, "bm25-bench", &request).unwrap());
    }
    let warm_elapsed = warm_started.elapsed();
    println!(
        "bm25_100k_indexed_query_benchmark cold_ms={:.3} warm_runs={warm_runs} warm_total_ms={:.3} warm_mean_ms={:.3}",
        cold_elapsed.as_secs_f64() * 1_000.0,
        warm_elapsed.as_secs_f64() * 1_000.0,
        warm_elapsed.as_secs_f64() * 1_000.0 / warm_runs as f64
    );
}
