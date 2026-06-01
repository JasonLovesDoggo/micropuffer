use micropuffer::{MiniStore, query_store};
use serde_json::{Value, json};

fn store() -> MiniStore {
    serde_json::from_str(include_str!("../fixtures/demo-store.json")).unwrap()
}

fn rows(response: &Value) -> &Vec<Value> {
    response.get("rows").and_then(Value::as_array).unwrap()
}

#[test]
fn vector_query_matches_fixture_order() {
    let response = query_store(
        &store(),
        "expertise",
        &json!({
            "rank_by": ["vector", "ANN", [0.0, 0.0, 0.0]],
            "limit": 2,
            "include_attributes": ["title", "tenant_id"]
        }),
    )
    .unwrap();
    assert_eq!(
        rows(&response),
        &vec![
            json!({"$dist": 0.010000000000000002, "id": "expertise:doc:001", "tenant_id": "stanley-production", "title": "Rust vector search engineer"}),
            json!({"$dist": 0.06000000000000001, "id": "expertise:doc:002", "tenant_id": "stanley-staging", "title": "Dashboard prototype specialist"})
        ]
    );
}

#[test]
fn filter_then_bm25_returns_public_hybrid_doc() {
    let response = query_store(
        &store(),
        "expertise",
        &json!({
            "rank_by": ["body", "BM25", "sparse vector recall"],
            "filters": ["And", [
                ["public", "Eq", true],
                ["tags", "Contains", "hybrid"]
            ]],
            "limit": 10,
            "include_attributes": true
        }),
    )
    .unwrap();
    let rows = rows(&response);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["id"], "expertise:doc:003");
    assert_eq!(rows[0]["title"], "Hybrid search relevance tuning");
}

#[test]
fn grouped_aggregation_over_tags_is_stable() {
    let response = query_store(
        &store(),
        "expertise",
        &json!({
            "aggregate_by": {"count": ["Count"]},
            "group_by": [{"tag": ["ForEachUnique", "tags"]}],
            "top_k": 10
        }),
    )
    .unwrap();
    let groups = response
        .get("aggregation_groups")
        .and_then(Value::as_array)
        .unwrap();
    assert!(
        groups
            .iter()
            .any(|group| group["tag"] == "rust" && group["count"] == 1)
    );
    assert!(
        groups
            .iter()
            .any(|group| group["tag"] == "hybrid" && group["count"] == 1)
    );

    let sum_response = query_store(
        &store(),
        "expertise",
        &json!({
            "aggregate_by": {"score_sum": ["Sum", "score"]},
            "group_by": [{"tag": ["ForEachUnique", "tags"]}],
            "top_k": 10
        }),
    )
    .unwrap();
    let sum_groups = sum_response
        .get("aggregation_groups")
        .and_then(Value::as_array)
        .unwrap();
    assert!(
        sum_groups
            .iter()
            .any(|group| group["tag"] == "rust" && group["score_sum"] == 97.0)
    );
    assert!(
        sum_groups
            .iter()
            .any(|group| group["tag"] == "hybrid" && group["score_sum"] == 89.0)
    );
}
