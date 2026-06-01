use crate::core::tests::*;

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
    assert!(recall.get("ground_truth").is_none());
    let capped_recall = clone
        .recall(
            "debug",
            &json!({
                "top_k": 10,
                "filters": ["public", "Eq", 1]
            }),
        )
        .unwrap();
    assert_eq!(capped_recall["avg_recall"], 1.0);
    assert_eq!(capped_recall["avg_exhaustive_count"], 10.0);
    assert_eq!(capped_recall["avg_ann_count"], 10.0);
    let ann_ranked_recall = clone
        .recall(
            "debug",
            &json!({
                "num": null,
                "top_k": 2,
                "rank_by": ["vector", "ANN", [1.0, 0.0]]
            }),
        )
        .unwrap();
    assert_eq!(ann_ranked_recall["avg_recall"], 1.0);
    assert_eq!(ann_ranked_recall["avg_exhaustive_count"], 2.0);
    assert_eq!(ann_ranked_recall["avg_ann_count"], 2.0);
    let bm25_ranked_recall = clone
        .recall(
            "debug",
            &json!({
                "top_k": 2,
                "rank_by": ["text", "BM25", "walrus"]
            }),
        )
        .unwrap();
    assert_eq!(bm25_ranked_recall["avg_recall"], 1.0);
    assert_eq!(bm25_ranked_recall["avg_exhaustive_count"], 2.0);
    assert_eq!(bm25_ranked_recall["avg_ann_count"], 2.0);
    let ranked_num = clone
        .recall(
            "debug",
            &json!({"num": 2, "top_k": 1, "rank_by": ["vector", "ANN", [1.0, 0.0]]}),
        )
        .unwrap_err();
    assert_eq!(ranked_num.status_code(), 400);
    assert_eq!(
        ranked_num.to_string(),
        "💔 rank_by and num cannot be specified together"
    );
    let invalid_rank_by = clone
        .recall("debug", &json!({"num": 1, "top_k": 1, "rank_by": "bad"}))
        .unwrap_err();
    assert_eq!(invalid_rank_by.status_code(), 422);
    assert_eq!(
        invalid_rank_by.to_string(),
        "Failed to deserialize the JSON body into the target type: rank_by: data did not match any variant of enum a valid variant of RankInput at line 1 column 35"
    );
    let invalid_num = clone
        .recall("debug", &json!({"num": "bad", "top_k": 1}))
        .unwrap_err();
    assert_eq!(invalid_num.status_code(), 422);
    assert_eq!(
        invalid_num.to_string(),
        "Failed to deserialize the JSON body into the target type: num: invalid type: string \"bad\", expected usize"
    );
    let zero_num = clone
        .recall("debug", &json!({"num": 0, "top_k": 1}))
        .unwrap_err();
    assert_eq!(zero_num.status_code(), 400);
    assert_eq!(zero_num.to_string(), "💔 samples must be between 1 and 200");
    let invalid_top_k = clone
        .recall("debug", &json!({"num": 1, "top_k": "bad"}))
        .unwrap_err();
    assert_eq!(invalid_top_k.status_code(), 422);
    assert_eq!(
        invalid_top_k.to_string(),
        "Failed to deserialize the JSON body into the target type: top_k: invalid type: string \"bad\", expected usize"
    );
    let zero_top_k = clone
        .recall("debug", &json!({"num": 1, "top_k": 0}))
        .unwrap_err();
    assert_eq!(zero_top_k.status_code(), 400);
    assert_eq!(
        zero_top_k.to_string(),
        "💔 top_k must be between 1 and 10000"
    );
    let invalid_filters = clone
        .recall("debug", &json!({"num": 1, "top_k": 1, "filters": "bad"}))
        .unwrap_err();
    assert_eq!(invalid_filters.status_code(), 422);
    assert_eq!(
        invalid_filters.to_string(),
        "Failed to deserialize the JSON body into the target type: filters: data did not match any variant of untagged enum FiltersInput"
    );
    let invalid_ground_truth = clone
        .recall(
            "debug",
            &json!({"num": 1, "top_k": 1, "include_ground_truth": "bad"}),
        )
        .unwrap_err();
    assert_eq!(invalid_ground_truth.status_code(), 422);
    assert_eq!(
        invalid_ground_truth.to_string(),
        "Failed to deserialize the JSON body into the target type: include_ground_truth: invalid type: string \"bad\", expected a boolean"
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
    let plan_text = explained["plan_text"].as_str().expect("plan text");
    assert!(plan_text.contains("explain_source=micropuffer-local"));
    assert!(plan_text.contains("parity=not-live-turbopuffer-plan"));
    assert!(plan_text.contains("async_realism=disabled"));
    assert!(plan_text.contains("operation=query"));
    assert!(plan_text.contains("ranker=BM25 attr=text"));
    assert!(plan_text.contains("candidate_source=bm25_postings_index+indexed_filter_candidates"));
    assert!(plan_text.contains("projection=default vector_encoding=float"));

    let ann_explained = clone
        .explain_query(
            "debug",
            &json!({
                "rank_by": ["vector", "ANN", [1.0, 0.0]],
                "top_k": 2,
                "include_attributes": ["vector"],
                "vector_encoding": "base64"
            }),
        )
        .unwrap();
    let ann_plan_text = ann_explained["plan_text"].as_str().expect("plan text");
    assert!(ann_plan_text.contains("ranker=ANN attr=vector engine=exact_linear_scan"));
    assert!(ann_plan_text.contains("root_vector_encoding=base64"));
    assert!(ann_plan_text.contains("projection=include=[\"vector\"] vector_encoding=base64"));

    let multi_explained = clone
        .explain_query(
            "debug",
            &json!({
                "queries": [
                    {"rank_by": ["vector", "ANN", [1.0, 0.0]], "limit": 2},
                    {"rank_by": ["text", "BM25", "fish"], "limit": 2}
                ],
                "rank_by": ["id", "desc"],
                "filters": ["public", "Eq", 0],
                "vector_encoding": "base64"
            }),
        )
        .unwrap();
    let multi_plan_text = multi_explained["plan_text"].as_str().expect("plan text");
    assert!(multi_plan_text.contains("operation=multi_query"));
    assert!(multi_plan_text.contains("subqueries=2"));
    assert!(multi_plan_text.contains("subquery[0].ranker=ANN attr=vector"));
    assert!(multi_plan_text.contains("subquery[1].ranker=BM25 attr=text"));
}
