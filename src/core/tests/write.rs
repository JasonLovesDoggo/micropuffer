use crate::core::tests::*;

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
                    "vector": {"type": "[2]f32", "ann": true},
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
fn indexed_order_continues_past_stale_candidate_until_limit_is_satisfied() {
    let namespace: Namespace = serde_json::from_value(json!({
        "name": "source-of-truth-limit",
        "documents": [
            {"id": 1, "group": "skip", "score": 1},
            {"id": 2, "group": "keep", "score": 2},
            {"id": 3, "group": "keep", "score": 3}
        ]
    }))
    .unwrap();
    query_namespace(
        &namespace,
        &json!({
            "rank_by": ["score", "asc"],
            "filters": ["group", "Eq", "keep"],
            "limit": 10
        }),
    )
    .unwrap();

    namespace
        .query_indexes
        .guard()
        .equality
        .get_mut("group")
        .unwrap()
        .postings
        .entry(ScalarEqKey::String("keep".to_string()))
        .or_default()
        .insert(0);

    let response = query_namespace(
        &namespace,
        &json!({
            "rank_by": ["score", "asc"],
            "filters": ["group", "Eq", "keep"],
            "limit": 1
        }),
    )
    .unwrap();
    assert_eq!(
        rows(&response)
            .iter()
            .map(|row| row["id"].clone())
            .collect::<Vec<_>>(),
        vec![json!(2)]
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
                    "vector": {"type": "[2]f32", "ann": true},
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
    assert_eq!(
        patch_vector.to_string(),
        "💔 patching vectors is currently unsupported"
    );

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
            .contains("patching vectors is currently unsupported")
    );
}

#[test]
fn patches_cannot_introduce_dense_vector_attribute_into_scalar_namespace() {
    fn scalar_namespace(name: &str) -> Micropuffer {
        let mut clone = Micropuffer::new();
        clone
            .write(
                name,
                &json!({
                    "upsert_rows": [
                        {"id": 1, "title": "x"}
                    ]
                }),
            )
            .unwrap();
        clone
    }

    let cases = [
        (
            "scalar-patch-rows-vector",
            json!({
                "patch_rows": [
                    {"id": 1, "vector": [1.0, 0.0]}
                ]
            }),
        ),
        (
            "scalar-patch-columns-vector",
            json!({
                "patch_columns": {
                    "id": [1],
                    "vector": [[1.0, 0.0]]
                }
            }),
        ),
        (
            "scalar-patch-by-filter-vector",
            json!({
                "patch_by_filter": {
                    "filters": ["id", "Eq", 1],
                    "patch": {"vector": [1.0, 0.0]}
                }
            }),
        ),
        (
            "scalar-patch-rows-vector-null",
            json!({
                "patch_rows": [
                    {"id": 1, "vector": null}
                ]
            }),
        ),
    ];

    for (namespace, request) in cases {
        let mut clone = scalar_namespace(namespace);
        let error = clone.write(namespace, &request).unwrap_err();
        assert_eq!(error.status_code(), 400);
        assert_eq!(
            error.to_string(),
            "💔 patching vectors is currently unsupported"
        );

        let response = clone
            .query(
                namespace,
                &json!({
                    "rank_by": ["id", "asc"],
                    "limit": 10,
                    "include_attributes": true
                }),
            )
            .unwrap();
        assert_eq!(rows(&response), &[json!({"id": 1, "title": "x"})]);
        assert!(clone.schema(namespace).unwrap().get("vector").is_none());
    }
}

