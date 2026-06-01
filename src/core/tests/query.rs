use crate::core::tests::*;

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
fn filters_support_documented_fuzzy_options_and_null_comparisons() {
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
    .unwrap_err();
    assert_eq!(token_array.status_code(), 400);
    assert_eq!(
        token_array.to_string(),
        "filter error in key `body`: type mismatch, ContainsAllTokens expects string, but got '[rust, ranking]'"
    );

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

    clone
        .write(
            "euclidean-demo",
            &json!({
                "distance_metric": "euclidean",
                "upsert_rows": [
                    {"id": "unit", "vector": [1.0, 0.0]},
                    {"id": "double", "vector": [0.0, 2.0]}
                ]
            }),
        )
        .unwrap();
    let euclidean = clone
        .query(
            "euclidean-demo",
            &json!({
                "rank_by": ["vector", "ANN", [0.0, 0.0]],
                "limit": 2
            }),
        )
        .unwrap();
    assert_eq!(rows(&euclidean)[0]["id"], "unit");
    assert_eq!(rows(&euclidean)[0]["$dist"], 1.0);
    assert_eq!(rows(&euclidean)[1]["id"], "double");
    assert_eq!(rows(&euclidean)[1]["$dist"], 2.0);
    assert_eq!(
        clone.metadata("euclidean-demo").unwrap()["schema"]["vector"]["ann"]["distance_metric"],
        "euclidean"
    );
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
fn aggregate_filters_propagate_filter_errors() {
    for aggregate_by in [
        json!({"count": ["Count"]}),
        json!({"score_sum": ["Sum", "score"]}),
    ] {
        let error = query_namespace(
            &namespace(),
            &json!({
                "aggregate_by": aggregate_by,
                "filters": ["score", "Bogus", 1]
            }),
        )
        .unwrap_err();
        assert_eq!(error.status_code(), 400);
        assert_eq!(error.to_string(), "Unsupported filter operator 'Bogus'.");
    }
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
fn multi_query_matches_live_root_leniency_and_validation_errors() {
    let root_fields = query_namespace(
        &namespace(),
        &json!({
            "queries": [
                {
                    "rank_by": ["id", "asc"],
                    "limit": 1,
                    "include_attributes": ["vector"]
                }
            ],
            "rank_by": ["id", "desc"],
            "filters": ["public", "Eq", false],
            "aggregate_by": {"count": ["Count"]},
            "vector_encoding": "base64"
        }),
    )
    .unwrap();
    assert_eq!(
        root_fields["results"][0]["rows"][0]["vector"],
        "AAAAAAAAAAA="
    );

    let string_queries = query_namespace(&namespace(), &json!({"queries": "bad"})).unwrap_err();
    assert_eq!(string_queries.status_code(), 422);
    assert_eq!(
        string_queries.to_string(),
        "Failed to deserialize the JSON body into the target type: invalid type: string \"bad\", expected a sequence at line 1 column 17"
    );

    let empty_queries = query_namespace(&namespace(), &json!({"queries": []})).unwrap_err();
    assert_eq!(empty_queries.status_code(), 400);
    assert_eq!(
        empty_queries.to_string(),
        "💔 must send at least one sub-query"
    );

    let too_many_queries = query_namespace(
        &namespace(),
        &json!({
            "queries": (0..17).map(|_| json!({"rank_by": ["id", "asc"], "limit": 1})).collect::<Vec<_>>()
        }),
    )
    .unwrap_err();
    assert_eq!(too_many_queries.status_code(), 400);
    assert_eq!(
        too_many_queries.to_string(),
        "💔 multi-query exceeds per-namespace concurrency budget: requires 17 permits, max is 16 (see https://turbopuffer.com/docs/limits)"
    );

    let invalid_subquery = query_namespace(&namespace(), &json!({"queries": ["bad"]})).unwrap_err();
    assert_eq!(invalid_subquery.status_code(), 422);
    assert_eq!(
        invalid_subquery.to_string(),
        "Failed to deserialize the JSON body into the target type: invalid type: string \"bad\", expected struct QueryRankBy at line 1 column 19"
    );
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
