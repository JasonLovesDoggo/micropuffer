use crate::core::tests::*;

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

    let missing_include_attribute = query_namespace(
        &namespace,
        &json!({
            "rank_by": ["id", "asc"],
            "limit": 1,
            "include_attributes": ["missing_attr"]
        }),
    )
    .unwrap_err();
    assert_eq!(missing_include_attribute.status_code(), 400);
    assert_eq!(
        missing_include_attribute.to_string(),
        "💔 attribute \"missing_attr\" not found in schema, cannot be part of `include_attributes`. consider passing `include_attributes=True` to return all attribute data instead"
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

    let invalid_rank_by_shape = query_namespace(
        &namespace,
        &json!({
            "rank_by": "bad",
            "limit": 1
        }),
    )
    .unwrap_err();
    assert_eq!(invalid_rank_by_shape.status_code(), 422);
    assert_eq!(
        invalid_rank_by_shape.to_string(),
        "Failed to deserialize the JSON body into the target type: data did not match any variant of enum a valid variant of RankInput at line 1 column 27"
    );

    let empty_rank_by = query_namespace(
        &namespace,
        &json!({
            "rank_by": [],
            "limit": 1
        }),
    )
    .unwrap_err();
    assert_eq!(empty_rank_by.status_code(), 400);
    assert_eq!(empty_rank_by.to_string(), "💔 rank_by cannot be empty");

    let invalid_rank_by_operator = query_namespace(
        &namespace,
        &json!({
            "rank_by": ["vector", "BAD", [1.0, 0.0]],
            "limit": 1
        }),
    )
    .unwrap_err();
    assert_eq!(invalid_rank_by_operator.status_code(), 422);
    assert_eq!(
        invalid_rank_by_operator.to_string(),
        "Failed to deserialize the JSON body into the target type: data did not match any variant of enum a valid variant of RankInput at line 1 column 44"
    );

    let missing_limit =
        query_namespace(&namespace, &json!({"rank_by": ["id", "asc"]})).unwrap_err();
    assert_eq!(missing_limit.status_code(), 400);
    assert_eq!(
        missing_limit.to_string(),
        "💔 rank_by queries must specify top_k or limit"
    );

    let zero_limit = query_namespace(
        &namespace,
        &json!({
            "rank_by": ["id", "asc"],
            "limit": 0
        }),
    )
    .unwrap_err();
    assert_eq!(zero_limit.status_code(), 400);
    assert_eq!(
        zero_limit.to_string(),
        "💔 top_k must be between 1 and 10000"
    );

    let invalid_limit_shape = query_namespace(
        &namespace,
        &json!({
            "rank_by": ["id", "asc"],
            "limit": "bad"
        }),
    )
    .unwrap_err();
    assert_eq!(invalid_limit_shape.status_code(), 422);
    assert_eq!(
        invalid_limit_shape.to_string(),
        "Failed to deserialize the JSON body into the target type: data did not match any variant of untagged enum LimitInput at line 1 column 38"
    );

    let invalid_consistency_level = query_namespace(
        &namespace,
        &json!({
            "rank_by": ["id", "asc"],
            "limit": 1,
            "consistency": {"level": "bad"}
        }),
    )
    .unwrap_err();
    assert_eq!(invalid_consistency_level.status_code(), 422);
    assert_eq!(
        invalid_consistency_level.to_string(),
        "Failed to deserialize the JSON body into the target type: consistency.level: unknown variant `bad`, expected `strong` or `eventual`"
    );

    let invalid_consistency_shape = query_namespace(
        &namespace,
        &json!({
            "rank_by": ["id", "asc"],
            "limit": 1,
            "consistency": "eventual"
        }),
    )
    .unwrap_err();
    assert_eq!(invalid_consistency_shape.status_code(), 422);
    assert_eq!(
        invalid_consistency_shape.to_string(),
        "Failed to deserialize the JSON body into the target type: consistency: invalid type: string \"eventual\", expected struct Consistency"
    );

    let missing_consistency_level = query_namespace(
        &namespace,
        &json!({
            "rank_by": ["id", "asc"],
            "limit": 1,
            "consistency": {}
        }),
    )
    .unwrap_err();
    assert_eq!(missing_consistency_level.status_code(), 422);
    assert_eq!(
        missing_consistency_level.to_string(),
        "Failed to deserialize the JSON body into the target type: consistency: missing field `level`"
    );

    let invalid_consistency_level_type = query_namespace(
        &namespace,
        &json!({
            "rank_by": ["id", "asc"],
            "limit": 1,
            "consistency": {"level": 1}
        }),
    )
    .unwrap_err();
    assert_eq!(invalid_consistency_level_type.status_code(), 400);
    assert_eq!(
        invalid_consistency_level_type.to_string(),
        "Failed to parse the request body as JSON: consistency.level: expected value"
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

    let missing_ann = clone
        .write(
            "ann-required",
            &json!({
                "distance_metric": "cosine_distance",
                "schema": {"vector": "[2]f32"},
                "upsert_rows": [{"id": 1, "vector": [1.0, 0.0]}]
            }),
        )
        .unwrap_err();
    assert_eq!(missing_ann.status_code(), 400);
    assert_eq!(
        missing_ann.to_string(),
        "💔 the `vector` attribute must have `ann` set to `true`, eg: `\"vector\": { \"type\": \"[1024]f32\", \"ann\": true }`"
    );

    let custom_missing_ann = clone
        .write(
            "custom-ann-required",
            &json!({
                "distance_metric": "cosine_distance",
                "schema": {"embedding": {"type": "[2]f32"}},
                "upsert_rows": [{"id": 1, "embedding": [1.0, 0.0]}]
            }),
        )
        .unwrap_err();
    assert_eq!(custom_missing_ann.status_code(), 400);
    assert_eq!(
        custom_missing_ann.to_string(),
        "💔 vector attribute 'embedding' must have ann:true"
    );

    let schema_ann_metric = clone
        .write(
            "metric-required",
            &json!({
                "schema": {
                    "vector": {
                        "type": "[2]f32",
                        "ann": {"distance_metric": "euclidean"}
                    }
                },
                "upsert_rows": [{"id": 1, "vector": [1.0, 0.0]}]
            }),
        )
        .unwrap_err();
    assert_eq!(schema_ann_metric.status_code(), 400);
    assert_eq!(
        schema_ann_metric.to_string(),
        "💔 `distance_metric` must be specified as a field at the top level of the write request, not in the `ann` configuration for an attribute"
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

    let malformed_row_id = clone
        .write("bad-write", &json!({"upsert_rows": [{"id": false}]}))
        .unwrap_err();
    assert_eq!(malformed_row_id.status_code(), 422);
    assert_eq!(
        malformed_row_id.to_string(),
        "Failed to deserialize the JSON body into the target type: upsert_rows[0].id: data did not match any variant of untagged enum Id"
    );

    let malformed_patch_id = clone
        .write("bad-write", &json!({"patch_rows": [{"id": 1.5}]}))
        .unwrap_err();
    assert_eq!(malformed_patch_id.status_code(), 422);
    assert_eq!(
        malformed_patch_id.to_string(),
        "Failed to deserialize the JSON body into the target type: patch_rows[0].id: data did not match any variant of untagged enum Id"
    );

    let malformed_column_id = clone
        .write(
            "bad-write",
            &json!({"upsert_columns": {"id": [true], "title": ["bad"]}}),
        )
        .unwrap_err();
    assert_eq!(malformed_column_id.status_code(), 422);
    assert_eq!(
        malformed_column_id.to_string(),
        "Failed to deserialize the JSON body into the target type: upsert_columns.id: data did not match any variant of untagged enum IdVec"
    );

    let malformed_delete_id = clone
        .write("bad-write", &json!({"deletes": [true]}))
        .unwrap_err();
    assert_eq!(malformed_delete_id.status_code(), 422);
    assert_eq!(
        malformed_delete_id.to_string(),
        "Failed to deserialize the JSON body into the target type: deletes: data did not match any variant of untagged enum IdVec"
    );
}
