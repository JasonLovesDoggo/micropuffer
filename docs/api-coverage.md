# micropuffer API coverage

This tracks micropuffer against the turbopuffer docs and OpenAPI surface.

Sources checked:

- https://turbopuffer.com/docs/query
- https://turbopuffer.com/docs/write
- https://turbopuffer.com/docs/fts
- https://turbopuffer.com/docs/recall
- https://turbopuffer.com/docs/warm-cache
- https://turbopuffer.com/docs/pinning
- https://github.com/turbopuffer/turbopuffer-openapi/blob/main/openapi.yml

## Covered by live parity tests

- `POST /v2/namespaces/:namespace`
  - row upsert
  - `patch_by_filter`
  - `delete_by_filter`
  - column upserts
  - conditional upserts, patches, and deletes
  - typed `id` filters inside write conditions
  - `return_affected_ids`
  - `copy_from_namespace`
  - `distance_metric` required for first vector writes
  - top-level `distance_metric` requirement when schema `ann.distance_metric` is present
  - explicit dense vector schemas must enable `ann`
  - distance metric mismatch rejection
  - scalar namespace rejection when adding a vector later
  - `schema.id` validation for `string`, `uint`, and `uuid`, including canonical UUID ID normalization and invalid/mixed ID rejection for row, column, patch, and delete writes
- `POST /v2/namespaces/:namespace/query`
  - `ANN`
  - distance metrics: `cosine_distance`, `euclidean`, `euclidean_squared`
  - `kNN`
  - `BM25`
  - BM25 array-query rejection for word-tokenized fields
  - `SparseKNN`
  - order by one attribute
  - order by multiple attributes
  - rank expressions using `Sum`, `Max`, `Product`, `Attribute`, filters-as-scores
  - filters: equality, membership, array containment, numeric comparisons, array comparisons, glob, case-insensitive glob, regex, fuzzy, token filters, boolean combinators
  - typed `id` filter validation and UUID ID filter canonicalization
  - projections with `include_attributes`
  - projections with `include_attributes: true`
  - projections with `include_attributes: false`
  - projections with `include_attributes: []`
  - `include_attributes` missing-attribute rejection
  - projections with `exclude_attributes`
  - `vector_encoding: "base64"` output
  - base64 dense vector input for writes and ANN queries
  - `consistency.level` validation for `strong` and `eventual`
  - `limit.per` for order-by-attribute queries
  - ungrouped count aggregation
  - ungrouped sum aggregation
  - grouped count aggregation
  - grouped aggregation default `top_k`
  - multi-query
- `POST /v1/namespaces/:namespace/_debug/recall`, response-shape parity only
- `POST /v2/namespaces/:namespace/explain_query`, local shape only; live returned `400` for the temp namespace/index state
- deprecated `GET /v1/namespaces/:namespace` columnar export
- `GET /v1/namespaces`
- `GET /v1/namespaces/:namespace/metadata`, schema-shape parity only
- `GET /v1/namespaces/:namespace/schema`, schema-shape parity only
- `POST /v1/namespaces/:namespace/schema`, schema-shape parity only
  - unknown schema option keys are ignored when `type` is present
  - object definitions without `type` return live-style `422` deserialize errors
- `GET /v1/namespaces/:namespace/hint_cache_warm`, status parity only
- path namespace validation for invalid characters and names over 128 characters
- exact status/body parity for include/exclude projection validation
- exact status/body parity for selected consistency validation failures
- exact status/body parity for selected Serde-style query validation failures
- exact status/body parity for missing-namespace query failures
- exact status/body parity for selected write validation failures
  - malformed `upsert_rows`
  - malformed row, column, patch, and delete document IDs
  - missing row `id`
  - malformed `deletes`
  - `copy_from_namespace` mixed with ordinary write fields
  - duplicate document IDs, including duplicate IDs after UUID canonicalization
- exact status/body parity for schema type-change validation, including the `attribute` body field
- exact status/body parity for selected malformed schema update failures
- live-style plain-text HTTP response envelopes for URL-layer namespace validation errors

## Covered by local Rust tests only

- schema get/update helpers
- schema type validation and vector dimensionality checks
- FTS schema knobs: `k1`, `b`, `k3`, `language`, `stemming`, `remove_stopwords`, `ascii_folding`, `case_sensitive`, `max_token_length`, `tokenizer`
- FTS tokenizer modes: `word_v0`, `word_v1`, `word_v2`, `word_v3`, and `pre_tokenized_array`
- non-English stopword removal using the supported TPUF language list
- schema options: `filterable`, `regex`, `glob`, `fuzzy`, `full_text_search`, `ann`, `sparse_knn`
- vector base64 input and output
- export helper filters and projections beyond the deprecated live endpoint
- metadata pinning helper
- namespace delete helper
- WASM `*Response` helpers for HTTP-style status/body mock envelopes
- `Saturate`, `Decay`, and `Dist` rank operators
- current documented filter-write partial limits: 500k rows for `patch_by_filter`, 5M rows for `delete_by_filter`
- recall ground-truth projection
- explain-query plan text

## Known gaps

- live branch parity: the current test key returns `403` for `branch_from_namespace`
- full live `explain_query` parity: the live endpoint returned `400` (`index does not exist, cannot explain`) for the temp namespace
- live pinning parity: micropuffer has a metadata helper, but this is not verified against live pinning because it can have account and billing effects
- exact billing and performance values
- exact async/indexing behavior, including approximate metadata lag
- exact error text and status-code parity for all validation failures; selected query validation failures now expose live-style HTTP response envelopes, but not every write/schema failure has been audited
- exact TPUF tokenizer parity for `word_v0` through `word_v3`; micropuffer models the documented differences, but does not embed TPUF's exact Unicode v10/v16/v17 segmenter tables
- exact stemming implementation parity beyond the shared Snowball language families

## Spec/live mismatches found while testing

- `include_attributes: false` is accepted by live turbopuffer and behaves like omitting `include_attributes`.
- OpenAPI lists BM25 array-token variants, but live turbopuffer currently rejects `["text", "BM25", ["quick", "fish"]]`.
- Ungrouped aggregation rejects `top_k`; grouped aggregation defaults when `top_k` is omitted.
- Live aggregate queries reject the `limit` field before execution; micropuffer rejects it with the same stable field name but does not reproduce Serde's byte-offset wording.
- The query docs imply multiple `aggregate_by` labels can be supplied, but live turbopuffer currently rejects multiple aggregate functions with `💔 aggregate_by currently requires exactly one function`.
- Live schema updates require `type` for object definitions, but ignore unknown option keys when `type` is present.