#[test]
fn uuid_id_schema_normalizes_and_rejects_written_ids() {
    let mut clone = Micropuffer::new();
    let invalid_first_write = clone
        .write(
            "uuid-invalid",
            &json!({
                "schema": {"id": "uuid", "title": "string"},
                "upsert_rows": [{"id": "not-a-uuid", "title": "bad"}]
            }),
        )
        .unwrap_err();
    assert_eq!(invalid_first_write.status_code(), 400);
    assert_eq!(
        invalid_first_write.to_string(),
        "💔 namespace ID type is uuid, but a written ID could not be parsed as uuid"
    );
    assert_eq!(
        clone.metadata("uuid-invalid").unwrap_err().status_code(),
        404
    );

    let write = clone
        .write(
            "uuid-demo",
            &json!({
                "schema": {"id": "uuid", "title": "string"},
                "upsert_rows": [
                    {"id": "769C134D-07B8-4225-954A-B6CC5FFC320C", "title": "row-0"},
                    {"id": "769c134d07b84225954ab6cc5ffc320d", "title": "row-1"},
                    {"id": "{769c134d-07b8-4225-954a-b6cc5ffc320e}", "title": "row-2"},
                    {"id": "urn:uuid:769c134d-07b8-4225-954a-b6cc5ffc320f", "title": "row-3"}
                ],
                "return_affected_ids": true
            }),
        )
        .unwrap();
    assert_eq!(
        write["upserted_ids"],
        json!([
            "769c134d-07b8-4225-954a-b6cc5ffc320c",
            "769c134d-07b8-4225-954a-b6cc5ffc320d",
            "769c134d-07b8-4225-954a-b6cc5ffc320e",
            "769c134d-07b8-4225-954a-b6cc5ffc320f"
        ])
    );
    assert_eq!(clone.schema("uuid-demo").unwrap()["id"]["type"], "uuid");

    let query = clone
        .query(
            "uuid-demo",
            &json!({
                "rank_by": ["title", "asc"],
                "limit": 10,
                "include_attributes": ["title"]
            }),
        )
        .unwrap();
    assert_eq!(
        rows(&query)
            .iter()
            .map(|row| row["id"].clone())
            .collect::<Vec<_>>(),
        vec![
            json!("769c134d-07b8-4225-954a-b6cc5ffc320c"),
            json!("769c134d-07b8-4225-954a-b6cc5ffc320d"),
            json!("769c134d-07b8-4225-954a-b6cc5ffc320e"),
            json!("769c134d-07b8-4225-954a-b6cc5ffc320f")
        ]
    );

    let patched = clone
        .write(
            "uuid-demo",
            &json!({
                "patch_rows": [
                    {"id": "769C134D-07B8-4225-954A-B6CC5FFC320C", "title": "patched"}
                ],
                "return_affected_ids": true
            }),
        )
        .unwrap();
    assert_eq!(
        patched["patched_ids"],
        json!(["769c134d-07b8-4225-954a-b6cc5ffc320c"])
    );

    let column_patch = clone
        .write(
            "uuid-demo",
            &json!({
                "patch_columns": {
                    "id": ["769c134d07b84225954ab6cc5ffc320d"],
                    "title": ["column patched"]
                },
                "return_affected_ids": true
            }),
        )
        .unwrap();
    assert_eq!(
        column_patch["patched_ids"],
        json!(["769c134d-07b8-4225-954a-b6cc5ffc320d"])
    );

    let deleted = clone
        .write(
            "uuid-demo",
            &json!({
                "deletes": ["urn:uuid:769c134d-07b8-4225-954a-b6cc5ffc320f"],
                "return_affected_ids": true
            }),
        )
        .unwrap();
    assert_eq!(
        deleted["deleted_ids"],
        json!(["769c134d-07b8-4225-954a-b6cc5ffc320f"])
    );

    let invalid_existing_write = clone
        .write("uuid-demo", &json!({"deletes": ["not-a-uuid"]}))
        .unwrap_err();
    assert_eq!(
        invalid_existing_write.to_string(),
        "💔 namespace ID type is uuid, but a written ID could not be parsed as uuid"
    );

    let duplicate = clone
        .write(
            "uuid-demo",
            &json!({
                "upsert_rows": [
                    {"id": "769C134D-07B8-4225-954A-B6CC5FFC320C", "title": "dupe-0"},
                    {"id": "769c134d-07b8-4225-954a-b6cc5ffc320c", "title": "dupe-1"}
                ]
            }),
        )
        .unwrap_err();
    assert_eq!(
        duplicate.to_string(),
        "💔 This upsert contains duplicate document IDs and was not written. You should ensure that individual upserts do not include duplicate documents. The duplicated IDs in this batch were the following: 769c134d-07b8-4225-954a-b6cc5ffc320c"
    );
}

