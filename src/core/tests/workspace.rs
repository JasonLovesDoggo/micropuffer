use crate::core::tests::*;

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
                    "vector": {"type": "[2]f32", "ann": true},
                    "image_vector": {"type": "[2]f32", "ann": true},
                    "audio_vector": {"type": "[2]f32", "ann": true}
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
    assert_eq!(listed["next_cursor"], Value::Null);
    let first_page = clone.list_namespaces(Some("demo"), None, 1).unwrap();
    assert_eq!(first_page["namespaces"], json!([{"id": "demo-copy"}]));
    let cursor = first_page["next_cursor"].as_str().unwrap();
    assert_eq!(
        String::from_utf8(STANDARD_NO_PAD.decode(cursor).unwrap()).unwrap(),
        r#"{"continuation_token":null,"start_after":"demo-copy-table/"}"#
    );
    let second_page = clone
        .list_namespaces(Some("demo"), Some(cursor), 1)
        .unwrap();
    assert_eq!(second_page["namespaces"], json!([{"id": "demo"}]));
    let second_cursor = second_page["next_cursor"].as_str().unwrap();
    assert_eq!(
        String::from_utf8(STANDARD_NO_PAD.decode(second_cursor).unwrap()).unwrap(),
        r#"{"continuation_token":null,"start_after":"demo-table/"}"#
    );
    let empty_page = clone
        .list_namespaces(Some("demo"), Some(second_cursor), 1)
        .unwrap();
    assert_eq!(empty_page["namespaces"], json!([]));
    assert_eq!(empty_page["next_cursor"], Value::Null);
    let empty_cursor = STANDARD_NO_PAD.encode("{}");
    let empty_cursor_page = clone
        .list_namespaces(Some("demo"), Some(&empty_cursor), 1)
        .unwrap();
    assert_eq!(
        empty_cursor_page["namespaces"],
        json!([{"id": "demo-copy"}])
    );
    let invalid_cursor = clone
        .list_namespaces(Some("demo"), Some("bad"), 1)
        .unwrap_err();
    assert_eq!(invalid_cursor.status_code(), 500);
    assert_eq!(
        invalid_cursor.to_string(),
        "🐡 Unknown error, if this persists, please contact support!! We'll be happy to help :)"
    );
    let zero_page_size = clone.list_namespaces(Some("demo"), None, 0).unwrap_err();
    assert_eq!(zero_page_size.status_code(), 400);
    assert_eq!(
        zero_page_size.to_string(),
        "💔 Page size must be in range 1..=1000, was 0"
    );
    let large_page_size = clone.list_namespaces(Some("demo"), None, 1001).unwrap_err();
    assert_eq!(large_page_size.status_code(), 400);
    assert_eq!(
        large_page_size.to_string(),
        "💔 Page size must be in range 1..=1000, was 1001"
    );
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
                    "published_at": "datetime",
                    "sparse_vector": {
                        "type": "{}f16",
                        "sparse_knn": {"distance_metric": "dot_product"}
                    }
                },
                "upsert_rows": [
                    {"id": 1, "vector": [1.0, 0.0], "sparse_vector": {"1": 1.0}, "title": "alpha", "published_at": "2026-05-31T00:00:00Z", "score": 2},
                    {"id": 2, "vector": [0.0, 1.0], "sparse_vector": {"2": 1.0}, "title": "beta", "published_at": "2026-05-30T00:00:00Z", "score": 3}
                ]
            }),
        )
        .unwrap();
    let metadata = clone.metadata("workspace").unwrap();
    assert_eq!(metadata["approx_row_count"], 2);
    assert_eq!(metadata["schema"]["title"]["type"], "string");
    assert_eq!(metadata["schema"]["title"]["filterable"], false);
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
    assert_eq!(metadata["schema"]["sparse_vector"]["type"], "{}f16");
    assert_eq!(metadata["schema"]["sparse_vector"]["filterable"], false);
    assert_eq!(
        metadata["schema"]["sparse_vector"]["sparse_knn"]["distance_metric"],
        "dot_product"
    );
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
fn metadata_pinning_errors_match_live_status_and_text() {
    let mut clone = Micropuffer::new();
    clone
        .write(
            "workspace",
            &json!({
                "distance_metric": "cosine_distance",
                "upsert_rows": [
                    {"id": 1, "vector": [1.0, 0.0]}
                ]
            }),
        )
        .unwrap();

    let invalid_pinning = clone
        .patch_metadata("workspace", &json!({"pinning": "bad"}))
        .unwrap_err();
    assert_eq!(invalid_pinning.status_code(), 422);
    assert_eq!(
        invalid_pinning.to_string(),
        "Failed to deserialize the JSON body into the target type: pinning: data did not match any variant of untagged enum UpdatePinningInput at line 1 column 17"
    );

    let invalid_replicas = clone
        .patch_metadata("workspace", &json!({"pinning": {"replicas": "bad"}}))
        .unwrap_err();
    assert_eq!(invalid_replicas.status_code(), 422);
    assert_eq!(
        invalid_replicas.to_string(),
        "Failed to deserialize the JSON body into the target type: pinning: data did not match any variant of untagged enum UpdatePinningInput at line 1 column 30"
    );

    let fractional_replicas = clone
        .patch_metadata("workspace", &json!({"pinning": {"replicas": 1.5}}))
        .unwrap_err();
    assert_eq!(fractional_replicas.status_code(), 422);
    assert_eq!(
        fractional_replicas.to_string(),
        "Failed to deserialize the JSON body into the target type: pinning: data did not match any variant of untagged enum UpdatePinningInput at line 1 column 28"
    );

    let negative_replicas = clone
        .patch_metadata("workspace", &json!({"pinning": {"replicas": -1}}))
        .unwrap_err();
    assert_eq!(negative_replicas.status_code(), 422);
    assert_eq!(
        negative_replicas.to_string(),
        "Failed to deserialize the JSON body into the target type: pinning: data did not match any variant of untagged enum UpdatePinningInput at line 1 column 27"
    );

    let zero_replicas = clone
        .patch_metadata("workspace", &json!({"pinning": {"replicas": 0}}))
        .unwrap_err();
    assert_eq!(zero_replicas.status_code(), 400);
    assert_eq!(
        zero_replicas.to_string(),
        "💔 replicas must be greater than 0"
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
                "schema": {
                    "sparse_vector": {
                        "type": "{}f16",
                        "sparse_knn": {"distance_metric": "dot_product"}
                    }
                },
                "upsert_rows": [
                    {"id": 1, "vector": [0.0, 0.0], "sparse_vector": {"1": 1.0}, "title": "hello"}
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
    assert_eq!(schema["sparse_vector"]["type"], "{}f16");
    assert_eq!(schema["sparse_vector"]["filterable"], false);
    assert!(schema["sparse_vector"]["full_text_search"].is_null());
    assert!(schema["sparse_vector"].get("sparse_knn").is_none());

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
    assert_eq!(updated["title"]["filterable"], false);
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
    assert_eq!(warmed["message"], "cache warm hint accepted");
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
