use crate::core::tests::*;

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
    let metadata = clone.metadata("pretokenized").unwrap();
    assert_eq!(
        metadata["schema"]["tokens"]["full_text_search"]["language"],
        Value::Null
    );
    assert_eq!(
        metadata["schema"]["tokens"]["full_text_search"]["max_token_length"],
        Value::Null
    );
    assert_eq!(
        metadata["schema"]["tokens"]["full_text_search"]["case_sensitive"],
        true
    );
    let token_filter_hit = clone
        .query(
            "pretokenized",
            &json!({
                "rank_by": ["id", "asc"],
                "filters": ["tokens", "ContainsAllTokens", ["Foo"]],
                "limit": 10
            }),
        )
        .unwrap();
    assert_eq!(rows(&token_filter_hit)[0]["id"], 1);
    let string_filter = clone
        .query(
            "pretokenized",
            &json!({
                "rank_by": ["id", "asc"],
                "filters": ["tokens", "ContainsAllTokens", "Foo"],
                "limit": 10
            }),
        )
        .unwrap_err();
    assert_eq!(
        string_filter.to_string(),
        "filter error in key `tokens`: type mismatch, ContainsAllTokens expects []string, but got 'Foo'"
    );
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