#[test]
fn typed_id_schema_rejects_mixed_ids_and_normalizes_uuid_filters() {
    let mut clone = Micropuffer::new();
    let invalid_schema = clone
        .write(
            "id-int",
            &json!({
                "schema": {"id": "int"},
                "upsert_rows": [{"id": 1, "title": "bad"}]
            }),
        )
        .unwrap_err();
    assert_eq!(invalid_schema.to_string(), "💔 int is not a valid ID type");
    assert_eq!(clone.metadata("id-int").unwrap_err().status_code(), 404);

    let string_mismatch = clone
        .write(
            "id-string",
            &json!({
                "schema": {"id": "string", "title": "string"},
                "upsert_rows": [{"id": 1, "title": "bad"}]
            }),
        )
        .unwrap_err();
    assert_eq!(
        string_mismatch.to_string(),
        "💔 namespace ID type is string, but you sent uint; you are not allowed to mix ID types in the same namespace"
    );

    clone
        .write(
            "id-string",
            &json!({
                "schema": {"id": "string", "title": "string"},
                "upsert_rows": [{"id": "abc", "title": "good"}]
            }),
        )
        .unwrap();
    let string_filter_mismatch = clone
        .query(
            "id-string",
            &json!({
                "rank_by": ["id", "asc"],
                "limit": 10,
                "filters": ["id", "Eq", 1]
            }),
        )
        .unwrap_err();
    assert_eq!(
        string_filter_mismatch.to_string(),
        "filter error in key `id`: type mismatch, Eq expects string, but got '1'"
    );

    let uint_mismatch = clone
        .write(
            "id-uint",
            &json!({
                "schema": {"id": "uint", "title": "string"},
                "upsert_rows": [{"id": "abc", "title": "bad"}]
            }),
        )
        .unwrap_err();
    assert_eq!(
        uint_mismatch.to_string(),
        "💔 namespace ID type is uint, but you sent string; you are not allowed to mix ID types in the same namespace"
    );

    clone
        .write(
            "inferred-uint",
            &json!({"upsert_rows": [{"id": 1, "title": "one"}]}),
        )
        .unwrap();
    let inferred_mismatch = clone
        .write(
            "inferred-uint",
            &json!({"upsert_rows": [{"id": "1", "title": "bad"}]}),
        )
        .unwrap_err();
    assert_eq!(
        inferred_mismatch.to_string(),
        "💔 namespace ID type is uint, but you sent string; you are not allowed to mix ID types in the same namespace"
    );

    clone
        .write(
            "id-uuid-filter",
            &json!({
                "schema": {"id": "uuid", "title": "string"},
                "upsert_rows": [
                    {"id": "769c134d-07b8-4225-954a-b6cc5ffc320c", "title": "good"}
                ]
            }),
        )
        .unwrap();
    let uppercase_filter = clone
        .query(
            "id-uuid-filter",
            &json!({
                "rank_by": ["id", "asc"],
                "limit": 10,
                "filters": ["id", "Eq", "769C134D-07B8-4225-954A-B6CC5FFC320C"]
            }),
        )
        .unwrap();
    assert_eq!(
        rows(&uppercase_filter)[0]["id"],
        "769c134d-07b8-4225-954a-b6cc5ffc320c"
    );

    let simple_gte_filter = clone
        .query(
            "id-uuid-filter",
            &json!({
                "rank_by": ["id", "asc"],
                "limit": 10,
                "filters": ["id", "Gte", "769c134d07b84225954ab6cc5ffc320c"]
            }),
        )
        .unwrap();
    assert_eq!(rows(&simple_gte_filter).len(), 1);

    let invalid_uuid_filter = clone
        .query(
            "id-uuid-filter",
            &json!({
                "rank_by": ["id", "asc"],
                "limit": 10,
                "filters": ["id", "Eq", "not-a-uuid"]
            }),
        )
        .unwrap_err();
    assert_eq!(
        invalid_uuid_filter.to_string(),
        "filter error in key `id`: type mismatch, Eq expects uuid, but got 'not-a-uuid'"
    );

    let invalid_uuid_in_filter = clone
        .query(
            "id-uuid-filter",
            &json!({
                "rank_by": ["id", "asc"],
                "limit": 10,
                "filters": [
                    "id",
                    "In",
                    ["not-a-uuid", "769C134D-07B8-4225-954A-B6CC5FFC320C"]
                ]
            }),
        )
        .unwrap_err();
    assert_eq!(
        invalid_uuid_in_filter.to_string(),
        "filter error in key `id`: type mismatch, In expects uuid or []uuid, but got '[not-a-uuid, 769C134D-07B8-4225-954A-B6CC5FFC320C]'"
    );

    let conditioned_upsert = clone
        .write(
            "id-uuid-filter",
            &json!({
                "upsert_rows": [
                    {"id": "769C134D-07B8-4225-954A-B6CC5FFC320C", "title": "conditioned"}
                ],
                "upsert_condition": ["id", "Eq", "769C134D-07B8-4225-954A-B6CC5FFC320C"],
                "return_affected_ids": true
            }),
        )
        .unwrap();
    assert_eq!(
        conditioned_upsert["upserted_ids"],
        json!(["769c134d-07b8-4225-954a-b6cc5ffc320c"])
    );

    let conditioned_patch = clone
        .write(
            "id-uuid-filter",
            &json!({
                "patch_rows": [
                    {"id": "769c134d07b84225954ab6cc5ffc320c", "title": "patched"}
                ],
                "patch_condition": ["id", "Eq", "769c134d07b84225954ab6cc5ffc320c"],
                "return_affected_ids": true
            }),
        )
        .unwrap();
    assert_eq!(
        conditioned_patch["patched_ids"],
        json!(["769c134d-07b8-4225-954a-b6cc5ffc320c"])
    );

    let conditioned_delete = clone
        .write(
            "id-uuid-filter",
            &json!({
                "deletes": ["urn:uuid:769c134d-07b8-4225-954a-b6cc5ffc320c"],
                "delete_condition": ["id", "Eq", "urn:uuid:769c134d-07b8-4225-954a-b6cc5ffc320c"],
                "return_affected_ids": true
            }),
        )
        .unwrap();
    assert_eq!(
        conditioned_delete["deleted_ids"],
        json!(["769c134d-07b8-4225-954a-b6cc5ffc320c"])
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
    assert_eq!(PATCH_BY_FILTER_LIMIT, 50_000);
    const TEST_LIMIT: usize = 3;
    let mut documents = Vec::with_capacity(TEST_LIMIT + 1);
    for index in 0..=TEST_LIMIT {
        documents.push(Document::new(
            Value::Number(Number::from(index as u64)),
            Map::from_iter([
                ("group".to_string(), Value::String("all".to_string())),
                ("patched".to_string(), Value::Bool(false)),
            ]),
        ));
    }
    let mut store = MiniStore {
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
    };
    let request = json!({
        "filters": ["group", "Eq", "all"],
        "patch": {"patched": true}
    });
    let too_many = patch_by_filter_with_limit(
        store.namespace_mut("partial").unwrap(),
        &request,
        false,
        TEST_LIMIT,
    )
    .unwrap_err();
    assert!(too_many.to_string().contains("more documents than allowed"));

    let partial = patch_by_filter_with_limit(
        store.namespace_mut("partial").unwrap(),
        &request,
        true,
        TEST_LIMIT,
    );
    let partial = partial.unwrap();
    assert_eq!(partial.ids.len(), TEST_LIMIT);
    assert!(partial.rows_remaining);
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
