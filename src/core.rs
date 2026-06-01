use crate::QueryError;
use base64::{Engine, engine::general_purpose::STANDARD};
use chrono::{DateTime, NaiveDate, Utc};
use deunicode::deunicode;
use globset::GlobBuilder;
use regex::Regex;
use rust_stemmers::{Algorithm, Stemmer};
use serde::ser::SerializeMap;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Number, Value, json};
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::{Mutex, MutexGuard};
use unicode_segmentation::UnicodeSegmentation;

const PATCH_BY_FILTER_LIMIT: usize = 50_000;
const DELETE_BY_FILTER_LIMIT: usize = 5_000_000;

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct MiniStore {
    pub namespaces: Vec<Namespace>,
}

impl MiniStore {
    pub fn namespace(&self, name: &str) -> Result<&Namespace, QueryError> {
        self.namespaces
            .iter()
            .find(|namespace| namespace.name == name)
            .ok_or_else(|| namespace_not_found(name))
    }

    fn namespace_mut(&mut self, name: &str) -> Option<&mut Namespace> {
        self.namespaces
            .iter_mut()
            .find(|namespace| namespace.name == name)
    }
}

#[derive(Debug, Clone, Default)]
pub struct Micropuffer {
    store: MiniStore,
}

impl Micropuffer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_store(store: MiniStore) -> Self {
        Self { store }
    }

    pub fn store(&self) -> &MiniStore {
        &self.store
    }

    pub fn query(&self, namespace_name: &str, request: &Value) -> Result<Value, QueryError> {
        query_store(&self.store, namespace_name, request)
    }

    pub fn write(&mut self, namespace_name: &str, request: &Value) -> Result<Value, QueryError> {
        write_store(&mut self.store, namespace_name, request)
    }

    pub fn delete_namespace(&mut self, namespace_name: &str) -> Result<Value, QueryError> {
        validate_namespace_name(namespace_name)?;
        let Some(index) = self
            .store
            .namespaces
            .iter()
            .position(|namespace| namespace.name == namespace_name)
        else {
            return Err(namespace_not_found(namespace_name));
        };
        self.store.namespaces.remove(index);
        Ok(json!({ "status": "OK" }))
    }

    pub fn list_namespaces(
        &self,
        prefix: Option<&str>,
        cursor: Option<&str>,
        page_size: usize,
    ) -> Result<Value, QueryError> {
        let page_size = page_size.clamp(1, 1000);
        let start = cursor
            .map(|cursor| {
                cursor
                    .parse::<usize>()
                    .map_err(|_| QueryError::new("cursor must be a numeric offset."))
            })
            .transpose()?
            .unwrap_or(0);
        let mut names = self
            .store
            .namespaces
            .iter()
            .map(|namespace| namespace.name.as_str())
            .filter(|name| {
                prefix
                    .map(|prefix| name.starts_with(prefix))
                    .unwrap_or(true)
            })
            .collect::<Vec<_>>();
        names.sort_unstable();
        let page = names
            .iter()
            .skip(start)
            .take(page_size)
            .map(|name| json!({ "id": name }))
            .collect::<Vec<_>>();
        let next = start + page_size;
        let mut response = Map::new();
        response.insert("namespaces".to_string(), Value::Array(page));
        if next < names.len() {
            response.insert("next_cursor".to_string(), Value::String(next.to_string()));
        }
        Ok(Value::Object(response))
    }

    pub fn metadata(&self, namespace_name: &str) -> Result<Value, QueryError> {
        validate_namespace_name(namespace_name)?;
        namespace_metadata(self.store.namespace(namespace_name)?)
    }

    pub fn schema(&self, namespace_name: &str) -> Result<Value, QueryError> {
        validate_namespace_name(namespace_name)?;
        namespace_schema(self.store.namespace(namespace_name)?)
    }

    pub fn update_schema(
        &mut self,
        namespace_name: &str,
        request: &Value,
    ) -> Result<Value, QueryError> {
        update_namespace_schema(&mut self.store, namespace_name, request)
    }

    pub fn patch_metadata(
        &mut self,
        namespace_name: &str,
        request: &Value,
    ) -> Result<Value, QueryError> {
        patch_namespace_metadata(&mut self.store, namespace_name, request)
    }

    pub fn export_namespace(
        &self,
        namespace_name: &str,
        request: &Value,
    ) -> Result<Value, QueryError> {
        validate_namespace_name(namespace_name)?;
        export_namespace(self.store.namespace(namespace_name)?, request)
    }

    pub fn warm_cache(&self, namespace_name: &str) -> Result<Value, QueryError> {
        validate_namespace_name(namespace_name)?;
        self.store.namespace(namespace_name)?;
        Ok(json!({
            "status": "ACCEPTED",
            "message": "micropuffer cache is already memory-resident"
        }))
    }

    pub fn recall(&self, namespace_name: &str, request: &Value) -> Result<Value, QueryError> {
        validate_namespace_name(namespace_name)?;
        recall_namespace(self.store.namespace(namespace_name)?, request)
    }

    pub fn explain_query(
        &self,
        namespace_name: &str,
        request: &Value,
    ) -> Result<Value, QueryError> {
        validate_namespace_name(namespace_name)?;
        explain_query(self.store.namespace(namespace_name)?, request)
    }
}

fn namespace_not_found(name: &str) -> QueryError {
    QueryError::not_found(format!("🤷 namespace '{name}' was not found"))
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Namespace {
    pub name: String,
    #[serde(default)]
    pub distance_metric: DistanceMetric,
    #[serde(default)]
    pub schema: Map<String, Value>,
    #[serde(default = "default_created_at")]
    pub created_at: String,
    #[serde(default)]
    pub last_write_at: Option<String>,
    #[serde(default = "default_created_at")]
    pub updated_at: String,
    #[serde(default = "default_encryption")]
    pub encryption: Value,
    #[serde(default)]
    pub pinning: Option<Value>,
    #[serde(default)]
    pub branching_parent: Option<String>,
    pub documents: Vec<Document>,
    #[serde(skip, default)]
    query_indexes: NamespaceQueryIndexCache,
    #[serde(skip)]
    logical_bytes_cache: NamespaceLogicalBytesCache,
    #[serde(skip)]
    fts_index_cache: NamespaceFtsIndexCache,
}

impl Namespace {
    fn invalidate_logical_bytes(&self) {
        self.logical_bytes_cache.clear();
    }

    fn invalidate_fts_indexes(&self) {
        self.fts_index_cache.clear();
    }

    fn invalidate_query_caches(&self) {
        self.invalidate_logical_bytes();
        self.invalidate_fts_indexes();
    }

    fn set_cached_logical_bytes(&self, bytes: usize) {
        self.logical_bytes_cache.set(bytes);
    }

    #[cfg(test)]
    fn logical_bytes_recompute_count(&self) -> usize {
        self.logical_bytes_cache.recompute_count()
    }

    #[cfg(test)]
    fn has_cached_logical_bytes(&self) -> bool {
        self.logical_bytes_cache.get().is_some()
    }
}

#[derive(Debug, Default)]
struct NamespaceFtsIndexCache {
    fields: Mutex<HashMap<String, FtsFieldIndex>>,
}

impl Clone for NamespaceFtsIndexCache {
    fn clone(&self) -> Self {
        Self::default()
    }
}

impl NamespaceFtsIndexCache {
    fn clear(&self) {
        self.fields_guard().clear();
    }

    fn fields_guard(&self) -> MutexGuard<'_, HashMap<String, FtsFieldIndex>> {
        match self.fields.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}

#[derive(Debug, Default)]
struct NamespaceLogicalBytesCache {
    bytes: Mutex<Option<usize>>,
    #[cfg(test)]
    recompute_count: Mutex<usize>,
}

impl Clone for NamespaceLogicalBytesCache {
    fn clone(&self) -> Self {
        Self {
            bytes: Mutex::new(self.get()),
            #[cfg(test)]
            recompute_count: Mutex::new(self.recompute_count()),
        }
    }
}

impl NamespaceLogicalBytesCache {
    fn get(&self) -> Option<usize> {
        *self.bytes_guard()
    }

    fn set(&self, bytes: usize) {
        *self.bytes_guard() = Some(bytes);
    }

    fn clear(&self) {
        *self.bytes_guard() = None;
    }

    fn bytes_guard(&self) -> MutexGuard<'_, Option<usize>> {
        match self.bytes.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    #[cfg(test)]
    fn record_recompute(&self) {
        let mut recompute_count = self.recompute_count_guard();
        *recompute_count = (*recompute_count).saturating_add(1);
    }

    #[cfg(test)]
    fn recompute_count(&self) -> usize {
        *self.recompute_count_guard()
    }

    #[cfg(test)]
    fn recompute_count_guard(&self) -> MutexGuard<'_, usize> {
        match self.recompute_count.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}

fn default_created_at() -> String {
    "1970-01-01T00:00:00Z".to_string()
}

fn default_encryption() -> Value {
    json!({ "sse": true })
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DistanceMetric {
    CosineDistance,
    Euclidean,
    #[default]
    EuclideanSquared,
}

impl DistanceMetric {
    fn parse(value: &Value) -> Result<Self, QueryError> {
        match value.as_str() {
            Some("cosine_distance") => Ok(Self::CosineDistance),
            Some("euclidean") => Ok(Self::Euclidean),
            Some("euclidean_squared") => Ok(Self::EuclideanSquared),
            Some(metric) => Err(QueryError::unprocessable(format!(
                "Failed to deserialize the JSON body into the target type: distance_metric: unknown variant `{metric}`, expected one of `Unknown`, `euclidean_squared`, `cosine_distance`, `euclidean`, `Query`"
            ))),
            _ => Err(QueryError::unprocessable(
                "Failed to deserialize the JSON body into the target type: distance_metric: invalid type, expected `cosine_distance`, `euclidean`, or `euclidean_squared`",
            )),
        }
    }

    fn as_write_error_str(self) -> &'static str {
        match self {
            Self::CosineDistance => "cosine_distance",
            Self::Euclidean => "euclidean",
            Self::EuclideanSquared => "euclidean_squared",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Document {
    pub id: Value,
    pub attributes: Map<String, Value>,
    pub typed: TypedDocumentFields,
}

#[derive(Debug, Clone, Default)]
pub struct TypedDocumentFields {
    vector: Option<Vec<f64>>,
}

impl Document {
    pub fn new(id: Value, attributes: Map<String, Value>) -> Self {
        let typed = TypedDocumentFields::from_attributes(&attributes);
        Self {
            id,
            attributes,
            typed,
        }
    }

    fn set_attribute(&mut self, key: String, value: Value) {
        self.typed.set_attribute(&key, &value);
        self.attributes.insert(key, value);
    }
}

impl Serialize for Document {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut len = self.attributes.len() + 1;
        if self.typed.vector.is_some() && !self.attributes.contains_key("vector") {
            len += 1;
        }
        let mut map = serializer.serialize_map(Some(len))?;
        map.serialize_entry("id", &self.id)?;
        if let Some(vector) = &self.typed.vector
            && !self.attributes.contains_key("vector")
        {
            map.serialize_entry("vector", vector)?;
        }
        for (key, value) in &self.attributes {
            map.serialize_entry(key, value)?;
        }
        map.end()
    }
}

impl<'de> Deserialize<'de> for Document {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct DocumentWire {
            id: Value,
            #[serde(flatten)]
            attributes: Map<String, Value>,
        }

        let wire = DocumentWire::deserialize(deserializer)?;
        Ok(Document::new(wire.id, wire.attributes))
    }
}

impl TypedDocumentFields {
    fn from_attributes(attributes: &Map<String, Value>) -> Self {
        let mut typed = Self::default();
        for (key, value) in attributes {
            typed.set_attribute(key, value);
        }
        typed
    }

    fn set_attribute(&mut self, key: &str, value: &Value) {
        if key == "vector" {
            self.vector = None;
            if let Some(vector) = dense_vector_from_value(value) {
                self.vector = Some(vector);
            }
        }
    }

    fn dense_vector(&self, key: &str) -> Option<&[f64]> {
        if key == "vector" {
            return self.vector.as_deref();
        }
        None
    }
}

#[derive(Debug, Clone, Default)]
struct NamespaceQueryIndexes {
    equality: HashMap<String, EqualityAttributeIndex>,
    order: HashMap<OrderIndexKey, Vec<OrderIndexEntry>>,
    group_counts: HashMap<String, GroupCountIndex>,
    sparse_vectors: HashMap<String, SparseVectorIndex>,
}

impl NamespaceQueryIndexes {
    fn clear(&mut self) {
        self.equality.clear();
        self.order.clear();
        self.group_counts.clear();
        self.sparse_vectors.clear();
    }

    fn is_empty(&self) -> bool {
        self.equality.is_empty()
            && self.order.is_empty()
            && self.group_counts.is_empty()
            && self.sparse_vectors.is_empty()
    }
}

#[derive(Debug, Default)]
struct NamespaceQueryIndexCache {
    indexes: Mutex<NamespaceQueryIndexes>,
}

impl Clone for NamespaceQueryIndexCache {
    fn clone(&self) -> Self {
        Self {
            indexes: Mutex::new(self.guard().clone()),
        }
    }
}

impl NamespaceQueryIndexCache {
    fn guard(&self) -> MutexGuard<'_, NamespaceQueryIndexes> {
        match self.indexes.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}

#[derive(Debug, Clone, Default)]
struct EqualityAttributeIndex {
    postings: HashMap<ScalarEqKey, BTreeSet<usize>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum ScalarEqKey {
    Null,
    Bool(bool),
    Number(u64),
    String(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct OrderIndexKey(Vec<(String, SortDirection)>);

#[derive(Debug, Clone)]
struct OrderIndexEntry {
    doc_index: usize,
    id: Value,
    values: Vec<Value>,
}

#[derive(Debug, Clone, Default)]
struct GroupCountIndex {
    counts: HashMap<ScalarEqKey, GroupCountEntry>,
}

#[derive(Debug, Clone)]
struct GroupCountEntry {
    value: Value,
    count: usize,
}

#[derive(Debug, Clone, Default)]
struct SparseVectorIndex {
    postings: HashMap<String, Vec<SparsePosting>>,
}

#[derive(Debug, Clone)]
struct SparsePosting {
    doc_index: usize,
    value: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum BorrowedScalarKey<'a> {
    Null,
    Bool(bool),
    Number(u64),
    String(&'a str),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum SortDirection {
    Asc,
    Desc,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum RankKind {
    SmallerIsBetter,
    LargerIsBetter,
    AttributeOrder {
        attribute: String,
        direction: SortDirection,
    },
    MultiAttributeOrder {
        attributes: Vec<(String, SortDirection)>,
    },
}

#[derive(Debug, Clone)]
struct RankedDocument<'a> {
    doc: &'a Document,
    score: f64,
}

#[derive(Debug, Clone)]
struct QueryOptions {
    vector_encoding: VectorEncoding,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VectorEncoding {
    Float,
    Base64,
}

#[derive(Debug, Clone)]
struct Limit {
    total: usize,
    per: Option<PerLimit>,
}

#[derive(Debug, Clone)]
struct PerLimit {
    attributes: Vec<String>,
    limit: usize,
}

#[derive(Debug, Clone)]
struct RankPlan<'a> {
    rank_by: &'a Value,
    kind: RankKind,
}

#[derive(Debug, Clone)]
struct PreparedRankPlan<'a> {
    kind: RankKind,
    expression: Option<PreparedRankExpr<'a>>,
}

#[derive(Debug, Clone)]
enum PreparedRankExpr<'a> {
    Literal(f64),
    Sum(Vec<PreparedRankExpr<'a>>),
    Max(Vec<PreparedRankExpr<'a>>),
    Product {
        weight: f64,
        expression: Box<PreparedRankExpr<'a>>,
    },
    Attribute(&'a str),
    Saturate {
        expression: Box<PreparedRankExpr<'a>>,
        midpoint: f64,
        exponent: f64,
    },
    Decay {
        expression: Box<PreparedRankExpr<'a>>,
        midpoint: f64,
        exponent: f64,
    },
    Dist {
        expression: PreparedDistExpr<'a>,
        origin: &'a Value,
    },
    DenseDistance {
        attribute: &'a str,
        query: PreparedDenseQuery,
    },
    DenseDistanceVector {
        query: PreparedDenseQuery,
    },
    SparseDotProduct {
        attribute: &'a str,
        query: HashMap<String, f64>,
    },
    Bm25 {
        scores: PreparedBm25Scores,
    },
    Filter(&'a Value),
}

#[derive(Debug, Clone)]
enum PreparedDistExpr<'a> {
    Attribute(&'a str),
    Rank(Box<PreparedRankExpr<'a>>),
}

#[derive(Debug, Clone)]
struct PreparedDenseQuery {
    values: Vec<f64>,
    norm: f64,
}

#[derive(Debug, Clone)]
struct PreparedBm25Query {
    terms: Vec<PreparedBm25Term>,
}

#[derive(Debug, Clone)]
struct PreparedBm25Term {
    token: String,
    query_frequency: usize,
    prefix: bool,
}

#[derive(Debug, Clone)]
struct PreparedBm25Scores {
    by_doc: HashMap<usize, f64>,
    ranked: Vec<(usize, f64)>,
}

#[derive(Debug, Clone)]
struct FtsFieldIndex {
    doc_count: usize,
    avg_len: f64,
    doc_freqs: HashMap<String, usize>,
    doc_lengths: Vec<usize>,
    postings: HashMap<String, Vec<FtsPosting>>,
    config: FtsConfig,
}

#[derive(Debug, Clone)]
struct FtsPosting {
    doc_index: usize,
    term_frequency: usize,
}

impl PreparedBm25Scores {
    fn new(ranked: Vec<(usize, f64)>) -> Self {
        let by_doc = ranked.iter().copied().collect();
        Self { by_doc, ranked }
    }

    fn score(&self, doc_index: usize) -> f64 {
        self.by_doc.get(&doc_index).copied().unwrap_or(0.0)
    }
}

impl FtsFieldIndex {
    fn build(namespace: &Namespace, field: &str, config: FtsConfig) -> Self {
        let mut postings_by_doc: HashMap<String, HashMap<usize, usize>> = HashMap::new();
        let mut doc_lengths = vec![0; namespace.documents.len()];
        let mut doc_count = 0usize;
        let mut total_len = 0usize;

        for (doc_index, document) in namespace.documents.iter().enumerate() {
            let tokens = string_attr_tokens_with_config(document, field, &config);
            if tokens.is_empty() {
                continue;
            }
            doc_count += 1;
            total_len += tokens.len();
            doc_lengths[doc_index] = tokens.len();
            for token in tokens {
                let per_doc = postings_by_doc.entry(token).or_default();
                let count = per_doc.entry(doc_index).or_insert(0);
                *count += 1;
            }
        }

        let mut postings = HashMap::with_capacity(postings_by_doc.len());
        let mut doc_freqs = HashMap::with_capacity(postings_by_doc.len());
        for (token, per_doc) in postings_by_doc {
            doc_freqs.insert(token.clone(), per_doc.len());
            let mut token_postings = per_doc
                .into_iter()
                .map(|(doc_index, term_frequency)| FtsPosting {
                    doc_index,
                    term_frequency,
                })
                .collect::<Vec<_>>();
            token_postings.sort_by_key(|posting| posting.doc_index);
            postings.insert(token, token_postings);
        }

        let avg_len = if doc_count == 0 {
            0.0
        } else {
            total_len as f64 / doc_count as f64
        };
        Self {
            doc_count,
            avg_len,
            doc_freqs,
            doc_lengths,
            postings,
            config,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FtsConfig {
    k1_millis: u64,
    b_millis: u64,
    k3_millis: u64,
    language: FtsLanguage,
    stemming: bool,
    remove_stopwords: bool,
    ascii_folding: bool,
    case_sensitive: bool,
    max_token_length: usize,
    tokenizer: Tokenizer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FtsLanguage {
    Arabic,
    Danish,
    Dutch,
    English,
    Finnish,
    French,
    German,
    Greek,
    Hungarian,
    Italian,
    Norwegian,
    Portuguese,
    Romanian,
    Russian,
    Spanish,
    Swedish,
    Tamil,
    Turkish,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tokenizer {
    PreTokenizedArray,
    WordV0,
    WordV1,
    WordV2,
    WordV3,
}

impl Default for FtsConfig {
    fn default() -> Self {
        Self {
            k1_millis: 1_200,
            b_millis: 750,
            k3_millis: 8_000,
            language: FtsLanguage::English,
            stemming: false,
            remove_stopwords: false,
            ascii_folding: false,
            case_sensitive: false,
            max_token_length: 39,
            tokenizer: Tokenizer::WordV3,
        }
    }
}

impl FtsConfig {
    fn k1(&self) -> f64 {
        self.k1_millis as f64 / 1_000.0
    }

    fn b(&self) -> f64 {
        self.b_millis as f64 / 1_000.0
    }

    fn k3(&self) -> f64 {
        self.k3_millis as f64 / 1_000.0
    }
}

pub fn query_store(
    store: &MiniStore,
    namespace_name: &str,
    request: &Value,
) -> Result<Value, QueryError> {
    validate_namespace_name(namespace_name)?;
    let namespace = store.namespace(namespace_name)?;
    query_namespace(namespace, request)
}

pub fn namespace_metadata(namespace: &Namespace) -> Result<Value, QueryError> {
    let mut response = Map::new();
    let schema = schema_with_id(namespace)
        .into_iter()
        .map(|(attribute, definition)| {
            normalize_metadata_schema_definition(namespace, &attribute, &definition)
        })
        .collect::<Result<Map<_, _>, _>>()?;
    response.insert("schema".to_string(), Value::Object(schema));
    response.insert(
        "approx_logical_bytes".to_string(),
        Value::Number(Number::from(rough_namespace_bytes(namespace))),
    );
    response.insert(
        "approx_row_count".to_string(),
        Value::Number(Number::from(namespace.documents.len())),
    );
    response.insert(
        "created_at".to_string(),
        Value::String(namespace.created_at.clone()),
    );
    if let Some(last_write_at) = &namespace.last_write_at {
        response.insert(
            "last_write_at".to_string(),
            Value::String(last_write_at.clone()),
        );
    }
    response.insert(
        "updated_at".to_string(),
        Value::String(namespace.updated_at.clone()),
    );
    response.insert("encryption".to_string(), namespace.encryption.clone());
    response.insert(
        "index".to_string(),
        json!({
            "status": "up-to-date"
        }),
    );
    if let Some(pinning) = &namespace.pinning {
        response.insert("pinning".to_string(), pinning.clone());
    }
    if let Some(parent) = &namespace.branching_parent {
        response.insert("branching".to_string(), json!({ "parent": parent }));
    }
    Ok(Value::Object(response))
}

pub fn namespace_schema(namespace: &Namespace) -> Result<Value, QueryError> {
    let schema = schema_with_id(namespace)
        .into_iter()
        .map(|(attribute, definition)| {
            normalize_schema_response_definition(&attribute, &definition)
        })
        .collect::<Result<Map<_, _>, _>>()?;
    Ok(Value::Object(schema))
}

pub fn update_namespace_schema(
    store: &mut MiniStore,
    namespace_name: &str,
    request: &Value,
) -> Result<Value, QueryError> {
    validate_namespace_name(namespace_name)?;
    let schema = as_object(request, "schema update request")?;
    let namespace = store
        .namespace_mut(namespace_name)
        .ok_or_else(|| namespace_not_found(namespace_name))?;
    merge_schema(namespace, schema)?;
    namespace.invalidate_fts_indexes();
    namespace.updated_at = logical_now();
    namespace_schema(namespace)
}

pub fn patch_namespace_metadata(
    store: &mut MiniStore,
    namespace_name: &str,
    request: &Value,
) -> Result<Value, QueryError> {
    validate_namespace_name(namespace_name)?;
    let request = as_object(request, "metadata patch request")?;
    let namespace = store
        .namespace_mut(namespace_name)
        .ok_or_else(|| namespace_not_found(namespace_name))?;
    if let Some(pinning) = request.get("pinning") {
        namespace.pinning = parse_pinning(pinning)?;
    }
    namespace.updated_at = logical_now();
    namespace_metadata(namespace)
}

pub fn export_namespace(namespace: &Namespace, request: &Value) -> Result<Value, QueryError> {
    let object = as_object(request, "export request")?;
    let mut query = Map::new();
    query.insert("rank_by".to_string(), json!(["id", "asc"]));
    query.insert(
        "limit".to_string(),
        object.get("limit").cloned().unwrap_or_else(|| json!(1000)),
    );
    if let Some(filters) = object.get("filters") {
        query.insert("filters".to_string(), filters.clone());
    }
    match (
        object.get("include_attributes"),
        object.get("exclude_attributes"),
    ) {
        (Some(include_attributes), None) => {
            query.insert("include_attributes".to_string(), include_attributes.clone());
        }
        (None, Some(exclude_attributes)) => {
            query.insert("exclude_attributes".to_string(), exclude_attributes.clone());
        }
        (None, None) => {
            query.insert("include_attributes".to_string(), Value::Bool(true));
        }
        (Some(_), Some(_)) => {
            return Err(QueryError::new(
                "💔 cannot specify both include_attributes and exclude_attributes",
            ));
        }
    }
    if let Some(vector_encoding) = object.get("vector_encoding") {
        query.insert("vector_encoding".to_string(), vector_encoding.clone());
    }
    let query_result = query_namespace(namespace, &Value::Object(query))?;
    columnar_export_from_query_result(&query_result)
}

fn columnar_export_from_query_result(query_result: &Value) -> Result<Value, QueryError> {
    let rows = query_result
        .get("rows")
        .and_then(Value::as_array)
        .ok_or_else(|| QueryError::new("export query did not return rows."))?;
    let mut ids = Vec::with_capacity(rows.len());
    let mut vectors = Vec::with_capacity(rows.len());
    let mut attribute_names = BTreeSet::new();
    let mut row_objects = Vec::with_capacity(rows.len());

    for row in rows {
        let row_object = as_object(row, "export row")?;
        ids.push(
            row_object
                .get("id")
                .cloned()
                .ok_or_else(|| QueryError::new("export row is missing id."))?,
        );
        vectors.push(row_object.get("vector").cloned().unwrap_or(Value::Null));
        for key in row_object.keys() {
            if key != "id" && key != "vector" && key != "$dist" {
                attribute_names.insert(key.clone());
            }
        }
        row_objects.push(row_object);
    }

    let mut attributes = Map::new();
    for attribute in attribute_names {
        let values = row_objects
            .iter()
            .map(|row| row.get(&attribute).cloned().unwrap_or(Value::Null))
            .collect::<Vec<_>>();
        attributes.insert(attribute, Value::Array(values));
    }

    Ok(json!({
        "ids": ids,
        "vectors": vectors,
        "attributes": attributes,
        "next_cursor": null,
        "message": null
    }))
}

pub fn recall_namespace(namespace: &Namespace, request: &Value) -> Result<Value, QueryError> {
    let object = as_object(request, "recall request")?;
    let num = object
        .get("num")
        .and_then(value_as_u64)
        .and_then(|value| usize::try_from(value).ok())
        .unwrap_or(10)
        .clamp(1, 100);
    let top_k = object
        .get("top_k")
        .and_then(value_as_u64)
        .and_then(|value| usize::try_from(value).ok())
        .unwrap_or(10)
        .clamp(1, 10_000);
    let filters = object.get("filters");
    let include_ground_truth = object
        .get("include_ground_truth")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let candidates = namespace
        .documents
        .iter()
        .filter(|document| {
            filters
                .map(|filter| {
                    eval_filter_with_schema(document, filter, Some(&namespace.schema))
                        .unwrap_or(false)
                })
                .unwrap_or(true)
        })
        .filter(|document| {
            document.typed.vector.is_some() || document.attributes.get("vector").is_some()
        })
        .collect::<Vec<_>>();
    let sample = candidates.iter().take(num).copied().collect::<Vec<_>>();
    let mut ground_truth = Vec::new();
    for query_document in &sample {
        let query_vector = document_vector_value(query_document)
            .ok_or_else(|| QueryError::new("recall query document is missing vector."))?;
        let mut neighbors = candidates
            .iter()
            .map(|document| {
                dense_distance(document, "vector", &query_vector, namespace.distance_metric)
                    .map(|score| (*document, score))
            })
            .collect::<Result<Vec<_>, _>>()?;
        neighbors.sort_by(|left, right| {
            compare_f64(left.1, right.1).then_with(|| stable_id_compare(&left.0.id, &right.0.id))
        });
        let nearest = neighbors
            .into_iter()
            .take(top_k)
            .map(|(document, score)| project_recall_neighbor(document, score))
            .collect::<Vec<_>>();
        ground_truth.push(json!({
            "query_vector": query_vector,
            "nearest_neighbors": nearest
        }));
    }
    let avg_count = candidates.len().min(top_k) as f64;
    let mut response = Map::new();
    response.insert("avg_recall".to_string(), json!(1.0));
    response.insert(
        "avg_exhaustive_count".to_string(),
        number_value(candidates.len() as f64),
    );
    response.insert("avg_ann_count".to_string(), number_value(avg_count));
    if include_ground_truth {
        response.insert("ground_truth".to_string(), Value::Array(ground_truth));
    }
    Ok(Value::Object(response))
}

fn project_recall_neighbor(document: &Document, score: f64) -> Value {
    let mut row = Map::new();
    row.insert("$dist".to_string(), number_value(score));
    row.insert("id".to_string(), document.id.clone());
    if let Some(vector) = document_vector_value(document) {
        row.insert("vector".to_string(), vector);
    }
    Value::Object(row)
}

fn document_vector_value(document: &Document) -> Option<Value> {
    document
        .typed
        .vector
        .as_ref()
        .map(|vector| Value::Array(vector.iter().copied().map(number_value).collect()))
        .or_else(|| document.attributes.get("vector").cloned())
}

pub fn explain_query(namespace: &Namespace, request: &Value) -> Result<Value, QueryError> {
    let object = as_object(request, "query request")?;
    if object.contains_key("queries") {
        validate_multi_query_root(object)?;
    } else if object.contains_key("aggregate_by") {
        if let Some(filters) = object.get("filters") {
            for document in &namespace.documents {
                eval_filter_with_schema(document, filters, Some(&namespace.schema))?;
            }
        }
    } else {
        let rank_by = object
            .get("rank_by")
            .ok_or_else(|| QueryError::new("rank_by is required unless aggregate_by is set."))?;
        parse_rank_plan(rank_by, object.get("filters").is_some())?;
        parse_limit(object)?;
    }
    let mut plan = Vec::new();
    plan.push(format!("namespace={}", namespace.name));
    if object.contains_key("queries") {
        plan.push("operation=multi_query".to_string());
    } else if object.contains_key("aggregate_by") {
        plan.push("operation=aggregate".to_string());
    } else {
        plan.push("operation=query".to_string());
    }
    if object.contains_key("filters") {
        plan.push("filters=enabled".to_string());
    }
    if let Some(limit) = object.get("limit").or_else(|| object.get("top_k")) {
        plan.push(format!("limit={limit}"));
    }
    Ok(json!({ "plan_text": plan.join("\n") }))
}

pub fn write_store(
    store: &mut MiniStore,
    namespace_name: &str,
    request: &Value,
) -> Result<Value, QueryError> {
    validate_namespace_name(namespace_name)?;
    let request = as_object(request, "write request")?;
    if request.contains_key("copy_from_namespace") && request.contains_key("branch_from_namespace")
    {
        return Err(QueryError::new(
            "copy_from_namespace and branch_from_namespace cannot be specified together.",
        ));
    }
    if let Some(copy_from) = request.get("copy_from_namespace") {
        validate_copy_request(request)?;
        return copy_namespace(
            store,
            namespace_name,
            copy_from,
            false,
            request.get("encryption"),
        );
    }
    if let Some(branch_from) = request.get("branch_from_namespace") {
        validate_branch_request(request)?;
        return copy_namespace(store, namespace_name, branch_from, true, None);
    }
    let parsed_distance_metric = request
        .get("distance_metric")
        .map(DistanceMetric::parse)
        .transpose()?;
    let mut upserts = collect_write_rows(
        "upsert_rows",
        request.get("upsert_rows"),
        "upsert_columns",
        request.get("upsert_columns"),
    )?;
    let mut patches = collect_write_rows(
        "patch_rows",
        request.get("patch_rows"),
        "patch_columns",
        request.get("patch_columns"),
    )?;
    let mut deletes = collect_delete_ids(request.get("deletes"))?;
    let id_schema_type = effective_write_id_schema_type(
        store.namespace(namespace_name).ok(),
        request.get("schema"),
    )?;
    normalize_write_ids_against_id_schema(
        id_schema_type.as_deref(),
        &mut upserts,
        &mut patches,
        &mut deletes,
    )?;
    reject_duplicate_write_ids(&upserts, &patches, &deletes)?;
    validate_distance_metric_for_write(
        store.namespace(namespace_name).ok(),
        parsed_distance_metric,
        request.get("schema"),
        &upserts,
    )?;
    ensure_namespace(store, namespace_name);
    if let Some(metric) = parsed_distance_metric {
        let namespace = store
            .namespace_mut(namespace_name)
            .ok_or_else(|| QueryError::new("namespace disappeared during write."))?;
        namespace.distance_metric = metric;
    }
    if let Some(schema) = request.get("schema") {
        let namespace = store
            .namespace_mut(namespace_name)
            .ok_or_else(|| QueryError::new("namespace disappeared during write."))?;
        merge_schema(namespace, as_object(schema, "schema")?)?;
        namespace.invalidate_fts_indexes();
        namespace.updated_at = logical_now();
    }
    if let Some(encryption) = request.get("encryption") {
        let namespace = store
            .namespace_mut(namespace_name)
            .ok_or_else(|| QueryError::new("namespace disappeared during write."))?;
        namespace.encryption = encryption.clone();
        namespace.updated_at = logical_now();
    }
    let mut summary = WriteSummary {
        upsert_requested: request.contains_key("upsert_rows")
            || request.contains_key("upsert_columns"),
        patch_requested: request.contains_key("patch_rows")
            || request.contains_key("patch_columns")
            || request.contains_key("patch_by_filter"),
        delete_requested: request.contains_key("deletes")
            || request.contains_key("delete_by_filter"),
        query_billing_requested: request.contains_key("upsert_condition")
            || request.contains_key("patch_condition")
            || request.contains_key("delete_condition")
            || request.contains_key("patch_by_filter")
            || request.contains_key("delete_by_filter"),
        ..WriteSummary::default()
    };

    if let Some(filter) = request.get("delete_by_filter") {
        let allow_partial = request
            .get("delete_by_filter_allow_partial")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let namespace = store
            .namespace_mut(namespace_name)
            .ok_or_else(|| QueryError::new("namespace disappeared during write."))?;
        let outcome = delete_by_filter(namespace, filter, allow_partial)?;
        summary.rows_remaining |= outcome.rows_remaining;
        summary.deleted_ids.extend(outcome.ids);
    }

    if let Some(patch_by_filter_request) = request.get("patch_by_filter") {
        let allow_partial = request
            .get("patch_by_filter_allow_partial")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let namespace = store
            .namespace_mut(namespace_name)
            .ok_or_else(|| QueryError::new("namespace disappeared during write."))?;
        let outcome = patch_by_filter(namespace, patch_by_filter_request, allow_partial)?;
        summary.rows_remaining |= outcome.rows_remaining;
        summary.patched_ids.extend(outcome.ids);
    }

    let upsert_condition = request.get("upsert_condition");
    let patch_condition = request.get("patch_condition");
    let delete_condition = request.get("delete_condition");
    let namespace = store
        .namespace_mut(namespace_name)
        .ok_or_else(|| QueryError::new("namespace disappeared during write."))?;
    validate_write_rows_against_schema(namespace, &upserts, &patches)?;
    normalize_write_rows_against_schema(namespace, &mut upserts, &mut patches)?;
    let mut write_id_index = WriteIdIndex::new(namespace);
    summary.deleted_ids.extend(delete_documents(
        namespace,
        deletes,
        delete_condition,
        &mut write_id_index,
    )?);
    for row in patches {
        summary.billable_logical_bytes_written += write_row_logical_bytes(&row);
        let id = row.id.clone();
        if patch_document(namespace, row, patch_condition, &write_id_index)? {
            summary.patched_ids.push(id);
        }
    }
    for row in upserts {
        summary.billable_logical_bytes_written += write_row_logical_bytes(&row);
        let id = row.id.clone();
        if upsert_document(namespace, row, upsert_condition, &mut write_id_index)? {
            summary.upserted_ids.push(id);
        }
    }
    refresh_inferred_schema(namespace)?;
    if !summary.upserted_ids.is_empty()
        || !summary.patched_ids.is_empty()
        || !summary.deleted_ids.is_empty()
    {
        namespace.invalidate_query_caches();
        let now = logical_now();
        namespace.last_write_at = Some(now.clone());
        namespace.updated_at = now;
    }
    if summary.query_billing_requested {
        summary.billable_logical_bytes_queried = rough_namespace_bytes(namespace);
        summary.billable_logical_bytes_returned = summary.billable_logical_bytes_queried.min(4096);
    }
    Ok(summary.into_response(
        request
            .get("return_affected_ids")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    ))
}

pub fn query_namespace(namespace: &Namespace, request: &Value) -> Result<Value, QueryError> {
    let object = as_object(request, "query request")?;
    validate_consistency_shape(object)?;
    let options = QueryOptions {
        vector_encoding: parse_vector_encoding(object.get("vector_encoding"))?,
    };
    if let Some(queries) = object.get("queries") {
        validate_multi_query_root(object)?;
        let subqueries = as_array(queries, "queries")?;
        if subqueries.len() > 16 {
            return Err(QueryError::new(
                "queries cannot contain more than 16 subqueries.",
            ));
        }
        let mut results = Vec::with_capacity(subqueries.len());
        for query in subqueries {
            results.push(query_single(namespace, query, &options)?);
        }
        return Ok(with_metrics(json!({ "results": results }), namespace));
    }
    query_single(namespace, request, &options)
}

fn query_single(
    namespace: &Namespace,
    request: &Value,
    options: &QueryOptions,
) -> Result<Value, QueryError> {
    let object = as_object(request, "query request")?;
    validate_consistency_shape(object)?;
    if object.contains_key("rank_by") && object.contains_key("aggregate_by") {
        return Err(QueryError::new(
            "rank_by and aggregate_by cannot be specified together.",
        ));
    }
    if object.contains_key("include_attributes") && object.contains_key("exclude_attributes") {
        return Err(QueryError::new(
            "💔 cannot specify both include_attributes and exclude_attributes",
        ));
    }
    validate_projection_shape(object)?;
    validate_included_attributes(namespace, object)?;
    if let Some(aggregate_by) = object.get("aggregate_by") {
        if object.contains_key("include_attributes") {
            return Err(QueryError::new(
                "aggregate_by and include_attributes cannot be specified together.",
            ));
        }
        return query_aggregations(namespace, object, aggregate_by);
    }
    let rank_by = object
        .get("rank_by")
        .ok_or_else(|| QueryError::new("rank_by is required unless aggregate_by is set."))?;
    let limit = parse_limit(object)?;
    let filters = object.get("filters");
    let rank_plan = parse_rank_plan(rank_by, filters.is_some())?;
    let prepared_rank_plan = prepare_rank_plan(rank_plan, namespace)?;
    let ranked = if limit.per.is_none() {
        indexed_order_ranked(namespace, &prepared_rank_plan.kind, filters, limit.total)?
    } else {
        None
    };
    let ranked = match ranked {
        Some(ranked) => ranked,
        None => {
            let ranked = rank_documents(namespace, &prepared_rank_plan, filters)?;
            apply_limit(ranked, &limit, &prepared_rank_plan.kind)?
        }
    };
    let mut rows = Vec::with_capacity(ranked.len());
    for ranked_doc in ranked {
        rows.push(project_document(
            ranked_doc.doc,
            object,
            options.vector_encoding,
            prepared_rank_plan.kind.clone(),
            ranked_doc.score,
        )?);
    }
    Ok(with_metrics(json!({ "rows": rows }), namespace))
}

fn with_metrics(mut response: Value, namespace: &Namespace) -> Value {
    if let Value::Object(object) = &mut response {
        let logical_bytes = rough_namespace_bytes(namespace);
        object.insert(
            "billing".to_string(),
            json!({
                "billable_logical_bytes_queried": logical_bytes,
                "billable_logical_bytes_returned": logical_bytes.min(4096)
            }),
        );
        object.insert(
            "performance".to_string(),
            json!({
                "cache_hit_ratio": 1.0,
                "cache_temperature": "hot",
                "server_total_ms": 0,
                "query_execution_ms": 0,
                "exhaustive_search_count": namespace.documents.len(),
                "approx_namespace_size": namespace.documents.len(),
                "last_included_write_at": "1970-01-01T00:00:00Z"
            }),
        );
    }
    response
}

fn rough_namespace_bytes(namespace: &Namespace) -> usize {
    if let Some(bytes) = namespace.logical_bytes_cache.get() {
        return bytes;
    }
    let bytes = compute_namespace_bytes(namespace);
    namespace.logical_bytes_cache.set(bytes);
    #[cfg(test)]
    namespace.logical_bytes_cache.record_recompute();
    bytes
}

fn compute_namespace_bytes(namespace: &Namespace) -> usize {
    namespace
        .documents
        .iter()
        .filter_map(|document| serde_json::to_vec(document).ok())
        .map(|bytes| bytes.len())
        .sum()
}

#[cfg(not(target_arch = "wasm32"))]
fn logical_now() -> String {
    Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

#[cfg(target_arch = "wasm32")]
fn logical_now() -> String {
    default_created_at()
}

fn parse_pinning(value: &Value) -> Result<Option<Value>, QueryError> {
    match value {
        Value::Null | Value::Bool(false) => Ok(None),
        Value::Bool(true) => Ok(Some(json!({
            "replicas": 1,
            "status": {
                "ready_replicas": 1,
                "utilization": 0.0
            }
        }))),
        Value::Object(object) => {
            let replicas = object.get("replicas").and_then(Value::as_u64).unwrap_or(1);
            if replicas == 0 {
                return Err(QueryError::new(
                    "pinning.replicas must be greater than zero.",
                ));
            }
            Ok(Some(json!({
                "replicas": replicas,
                "status": {
                    "ready_replicas": replicas,
                    "utilization": 0.0
                }
            })))
        }
        _ => Err(QueryError::new(
            "pinning must be true, false, null, or an object.",
        )),
    }
}

fn inferred_schema(namespace: &Namespace) -> Map<String, Value> {
    let mut schema = namespace.schema.clone();
    for document in &namespace.documents {
        infer_typed_document_schema(document, &mut schema);
        for (attribute, value) in &document.attributes {
            if schema.contains_key(attribute) {
                continue;
            }
            schema.insert(attribute.clone(), infer_schema_type(attribute, value));
        }
    }
    schema
}

fn schema_with_id(namespace: &Namespace) -> Map<String, Value> {
    let mut schema = inferred_schema(namespace);
    if !schema.contains_key("id") {
        schema.insert("id".to_string(), infer_id_schema_type(namespace));
    }
    schema
}

fn infer_schema_type(attribute: &str, value: &Value) -> Value {
    if value.is_null() {
        return json!("string");
    }
    if value.as_bool().is_some() {
        return json!("bool");
    }
    if value.as_i64().is_some() {
        return json!("int");
    }
    if value.as_u64().is_some() {
        return json!("uint");
    }
    if value.as_f64().is_some() {
        return json!("float");
    }
    if value.as_str().is_some() {
        return json!("string");
    }
    if let Some(array) = value.as_array() {
        if attribute == "vector"
            && !array.is_empty()
            && array.iter().all(|item| value_as_f64(item).is_some())
        {
            return json!(format!("[{}]f32", array.len()));
        }
        let first_non_null = array.iter().find(|item| !item.is_null());
        return match first_non_null {
            Some(Value::Bool(_)) => json!("[]bool"),
            Some(Value::Number(number)) if number.as_i64().is_some() => json!("[]int"),
            Some(Value::Number(number)) if number.as_u64().is_some() => json!("[]uint"),
            Some(Value::Number(_)) => json!("[]float"),
            Some(Value::String(_)) => json!("[]string"),
            _ => json!("[]string"),
        };
    }
    if value.as_object().is_some() {
        return json!("{}f16");
    }
    json!("string")
}

fn infer_id_schema_type(namespace: &Namespace) -> Value {
    namespace
        .documents
        .first()
        .map(|document| match &document.id {
            Value::Number(number) if number.as_u64().is_some() => json!("uint"),
            Value::Number(number) if number.as_i64().is_some_and(|id| id >= 0) => json!("uint"),
            Value::Number(_) => json!("int"),
            Value::String(_) => json!("string"),
            _ => json!("string"),
        })
        .unwrap_or_else(|| json!("uint"))
}

fn merge_schema(namespace: &mut Namespace, schema: &Map<String, Value>) -> Result<(), QueryError> {
    for (attribute, definition) in schema {
        validate_attribute_name(attribute)?;
        let incoming_type = incoming_schema_type_name(attribute, definition)?;
        validate_schema_definition(attribute, definition, incoming_type)?;
        if namespace.schema.contains_key(attribute) {
            let current_type = schema_type_name(
                namespace
                    .schema
                    .get(attribute)
                    .ok_or_else(|| QueryError::new("schema changed unexpectedly."))?,
            )?;
            if current_type != incoming_type {
                return Err(QueryError::schema_type_change(
                    attribute,
                    current_type,
                    incoming_type,
                ));
            }
        } else if inferred_schema(namespace).contains_key(attribute) {
            let current = inferred_schema(namespace);
            let current_type = schema_type_name(
                current
                    .get(attribute)
                    .ok_or_else(|| QueryError::new("schema inference changed unexpectedly."))?,
            )?;
            if current_type != incoming_type {
                return Err(QueryError::schema_type_change(
                    attribute,
                    current_type,
                    incoming_type,
                ));
            }
        }
        if is_dense_vector_type(incoming_type) && !namespace.documents.is_empty() {
            let Some(existing_definition) = inferred_schema(namespace).get(attribute).cloned()
            else {
                return Err(QueryError::new(format!(
                    "Cannot add vector column '{attribute}' after namespace creation."
                )));
            };
            let existing_type = schema_type_name(&existing_definition)?;
            if !is_dense_vector_type(existing_type) {
                return Err(QueryError::new(format!(
                    "Cannot add vector column '{attribute}' after namespace creation."
                )));
            }
        }
        namespace
            .schema
            .insert(attribute.clone(), sanitize_schema_definition(definition));
    }
    validate_vector_column_count(&namespace.schema)
}

fn sanitize_schema_definition(definition: &Value) -> Value {
    let Some(object) = definition.as_object() else {
        return definition.clone();
    };
    let mut sanitized = Map::new();
    for key in [
        "type",
        "filterable",
        "regex",
        "glob",
        "fuzzy",
        "full_text_search",
        "ann",
        "sparse_knn",
    ] {
        if let Some(value) = object.get(key) {
            sanitized.insert(key.to_string(), value.clone());
        }
    }
    Value::Object(sanitized)
}

fn refresh_inferred_schema(namespace: &mut Namespace) -> Result<(), QueryError> {
    let mut schema = namespace.schema.clone();
    for document in &namespace.documents {
        infer_typed_document_schema(document, &mut schema);
        for (attribute, value) in &document.attributes {
            if schema.contains_key(attribute) {
                continue;
            }
            schema.insert(attribute.clone(), infer_schema_type(attribute, value));
        }
    }
    validate_vector_column_count(&schema)?;
    namespace.schema = schema;
    Ok(())
}

fn infer_typed_document_schema(document: &Document, schema: &mut Map<String, Value>) {
    if !schema.contains_key("vector")
        && let Some(vector) = &document.typed.vector
    {
        schema.insert(
            "vector".to_string(),
            json!(format!("[{}]f32", vector.len())),
        );
    }
}

fn validate_write_rows_against_schema(
    namespace: &Namespace,
    upserts: &[WriteRow],
    patches: &[WriteRow],
) -> Result<(), QueryError> {
    let schema = inferred_schema(namespace);
    let vector_attributes = dense_vector_attributes(&schema)?;
    for row in upserts {
        for vector_attribute in &vector_attributes {
            if !row.attributes.contains_key(vector_attribute) {
                return Err(QueryError::new(format!(
                    "Upsert row {} is missing required vector attribute '{vector_attribute}'.",
                    row.id
                )));
            }
        }
        validate_row_types(&schema, row, true)?;
    }
    for row in patches {
        for vector_attribute in &vector_attributes {
            if row.attributes.contains_key(vector_attribute) {
                return Err(QueryError::new(format!(
                    "Vector attribute '{vector_attribute}' cannot be patched; upsert the full document instead."
                )));
            }
        }
        validate_row_types(&schema, row, false)?;
    }
    Ok(())
}

fn normalize_write_rows_against_schema(
    namespace: &Namespace,
    upserts: &mut [WriteRow],
    patches: &mut [WriteRow],
) -> Result<(), QueryError> {
    let schema = inferred_schema(namespace);
    for row in upserts.iter_mut().chain(patches.iter_mut()) {
        normalize_row_against_schema(&schema, row)?;
    }
    Ok(())
}

fn normalize_row_against_schema(
    schema: &Map<String, Value>,
    row: &mut WriteRow,
) -> Result<(), QueryError> {
    for (attribute, value) in &mut row.attributes {
        let Some(definition) = schema.get(attribute) else {
            continue;
        };
        normalize_value_for_schema_type(value, schema_type_name(definition)?);
    }
    Ok(())
}

fn normalize_value_for_schema_type(value: &mut Value, schema_type: &str) {
    match schema_type {
        "datetime" => {
            if let Some(text) = value.as_str()
                && let Some(datetime) = parse_datetime_text(text)
            {
                *value = Value::String(format_datetime(datetime));
            }
        }
        "[]datetime" => {
            if let Some(items) = value.as_array_mut() {
                for item in items {
                    normalize_value_for_schema_type(item, "datetime");
                }
            }
        }
        _ => {}
    }
}

fn validate_row_types(
    schema: &Map<String, Value>,
    row: &WriteRow,
    upsert: bool,
) -> Result<(), QueryError> {
    for (attribute, value) in &row.attributes {
        if value.is_null() {
            continue;
        }
        let expected = schema
            .get(attribute)
            .map(schema_type_name)
            .transpose()?
            .map(ToString::to_string);
        let inferred = schema_type_name(&infer_schema_type(attribute, value))?.to_string();
        if let Some(expected) = expected {
            validate_value_matches_type(attribute, value, &expected)?;
        } else if !upsert {
            validate_value_matches_type(attribute, value, &inferred)?;
        }
    }
    Ok(())
}

fn validate_value_matches_type(
    attribute: &str,
    value: &Value,
    expected_type: &str,
) -> Result<(), QueryError> {
    let valid = match expected_type {
        "string" | "uuid" => value.as_str().is_some(),
        "datetime" => value.as_str().and_then(parse_datetime_text).is_some(),
        "int" => value.as_i64().is_some(),
        "uint" => value.as_u64().is_some(),
        "float" => value_as_f64(value).is_some(),
        "bool" => value.as_bool().is_some(),
        "[]string" | "[]uuid" => array_items(value, |item| item.as_str().is_some()),
        "[]datetime" => array_items(value, |item| {
            item.as_str().and_then(parse_datetime_text).is_some()
        }),
        "[]int" => array_items(value, |item| item.as_i64().is_some()),
        "[]uint" => array_items(value, |item| item.as_u64().is_some()),
        "[]float" => array_items(value, |item| value_as_f64(item).is_some()),
        "[]bool" => array_items(value, |item| item.as_bool().is_some()),
        "{}f16" => value
            .as_object()
            .map(|object| object.values().all(|item| value_as_f64(item).is_some()))
            .unwrap_or(false),
        other if is_dense_vector_type(other) => {
            let dimensions = dense_vector_dimensions(other)?;
            numeric_array(value, attribute)
                .map(|values| values.len() == dimensions)
                .unwrap_or(false)
        }
        _ => true,
    };
    if valid {
        Ok(())
    } else {
        Err(QueryError::new(format!(
            "Attribute '{attribute}' does not match schema type '{expected_type}'."
        )))
    }
}

fn array_items(value: &Value, predicate: impl Fn(&Value) -> bool) -> bool {
    value
        .as_array()
        .map(|items| items.iter().all(|item| item.is_null() || predicate(item)))
        .unwrap_or(false)
}

fn dense_vector_attributes(schema: &Map<String, Value>) -> Result<Vec<String>, QueryError> {
    let mut attributes = Vec::new();
    for (attribute, definition) in schema {
        if is_dense_vector_type(schema_type_name(definition)?) {
            attributes.push(attribute.clone());
        }
    }
    Ok(attributes)
}

fn validate_vector_column_count(schema: &Map<String, Value>) -> Result<(), QueryError> {
    let count = dense_vector_attributes(schema)?.len();
    if count > 2 {
        return Err(QueryError::new(
            "A namespace can currently have up to 2 vector columns.",
        ));
    }
    Ok(())
}

fn schema_type_name(definition: &Value) -> Result<&str, QueryError> {
    if let Some(name) = definition.as_str() {
        return Ok(name);
    }
    definition
        .as_object()
        .and_then(|object| object.get("type"))
        .and_then(Value::as_str)
        .ok_or_else(|| {
            QueryError::new("schema definitions must be strings or objects with a type.")
        })
}

fn incoming_schema_type_name<'a>(
    attribute: &str,
    definition: &'a Value,
) -> Result<&'a str, QueryError> {
    if let Some(name) = definition.as_str() {
        return Ok(name);
    }
    let Some(object) = definition.as_object() else {
        return Err(schema_deserialize_error(attribute));
    };
    object
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| schema_deserialize_error(attribute))
}

fn schema_deserialize_error(attribute: &str) -> QueryError {
    QueryError::unprocessable(format!(
        "Failed to deserialize the JSON body into the target type: {attribute}: data did not match any variant of untagged enum AttributeSchemaInput"
    ))
}

fn normalize_metadata_schema_definition(
    namespace: &Namespace,
    attribute: &str,
    definition: &Value,
) -> Result<(String, Value), QueryError> {
    let schema_type = schema_type_name(definition)?.to_string();
    let mut config = match definition {
        Value::Object(object) => object.clone(),
        Value::String(_) => Map::new(),
        _ => {
            return Err(QueryError::new(
                "schema definitions must be strings or objects with a type.",
            ));
        }
    };
    config.insert("type".to_string(), Value::String(schema_type.clone()));
    if is_dense_vector_type(&schema_type) {
        config.insert(
            "ann".to_string(),
            json!({ "distance_metric": namespace.distance_metric }),
        );
    } else if attribute != "id" && !config.contains_key("filterable") {
        config.insert("filterable".to_string(), Value::Bool(true));
    }
    if let Some(full_text_search) = config.get("full_text_search").cloned()
        && full_text_search != Value::Bool(false)
    {
        config.insert(
            "full_text_search".to_string(),
            fts_config_schema_value(&parse_fts_config(&full_text_search)?),
        );
    }
    Ok((attribute.to_string(), Value::Object(config)))
}

fn normalize_schema_response_definition(
    attribute: &str,
    definition: &Value,
) -> Result<(String, Value), QueryError> {
    let schema_type = schema_type_name(definition)?.to_string();
    let mut config = match definition {
        Value::Object(object) => object.clone(),
        Value::String(_) => Map::new(),
        _ => {
            return Err(QueryError::new(
                "schema definitions must be strings or objects with a type.",
            ));
        }
    };
    config.insert("type".to_string(), Value::String(schema_type.clone()));
    if !config.contains_key("filterable") {
        let inferred_filterable = attribute != "id"
            && !is_dense_vector_type(&schema_type)
            && matches!(definition, Value::String(_));
        config.insert(
            "filterable".to_string(),
            if inferred_filterable {
                Value::Bool(true)
            } else {
                Value::Null
            },
        );
    }
    if let Some(full_text_search) = config.get("full_text_search").cloned()
        && full_text_search != Value::Null
        && full_text_search != Value::Bool(false)
    {
        config.insert(
            "full_text_search".to_string(),
            fts_config_schema_value(&parse_fts_config(&full_text_search)?),
        );
    }
    config
        .entry("full_text_search".to_string())
        .or_insert(Value::Null);
    if is_dense_vector_type(&schema_type) {
        config.insert("ann".to_string(), Value::Bool(true));
    } else if attribute == "id" {
        config.remove("ann");
    }
    Ok((attribute.to_string(), Value::Object(config)))
}

fn validate_schema_definition(
    attribute: &str,
    definition: &Value,
    schema_type: &str,
) -> Result<(), QueryError> {
    let Some(object) = definition.as_object() else {
        return Ok(());
    };
    if let Some(filterable) = object.get("filterable")
        && filterable.as_bool().is_none()
    {
        return Err(QueryError::new(format!(
            "schema.{attribute}.filterable must be a boolean."
        )));
    }
    if let Some(regex) = object.get("regex")
        && regex.as_bool().is_none()
    {
        return Err(QueryError::new(format!(
            "schema.{attribute}.regex must be a boolean."
        )));
    }
    if let Some(glob) = object.get("glob")
        && glob.as_bool().is_none()
    {
        return Err(QueryError::new(format!(
            "schema.{attribute}.glob must be a boolean."
        )));
    }
    if let Some(fuzzy) = object.get("fuzzy")
        && fuzzy.as_bool().is_none()
    {
        return Err(QueryError::new(format!(
            "schema.{attribute}.fuzzy must be a boolean."
        )));
    }
    if let Some(ann) = object.get("ann") {
        validate_ann_config(attribute, ann)?;
    }
    if let Some(sparse_knn) = object.get("sparse_knn") {
        validate_sparse_knn_config(attribute, sparse_knn)?;
    }
    if let Some(full_text_search) = object.get("full_text_search") {
        if schema_type != "string" && schema_type != "[]string" {
            return Err(QueryError::new(format!(
                "schema.{attribute}.full_text_search requires string or []string."
            )));
        }
        parse_fts_config(full_text_search)?;
        if definition
            .as_object()
            .and_then(|object| object.get("full_text_search"))
            .map(parse_fts_config)
            .transpose()?
            .is_some_and(|config| config.tokenizer == Tokenizer::PreTokenizedArray)
            && schema_type != "[]string"
        {
            return Err(QueryError::new(format!(
                "schema.{attribute}.full_text_search tokenizer pre_tokenized_array requires []string."
            )));
        }
    }
    Ok(())
}

fn validate_ann_config(attribute: &str, value: &Value) -> Result<(), QueryError> {
    match value {
        Value::Bool(_) => Ok(()),
        Value::Object(object) => {
            for key in object.keys() {
                if key != "distance_metric" {
                    return Err(QueryError::new(format!(
                        "schema.{attribute}.ann.{key} is not supported."
                    )));
                }
            }
            if let Some(distance_metric) = object.get("distance_metric") {
                DistanceMetric::parse(distance_metric)?;
            }
            Ok(())
        }
        _ => Err(QueryError::new(format!(
            "schema.{attribute}.ann must be a boolean or object."
        ))),
    }
}

fn validate_sparse_knn_config(attribute: &str, value: &Value) -> Result<(), QueryError> {
    let object = as_object(value, "sparse_knn")?;
    for key in object.keys() {
        if key != "distance_metric" {
            return Err(QueryError::new(format!(
                "schema.{attribute}.sparse_knn.{key} is not supported."
            )));
        }
    }
    match object.get("distance_metric").and_then(Value::as_str) {
        Some("dot_product") => Ok(()),
        _ => Err(QueryError::new(format!(
            "schema.{attribute}.sparse_knn.distance_metric must be 'dot_product'."
        ))),
    }
}

fn parse_fts_config(value: &Value) -> Result<FtsConfig, QueryError> {
    match value {
        Value::Bool(false) => Ok(FtsConfig::default()),
        Value::Bool(true) => Ok(FtsConfig::default()),
        Value::Object(object) => {
            let mut config = FtsConfig::default();
            for key in object.keys() {
                match key.as_str() {
                    "k1" | "b" | "k3" | "language" | "stemming" | "remove_stopwords"
                    | "ascii_folding" | "case_sensitive" | "max_token_length" | "tokenizer" => {}
                    _ => {
                        return Err(QueryError::new(format!(
                            "full_text_search.{key} is not supported."
                        )));
                    }
                }
            }
            if let Some(k1) = object.get("k1") {
                config.k1_millis = parse_fts_ratio(k1, "full_text_search.k1")?;
            }
            if let Some(b) = object.get("b") {
                config.b_millis = parse_fts_ratio(b, "full_text_search.b")?;
            }
            if let Some(k3) = object.get("k3") {
                config.k3_millis = parse_fts_ratio(k3, "full_text_search.k3")?;
            }
            if let Some(language) = object.get("language") {
                config.language = parse_fts_language(language)?;
            }
            if let Some(stemming) = object.get("stemming") {
                config.stemming = as_bool(stemming, "full_text_search.stemming")?;
            }
            if let Some(remove_stopwords) = object.get("remove_stopwords") {
                config.remove_stopwords =
                    as_bool(remove_stopwords, "full_text_search.remove_stopwords")?;
            }
            if let Some(ascii_folding) = object.get("ascii_folding") {
                config.ascii_folding = as_bool(ascii_folding, "full_text_search.ascii_folding")?;
            }
            if let Some(case_sensitive) = object.get("case_sensitive") {
                config.case_sensitive = as_bool(case_sensitive, "full_text_search.case_sensitive")?;
            }
            if let Some(max_token_length) = object.get("max_token_length") {
                let raw = max_token_length.as_u64().ok_or_else(|| {
                    QueryError::new("full_text_search.max_token_length must be an integer.")
                })?;
                if !(1..=254).contains(&raw) {
                    return Err(QueryError::new(
                        "full_text_search.max_token_length must be between 1 and 254.",
                    ));
                }
                config.max_token_length = usize::try_from(raw).map_err(|_| {
                    QueryError::new("full_text_search.max_token_length is too large.")
                })?;
            }
            if let Some(tokenizer) = object.get("tokenizer") {
                config.tokenizer = parse_tokenizer(tokenizer)?;
            }
            if config.tokenizer == Tokenizer::PreTokenizedArray {
                validate_pre_tokenized_config(object, &mut config)?;
            }
            Ok(config)
        }
        _ => Err(QueryError::new(
            "full_text_search must be a boolean or object.",
        )),
    }
}

fn validate_pre_tokenized_config(
    object: &Map<String, Value>,
    config: &mut FtsConfig,
) -> Result<(), QueryError> {
    if object.contains_key("language") {
        return Err(QueryError::new(
            "full_text_search.language cannot be specified with pre_tokenized_array.",
        ));
    }
    if config.stemming {
        return Err(QueryError::new(
            "full_text_search.stemming cannot be true with pre_tokenized_array.",
        ));
    }
    if object
        .get("remove_stopwords")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return Err(QueryError::new(
            "full_text_search.remove_stopwords cannot be true with pre_tokenized_array.",
        ));
    }
    if object
        .get("case_sensitive")
        .and_then(Value::as_bool)
        .is_some_and(|case_sensitive| !case_sensitive)
    {
        return Err(QueryError::new(
            "full_text_search.case_sensitive cannot be false with pre_tokenized_array.",
        ));
    }
    config.remove_stopwords = false;
    config.stemming = false;
    config.case_sensitive = true;
    Ok(())
}

fn parse_fts_ratio(value: &Value, label: &str) -> Result<u64, QueryError> {
    let ratio =
        value_as_f64(value).ok_or_else(|| QueryError::new(format!("{label} must be numeric.")))?;
    if !ratio.is_finite() {
        return Err(QueryError::new(format!("{label} must be a finite number.")));
    }
    match label {
        "full_text_search.k1" if ratio <= 0.0 => {
            return Err(QueryError::new(
                "full_text_search.k1 must be greater than 0.",
            ));
        }
        "full_text_search.k3" if ratio <= 0.0 => {
            return Err(QueryError::new(
                "full_text_search.k3 must be greater than 0.",
            ));
        }
        "full_text_search.b" if !(0.0..=1.0).contains(&ratio) => {
            return Err(QueryError::new(
                "full_text_search.b must be between 0.0 and 1.0.",
            ));
        }
        _ => {}
    }
    Ok((ratio * 1_000.0).round() as u64)
}

fn as_bool(value: &Value, label: &str) -> Result<bool, QueryError> {
    value
        .as_bool()
        .ok_or_else(|| QueryError::new(format!("{label} must be a boolean.")))
}

fn parse_fts_language(value: &Value) -> Result<FtsLanguage, QueryError> {
    match value.as_str() {
        Some("arabic") => Ok(FtsLanguage::Arabic),
        Some("danish") => Ok(FtsLanguage::Danish),
        Some("dutch") => Ok(FtsLanguage::Dutch),
        Some("english") => Ok(FtsLanguage::English),
        Some("finnish") => Ok(FtsLanguage::Finnish),
        Some("french") => Ok(FtsLanguage::French),
        Some("german") => Ok(FtsLanguage::German),
        Some("greek") => Ok(FtsLanguage::Greek),
        Some("hungarian") => Ok(FtsLanguage::Hungarian),
        Some("italian") => Ok(FtsLanguage::Italian),
        Some("norwegian") => Ok(FtsLanguage::Norwegian),
        Some("portuguese") => Ok(FtsLanguage::Portuguese),
        Some("romanian") => Ok(FtsLanguage::Romanian),
        Some("russian") => Ok(FtsLanguage::Russian),
        Some("spanish") => Ok(FtsLanguage::Spanish),
        Some("swedish") => Ok(FtsLanguage::Swedish),
        Some("tamil") => Ok(FtsLanguage::Tamil),
        Some("turkish") => Ok(FtsLanguage::Turkish),
        _ => Err(QueryError::new(
            "full_text_search.language is not supported.",
        )),
    }
}

fn parse_tokenizer(value: &Value) -> Result<Tokenizer, QueryError> {
    match value.as_str() {
        Some("pre_tokenized_array") => Ok(Tokenizer::PreTokenizedArray),
        Some("word_v0") => Ok(Tokenizer::WordV0),
        Some("word_v1") => Ok(Tokenizer::WordV1),
        Some("word_v2") => Ok(Tokenizer::WordV2),
        Some("word_v3") => Ok(Tokenizer::WordV3),
        _ => Err(QueryError::new(
            "full_text_search.tokenizer must be pre_tokenized_array, word_v0, word_v1, word_v2, or word_v3.",
        )),
    }
}

fn fts_config_for_field(namespace: &Namespace, field: &str) -> FtsConfig {
    namespace
        .schema
        .get(field)
        .and_then(Value::as_object)
        .and_then(|object| object.get("full_text_search"))
        .map(parse_fts_config)
        .transpose()
        .ok()
        .flatten()
        .unwrap_or_default()
}

fn fts_config_schema_value(config: &FtsConfig) -> Value {
    json!({
        "k1": config.k1(),
        "b": config.b(),
        "k3": config.k3(),
        "language": fts_language_name(config.language),
        "stemming": config.stemming,
        "remove_stopwords": config.remove_stopwords,
        "ascii_folding": config.ascii_folding,
        "case_sensitive": config.case_sensitive,
        "max_token_length": config.max_token_length,
        "tokenizer": tokenizer_name(config.tokenizer),
    })
}

fn fts_language_name(language: FtsLanguage) -> &'static str {
    match language {
        FtsLanguage::Arabic => "arabic",
        FtsLanguage::Danish => "danish",
        FtsLanguage::Dutch => "dutch",
        FtsLanguage::English => "english",
        FtsLanguage::Finnish => "finnish",
        FtsLanguage::French => "french",
        FtsLanguage::German => "german",
        FtsLanguage::Greek => "greek",
        FtsLanguage::Hungarian => "hungarian",
        FtsLanguage::Italian => "italian",
        FtsLanguage::Norwegian => "norwegian",
        FtsLanguage::Portuguese => "portuguese",
        FtsLanguage::Romanian => "romanian",
        FtsLanguage::Russian => "russian",
        FtsLanguage::Spanish => "spanish",
        FtsLanguage::Swedish => "swedish",
        FtsLanguage::Tamil => "tamil",
        FtsLanguage::Turkish => "turkish",
    }
}

fn tokenizer_name(tokenizer: Tokenizer) -> &'static str {
    match tokenizer {
        Tokenizer::PreTokenizedArray => "pre_tokenized_array",
        Tokenizer::WordV0 => "word_v0",
        Tokenizer::WordV1 => "word_v1",
        Tokenizer::WordV2 => "word_v2",
        Tokenizer::WordV3 => "word_v3",
    }
}

fn fts_config_from_schema(schema: Option<&Map<String, Value>>, field: &str) -> FtsConfig {
    schema
        .and_then(|schema| schema.get(field))
        .and_then(Value::as_object)
        .and_then(|object| object.get("full_text_search"))
        .map(parse_fts_config)
        .transpose()
        .ok()
        .flatten()
        .unwrap_or_default()
}

fn is_dense_vector_type(schema_type: &str) -> bool {
    schema_type.starts_with('[') && (schema_type.ends_with("]f32") || schema_type.ends_with("]f16"))
}

fn dense_vector_dimensions(schema_type: &str) -> Result<usize, QueryError> {
    let Some(end) = schema_type.find(']') else {
        return Err(QueryError::new("vector schema type is malformed."));
    };
    schema_type[1..end]
        .parse::<usize>()
        .map_err(|_| QueryError::new("vector schema dimensions are malformed."))
}

#[derive(Debug, Clone)]
struct WriteRow {
    id: Value,
    attributes: Map<String, Value>,
}

#[derive(Debug)]
struct WriteIdIndex {
    indexes: HashMap<String, usize>,
}

impl WriteIdIndex {
    fn new(namespace: &Namespace) -> Self {
        let mut indexes = HashMap::with_capacity(namespace.documents.len());
        for (index, document) in namespace.documents.iter().enumerate() {
            indexes.entry(id_key(&document.id)).or_insert(index);
        }
        Self { indexes }
    }

    fn get(&self, id: &Value) -> Option<usize> {
        self.indexes.get(&id_key(id)).copied()
    }

    fn insert_new(&mut self, id: &Value, index: usize) {
        self.indexes.insert(id_key(id), index);
    }

    fn rebuild(&mut self, namespace: &Namespace) {
        *self = Self::new(namespace);
    }
}

#[derive(Debug)]
struct FilterWriteOutcome {
    ids: Vec<Value>,
    rows_remaining: bool,
}

#[derive(Debug, Default)]
struct WriteSummary {
    upsert_requested: bool,
    patch_requested: bool,
    delete_requested: bool,
    rows_remaining: bool,
    query_billing_requested: bool,
    billable_logical_bytes_written: usize,
    billable_logical_bytes_queried: usize,
    billable_logical_bytes_returned: usize,
    upserted_ids: Vec<Value>,
    patched_ids: Vec<Value>,
    deleted_ids: Vec<Value>,
}

impl WriteSummary {
    fn into_response(self, return_affected_ids: bool) -> Value {
        let rows_upserted = self.upserted_ids.len();
        let rows_patched = self.patched_ids.len();
        let rows_deleted = self.deleted_ids.len();
        let mut response = Map::new();
        response.insert("status".to_string(), Value::String("OK".to_string()));
        response.insert(
            "message".to_string(),
            Value::String("documents committed successfully".to_string()),
        );
        response.insert(
            "rows_affected".to_string(),
            Value::Number(Number::from(rows_upserted + rows_patched + rows_deleted)),
        );
        if self.upsert_requested {
            response.insert(
                "rows_upserted".to_string(),
                Value::Number(Number::from(rows_upserted)),
            );
        }
        if self.patch_requested {
            response.insert(
                "rows_patched".to_string(),
                Value::Number(Number::from(rows_patched)),
            );
        }
        if self.delete_requested {
            response.insert(
                "rows_deleted".to_string(),
                Value::Number(Number::from(rows_deleted)),
            );
        }
        response.insert(
            "rows_remaining".to_string(),
            Value::Bool(self.rows_remaining),
        );
        let mut billing = Map::new();
        billing.insert(
            "billable_logical_bytes_written".to_string(),
            Value::Number(Number::from(self.billable_logical_bytes_written)),
        );
        if self.query_billing_requested {
            billing.insert(
                "query".to_string(),
                json!({
                    "billable_logical_bytes_queried": self.billable_logical_bytes_queried,
                    "billable_logical_bytes_returned": self.billable_logical_bytes_returned
                }),
            );
        }
        response.insert("billing".to_string(), Value::Object(billing));
        response.insert("performance".to_string(), json!({ "server_total_ms": 0 }));
        if return_affected_ids {
            if rows_upserted > 0 {
                response.insert("upserted_ids".to_string(), Value::Array(self.upserted_ids));
            }
            if rows_patched > 0 {
                response.insert("patched_ids".to_string(), Value::Array(self.patched_ids));
            }
            if rows_deleted > 0 {
                response.insert("deleted_ids".to_string(), Value::Array(self.deleted_ids));
            }
        }
        Value::Object(response)
    }
}

fn validate_namespace_name(name: &str) -> Result<(), QueryError> {
    if name.is_empty() {
        return Err(namespace_not_found(name));
    }
    if name.len() > 128 {
        return Err(QueryError::invalid_url(format!(
            "Invalid URL: Namespace `{name}` is too long, limit is currently 128 characters"
        )));
    }
    if !name
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || "-_.".contains(character))
    {
        return Err(QueryError::invalid_url(
            "Invalid URL: Namespace contains invalid characters, must be [A-Za-z0-9-_.]",
        ));
    }
    Ok(())
}

fn ensure_namespace(store: &mut MiniStore, namespace_name: &str) {
    if store.namespace_mut(namespace_name).is_none() {
        store.namespaces.push(Namespace {
            name: namespace_name.to_string(),
            distance_metric: DistanceMetric::default(),
            schema: Map::new(),
            created_at: logical_now(),
            last_write_at: None,
            updated_at: logical_now(),
            encryption: default_encryption(),
            pinning: None,
            branching_parent: None,
            documents: Vec::new(),
            query_indexes: NamespaceQueryIndexCache::default(),
            logical_bytes_cache: NamespaceLogicalBytesCache::default(),
            fts_index_cache: NamespaceFtsIndexCache::default(),
        });
    }
}

fn copy_namespace(
    store: &mut MiniStore,
    destination_name: &str,
    copy_from: &Value,
    branch: bool,
    encryption_override: Option<&Value>,
) -> Result<Value, QueryError> {
    let source_name = match copy_from {
        Value::String(name) => name.as_str(),
        Value::Object(object) => object
            .get("source_namespace")
            .and_then(Value::as_str)
            .ok_or_else(|| QueryError::new("copy_from_namespace.source_namespace is required."))?,
        _ => {
            return Err(QueryError::new(
                "copy_from_namespace must be a string or object.",
            ));
        }
    };
    let source = store.namespace(source_name)?.clone();
    let rows_affected = source.documents.len();
    let bytes_written = rough_namespace_bytes(&source);
    let encryption = encryption_override
        .cloned()
        .unwrap_or_else(|| source.encryption.clone());
    if let Some(destination) = store.namespace_mut(destination_name) {
        if !destination.documents.is_empty() {
            return Err(QueryError::new(
                "copy_from_namespace destination namespace must be empty.",
            ));
        }
        destination.distance_metric = source.distance_metric;
        destination.schema = source.schema.clone();
        destination.encryption = encryption;
        destination.branching_parent = branch.then(|| source.name.clone());
        destination.updated_at = logical_now();
        destination.last_write_at = Some(logical_now());
        destination.documents = source.documents;
        destination.query_indexes.guard().clear();
        destination.invalidate_fts_indexes();
        destination.set_cached_logical_bytes(bytes_written);
    } else {
        store.namespaces.push(Namespace {
            name: destination_name.to_string(),
            distance_metric: source.distance_metric,
            schema: source.schema.clone(),
            created_at: logical_now(),
            last_write_at: Some(logical_now()),
            updated_at: logical_now(),
            encryption,
            pinning: None,
            branching_parent: branch.then(|| source.name.clone()),
            documents: source.documents,
            query_indexes: NamespaceQueryIndexCache::default(),
            logical_bytes_cache: NamespaceLogicalBytesCache::default(),
            fts_index_cache: NamespaceFtsIndexCache::default(),
        });
        store
            .namespace_mut(destination_name)
            .ok_or_else(|| QueryError::new("namespace disappeared during copy."))?
            .set_cached_logical_bytes(bytes_written);
    }
    Ok(json!({
        "status": "OK",
        "message": "namespace cloned successfully",
        "rows_affected": rows_affected,
        "rows_upserted": rows_affected,
        "rows_remaining": false,
        "billing": { "billable_logical_bytes_written": bytes_written },
        "performance": { "server_total_ms": 0 }
    }))
}

fn validate_copy_request(request: &Map<String, Value>) -> Result<(), QueryError> {
    let allowed = BTreeSet::from(["copy_from_namespace", "encryption"]);
    reject_unexpected_write_fields(request, &allowed, "copy_from_namespace")?;
    let copy_from = request
        .get("copy_from_namespace")
        .ok_or_else(|| QueryError::new("copy_from_namespace is required."))?;
    match copy_from {
        Value::String(source) => validate_namespace_name(source),
        Value::Object(object) => {
            let source = object
                .get("source_namespace")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    QueryError::new("copy_from_namespace.source_namespace is required.")
                })?;
            validate_namespace_name(source)?;
            for key in object.keys() {
                if !matches!(
                    key.as_str(),
                    "source_namespace" | "source_api_key" | "source_region"
                ) {
                    return Err(QueryError::new(format!(
                        "copy_from_namespace.{key} is not supported by micropuffer."
                    )));
                }
            }
            for key in ["source_api_key", "source_region"] {
                if object
                    .get(key)
                    .is_some_and(|value| value.as_str().is_none())
                {
                    return Err(QueryError::new(format!(
                        "copy_from_namespace.{key} must be a string."
                    )));
                }
            }
            Ok(())
        }
        _ => Err(QueryError::new(
            "copy_from_namespace must be a string or object.",
        )),
    }
}

fn validate_branch_request(request: &Map<String, Value>) -> Result<(), QueryError> {
    let allowed = BTreeSet::from(["branch_from_namespace"]);
    reject_unexpected_write_fields(request, &allowed, "branch_from_namespace")?;
    let branch_from = request
        .get("branch_from_namespace")
        .ok_or_else(|| QueryError::new("branch_from_namespace is required."))?;
    let Some(source) = branch_from.as_str() else {
        return Err(QueryError::new("branch_from_namespace must be a string."));
    };
    validate_namespace_name(source)
}

fn reject_unexpected_write_fields(
    request: &Map<String, Value>,
    allowed: &BTreeSet<&str>,
    operation: &str,
) -> Result<(), QueryError> {
    for key in request.keys() {
        if !allowed.contains(key.as_str()) {
            return Err(QueryError::new(format!(
                "💔 {operation} cannot be used with other write request fields"
            )));
        }
    }
    Ok(())
}

fn validate_distance_metric_for_write(
    namespace: Option<&Namespace>,
    distance_metric: Option<DistanceMetric>,
    schema: Option<&Value>,
    upserts: &[WriteRow],
) -> Result<(), QueryError> {
    if let (Some(namespace), Some(distance_metric)) = (namespace, distance_metric)
        && namespace.distance_metric != distance_metric
    {
        return Err(QueryError::new(format!(
            "💔 distance metric mismatch, expected {}, got {}",
            namespace.distance_metric.as_write_error_str(),
            distance_metric.as_write_error_str()
        )));
    }

    let existing_vector_columns = namespace
        .map(namespace_has_dense_vector_columns)
        .transpose()?
        .unwrap_or(false);
    let introduces_vector_column =
        write_schema_has_dense_vector(schema)? || upserts.iter().any(write_row_has_vector);

    if !introduces_vector_column || existing_vector_columns {
        return Ok(());
    }
    if namespace.is_some_and(|namespace| !namespace.documents.is_empty()) {
        return Err(QueryError::new(
            "💔 Vector provided for namespace without vector attribute",
        ));
    }
    if distance_metric.is_none() {
        if write_schema_has_ann_distance_metric(schema) {
            return Err(QueryError::new(
                "💔 `distance_metric` must be specified as a field at the top level of the write request, not in the `ann` configuration for an attribute",
            ));
        }
        return Err(QueryError::new(
            "💔 distance_metric must be specified for write to namespace with a vector",
        ));
    }
    Ok(())
}

fn namespace_has_dense_vector_columns(namespace: &Namespace) -> Result<bool, QueryError> {
    Ok(!dense_vector_attributes(&inferred_schema(namespace))?.is_empty())
}

fn write_schema_has_ann_distance_metric(schema: Option<&Value>) -> bool {
    schema
        .and_then(Value::as_object)
        .map(|schema| {
            schema.values().any(|definition| {
                definition
                    .as_object()
                    .and_then(|definition| definition.get("ann"))
                    .and_then(Value::as_object)
                    .and_then(|ann| ann.get("distance_metric"))
                    .is_some()
            })
        })
        .unwrap_or(false)
}

fn write_schema_has_dense_vector(schema: Option<&Value>) -> Result<bool, QueryError> {
    let Some(schema) = schema else {
        return Ok(false);
    };
    let schema = as_object(schema, "schema")?;
    for definition in schema.values() {
        if is_dense_vector_type(schema_type_name(definition)?) {
            return Ok(true);
        }
    }
    Ok(false)
}

fn write_row_has_vector(row: &WriteRow) -> bool {
    row.attributes
        .get("vector")
        .is_some_and(|value| value.is_array() || value.is_string())
}

fn collect_write_rows(
    rows_field: &str,
    rows_value: Option<&Value>,
    columns_field: &str,
    columns_value: Option<&Value>,
) -> Result<Vec<WriteRow>, QueryError> {
    let mut rows = Vec::new();
    if let Some(rows_value) = rows_value {
        let row_values = rows_value.as_array().ok_or_else(|| {
            QueryError::unprocessable(format!(
                "Failed to deserialize the JSON body into the target type: {rows_field}: invalid type: {}, expected a sequence",
                serde_type_name(rows_value)
            ))
        })?;
        for (index, row) in row_values.iter().enumerate() {
            rows.push(write_row_from_value(rows_field, index, row)?);
        }
    }
    if let Some(columns_value) = columns_value {
        rows.extend(write_rows_from_columns(columns_field, columns_value)?);
    }
    Ok(rows)
}

fn write_row_from_value(field: &str, index: usize, value: &Value) -> Result<WriteRow, QueryError> {
    let object = value.as_object().ok_or_else(|| {
        QueryError::unprocessable(format!(
            "Failed to deserialize the JSON body into the target type: {field}[{index}]: invalid type: {}, expected a map",
            serde_type_name(value)
        ))
    })?;
    write_row_from_object(field, index, object)
}

fn write_row_from_object(
    field: &str,
    index: usize,
    object: &Map<String, Value>,
) -> Result<WriteRow, QueryError> {
    let id = object
        .get("id")
        .cloned()
        .ok_or_else(|| {
            QueryError::unprocessable(format!(
                "Failed to deserialize the JSON body into the target type: {field}[{index}]: missing field `id`"
            ))
        })?;
    validate_document_id(&id)?;
    let attributes = object
        .iter()
        .filter(|(key, _)| key.as_str() != "id")
        .map(|(key, value)| validate_attribute_name(key).map(|()| (key.clone(), value.clone())))
        .collect::<Result<Map<_, _>, _>>()?;
    Ok(WriteRow { id, attributes })
}

fn write_rows_from_columns(
    field: &str,
    columns_value: &Value,
) -> Result<Vec<WriteRow>, QueryError> {
    let columns = columns_value.as_object().ok_or_else(|| {
        QueryError::unprocessable(format!(
            "Failed to deserialize the JSON body into the target type: {field}: invalid type: {}, expected a map",
            serde_type_name(columns_value)
        ))
    })?;
    let ids = as_array(
        columns
            .get("id")
            .ok_or_else(|| QueryError::new("write columns require an id column."))?,
        "write columns id",
    )?;
    for (column, values) in columns {
        validate_attribute_name(column)?;
        let values = as_array(values, "write column values")?;
        if values.len() != ids.len() {
            return Err(QueryError::new(
                "all write columns must have the same length as id.",
            ));
        }
    }
    let mut rows = Vec::with_capacity(ids.len());
    for (index, id) in ids.iter().enumerate() {
        validate_document_id(id)?;
        let mut attributes = Map::new();
        for (column, values) in columns {
            if column == "id" {
                continue;
            }
            attributes.insert(column.clone(), values[index].clone());
        }
        rows.push(WriteRow {
            id: id.clone(),
            attributes,
        });
    }
    Ok(rows)
}

fn write_row_logical_bytes(row: &WriteRow) -> usize {
    serde_json::to_vec(&json!({
        "id": row.id,
        "attributes": row.attributes
    }))
    .map(|bytes| bytes.len())
    .unwrap_or(0)
}

fn collect_delete_ids(deletes: Option<&Value>) -> Result<Vec<Value>, QueryError> {
    let Some(deletes) = deletes else {
        return Ok(Vec::new());
    };
    let deletes = deletes.as_array().ok_or_else(|| {
        QueryError::unprocessable(
            "Failed to deserialize the JSON body into the target type: deletes: data did not match any variant of untagged enum IdVec",
        )
    })?;
    deletes
        .iter()
        .map(|id| validate_document_id(id).map(|()| id.clone()))
        .collect()
}

fn reject_duplicate_write_ids(
    upserts: &[WriteRow],
    patches: &[WriteRow],
    deletes: &[Value],
) -> Result<(), QueryError> {
    let mut seen = BTreeSet::new();
    let mut duplicates = BTreeMap::new();
    for id in upserts
        .iter()
        .map(|row| &row.id)
        .chain(patches.iter().map(|row| &row.id))
        .chain(deletes.iter())
    {
        let key = id_key(id);
        if !seen.insert(key) {
            duplicates
                .entry(id_key(id))
                .or_insert_with(|| document_id_error_value(id));
        }
    }
    if !duplicates.is_empty() {
        let duplicated_ids = duplicates.into_values().collect::<Vec<_>>().join(",");
        return Err(QueryError::new(format!(
            "💔 This upsert contains duplicate document IDs and was not written. You should ensure that individual upserts do not include duplicate documents. The duplicated IDs in this batch were the following: {duplicated_ids}"
        )));
    }
    Ok(())
}

fn upsert_document(
    namespace: &mut Namespace,
    row: WriteRow,
    condition: Option<&Value>,
    id_index: &mut WriteIdIndex,
) -> Result<bool, QueryError> {
    if let Some(index) = id_index.get(&row.id) {
        if !condition
            .map(|filter| eval_filter_with_new(&namespace.documents[index], filter, Some(&row)))
            .transpose()?
            .unwrap_or(true)
        {
            return Ok(false);
        }
        let previous = namespace.documents[index].clone();
        remove_document_from_cached_indexes(namespace, index, &previous);
        namespace.documents[index] = Document::new(row.id, row.attributes);
        let updated = namespace.documents[index].clone();
        insert_document_into_cached_indexes(namespace, index, &updated);
    } else {
        let index = namespace.documents.len();
        let id = row.id;
        id_index.insert_new(&id, index);
        namespace.documents.push(Document::new(id, row.attributes));
        let inserted = namespace.documents[index].clone();
        insert_document_into_cached_indexes(namespace, index, &inserted);
    }
    Ok(true)
}

fn patch_document(
    namespace: &mut Namespace,
    row: WriteRow,
    condition: Option<&Value>,
    id_index: &WriteIdIndex,
) -> Result<bool, QueryError> {
    let Some(index) = id_index.get(&row.id) else {
        return Ok(false);
    };
    if !condition
        .map(|filter| eval_filter_with_new(&namespace.documents[index], filter, Some(&row)))
        .transpose()?
        .unwrap_or(true)
    {
        return Ok(false);
    }
    let previous = namespace.documents[index].clone();
    remove_document_from_cached_indexes(namespace, index, &previous);
    for (key, value) in row.attributes {
        namespace.documents[index].set_attribute(key, value);
    }
    let updated = namespace.documents[index].clone();
    insert_document_into_cached_indexes(namespace, index, &updated);
    Ok(true)
}

fn delete_documents(
    namespace: &mut Namespace,
    ids: Vec<Value>,
    condition: Option<&Value>,
    id_index: &mut WriteIdIndex,
) -> Result<Vec<Value>, QueryError> {
    let mut deleted = Vec::new();
    let mut indexes = Vec::new();
    for id in ids {
        let Some(index) = id_index.get(&id) else {
            continue;
        };
        if condition
            .map(|filter| eval_filter_with_new(&namespace.documents[index], filter, None))
            .transpose()?
            .unwrap_or(true)
        {
            deleted.push(id);
            indexes.push(index);
        }
    }
    if !indexes.is_empty() {
        let selected = indexes.into_iter().collect::<BTreeSet<_>>();
        let mut kept = Vec::with_capacity(namespace.documents.len() - selected.len());
        for (index, document) in namespace.documents.drain(..).enumerate() {
            if !selected.contains(&index) {
                kept.push(document);
            }
        }
        namespace.documents = kept;
        namespace.query_indexes.guard().clear();
        id_index.rebuild(namespace);
    }
    Ok(deleted)
}

fn delete_by_filter(
    namespace: &mut Namespace,
    filter: &Value,
    allow_partial: bool,
) -> Result<FilterWriteOutcome, QueryError> {
    let matching = namespace
        .documents
        .iter()
        .enumerate()
        .map(|(index, document)| {
            eval_filter_with_schema(document, filter, Some(&namespace.schema))
                .map(|matches| (index, matches))
        })
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .filter_map(|(index, matches)| matches.then_some(index))
        .collect::<Vec<_>>();
    let matched_count = matching.len();
    if matched_count > DELETE_BY_FILTER_LIMIT && !allow_partial {
        return Err(QueryError::new(
            "delete_by_filter matched more documents than allowed for one request.",
        ));
    }
    let selected = matching
        .into_iter()
        .take(DELETE_BY_FILTER_LIMIT)
        .collect::<BTreeSet<_>>();
    let rows_remaining = matched_count > DELETE_BY_FILTER_LIMIT;
    if selected.is_empty() {
        return Ok(FilterWriteOutcome {
            ids: Vec::new(),
            rows_remaining,
        });
    }
    let mut deleted = Vec::new();
    let mut kept = Vec::new();
    for (index, document) in namespace.documents.drain(..).enumerate() {
        if selected.contains(&index) {
            deleted.push(document.id);
        } else {
            kept.push(document);
        }
    }
    namespace.documents = kept;
    namespace.query_indexes.guard().clear();
    Ok(FilterWriteOutcome {
        ids: deleted,
        rows_remaining,
    })
}

fn patch_by_filter(
    namespace: &mut Namespace,
    request: &Value,
    allow_partial: bool,
) -> Result<FilterWriteOutcome, QueryError> {
    let request = as_object(request, "patch_by_filter")?;
    let filter = request
        .get("filters")
        .ok_or_else(|| QueryError::new("patch_by_filter.filters is required."))?;
    let patch = as_object(
        request
            .get("patch")
            .ok_or_else(|| QueryError::new("patch_by_filter.patch is required."))?,
        "patch_by_filter.patch",
    )?;
    for key in patch.keys() {
        validate_attribute_name(key)?;
    }
    let patch_row = WriteRow {
        id: Value::String("$patch_by_filter".to_string()),
        attributes: patch.clone(),
    };
    validate_write_rows_against_schema(namespace, &[], &[patch_row])?;
    let matching = namespace
        .documents
        .iter()
        .enumerate()
        .map(|(index, document)| {
            eval_filter_with_schema(document, filter, Some(&namespace.schema))
                .map(|matches| (index, matches))
        })
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .filter_map(|(index, matches)| matches.then_some(index))
        .collect::<Vec<_>>();
    if matching.len() > PATCH_BY_FILTER_LIMIT && !allow_partial {
        return Err(QueryError::new(
            "patch_by_filter matched more documents than allowed for one request.",
        ));
    }
    let rows_remaining = matching.len() > PATCH_BY_FILTER_LIMIT;
    let selected = matching
        .into_iter()
        .take(PATCH_BY_FILTER_LIMIT)
        .collect::<BTreeSet<_>>();
    let mut patched = Vec::new();
    let previous = selected
        .iter()
        .filter_map(|index| {
            namespace
                .documents
                .get(*index)
                .cloned()
                .map(|document| (*index, document))
        })
        .collect::<Vec<_>>();
    for (index, document) in &previous {
        remove_document_from_cached_indexes(namespace, *index, document);
    }
    for (index, document) in namespace.documents.iter_mut().enumerate() {
        if selected.contains(&index) {
            for (key, value) in patch {
                document.set_attribute(key.clone(), value.clone());
            }
            patched.push(document.id.clone());
        }
    }
    let updated = selected
        .iter()
        .filter_map(|index| {
            namespace
                .documents
                .get(*index)
                .cloned()
                .map(|document| (*index, document))
        })
        .collect::<Vec<_>>();
    for (index, document) in &updated {
        insert_document_into_cached_indexes(namespace, *index, document);
    }
    Ok(FilterWriteOutcome {
        ids: patched,
        rows_remaining,
    })
}

fn validate_document_id(id: &Value) -> Result<(), QueryError> {
    match id {
        Value::Number(number) if number.as_u64().is_some() => Ok(()),
        Value::String(text) if text.len() <= 64 => Ok(()),
        _ => Err(QueryError::new(
            "document IDs must be unsigned integers or strings up to 64 bytes.",
        )),
    }
}

fn effective_write_id_schema_type(
    namespace: Option<&Namespace>,
    schema: Option<&Value>,
) -> Result<Option<String>, QueryError> {
    let requested = schema
        .map(|schema| -> Result<Option<String>, QueryError> {
            let schema = as_object(schema, "schema")?;
            schema
                .get("id")
                .map(|definition| {
                    incoming_schema_type_name("id", definition).map(ToString::to_string)
                })
                .transpose()
        })
        .transpose()?
        .flatten();
    if requested.is_some() {
        return Ok(requested);
    }
    namespace
        .and_then(|namespace| namespace.schema.get("id"))
        .map(schema_type_name)
        .transpose()
        .map(|schema_type| schema_type.map(ToString::to_string))
}

fn normalize_write_ids_against_id_schema(
    id_schema_type: Option<&str>,
    upserts: &mut [WriteRow],
    patches: &mut [WriteRow],
    deletes: &mut [Value],
) -> Result<(), QueryError> {
    if id_schema_type != Some("uuid") {
        return Ok(());
    }
    let all_ids = upserts
        .iter()
        .map(|row| &row.id)
        .chain(patches.iter().map(|row| &row.id))
        .chain(deletes.iter());
    for id in all_ids {
        canonical_uuid_id(id)?;
    }
    for id in upserts
        .iter_mut()
        .map(|row| &mut row.id)
        .chain(patches.iter_mut().map(|row| &mut row.id))
        .chain(deletes.iter_mut())
    {
        *id = Value::String(canonical_uuid_id(id)?);
    }
    Ok(())
}

fn uuid_id_error() -> QueryError {
    QueryError::new("💔 namespace ID type is uuid, but a written ID could not be parsed as uuid")
}

fn canonical_uuid_id(id: &Value) -> Result<String, QueryError> {
    let Some(text) = id.as_str() else {
        return Err(uuid_id_error());
    };
    canonical_uuid_text(text).ok_or_else(uuid_id_error)
}

fn canonical_uuid_text(text: &str) -> Option<String> {
    if let Some(rest) = text.strip_prefix("urn:uuid:") {
        return canonical_hyphenated_uuid_text(rest);
    }
    if let Some(rest) = text
        .strip_prefix('{')
        .and_then(|rest| rest.strip_suffix('}'))
    {
        return canonical_hyphenated_uuid_text(rest);
    }
    if text.len() == 32 {
        return canonical_simple_uuid_text(text);
    }
    canonical_hyphenated_uuid_text(text)
}

fn canonical_simple_uuid_text(text: &str) -> Option<String> {
    if text.len() != 32 || !text.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    let lower = text.to_ascii_lowercase();
    Some(format!(
        "{}-{}-{}-{}-{}",
        &lower[0..8],
        &lower[8..12],
        &lower[12..16],
        &lower[16..20],
        &lower[20..32]
    ))
}

fn canonical_hyphenated_uuid_text(text: &str) -> Option<String> {
    if text.len() != 36 {
        return None;
    }
    if !text.bytes().enumerate().all(|(index, byte)| {
        matches!(index, 8 | 13 | 18 | 23) && byte == b'-'
            || !matches!(index, 8 | 13 | 18 | 23) && byte.is_ascii_hexdigit()
    }) {
        return None;
    }
    Some(text.to_ascii_lowercase())
}

fn validate_attribute_name(name: &str) -> Result<(), QueryError> {
    if name == "id" {
        return Ok(());
    }
    if name.is_empty() || name.len() > 128 || name.starts_with('$') {
        return Err(QueryError::new(
            "attribute names must be 1 to 128 chars and must not start with '$'.",
        ));
    }
    Ok(())
}

fn id_key(id: &Value) -> String {
    match id {
        Value::Number(number) => number
            .as_u64()
            .map(|id| format!("number:{id}"))
            .or_else(|| legacy_float_id_key(number))
            .unwrap_or_else(|| id.to_string()),
        Value::String(text) => format!("string:{text}"),
        _ => id.to_string(),
    }
}

fn document_id_error_value(id: &Value) -> String {
    match id {
        Value::String(text) => text.clone(),
        _ => id.to_string(),
    }
}

fn legacy_float_id_key(number: &Number) -> Option<String> {
    const MAX_SAFE_INTEGER: f64 = 9_007_199_254_740_991.0;
    let id = number.as_f64()?;
    if id.is_finite() && (0.0..=MAX_SAFE_INTEGER).contains(&id) && id.fract() == 0.0 {
        Some(format!("number:{}", id as u64))
    } else {
        None
    }
}

fn validate_multi_query_root(object: &Map<String, Value>) -> Result<(), QueryError> {
    for key in object.keys() {
        if key != "queries" && key != "vector_encoding" && key != "consistency" {
            return Err(QueryError::new(
                "queries is mutually exclusive with ordinary query fields.",
            ));
        }
    }
    Ok(())
}

fn parse_vector_encoding(value: Option<&Value>) -> Result<VectorEncoding, QueryError> {
    match value {
        None => Ok(VectorEncoding::Float),
        Some(Value::String(encoding)) if encoding == "float" => Ok(VectorEncoding::Float),
        Some(Value::String(encoding)) if encoding == "base64" => Ok(VectorEncoding::Base64),
        Some(Value::String(encoding)) => Err(QueryError::unprocessable(format!(
            "Failed to deserialize the JSON body into the target type: vector_encoding: unknown variant `{encoding}`, expected `float` or `base64`"
        ))),
        Some(value) => Err(QueryError::unprocessable(format!(
            "Failed to deserialize the JSON body into the target type: vector_encoding: invalid type: {}, expected `float` or `base64`",
            serde_type_name(value)
        ))),
    }
}

fn validate_consistency_shape(object: &Map<String, Value>) -> Result<(), QueryError> {
    let Some(consistency) = object.get("consistency") else {
        return Ok(());
    };
    let Some(consistency) = consistency.as_object() else {
        return Err(QueryError::unprocessable(format!(
            "Failed to deserialize the JSON body into the target type: consistency: invalid type: {}, expected struct Consistency",
            serde_consistency_type_name(consistency)
        )));
    };
    let Some(level) = consistency.get("level") else {
        return Err(QueryError::unprocessable(
            "Failed to deserialize the JSON body into the target type: consistency: missing field `level`",
        ));
    };
    let Some(level) = level.as_str() else {
        return Err(QueryError::new(
            "Failed to parse the request body as JSON: consistency.level: expected value",
        ));
    };
    match level {
        "strong" | "eventual" => Ok(()),
        level => Err(QueryError::unprocessable(format!(
            "Failed to deserialize the JSON body into the target type: consistency.level: unknown variant `{level}`, expected `strong` or `eventual`"
        ))),
    }
}

fn serde_consistency_type_name(value: &Value) -> String {
    match value {
        Value::String(value) => format!("string \"{value}\""),
        _ => serde_type_name(value),
    }
}

fn validate_projection_shape(object: &Map<String, Value>) -> Result<(), QueryError> {
    if let Some(include_attributes) = object.get("include_attributes") {
        match include_attributes {
            Value::Bool(_) => {}
            Value::Array(attributes) if attributes.iter().all(Value::is_string) => {}
            _ => {
                return Err(QueryError::unprocessable(
                    "Failed to deserialize the JSON body into the target type: data did not match any variant of untagged enum IncludeAttributes",
                ));
            }
        }
    }
    if let Some(exclude_attributes) = object.get("exclude_attributes") {
        let attributes = match exclude_attributes {
            Value::Array(attributes) => attributes,
            Value::Bool(value) => {
                return Err(QueryError::unprocessable(format!(
                    "Failed to deserialize the JSON body into the target type: invalid type: boolean `{value}`, expected a sequence"
                )));
            }
            value => {
                return Err(QueryError::unprocessable(format!(
                    "Failed to deserialize the JSON body into the target type: invalid type: {}, expected a sequence",
                    serde_type_name(value)
                )));
            }
        };
        if let Some(invalid) = attributes.iter().find(|attribute| !attribute.is_string()) {
            return Err(QueryError::unprocessable(format!(
                "Failed to deserialize the JSON body into the target type: invalid type: {}, expected a string",
                serde_type_name(invalid)
            )));
        }
    }
    Ok(())
}

fn validate_included_attributes(
    namespace: &Namespace,
    object: &Map<String, Value>,
) -> Result<(), QueryError> {
    let Some(Value::Array(attributes)) = object.get("include_attributes") else {
        return Ok(());
    };
    let schema = schema_with_id(namespace);
    for attribute in attributes {
        let name = as_string(attribute, "include_attributes item")?;
        if !schema.contains_key(name) {
            return Err(QueryError::new(format!(
                "💔 attribute \"{name}\" not found in schema, cannot be part of `include_attributes`. consider passing `include_attributes=True` to return all attribute data instead"
            )));
        }
    }
    Ok(())
}

fn serde_type_name(value: &Value) -> String {
    match value {
        Value::Null => "null".to_string(),
        Value::Bool(value) => format!("boolean `{value}`"),
        Value::Number(number) if number.is_i64() || number.is_u64() => {
            format!("integer `{number}`")
        }
        Value::Number(number) => format!("floating point `{number}`"),
        Value::String(value) => format!("string `{value}`"),
        Value::Array(_) => "sequence".to_string(),
        Value::Object(_) => "map".to_string(),
    }
}

fn parse_limit(object: &Map<String, Value>) -> Result<Limit, QueryError> {
    if let Some(top_k) = object.get("top_k") {
        return Ok(Limit {
            total: parse_positive_usize(top_k, "top_k")?,
            per: None,
        });
    }
    let limit = object
        .get("limit")
        .ok_or_else(|| QueryError::new("limit is required."))?;
    if let Some(total) = value_as_u64(limit) {
        return Ok(Limit {
            total: clamp_limit(total, "limit")?,
            per: None,
        });
    }
    let limit_object = as_object(limit, "limit")?;
    let total = parse_positive_usize(
        limit_object
            .get("total")
            .ok_or_else(|| QueryError::new("limit.total is required."))?,
        "limit.total",
    )?;
    let per = match limit_object.get("per") {
        None => None,
        Some(value) => {
            let per_object = as_object(value, "limit.per")?;
            let attrs = as_array(
                per_object
                    .get("attributes")
                    .ok_or_else(|| QueryError::new("limit.per.attributes is required."))?,
                "limit.per.attributes",
            )?
            .iter()
            .map(|attr| as_string(attr, "limit.per.attributes item").map(ToString::to_string))
            .collect::<Result<Vec<_>, _>>()?;
            let per_limit = parse_positive_usize(
                per_object
                    .get("limit")
                    .ok_or_else(|| QueryError::new("limit.per.limit is required."))?,
                "limit.per.limit",
            )?;
            Some(PerLimit {
                attributes: attrs,
                limit: per_limit,
            })
        }
    };
    Ok(Limit { total, per })
}

fn parse_positive_usize(value: &Value, label: &str) -> Result<usize, QueryError> {
    let raw =
        value_as_u64(value).ok_or_else(|| QueryError::new(format!("{label} must be a number.")))?;
    clamp_limit(raw, label)
}

fn clamp_limit(raw: u64, label: &str) -> Result<usize, QueryError> {
    if raw == 0 || raw > 10_000 {
        return Err(QueryError::new(format!(
            "{label} must be between 1 and 10,000."
        )));
    }
    usize::try_from(raw).map_err(|_| QueryError::new(format!("{label} is too large.")))
}

fn parse_rank_plan<'a>(rank_by: &'a Value, has_filters: bool) -> Result<RankPlan<'a>, QueryError> {
    let array = as_array(rank_by, "rank_by")?;
    let kind = if !array.is_empty() && array.iter().all(Value::is_array) {
        let attributes = array
            .iter()
            .map(parse_attribute_order)
            .collect::<Result<Vec<_>, _>>()?;
        RankKind::MultiAttributeOrder { attributes }
    } else if array.len() == 2 {
        let attr = as_string(&array[0], "rank_by attribute")?;
        if array[1].as_str() == Some("asc") {
            RankKind::AttributeOrder {
                attribute: attr.to_string(),
                direction: SortDirection::Asc,
            }
        } else if array[1].as_str() == Some("desc") {
            RankKind::AttributeOrder {
                attribute: attr.to_string(),
                direction: SortDirection::Desc,
            }
        } else {
            rank_expression_kind(rank_by, has_filters)?
        }
    } else {
        rank_expression_kind(rank_by, has_filters)?
    };
    Ok(RankPlan { rank_by, kind })
}

fn parse_attribute_order(value: &Value) -> Result<(String, SortDirection), QueryError> {
    let array = as_array(value, "rank_by attribute order")?;
    if array.len() != 2 {
        return Err(QueryError::new(
            "rank_by attribute order must have [attribute, direction].",
        ));
    }
    let attribute = as_string(&array[0], "rank_by attribute")?.to_string();
    let direction = match array[1].as_str() {
        Some("asc") => SortDirection::Asc,
        Some("desc") => SortDirection::Desc,
        _ => {
            return Err(QueryError::new(
                "rank_by attribute order direction must be 'asc' or 'desc'.",
            ));
        }
    };
    Ok((attribute, direction))
}

fn rank_expression_kind(rank_by: &Value, has_filters: bool) -> Result<RankKind, QueryError> {
    let array = as_array(rank_by, "rank_by")?;
    if array.len() >= 2
        && let Some(op) = array[1].as_str()
    {
        if op == "ANN" {
            return Ok(RankKind::SmallerIsBetter);
        }
        if op == "kNN" {
            if !has_filters {
                return Err(QueryError::new("kNN requires filters."));
            }
            return Ok(RankKind::SmallerIsBetter);
        }
        if op == "SparseKNN" {
            return Ok(RankKind::LargerIsBetter);
        }
        if op == "BM25" {
            return Ok(RankKind::LargerIsBetter);
        }
    }
    Ok(RankKind::LargerIsBetter)
}

fn prepare_rank_plan<'a>(
    rank_plan: RankPlan<'a>,
    namespace: &Namespace,
) -> Result<PreparedRankPlan<'a>, QueryError> {
    let expression = match rank_plan.kind {
        RankKind::AttributeOrder { .. } | RankKind::MultiAttributeOrder { .. } => None,
        RankKind::SmallerIsBetter | RankKind::LargerIsBetter => {
            Some(prepare_rank_expr(rank_plan.rank_by, namespace)?)
        }
    };
    Ok(PreparedRankPlan {
        kind: rank_plan.kind,
        expression,
    })
}

fn prepare_rank_expr<'a>(
    expression: &'a Value,
    namespace: &Namespace,
) -> Result<PreparedRankExpr<'a>, QueryError> {
    if let Some(number) = expression.as_f64() {
        return Ok(PreparedRankExpr::Literal(number));
    }
    let array = as_array(expression, "rank expression")?;
    if array.is_empty() {
        return Err(QueryError::new("rank expression cannot be empty."));
    }
    if let Some(op) = array[0].as_str() {
        match op {
            "Sum" => return prepare_sum(array, namespace),
            "Max" => return prepare_max(array, namespace),
            "Product" => return prepare_product(array, namespace),
            "Attribute" => {
                if array.len() != 2 {
                    return Err(QueryError::new("Attribute requires one attribute name."));
                }
                return Ok(PreparedRankExpr::Attribute(as_string(
                    &array[1],
                    "Attribute name",
                )?));
            }
            "Saturate" => return prepare_saturate(array, namespace),
            "Decay" => return prepare_decay(array, namespace),
            "Dist" => return prepare_dist(array, namespace),
            "And" | "Or" | "Not" => return Ok(PreparedRankExpr::Filter(expression)),
            _ => {}
        }
    }
    if array.len() >= 3 {
        let attr = as_string(&array[0], "rank_by attribute")?;
        let op = as_string(&array[1], "rank_by operator")?;
        return match op {
            "ANN" | "kNN" => {
                let values = numeric_array(&array[2], "query vector")?;
                let norm = vector_norm(&values);
                let query = PreparedDenseQuery { values, norm };
                if attr == "vector" {
                    Ok(PreparedRankExpr::DenseDistanceVector { query })
                } else {
                    Ok(PreparedRankExpr::DenseDistance {
                        attribute: attr,
                        query,
                    })
                }
            }
            "SparseKNN" => Ok(PreparedRankExpr::SparseDotProduct {
                attribute: attr,
                query: sparse_map(&array[2], "query sparse vector")?,
            }),
            "BM25" => {
                let query = prepare_bm25_query(attr, &array[2], array.get(3), namespace)?;
                Ok(PreparedRankExpr::Bm25 {
                    scores: PreparedBm25Scores::new(score_indexed_bm25(namespace, attr, &query)),
                })
            }
            _ => Ok(PreparedRankExpr::Filter(expression)),
        };
    }
    Err(QueryError::new("unsupported rank expression."))
}

fn prepare_sum<'a>(
    array: &'a [Value],
    namespace: &Namespace,
) -> Result<PreparedRankExpr<'a>, QueryError> {
    let terms = array
        .get(1)
        .ok_or_else(|| QueryError::new("Sum requires terms."))?;
    let terms = as_array(terms, "Sum terms")?
        .iter()
        .map(|term| prepare_rank_expr(term, namespace))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(PreparedRankExpr::Sum(terms))
}

fn prepare_max<'a>(
    array: &'a [Value],
    namespace: &Namespace,
) -> Result<PreparedRankExpr<'a>, QueryError> {
    let terms = if array.len() == 2 {
        as_array(&array[1], "Max terms")?.iter().collect::<Vec<_>>()
    } else {
        array.iter().skip(1).collect::<Vec<_>>()
    };
    Ok(PreparedRankExpr::Max(
        terms
            .into_iter()
            .map(|term| prepare_rank_expr(term, namespace))
            .collect::<Result<Vec<_>, _>>()?,
    ))
}

fn prepare_product<'a>(
    array: &'a [Value],
    namespace: &Namespace,
) -> Result<PreparedRankExpr<'a>, QueryError> {
    if array.len() != 3 {
        return Err(QueryError::new(
            "Product requires a weight and an expression.",
        ));
    }
    let weight = array[1]
        .as_f64()
        .ok_or_else(|| QueryError::new("Product weight must be numeric."))?;
    if weight < 0.0 {
        return Err(QueryError::new("Product weight must be non-negative."));
    }
    Ok(PreparedRankExpr::Product {
        weight,
        expression: Box::new(prepare_rank_expr(&array[2], namespace)?),
    })
}

fn prepare_saturate<'a>(
    array: &'a [Value],
    namespace: &Namespace,
) -> Result<PreparedRankExpr<'a>, QueryError> {
    if array.len() < 2 {
        return Err(QueryError::new("Saturate requires an expression."));
    }
    let options = array.get(2).and_then(Value::as_object);
    Ok(PreparedRankExpr::Saturate {
        expression: Box::new(prepare_rank_expr(&array[1], namespace)?),
        midpoint: options
            .and_then(|object| object.get("midpoint"))
            .and_then(value_as_f64)
            .unwrap_or(1.0),
        exponent: options
            .and_then(|object| object.get("exponent"))
            .and_then(value_as_f64)
            .unwrap_or(1.0),
    })
}

fn prepare_decay<'a>(
    array: &'a [Value],
    namespace: &Namespace,
) -> Result<PreparedRankExpr<'a>, QueryError> {
    if array.len() < 2 {
        return Err(QueryError::new("Decay requires an expression."));
    }
    let options = array.get(2).and_then(Value::as_object);
    Ok(PreparedRankExpr::Decay {
        expression: Box::new(prepare_rank_expr(&array[1], namespace)?),
        midpoint: options
            .and_then(|object| object.get("midpoint"))
            .map(parse_midpoint)
            .transpose()?
            .unwrap_or(1.0),
        exponent: options
            .and_then(|object| object.get("exponent"))
            .and_then(value_as_f64)
            .unwrap_or(1.0),
    })
}

fn prepare_dist<'a>(
    array: &'a [Value],
    namespace: &Namespace,
) -> Result<PreparedRankExpr<'a>, QueryError> {
    if array.len() != 3 {
        return Err(QueryError::new(
            "Dist requires an expression and an origin.",
        ));
    }
    let expression = if let Some(attribute_expr) = array[1].as_array() {
        if attribute_expr.first().and_then(Value::as_str) == Some("Attribute") {
            let attr = attribute_expr
                .get(1)
                .and_then(Value::as_str)
                .ok_or_else(|| QueryError::new("Dist Attribute requires a name."))?;
            PreparedDistExpr::Attribute(attr)
        } else {
            PreparedDistExpr::Rank(Box::new(prepare_rank_expr(&array[1], namespace)?))
        }
    } else {
        PreparedDistExpr::Rank(Box::new(prepare_rank_expr(&array[1], namespace)?))
    };
    Ok(PreparedRankExpr::Dist {
        expression,
        origin: &array[2],
    })
}

fn prepare_bm25_query(
    field: &str,
    query: &Value,
    options: Option<&Value>,
    namespace: &Namespace,
) -> Result<PreparedBm25Query, QueryError> {
    let config = fts_config_for_field(namespace, field);
    if config.tokenizer != Tokenizer::PreTokenizedArray && query.is_array() {
        return Err(QueryError::new(format!(
            "💔 invalid input '{}' for rank_by field \"{field}\", expecting string",
            format_bm25_query_input(query)
        )));
    }
    let query_tokens = query_tokens_with_config(query, "BM25 query", &config)?;
    if query_tokens.is_empty() {
        return Ok(PreparedBm25Query { terms: Vec::new() });
    }
    let last_as_prefix = options
        .and_then(Value::as_object)
        .and_then(|object| object.get("last_as_prefix"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let mut query_term_counts: HashMap<&String, usize> = HashMap::new();
    for token in &query_tokens {
        let count = query_term_counts.entry(token).or_insert(0);
        *count += 1;
    }
    let mut terms = Vec::new();
    for (index, token) in query_tokens.iter().enumerate() {
        if query_tokens[..index].contains(token) {
            continue;
        }
        terms.push(PreparedBm25Term {
            token: token.clone(),
            query_frequency: query_term_counts.get(token).copied().unwrap_or(1),
            prefix: last_as_prefix && index + 1 == query_tokens.len(),
        });
    }
    Ok(PreparedBm25Query { terms })
}

fn format_bm25_query_input(query: &Value) -> String {
    let Some(items) = query.as_array() else {
        return query.to_string();
    };
    let values = items
        .iter()
        .map(|item| {
            item.as_str()
                .map_or_else(|| item.to_string(), ToString::to_string)
        })
        .collect::<Vec<_>>();
    format!("[{}]", values.join(", "))
}

fn rank_documents<'a>(
    namespace: &'a Namespace,
    rank_plan: &PreparedRankPlan<'_>,
    filters: Option<&Value>,
) -> Result<Vec<RankedDocument<'a>>, QueryError> {
    if let Some(scores) = simple_bm25_scores(rank_plan) {
        return rank_indexed_bm25_documents(namespace, scores, filters);
    }
    if let Some((attribute, query)) = simple_sparse_query(rank_plan) {
        return rank_indexed_sparse_documents(namespace, attribute, query, filters);
    }

    let mut ranked = Vec::new();
    let indexed_candidates = filters
        .map(|filter| indexed_filter_candidates(namespace, filter))
        .transpose()?
        .flatten();
    let candidate_indexes = indexed_candidates
        .as_deref()
        .map(CandidateIndexes::Indexed)
        .unwrap_or_else(|| CandidateIndexes::All(0..namespace.documents.len()));
    for doc_index in candidate_indexes {
        let document = namespace
            .documents
            .get(doc_index)
            .ok_or_else(|| QueryError::new("query index referenced a missing document."))?;
        if !filters
            .map(|filter| eval_filter_with_schema(document, filter, Some(&namespace.schema)))
            .transpose()?
            .unwrap_or(true)
        {
            continue;
        }
        let score = match rank_plan.kind {
            RankKind::AttributeOrder { .. } | RankKind::MultiAttributeOrder { .. } => 0.0,
            RankKind::SmallerIsBetter | RankKind::LargerIsBetter => eval_prepared_rank_expr(
                doc_index,
                document,
                rank_plan
                    .expression
                    .as_ref()
                    .ok_or_else(|| QueryError::new("rank expression was not prepared."))?,
                namespace.distance_metric,
            )?,
        };
        if matches!(
            rank_plan.kind,
            RankKind::AttributeOrder { .. }
                | RankKind::MultiAttributeOrder { .. }
                | RankKind::SmallerIsBetter
        ) || score != 0.0
        {
            ranked.push(RankedDocument {
                doc: document,
                score,
            });
        }
    }
    Ok(ranked)
}

fn simple_bm25_scores<'a>(rank_plan: &'a PreparedRankPlan<'_>) -> Option<&'a PreparedBm25Scores> {
    if !matches!(rank_plan.kind, RankKind::LargerIsBetter) {
        return None;
    }
    match rank_plan.expression.as_ref()? {
        PreparedRankExpr::Bm25 { scores } => Some(scores),
        _ => None,
    }
}

fn simple_sparse_query<'a>(
    rank_plan: &'a PreparedRankPlan<'_>,
) -> Option<(&'a str, &'a HashMap<String, f64>)> {
    if !matches!(rank_plan.kind, RankKind::LargerIsBetter) {
        return None;
    }
    match rank_plan.expression.as_ref()? {
        PreparedRankExpr::SparseDotProduct { attribute, query } => Some((attribute, query)),
        _ => None,
    }
}

fn rank_indexed_bm25_documents<'a>(
    namespace: &'a Namespace,
    scores: &PreparedBm25Scores,
    filters: Option<&Value>,
) -> Result<Vec<RankedDocument<'a>>, QueryError> {
    let mut ranked = Vec::with_capacity(scores.ranked.len());
    for (doc_index, score) in scores.ranked.iter().copied() {
        let Some(document) = namespace.documents.get(doc_index) else {
            continue;
        };
        if !filters
            .map(|filter| eval_filter_with_schema(document, filter, Some(&namespace.schema)))
            .transpose()?
            .unwrap_or(true)
        {
            continue;
        }
        ranked.push(RankedDocument {
            doc: document,
            score,
        });
    }
    Ok(ranked)
}

fn rank_indexed_sparse_documents<'a>(
    namespace: &'a Namespace,
    attribute: &str,
    query: &HashMap<String, f64>,
    filters: Option<&Value>,
) -> Result<Vec<RankedDocument<'a>>, QueryError> {
    let scores = score_indexed_sparse(namespace, attribute, query);
    let mut ranked = Vec::with_capacity(scores.len());
    for (doc_index, score) in scores {
        let Some(document) = namespace.documents.get(doc_index) else {
            continue;
        };
        if !filters
            .map(|filter| eval_filter_with_schema(document, filter, Some(&namespace.schema)))
            .transpose()?
            .unwrap_or(true)
        {
            continue;
        }
        ranked.push(RankedDocument {
            doc: document,
            score,
        });
    }
    Ok(ranked)
}

enum CandidateIndexes<'a> {
    All(std::ops::Range<usize>),
    Indexed(&'a [usize]),
}

impl<'a> IntoIterator for CandidateIndexes<'a> {
    type Item = usize;
    type IntoIter = CandidateIndexIter<'a>;

    fn into_iter(self) -> Self::IntoIter {
        match self {
            CandidateIndexes::All(indexes) => CandidateIndexIter::All(indexes),
            CandidateIndexes::Indexed(indexes) => CandidateIndexIter::Indexed(indexes.iter()),
        }
    }
}

enum CandidateIndexIter<'a> {
    All(std::ops::Range<usize>),
    Indexed(std::slice::Iter<'a, usize>),
}

impl Iterator for CandidateIndexIter<'_> {
    type Item = usize;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            CandidateIndexIter::All(range) => range.next(),
            CandidateIndexIter::Indexed(indexes) => indexes.next().copied(),
        }
    }
}

fn indexed_order_ranked<'a>(
    namespace: &'a Namespace,
    kind: &RankKind,
    filters: Option<&Value>,
    limit: usize,
) -> Result<Option<Vec<RankedDocument<'a>>>, QueryError> {
    let order_key = match order_index_key(kind) {
        Some(order_key) => order_key,
        None => return Ok(None),
    };
    let filter_candidates = filters
        .map(|filter| indexed_filter_candidates(namespace, filter))
        .transpose()?
        .flatten();
    if filters.is_some() && filter_candidates.is_none() {
        return Ok(None);
    }
    if filter_candidates.as_ref().is_some_and(Vec::is_empty) {
        return Ok(Some(Vec::new()));
    }
    let order = order_index_positions(namespace, &order_key, filter_candidates.as_deref(), limit);
    let mut ranked = Vec::with_capacity(order.len());
    for index in order {
        let document = namespace
            .documents
            .get(index)
            .ok_or_else(|| QueryError::new("order index referenced a missing document."))?;
        if !filters
            .map(|filter| eval_filter_with_schema(document, filter, Some(&namespace.schema)))
            .transpose()?
            .unwrap_or(true)
        {
            continue;
        }
        ranked.push(RankedDocument {
            doc: document,
            score: 0.0,
        });
        if ranked.len() == limit {
            break;
        }
    }
    Ok(Some(ranked))
}

fn indexed_filter_candidates(
    namespace: &Namespace,
    filter: &Value,
) -> Result<Option<Vec<usize>>, QueryError> {
    let array = as_array(filter, "filter")?;
    if array.is_empty() {
        return Ok(None);
    }
    if let Some(op) = array[0].as_str() {
        match op {
            "And" => return indexed_and_candidates(namespace, array),
            "Or" => return indexed_or_candidates(namespace, array),
            "Not" => return Ok(None),
            _ => {}
        }
    }
    if array.len() < 3 {
        return Ok(None);
    }
    let attr = as_string(&array[0], "filter attribute")?;
    let op = as_string(&array[1], "filter operator")?;
    if op != "Eq" {
        return Ok(None);
    }
    let Some(key) = scalar_eq_key(&array[2]) else {
        return Ok(None);
    };
    Ok(Some(equality_postings(namespace, attr, &key)))
}

fn indexed_and_candidates(
    namespace: &Namespace,
    array: &[Value],
) -> Result<Option<Vec<usize>>, QueryError> {
    let filters = as_array(
        array
            .get(1)
            .ok_or_else(|| QueryError::new("And requires filters."))?,
        "And filters",
    )?;
    if filters.is_empty() {
        return Ok(None);
    }
    let mut candidates: Option<Vec<usize>> = None;
    for child in filters {
        let Some(child_candidates) = indexed_filter_candidates(namespace, child)? else {
            return Ok(None);
        };
        candidates = Some(match candidates {
            Some(current) => sorted_intersection(&current, &child_candidates),
            None => child_candidates,
        });
    }
    Ok(Some(candidates.unwrap_or_default()))
}

fn indexed_or_candidates(
    namespace: &Namespace,
    array: &[Value],
) -> Result<Option<Vec<usize>>, QueryError> {
    let filters = as_array(
        array
            .get(1)
            .ok_or_else(|| QueryError::new("Or requires filters."))?,
        "Or filters",
    )?;
    let mut candidates = Vec::new();
    for child in filters {
        let Some(child_candidates) = indexed_filter_candidates(namespace, child)? else {
            return Ok(None);
        };
        candidates = sorted_union(&candidates, &child_candidates);
    }
    Ok(Some(candidates))
}

fn equality_postings(namespace: &Namespace, attr: &str, key: &ScalarEqKey) -> Vec<usize> {
    let mut indexes = namespace.query_indexes.guard();
    let index = indexes
        .equality
        .entry(attr.to_string())
        .or_insert_with(|| build_equality_index(namespace, attr));
    index
        .postings
        .get(key)
        .map(|postings| postings.iter().copied().collect())
        .unwrap_or_default()
}

fn build_equality_index(namespace: &Namespace, attr: &str) -> EqualityAttributeIndex {
    let mut postings: HashMap<ScalarEqKey, BTreeSet<usize>> = HashMap::new();
    for (index, document) in namespace.documents.iter().enumerate() {
        let value = document_attr(document, attr).unwrap_or(&Value::Null);
        if let Some(key) = scalar_eq_key(value) {
            postings.entry(key).or_default().insert(index);
        }
    }
    EqualityAttributeIndex { postings }
}

fn group_count_index(namespace: &Namespace, attr: &str) -> Option<Vec<GroupCountEntry>> {
    let mut indexes = namespace.query_indexes.guard();
    if !indexes.group_counts.contains_key(attr) {
        let index = build_group_count_index(namespace, attr)?;
        indexes.group_counts.insert(attr.to_string(), index);
    }
    indexes
        .group_counts
        .get(attr)
        .map(|index| index.counts.values().cloned().collect())
}

fn build_group_count_index(namespace: &Namespace, attr: &str) -> Option<GroupCountIndex> {
    let mut index = GroupCountIndex::default();
    for document in &namespace.documents {
        if !increment_group_count(&mut index, document_attr(document, attr)) {
            return None;
        }
    }
    Some(index)
}

fn increment_group_count(index: &mut GroupCountIndex, value: Option<&Value>) -> bool {
    let Some((key, value)) = scalar_group_key_and_value(value) else {
        return false;
    };
    let entry = index
        .counts
        .entry(key)
        .or_insert(GroupCountEntry { value, count: 0 });
    entry.count += 1;
    true
}

fn decrement_group_count(index: &mut GroupCountIndex, value: Option<&Value>) -> bool {
    let Some((key, _)) = scalar_group_key_and_value(value) else {
        return false;
    };
    let remove_key = if let Some(entry) = index.counts.get_mut(&key) {
        entry.count = entry.count.saturating_sub(1);
        entry.count == 0
    } else {
        false
    };
    if remove_key {
        index.counts.remove(&key);
    }
    true
}

fn scalar_group_key_and_value(value: Option<&Value>) -> Option<(ScalarEqKey, Value)> {
    match value {
        Some(value) => scalar_eq_key(value).map(|key| (key, value.clone())),
        None => Some((ScalarEqKey::Null, Value::Null)),
    }
}

fn order_index_positions(
    namespace: &Namespace,
    key: &OrderIndexKey,
    filter_candidates: Option<&[usize]>,
    limit: usize,
) -> Vec<usize> {
    let mut indexes = namespace.query_indexes.guard();
    let order = indexes
        .order
        .entry(key.clone())
        .or_insert_with(|| build_order_index(namespace, key));
    let position_limit = if filter_candidates.is_some() {
        order.len()
    } else {
        limit
    };
    let mut positions = Vec::with_capacity(position_limit.min(order.len()));
    for entry in order {
        if let Some(candidates) = filter_candidates
            && candidates.binary_search(&entry.doc_index).is_err()
        {
            continue;
        }
        positions.push(entry.doc_index);
        if positions.len() == position_limit {
            break;
        }
    }
    positions
}

fn build_order_index(namespace: &Namespace, key: &OrderIndexKey) -> Vec<OrderIndexEntry> {
    let mut order = namespace
        .documents
        .iter()
        .enumerate()
        .map(|(index, document)| order_entry_for_document(index, document, key))
        .collect::<Vec<_>>();
    order.sort_by(|left, right| compare_order_entries(left, right, key));
    order
}

fn score_indexed_sparse(
    namespace: &Namespace,
    attr: &str,
    query: &HashMap<String, f64>,
) -> Vec<(usize, f64)> {
    let mut indexes = namespace.query_indexes.guard();
    if !indexes.sparse_vectors.contains_key(attr) {
        indexes
            .sparse_vectors
            .insert(attr.to_string(), build_sparse_vector_index(namespace, attr));
    }
    let Some(index) = indexes.sparse_vectors.get(attr) else {
        return Vec::new();
    };
    let mut scores: HashMap<usize, f64> = HashMap::new();
    for (dimension, query_value) in query {
        if *query_value == 0.0 {
            continue;
        }
        let Some(postings) = index.postings.get(dimension) else {
            continue;
        };
        for posting in postings {
            let score = scores.entry(posting.doc_index).or_insert(0.0);
            *score += posting.value * query_value;
        }
    }
    scores
        .into_iter()
        .filter(|(_, score)| *score != 0.0)
        .collect()
}

fn build_sparse_vector_index(namespace: &Namespace, attr: &str) -> SparseVectorIndex {
    let mut index = SparseVectorIndex::default();
    for (doc_index, document) in namespace.documents.iter().enumerate() {
        insert_sparse_document_postings(&mut index, doc_index, document_attr(document, attr));
    }
    index
}

fn insert_sparse_document_postings(
    index: &mut SparseVectorIndex,
    doc_index: usize,
    value: Option<&Value>,
) {
    let Some(Value::Object(vector)) = value else {
        return;
    };
    for (dimension, value) in vector {
        let Some(value) = value_as_f64(value) else {
            continue;
        };
        if value == 0.0 {
            continue;
        }
        index
            .postings
            .entry(dimension.clone())
            .or_default()
            .push(SparsePosting { doc_index, value });
    }
}

fn remove_document_from_cached_indexes(
    namespace: &Namespace,
    doc_index: usize,
    document: &Document,
) {
    let mut indexes = namespace.query_indexes.guard();
    if indexes.is_empty() {
        return;
    }
    for (attribute, equality_index) in &mut indexes.equality {
        let value = document_attr(document, attribute).unwrap_or(&Value::Null);
        let Some(key) = scalar_eq_key(value) else {
            continue;
        };
        let remove_key = if let Some(postings) = equality_index.postings.get_mut(&key) {
            postings.remove(&doc_index);
            postings.is_empty()
        } else {
            false
        };
        if remove_key {
            equality_index.postings.remove(&key);
        }
    }
    for order in indexes.order.values_mut() {
        order.retain(|entry| entry.doc_index != doc_index);
    }
    for sparse_index in indexes.sparse_vectors.values_mut() {
        let mut empty_dimensions = Vec::new();
        for (dimension, postings) in &mut sparse_index.postings {
            postings.retain(|posting| posting.doc_index != doc_index);
            if postings.is_empty() {
                empty_dimensions.push(dimension.clone());
            }
        }
        for dimension in empty_dimensions {
            sparse_index.postings.remove(&dimension);
        }
    }
    let mut invalid_group_indexes = Vec::new();
    for (attribute, group_index) in &mut indexes.group_counts {
        if !decrement_group_count(group_index, document_attr(document, attribute)) {
            invalid_group_indexes.push(attribute.clone());
        }
    }
    for attribute in invalid_group_indexes {
        indexes.group_counts.remove(&attribute);
    }
}

fn insert_document_into_cached_indexes(
    namespace: &Namespace,
    doc_index: usize,
    document: &Document,
) {
    let mut indexes = namespace.query_indexes.guard();
    if indexes.is_empty() {
        return;
    }
    for (attribute, equality_index) in &mut indexes.equality {
        let value = document_attr(document, attribute).unwrap_or(&Value::Null);
        if let Some(key) = scalar_eq_key(value) {
            equality_index
                .postings
                .entry(key)
                .or_default()
                .insert(doc_index);
        }
    }
    for (key, order) in &mut indexes.order {
        order.retain(|entry| entry.doc_index != doc_index);
        insert_order_entry(
            order,
            key,
            order_entry_for_document(doc_index, document, key),
        );
    }
    for (attribute, sparse_index) in &mut indexes.sparse_vectors {
        insert_sparse_document_postings(
            sparse_index,
            doc_index,
            document_attr(document, attribute),
        );
    }
    let mut invalid_group_indexes = Vec::new();
    for (attribute, group_index) in &mut indexes.group_counts {
        if !increment_group_count(group_index, document_attr(document, attribute)) {
            invalid_group_indexes.push(attribute.clone());
        }
    }
    for attribute in invalid_group_indexes {
        indexes.group_counts.remove(&attribute);
    }
}

fn order_entry_for_document(
    doc_index: usize,
    document: &Document,
    key: &OrderIndexKey,
) -> OrderIndexEntry {
    OrderIndexEntry {
        doc_index,
        id: document.id.clone(),
        values: key
            .0
            .iter()
            .map(|(attribute, _)| {
                document_attr(document, attribute)
                    .cloned()
                    .unwrap_or(Value::Null)
            })
            .collect(),
    }
}

fn insert_order_entry(
    order: &mut Vec<OrderIndexEntry>,
    key: &OrderIndexKey,
    entry: OrderIndexEntry,
) {
    let position = order
        .binary_search_by(|candidate| compare_order_entries(candidate, &entry, key))
        .unwrap_or_else(|position| position);
    order.insert(position, entry);
}

fn compare_order_entries(
    left: &OrderIndexEntry,
    right: &OrderIndexEntry,
    key: &OrderIndexKey,
) -> Ordering {
    left.values
        .iter()
        .zip(&right.values)
        .zip(&key.0)
        .map(|((left, right), (_, direction))| {
            let base = compare_values_for_order(Some(left), Some(right));
            match direction {
                SortDirection::Asc => base,
                SortDirection::Desc => base.reverse(),
            }
        })
        .find(|ordering| *ordering != Ordering::Equal)
        .unwrap_or(Ordering::Equal)
        .then_with(|| stable_id_compare(&left.id, &right.id))
}

fn order_index_key(kind: &RankKind) -> Option<OrderIndexKey> {
    match kind {
        RankKind::AttributeOrder {
            attribute,
            direction,
        } => Some(OrderIndexKey(vec![(attribute.clone(), *direction)])),
        RankKind::MultiAttributeOrder { attributes } => Some(OrderIndexKey(attributes.clone())),
        RankKind::SmallerIsBetter | RankKind::LargerIsBetter => None,
    }
}

fn scalar_eq_key(value: &Value) -> Option<ScalarEqKey> {
    match value {
        Value::Null => Some(ScalarEqKey::Null),
        Value::Bool(value) => Some(ScalarEqKey::Bool(*value)),
        Value::Number(_) => value_as_f64(value).map(|value| {
            let normalized = if value == 0.0 { 0.0 } else { value };
            ScalarEqKey::Number(normalized.to_bits())
        }),
        Value::String(value) => Some(ScalarEqKey::String(value.clone())),
        Value::Array(_) | Value::Object(_) => None,
    }
}

fn borrowed_scalar_key(value: &Value) -> Option<BorrowedScalarKey<'_>> {
    match value {
        Value::Null => Some(BorrowedScalarKey::Null),
        Value::Bool(value) => Some(BorrowedScalarKey::Bool(*value)),
        Value::Number(_) => value_as_f64(value).map(|value| {
            let normalized = if value == 0.0 { 0.0 } else { value };
            BorrowedScalarKey::Number(normalized.to_bits())
        }),
        Value::String(value) => Some(BorrowedScalarKey::String(value.as_str())),
        Value::Array(_) | Value::Object(_) => None,
    }
}

fn sorted_intersection(left: &[usize], right: &[usize]) -> Vec<usize> {
    let mut output = Vec::new();
    let mut left_index = 0;
    let mut right_index = 0;
    while left_index < left.len() && right_index < right.len() {
        match left[left_index].cmp(&right[right_index]) {
            Ordering::Less => left_index += 1,
            Ordering::Equal => {
                output.push(left[left_index]);
                left_index += 1;
                right_index += 1;
            }
            Ordering::Greater => right_index += 1,
        }
    }
    output
}

fn sorted_union(left: &[usize], right: &[usize]) -> Vec<usize> {
    let mut output = Vec::with_capacity(left.len() + right.len());
    let mut left_index = 0;
    let mut right_index = 0;
    while left_index < left.len() || right_index < right.len() {
        match (left.get(left_index), right.get(right_index)) {
            (Some(left_value), Some(right_value)) => match left_value.cmp(right_value) {
                Ordering::Less => {
                    output.push(*left_value);
                    left_index += 1;
                }
                Ordering::Equal => {
                    output.push(*left_value);
                    left_index += 1;
                    right_index += 1;
                }
                Ordering::Greater => {
                    output.push(*right_value);
                    right_index += 1;
                }
            },
            (Some(left_value), None) => {
                output.push(*left_value);
                left_index += 1;
            }
            (None, Some(right_value)) => {
                output.push(*right_value);
                right_index += 1;
            }
            (None, None) => break,
        }
    }
    output
}

fn score_indexed_bm25(
    namespace: &Namespace,
    field: &str,
    query: &PreparedBm25Query,
) -> Vec<(usize, f64)> {
    let config = fts_config_for_field(namespace, field);
    let mut indexes = namespace.fts_index_cache.fields_guard();
    let rebuild = indexes
        .get(field)
        .map(|index| index.config != config)
        .unwrap_or(true);
    if rebuild {
        indexes.insert(
            field.to_string(),
            FtsFieldIndex::build(namespace, field, config),
        );
    }
    let Some(index) = indexes.get(field) else {
        return Vec::new();
    };
    score_bm25_index(index, query)
}

fn score_bm25_index(index: &FtsFieldIndex, query: &PreparedBm25Query) -> Vec<(usize, f64)> {
    if index.doc_count == 0 || index.avg_len <= 0.0 {
        return Vec::new();
    }
    let mut scores: HashMap<usize, f64> = HashMap::new();
    for term in &query.terms {
        if term.prefix {
            let mut seen_docs = BTreeSet::new();
            for (token, postings) in &index.postings {
                if !token.starts_with(&term.token) {
                    continue;
                }
                for posting in postings {
                    if seen_docs.insert(posting.doc_index) {
                        let score = scores.entry(posting.doc_index).or_insert(0.0);
                        *score += 1.0;
                    }
                }
            }
            continue;
        }
        let Some(postings) = index.postings.get(&term.token) else {
            continue;
        };
        let doc_freq = index.doc_freqs.get(&term.token).copied().unwrap_or(0);
        if doc_freq == 0 {
            continue;
        }
        for posting in postings {
            let Some(doc_len) = index.doc_lengths.get(posting.doc_index).copied() else {
                continue;
            };
            if doc_len == 0 {
                continue;
            }
            let score = scores.entry(posting.doc_index).or_insert(0.0);
            *score += bm25_term_score(
                index,
                doc_freq,
                posting.term_frequency,
                doc_len,
                term.query_frequency,
            );
        }
    }
    scores
        .into_iter()
        .filter(|(_, score)| *score != 0.0)
        .collect()
}

fn bm25_term_score(
    index: &FtsFieldIndex,
    doc_freq: usize,
    term_frequency: usize,
    doc_len: usize,
    query_frequency: usize,
) -> f64 {
    let idf =
        ((index.doc_count as f64 - doc_freq as f64 + 0.5) / (doc_freq as f64 + 0.5) + 1.0).ln();
    let tf = term_frequency as f64;
    let doc_len = doc_len as f64;
    let k1 = index.config.k1();
    let b = index.config.b();
    let k3 = index.config.k3();
    let qtf = query_frequency as f64;
    let query_weight = (qtf * (k3 + 1.0)) / (qtf + k3);
    idf * (tf * (k1 + 1.0)) / (tf + k1 * (1.0 - b + b * doc_len / index.avg_len)) * query_weight
}

fn sort_ranked_documents(ranked: &mut [RankedDocument<'_>], kind: &RankKind) {
    ranked.sort_by(|left, right| ranked_document_order(left, right, kind));
}

fn ranked_document_order(
    left: &RankedDocument<'_>,
    right: &RankedDocument<'_>,
    kind: &RankKind,
) -> Ordering {
    match kind {
        RankKind::SmallerIsBetter => compare_f64(left.score, right.score)
            .then_with(|| stable_id_compare(&left.doc.id, &right.doc.id)),
        RankKind::LargerIsBetter => compare_f64(right.score, left.score)
            .then_with(|| stable_id_compare(&left.doc.id, &right.doc.id)),
        RankKind::AttributeOrder {
            attribute,
            direction,
        } => compare_order_attr(left.doc, right.doc, attribute, *direction)
            .then_with(|| stable_id_compare(&left.doc.id, &right.doc.id)),
        RankKind::MultiAttributeOrder { attributes } => {
            compare_order_attrs(left.doc, right.doc, attributes)
                .then_with(|| stable_id_compare(&left.doc.id, &right.doc.id))
        }
    }
}

fn compare_order_attr(
    left: &Document,
    right: &Document,
    attr: &str,
    direction: SortDirection,
) -> Ordering {
    let base = compare_values_for_order(document_attr(left, attr), document_attr(right, attr));
    match direction {
        SortDirection::Asc => base,
        SortDirection::Desc => base.reverse(),
    }
}

fn compare_order_attrs(
    left: &Document,
    right: &Document,
    attrs: &[(String, SortDirection)],
) -> Ordering {
    attrs
        .iter()
        .map(|(attribute, direction)| compare_order_attr(left, right, attribute, *direction))
        .find(|ordering| *ordering != Ordering::Equal)
        .unwrap_or(Ordering::Equal)
}

fn apply_limit<'a>(
    mut ranked: Vec<RankedDocument<'a>>,
    limit: &Limit,
    kind: &RankKind,
) -> Result<Vec<RankedDocument<'a>>, QueryError> {
    let Some(per) = &limit.per else {
        if ranked.len() > limit.total {
            ranked.select_nth_unstable_by(limit.total, |left, right| {
                ranked_document_order(left, right, kind)
            });
        }
        ranked.truncate(limit.total);
        sort_ranked_documents(&mut ranked, kind);
        return Ok(ranked);
    };
    if !matches!(
        kind,
        RankKind::AttributeOrder { .. } | RankKind::MultiAttributeOrder { .. }
    ) {
        return Err(QueryError::new(
            "💔 `limit.per` is only supported when ranking by an attribute",
        ));
    }
    sort_ranked_documents(&mut ranked, kind);
    let mut seen: BTreeMap<String, usize> = BTreeMap::new();
    let mut result = Vec::new();
    for ranked_doc in ranked {
        let key = per
            .attributes
            .iter()
            .map(|attribute| {
                document_attr(ranked_doc.doc, attribute)
                    .cloned()
                    .unwrap_or(Value::Null)
                    .to_string()
            })
            .collect::<Vec<_>>()
            .join("\u{1f}");
        let count = seen.entry(key).or_insert(0);
        if *count < per.limit {
            *count += 1;
            result.push(ranked_doc);
        }
        if result.len() >= limit.total {
            break;
        }
    }
    Ok(result)
}

fn eval_prepared_rank_expr(
    doc_index: usize,
    document: &Document,
    expression: &PreparedRankExpr<'_>,
    distance_metric: DistanceMetric,
) -> Result<f64, QueryError> {
    match expression {
        PreparedRankExpr::Literal(value) => Ok(*value),
        PreparedRankExpr::Sum(terms) => {
            let mut total = 0.0;
            for term in terms {
                total += eval_prepared_rank_expr(doc_index, document, term, distance_metric)?;
            }
            Ok(total)
        }
        PreparedRankExpr::Max(terms) => {
            let mut best = 0.0;
            for term in terms {
                let score = eval_prepared_rank_expr(doc_index, document, term, distance_metric)?;
                if score > best {
                    best = score;
                }
            }
            Ok(best)
        }
        PreparedRankExpr::Product { weight, expression } => Ok(
            *weight * eval_prepared_rank_expr(doc_index, document, expression, distance_metric)?
        ),
        PreparedRankExpr::Attribute(attribute) => Ok(document_attr(document, attribute)
            .and_then(value_as_f64)
            .unwrap_or(0.0)),
        PreparedRankExpr::Saturate {
            expression,
            midpoint,
            exponent,
        } => {
            let score =
                eval_prepared_rank_expr(doc_index, document, expression, distance_metric)?.max(0.0);
            if score <= 0.0 || *midpoint <= 0.0 || *exponent <= 0.0 {
                return Ok(0.0);
            }
            let powered = score.powf(*exponent);
            Ok(powered / (powered + midpoint.powf(*exponent)))
        }
        PreparedRankExpr::Decay {
            expression,
            midpoint,
            exponent,
        } => {
            let distance =
                eval_prepared_rank_expr(doc_index, document, expression, distance_metric)?.abs();
            if *midpoint <= 0.0 || *exponent <= 0.0 {
                return Ok(0.0);
            }
            let midpoint = midpoint.powf(*exponent);
            Ok(midpoint / (distance.powf(*exponent) + midpoint))
        }
        PreparedRankExpr::Dist { expression, origin } => {
            let value = match expression {
                PreparedDistExpr::Attribute(attribute) => document_attr(document, attribute)
                    .cloned()
                    .unwrap_or(Value::Null),
                PreparedDistExpr::Rank(expression) => number_value(eval_prepared_rank_expr(
                    doc_index,
                    document,
                    expression,
                    distance_metric,
                )?),
            };
            numeric_or_datetime_distance(&value, origin)
        }
        PreparedRankExpr::DenseDistance { attribute, query } => {
            dense_distance_to_query(document, attribute, query, distance_metric)
        }
        PreparedRankExpr::DenseDistanceVector { query } => {
            vector_distance_to_query(document, query, distance_metric)
        }
        PreparedRankExpr::SparseDotProduct { attribute, query } => {
            sparse_dot_product_with_query(document, attribute, query)
        }
        PreparedRankExpr::Bm25 { scores } => Ok(scores.score(doc_index)),
        PreparedRankExpr::Filter(filter) => Ok(if eval_filter(document, filter)? {
            1.0
        } else {
            0.0
        }),
    }
}

fn dense_distance(
    document: &Document,
    attr: &str,
    query: &Value,
    distance_metric: DistanceMetric,
) -> Result<f64, QueryError> {
    let right = numeric_array(query, "query vector")?;
    let prepared_query = PreparedDenseQuery {
        norm: vector_norm(&right),
        values: right,
    };
    if let Some(left) = document.typed.dense_vector(attr) {
        if left.len() != prepared_query.values.len() {
            return Err(QueryError::new(format!(
                "Vector dimension mismatch for '{attr}': document has {}, query has {}.",
                left.len(),
                prepared_query.values.len()
            )));
        }
        return Ok(dense_distance_values(
            left,
            &prepared_query,
            distance_metric,
        ));
    }
    let left = document
        .attributes
        .get(attr)
        .ok_or_else(|| QueryError::new(format!("Vector attribute '{attr}' is missing.")))?;
    let left = numeric_array(left, attr)?;
    if left.len() != prepared_query.values.len() {
        return Err(QueryError::new(format!(
            "Vector dimension mismatch for '{attr}': document has {}, query has {}.",
            left.len(),
            prepared_query.values.len()
        )));
    }
    Ok(dense_distance_values(
        &left,
        &prepared_query,
        distance_metric,
    ))
}

fn dense_distance_to_query(
    document: &Document,
    attr: &str,
    query: &PreparedDenseQuery,
    distance_metric: DistanceMetric,
) -> Result<f64, QueryError> {
    if let Some(values) = document.typed.dense_vector(attr) {
        if values.len() != query.values.len() {
            return Err(QueryError::new(format!(
                "Vector dimension mismatch for '{attr}': document has {}, query has {}.",
                values.len(),
                query.values.len()
            )));
        }
        return Ok(dense_distance_values(values, query, distance_metric));
    }
    let left = document
        .attributes
        .get(attr)
        .ok_or_else(|| QueryError::new(format!("Vector attribute '{attr}' is missing.")))?;
    if let Some(encoded) = left.as_str() {
        let bytes = STANDARD
            .decode(encoded)
            .map_err(|error| QueryError::new(format!("{attr} base64 is invalid: {error}")))?;
        if bytes.len() % std::mem::size_of::<f32>() != 0 {
            return Err(QueryError::new(format!(
                "{attr} base64 length must be a multiple of 4 bytes."
            )));
        }
        let document_len = bytes.len() / std::mem::size_of::<f32>();
        if document_len != query.values.len() {
            return Err(QueryError::new(format!(
                "Vector dimension mismatch for '{attr}': document has {}, query has {}.",
                document_len,
                query.values.len()
            )));
        }
        return Ok(dense_distance_f32_chunks(&bytes, query, distance_metric));
    }
    let values = as_array(left, attr)?;
    if values.len() != query.values.len() {
        return Err(QueryError::new(format!(
            "Vector dimension mismatch for '{attr}': document has {}, query has {}.",
            values.len(),
            query.values.len()
        )));
    }
    let mut dot = 0.0;
    let mut norm = 0.0;
    let mut distance = 0.0;
    for (index, (doc_value, query_value)) in values.iter().zip(query.values.iter()).enumerate() {
        let doc_value = value_as_f64(doc_value)
            .ok_or_else(|| QueryError::new(format!("{attr}[{index}] must be numeric.")))?;
        match distance_metric {
            DistanceMetric::Euclidean | DistanceMetric::EuclideanSquared => {
                let delta = doc_value - *query_value;
                distance += delta * delta;
            }
            DistanceMetric::CosineDistance => {
                dot += doc_value * *query_value;
                norm += doc_value * doc_value;
            }
        }
    }
    match distance_metric {
        DistanceMetric::Euclidean => Ok(distance.sqrt()),
        DistanceMetric::EuclideanSquared => Ok(distance),
        DistanceMetric::CosineDistance => {
            let norm = norm.sqrt();
            if norm == 0.0 || query.norm == 0.0 {
                return Ok(1.0);
            }
            Ok(1.0 - dot / (norm * query.norm))
        }
    }
}

fn vector_distance_to_query(
    document: &Document,
    query: &PreparedDenseQuery,
    distance_metric: DistanceMetric,
) -> Result<f64, QueryError> {
    if let Some(values) = document.typed.vector.as_deref() {
        if values.len() != query.values.len() {
            return Err(QueryError::new(format!(
                "Vector dimension mismatch for 'vector': document has {}, query has {}.",
                values.len(),
                query.values.len()
            )));
        }
        return Ok(dense_distance_values(values, query, distance_metric));
    }
    dense_distance_to_query(document, "vector", query, distance_metric)
}

fn dense_distance_values(
    values: &[f64],
    query: &PreparedDenseQuery,
    distance_metric: DistanceMetric,
) -> f64 {
    let mut dot = 0.0;
    let mut norm = 0.0;
    let mut distance = 0.0;
    for (doc_value, query_value) in values.iter().zip(query.values.iter()) {
        match distance_metric {
            DistanceMetric::Euclidean | DistanceMetric::EuclideanSquared => {
                let delta = doc_value - query_value;
                distance += delta * delta;
            }
            DistanceMetric::CosineDistance => {
                dot += doc_value * query_value;
                norm += doc_value * doc_value;
            }
        }
    }
    match distance_metric {
        DistanceMetric::Euclidean => distance.sqrt(),
        DistanceMetric::EuclideanSquared => distance,
        DistanceMetric::CosineDistance => {
            let norm = norm.sqrt();
            if norm == 0.0 || query.norm == 0.0 {
                return 1.0;
            }
            1.0 - dot / (norm * query.norm)
        }
    }
}

fn dense_distance_f32_chunks(
    bytes: &[u8],
    query: &PreparedDenseQuery,
    distance_metric: DistanceMetric,
) -> f64 {
    let mut dot = 0.0;
    let mut norm = 0.0;
    let mut distance = 0.0;
    for (chunk, query_value) in bytes
        .chunks_exact(std::mem::size_of::<f32>())
        .zip(query.values.iter())
    {
        let doc_value = f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]) as f64;
        match distance_metric {
            DistanceMetric::Euclidean | DistanceMetric::EuclideanSquared => {
                let delta = doc_value - query_value;
                distance += delta * delta;
            }
            DistanceMetric::CosineDistance => {
                dot += doc_value * query_value;
                norm += doc_value * doc_value;
            }
        }
    }
    match distance_metric {
        DistanceMetric::Euclidean => distance.sqrt(),
        DistanceMetric::EuclideanSquared => distance,
        DistanceMetric::CosineDistance => {
            let norm = norm.sqrt();
            if norm == 0.0 || query.norm == 0.0 {
                return 1.0;
            }
            1.0 - dot / (norm * query.norm)
        }
    }
}

fn vector_norm(values: &[f64]) -> f64 {
    values.iter().map(|value| value * value).sum::<f64>().sqrt()
}

fn sparse_dot_product_with_query(
    document: &Document,
    attr: &str,
    query_vector: &HashMap<String, f64>,
) -> Result<f64, QueryError> {
    let Some(value) = document.attributes.get(attr) else {
        return Ok(0.0);
    };
    let object = as_object(value, attr)?;
    let mut score = 0.0;
    for (key, value) in object {
        let doc_value = value_as_f64(value)
            .ok_or_else(|| QueryError::new(format!("{attr} value for '{key}' must be numeric.")))?;
        if let Some(query_value) = query_vector.get(key) {
            score += doc_value * query_value;
        }
    }
    Ok(score)
}

fn eval_filter(document: &Document, filter: &Value) -> Result<bool, QueryError> {
    eval_filter_with_schema(document, filter, None)
}

fn eval_filter_with_schema(
    document: &Document,
    filter: &Value,
    schema: Option<&Map<String, Value>>,
) -> Result<bool, QueryError> {
    let array = as_array(filter, "filter")?;
    if array.is_empty() {
        return Err(QueryError::new("filter cannot be empty."));
    }
    if let Some(op) = array[0].as_str() {
        match op {
            "And" => {
                let filters = as_array(
                    array
                        .get(1)
                        .ok_or_else(|| QueryError::new("And requires filters."))?,
                    "And filters",
                )?;
                for child in filters {
                    if !eval_filter_with_schema(document, child, schema)? {
                        return Ok(false);
                    }
                }
                return Ok(true);
            }
            "Or" => {
                let filters = as_array(
                    array
                        .get(1)
                        .ok_or_else(|| QueryError::new("Or requires filters."))?,
                    "Or filters",
                )?;
                for child in filters {
                    if eval_filter_with_schema(document, child, schema)? {
                        return Ok(true);
                    }
                }
                return Ok(false);
            }
            "Not" => {
                let child = array
                    .get(1)
                    .ok_or_else(|| QueryError::new("Not requires a filter."))?;
                return Ok(!eval_filter_with_schema(document, child, schema)?);
            }
            _ => {}
        }
    }
    if array.len() < 3 {
        return Err(QueryError::new(
            "attribute filter requires [attribute, operator, value].",
        ));
    }
    let attr = as_string(&array[0], "filter attribute")?;
    let op = as_string(&array[1], "filter operator")?;
    let expected = &array[2];
    let options = array.get(3);
    let actual = document_attr(document, attr).unwrap_or(&Value::Null);
    match op {
        "Eq" => Ok(values_equal(actual, expected)),
        "NotEq" if expected.is_null() => Ok(document_attr(document, attr).is_some()),
        "NotEq" => Ok(!values_equal(actual, expected)),
        "In" => Ok(as_array(expected, "In values")?
            .iter()
            .any(|value| values_equal(actual, value))),
        "NotIn" => Ok(!as_array(expected, "NotIn values")?
            .iter()
            .any(|value| values_equal(actual, value))),
        "Contains" => Ok(value_contains(actual, expected)),
        "NotContains" => Ok(!value_contains(actual, expected)),
        "ContainsAny" => Ok(as_array(expected, "ContainsAny values")?
            .iter()
            .any(|value| value_contains(actual, value))),
        "NotContainsAny" => Ok(!as_array(expected, "NotContainsAny values")?
            .iter()
            .any(|value| value_contains(actual, value))),
        "Lt" => Ok(compare_filter_value(
            actual,
            expected,
            Ordering::Less,
            false,
        )),
        "Lte" => Ok(compare_filter_value(actual, expected, Ordering::Less, true)),
        "Gt" => Ok(compare_filter_value(
            actual,
            expected,
            Ordering::Greater,
            false,
        )),
        "Gte" => Ok(compare_filter_value(
            actual,
            expected,
            Ordering::Greater,
            true,
        )),
        "AnyLt" => Ok(any_compare(actual, expected, Ordering::Less, false)),
        "AnyLte" => Ok(any_compare(actual, expected, Ordering::Less, true)),
        "AnyGt" => Ok(any_compare(actual, expected, Ordering::Greater, false)),
        "AnyGte" => Ok(any_compare(actual, expected, Ordering::Greater, true)),
        "Glob" => glob_match(actual, expected, false),
        "NotGlob" => Ok(!glob_match(actual, expected, false)?),
        "IGlob" => glob_match(actual, expected, true),
        "NotIGlob" => Ok(!glob_match(actual, expected, true)?),
        "Regex" => regex_match(actual, expected),
        "NotRegex" => Ok(!regex_match(actual, expected)?),
        "ContainsAllTokens" => token_filter(
            actual,
            expected,
            options,
            TokenMode::All,
            fts_config_from_schema(schema, attr),
        ),
        "ContainsAnyToken" => token_filter(
            actual,
            expected,
            options,
            TokenMode::Any,
            fts_config_from_schema(schema, attr),
        ),
        "ContainsTokenSequence" => token_filter(
            actual,
            expected,
            options,
            TokenMode::Sequence,
            fts_config_from_schema(schema, attr),
        ),
        "Fuzzy" => fuzzy_match(actual, expected, options),
        "NotFuzzy" => Ok(!fuzzy_match(actual, expected, options)?),
        _ => Err(QueryError::new(format!(
            "Unsupported filter operator '{op}'."
        ))),
    }
}

fn eval_filter_with_new(
    document: &Document,
    filter: &Value,
    new_row: Option<&WriteRow>,
) -> Result<bool, QueryError> {
    let resolved = resolve_ref_new(filter, new_row);
    eval_filter(document, &resolved)
}

fn resolve_ref_new(value: &Value, new_row: Option<&WriteRow>) -> Value {
    match value {
        Value::Object(object) if object.len() == 1 && object.contains_key("$ref_new") => object
            .get("$ref_new")
            .and_then(Value::as_str)
            .and_then(|attribute| {
                new_row.and_then(|row| {
                    if attribute == "id" {
                        Some(row.id.clone())
                    } else {
                        row.attributes.get(attribute).cloned()
                    }
                })
            })
            .unwrap_or(Value::Null),
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|item| resolve_ref_new(item, new_row))
                .collect(),
        ),
        Value::Object(object) => Value::Object(
            object
                .iter()
                .map(|(key, value)| (key.clone(), resolve_ref_new(value, new_row)))
                .collect(),
        ),
        _ => value.clone(),
    }
}

#[derive(Debug, Clone, Copy)]
enum TokenMode {
    All,
    Any,
    Sequence,
}

fn values_equal(left: &Value, right: &Value) -> bool {
    match (left, right) {
        (Value::Number(_), Value::Number(_)) => value_as_f64(left) == value_as_f64(right),
        _ => left == right,
    }
}

fn document_attr<'a>(document: &'a Document, attribute: &str) -> Option<&'a Value> {
    if attribute == "id" {
        Some(&document.id)
    } else {
        document.attributes.get(attribute)
    }
}

fn value_contains(actual: &Value, expected: &Value) -> bool {
    match actual {
        Value::Array(items) => items.iter().any(|item| values_equal(item, expected)),
        Value::String(text) => expected
            .as_str()
            .map(|needle| text.contains(needle))
            .unwrap_or(false),
        _ => false,
    }
}

fn compare_filter_value(
    actual: &Value,
    expected: &Value,
    desired: Ordering,
    allow_equal: bool,
) -> bool {
    if actual.is_null() {
        return match desired {
            Ordering::Less => !expected.is_null() || allow_equal,
            Ordering::Equal => allow_equal && expected.is_null(),
            Ordering::Greater => false,
        };
    }
    let Some(ordering) = compare_scalar(actual, expected) else {
        return false;
    };
    ordering == desired || (allow_equal && ordering == Ordering::Equal)
}

fn any_compare(actual: &Value, expected: &Value, desired: Ordering, allow_equal: bool) -> bool {
    let Some(items) = actual.as_array() else {
        return false;
    };
    items
        .iter()
        .any(|item| compare_filter_value(item, expected, desired, allow_equal))
}

fn glob_match(actual: &Value, pattern: &Value, case_insensitive: bool) -> Result<bool, QueryError> {
    let pattern = as_string(pattern, "glob pattern")?;
    let matcher = GlobBuilder::new(pattern)
        .case_insensitive(case_insensitive)
        .build()
        .map_err(|error| QueryError::new(format!("Invalid glob pattern: {error}")))?
        .compile_matcher();
    Ok(any_string_value(actual, |text| matcher.is_match(text)))
}

fn regex_match(actual: &Value, pattern: &Value) -> Result<bool, QueryError> {
    let pattern = as_string(pattern, "regex pattern")?;
    let regex = Regex::new(pattern)
        .map_err(|error| QueryError::new(format!("Invalid regex pattern: {error}")))?;
    Ok(any_string_value(actual, |text| regex.is_match(text)))
}

fn token_filter(
    actual: &Value,
    expected: &Value,
    options: Option<&Value>,
    mode: TokenMode,
    config: FtsConfig,
) -> Result<bool, QueryError> {
    let query_tokens = query_tokens_with_config(expected, "token query", &config)?;
    if query_tokens.is_empty() {
        return Ok(false);
    }
    let last_as_prefix = options
        .and_then(Value::as_object)
        .and_then(|object| object.get("last_as_prefix"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    Ok(any_string_value(actual, |text| {
        let tokens = tokenize(text, &config);
        match mode {
            TokenMode::All => query_tokens.iter().enumerate().all(|(index, query_token)| {
                token_present(
                    &tokens,
                    query_token,
                    last_as_prefix && index + 1 == query_tokens.len(),
                )
            }),
            TokenMode::Any => query_tokens.iter().enumerate().any(|(index, query_token)| {
                token_present(
                    &tokens,
                    query_token,
                    last_as_prefix && index + 1 == query_tokens.len(),
                )
            }),
            TokenMode::Sequence => contains_token_sequence(&tokens, &query_tokens, last_as_prefix),
        }
    }))
}

fn fuzzy_match(
    actual: &Value,
    expected: &Value,
    options: Option<&Value>,
) -> Result<bool, QueryError> {
    let query = as_string(expected, "fuzzy query")?.to_lowercase();
    if query.is_empty() {
        return Ok(false);
    }
    let thresholds = options
        .and_then(Value::as_object)
        .and_then(|object| {
            object
                .get("max_edit_distance")
                .or_else(|| object.get("max_edits"))
        })
        .and_then(Value::as_array);
    let Some(max_edits) = fuzzy_max_edits(&query, thresholds) else {
        return Ok(false);
    };
    Ok(any_string_value(actual, |text| {
        let lower = text.to_lowercase();
        if lower.contains(&query) {
            return true;
        }
        let query_len = query.chars().count();
        lower.split_whitespace().any(|token| {
            let token_len = token.chars().count();
            let distance = if token_len.abs_diff(query_len) > max_edits {
                max_edits + 1
            } else {
                strsim::levenshtein(token, &query)
            };
            distance <= max_edits
        })
    }))
}

fn fuzzy_max_edits(query: &str, thresholds: Option<&Vec<Value>>) -> Option<usize> {
    let Some(thresholds) = thresholds else {
        return Some(2);
    };
    let length = query.chars().count();
    let mut best = None;
    for threshold in thresholds {
        let (min_chars, edits) = if let Some(object) = threshold.as_object() {
            (
                object
                    .get("min_query_chars")
                    .and_then(Value::as_u64)
                    .and_then(|value| usize::try_from(value).ok()),
                object
                    .get("distance")
                    .and_then(Value::as_u64)
                    .and_then(|value| usize::try_from(value).ok()),
            )
        } else if let Some(array) = threshold.as_array() {
            if array.len() != 2 {
                continue;
            }
            (
                array[0]
                    .as_u64()
                    .and_then(|value| usize::try_from(value).ok()),
                array[1]
                    .as_u64()
                    .and_then(|value| usize::try_from(value).ok()),
            )
        } else {
            continue;
        };
        if let (Some(min_chars), Some(edits)) = (min_chars, edits)
            && length >= min_chars
        {
            best = Some(best.unwrap_or(0).max(edits));
        }
    }
    best
}

fn token_present(tokens: &[String], query_token: &str, prefix: bool) -> bool {
    if prefix {
        tokens.iter().any(|token| token.starts_with(query_token))
    } else {
        tokens.iter().any(|token| token == query_token)
    }
}

fn contains_token_sequence(
    tokens: &[String],
    query_tokens: &[String],
    last_as_prefix: bool,
) -> bool {
    if query_tokens.len() > tokens.len() {
        return false;
    }
    tokens.windows(query_tokens.len()).any(|window| {
        window
            .iter()
            .zip(query_tokens.iter())
            .enumerate()
            .all(|(index, (token, query_token))| {
                if last_as_prefix && index + 1 == query_tokens.len() {
                    token.starts_with(query_token)
                } else {
                    token == query_token
                }
            })
    })
}

fn any_string_value(actual: &Value, matches: impl Fn(&str) -> bool) -> bool {
    match actual {
        Value::String(text) => matches(text),
        Value::Array(items) => items.iter().filter_map(Value::as_str).any(matches),
        _ => false,
    }
}

fn project_document(
    document: &Document,
    request: &Map<String, Value>,
    vector_encoding: VectorEncoding,
    kind: RankKind,
    score: f64,
) -> Result<Value, QueryError> {
    let include = request.get("include_attributes");
    let exclude = request.get("exclude_attributes");
    let mut row = Map::new();
    row.insert("id".to_string(), document.id.clone());
    if !matches!(
        kind,
        RankKind::AttributeOrder { .. } | RankKind::MultiAttributeOrder { .. }
    ) {
        row.insert("$dist".to_string(), number_value(score));
    }
    match (include, exclude) {
        (None, None) => {}
        (Some(Value::Bool(false)), None) => {}
        (Some(Value::Bool(true)), None) => {
            if let Some(vector) = document_vector_value(document) {
                row.insert(
                    "vector".to_string(),
                    maybe_encode_vector(&vector, vector_encoding)?,
                );
            }
            for (key, value) in &document.attributes {
                row.insert(key.clone(), maybe_encode_vector(value, vector_encoding)?);
            }
        }
        (Some(Value::Array(attributes)), None) => {
            for attribute in attributes {
                let name = as_string(attribute, "include_attributes item")?;
                if name == "vector"
                    && let Some(vector) = document_vector_value(document)
                {
                    row.insert(
                        name.to_string(),
                        maybe_encode_vector(&vector, vector_encoding)?,
                    );
                } else if let Some(value) = document.attributes.get(name) {
                    row.insert(
                        name.to_string(),
                        maybe_encode_vector(value, vector_encoding)?,
                    );
                }
            }
        }
        (None, Some(Value::Array(attributes))) => {
            let excluded = attributes
                .iter()
                .map(|attribute| {
                    as_string(attribute, "exclude_attributes item").map(ToString::to_string)
                })
                .collect::<Result<BTreeSet<_>, _>>()?;
            if !excluded.contains("vector")
                && let Some(vector) = document_vector_value(document)
            {
                row.insert(
                    "vector".to_string(),
                    maybe_encode_vector(&vector, vector_encoding)?,
                );
            }
            for (key, value) in &document.attributes {
                if !excluded.contains(key) {
                    row.insert(key.clone(), maybe_encode_vector(value, vector_encoding)?);
                }
            }
        }
        _ => {
            return Err(QueryError::new(
                "include_attributes must be true or an array, and exclude_attributes must be an array.",
            ));
        }
    }
    Ok(Value::Object(row))
}

fn maybe_encode_vector(
    value: &Value,
    vector_encoding: VectorEncoding,
) -> Result<Value, QueryError> {
    if vector_encoding == VectorEncoding::Float || !is_numeric_array(value) {
        return Ok(value.clone());
    }
    let values = numeric_array(value, "vector attribute")?;
    let mut bytes = Vec::with_capacity(values.len() * std::mem::size_of::<f32>());
    for value in values {
        bytes.extend_from_slice(&(value as f32).to_le_bytes());
    }
    Ok(Value::String(STANDARD.encode(bytes)))
}

fn query_aggregations(
    namespace: &Namespace,
    request: &Map<String, Value>,
    aggregate_by: &Value,
) -> Result<Value, QueryError> {
    let aggregations = as_object(aggregate_by, "aggregate_by")?;
    validate_single_aggregate(aggregations)?;
    validate_aggregate_request_shape(request)?;
    let filters = request.get("filters");
    let count_labels = count_aggregate_labels(aggregations)?;
    if let Some(labels) = &count_labels {
        let indexed_candidates = filters
            .map(|filter| indexed_filter_candidates(namespace, filter))
            .transpose()?
            .flatten();
        let candidates_are_available = filters.is_none() || indexed_candidates.is_some();
        if candidates_are_available {
            if let Some(group_by) = request.get("group_by") {
                if let Some(response) = query_grouped_count_aggregations(
                    namespace,
                    request,
                    group_by,
                    labels,
                    indexed_candidates.as_deref(),
                )? {
                    return Ok(response);
                }
            } else {
                let count = indexed_candidates
                    .as_ref()
                    .map(Vec::len)
                    .unwrap_or(namespace.documents.len());
                let mut output = Map::new();
                for label in labels {
                    output.insert(label.clone(), Value::Number(Number::from(count)));
                }
                return Ok(with_metrics(json!({ "aggregations": output }), namespace));
            }
        }
    }
    let matching = namespace
        .documents
        .iter()
        .filter(|document| {
            filters
                .map(|filter| {
                    eval_filter_with_schema(document, filter, Some(&namespace.schema))
                        .unwrap_or(false)
                })
                .unwrap_or(true)
        })
        .collect::<Vec<_>>();
    if let Some(group_by) = request.get("group_by") {
        return query_grouped_aggregations(namespace, request, group_by, aggregations, &matching);
    }
    let mut output = Map::new();
    for (label, aggregate) in aggregations {
        output.insert(label.clone(), evaluate_aggregate(aggregate, &matching)?);
    }
    Ok(with_metrics(json!({ "aggregations": output }), namespace))
}

fn validate_single_aggregate(aggregations: &Map<String, Value>) -> Result<(), QueryError> {
    if aggregations.len() == 1 {
        return Ok(());
    }
    Err(QueryError::new(
        "💔 aggregate_by currently requires exactly one function",
    ))
}

fn validate_aggregate_request_shape(request: &Map<String, Value>) -> Result<(), QueryError> {
    if request.contains_key("limit") {
        return Err(QueryError::new(
            "unknown field `limit`, expected one of `aggregate_by`, `filters`, `group_by`, `top_k`",
        ));
    }
    if !request.contains_key("group_by") && request.contains_key("top_k") {
        return Err(QueryError::new(
            "💔 top_k is not supported in aggregation queries without group_by",
        ));
    }
    Ok(())
}

fn count_aggregate_labels(
    aggregations: &Map<String, Value>,
) -> Result<Option<Vec<String>>, QueryError> {
    let mut labels = Vec::with_capacity(aggregations.len());
    for (label, aggregate) in aggregations {
        let array = as_array(aggregate, "aggregate function")?;
        let op = array
            .first()
            .and_then(Value::as_str)
            .ok_or_else(|| QueryError::new("aggregate function requires an operator."))?;
        if op != "Count" {
            return Ok(None);
        }
        validate_count_aggregate(array)?;
        labels.push(label.clone());
    }
    Ok(Some(labels))
}

fn query_grouped_count_aggregations(
    namespace: &Namespace,
    request: &Map<String, Value>,
    group_by: &Value,
    labels: &[String],
    candidate_indexes: Option<&[usize]>,
) -> Result<Option<Value>, QueryError> {
    let group_exprs = as_array(group_by, "group_by")?;
    let [Value::String(attribute)] = group_exprs.as_slice() else {
        return Ok(None);
    };
    let limit = parse_aggregate_group_limit(request)?;
    if candidate_indexes.is_none()
        && let Some(entries) = group_count_index(namespace, attribute)
    {
        return Ok(Some(grouped_count_response(
            namespace, attribute, labels, entries, limit,
        )));
    }
    let mut groups: HashMap<BorrowedScalarKey<'_>, (Value, usize)> = HashMap::new();
    if let Some(indexes) = candidate_indexes {
        for index in indexes {
            let document = namespace
                .documents
                .get(*index)
                .ok_or_else(|| QueryError::new("filter index referenced a missing document."))?;
            if !count_document_group(document, attribute, &mut groups) {
                return Ok(None);
            }
        }
    } else {
        for document in &namespace.documents {
            if !count_document_group(document, attribute, &mut groups) {
                return Ok(None);
            }
        }
    }
    let entries = groups
        .into_values()
        .map(|(value, count)| GroupCountEntry { value, count })
        .collect::<Vec<_>>();
    Ok(Some(grouped_count_response(
        namespace, attribute, labels, entries, limit,
    )))
}

fn grouped_count_response(
    namespace: &Namespace,
    attribute: &str,
    labels: &[String],
    entries: Vec<GroupCountEntry>,
    limit: usize,
) -> Value {
    let mut keyed_rows = Vec::with_capacity(entries.len());
    for entry in entries {
        let mut group_key = Map::new();
        group_key.insert(attribute.to_string(), entry.value);
        let sort_key = Value::Object(group_key.clone()).to_string();
        for label in labels {
            group_key.insert(label.clone(), Value::Number(Number::from(entry.count)));
        }
        keyed_rows.push((sort_key, Value::Object(group_key)));
    }
    keyed_rows.sort_by(|left, right| left.0.cmp(&right.0));
    let rows = keyed_rows
        .into_iter()
        .take(limit)
        .map(|(_, row)| row)
        .collect::<Vec<_>>();
    with_metrics(json!({ "aggregation_groups": rows }), namespace)
}

fn count_document_group<'a>(
    document: &'a Document,
    attribute: &str,
    groups: &mut HashMap<BorrowedScalarKey<'a>, (Value, usize)>,
) -> bool {
    let (key, value) = match document_attr(document, attribute) {
        Some(value) => {
            let Some(key) = borrowed_scalar_key(value) else {
                return false;
            };
            (key, value.clone())
        }
        None => (BorrowedScalarKey::Null, Value::Null),
    };
    let entry = groups.entry(key).or_insert_with(|| (value, 0));
    entry.1 += 1;
    true
}

fn query_grouped_aggregations(
    namespace: &Namespace,
    request: &Map<String, Value>,
    group_by: &Value,
    aggregations: &Map<String, Value>,
    documents: &[&Document],
) -> Result<Value, QueryError> {
    let group_exprs = as_array(group_by, "group_by")?;
    let limit = parse_aggregate_group_limit(request)?;
    let mut groups: BTreeMap<String, (Map<String, Value>, Vec<&Document>)> = BTreeMap::new();
    for document in documents {
        for group_key in group_keys(document, group_exprs)? {
            let serialized = Value::Object(group_key.clone()).to_string();
            groups
                .entry(serialized)
                .or_insert_with(|| (group_key, Vec::new()))
                .1
                .push(*document);
        }
    }
    let mut rows = Vec::new();
    for (_, (mut group_key, docs)) in groups.into_iter().take(limit) {
        for (label, aggregate) in aggregations {
            group_key.insert(label.clone(), evaluate_aggregate(aggregate, &docs)?);
        }
        rows.push(Value::Object(group_key));
    }
    Ok(with_metrics(
        json!({ "aggregation_groups": rows }),
        namespace,
    ))
}

fn parse_aggregate_group_limit(request: &Map<String, Value>) -> Result<usize, QueryError> {
    request
        .get("top_k")
        .map(|top_k| parse_positive_usize(top_k, "top_k"))
        .transpose()
        .map(|limit| limit.unwrap_or(10))
}

fn group_keys(
    document: &Document,
    group_exprs: &[Value],
) -> Result<Vec<Map<String, Value>>, QueryError> {
    let mut keys = vec![Map::new()];
    for expr in group_exprs {
        let next_values = group_expr_values(document, expr)?;
        let mut next_keys = Vec::new();
        for key in &keys {
            for (name, value) in &next_values {
                let mut next_key = key.clone();
                next_key.insert(name.clone(), value.clone());
                next_keys.push(next_key);
            }
        }
        keys = next_keys;
    }
    Ok(keys)
}

fn group_expr_values(
    document: &Document,
    expr: &Value,
) -> Result<Vec<(String, Value)>, QueryError> {
    if let Some(attribute) = expr.as_str() {
        return Ok(vec![(
            attribute.to_string(),
            document_attr(document, attribute)
                .cloned()
                .unwrap_or(Value::Null),
        )]);
    }
    let object = as_object(expr, "group_by expression")?;
    if object.len() != 1 {
        return Err(QueryError::new(
            "group_by expression object must have one output label.",
        ));
    }
    let Some((label, expression)) = object.iter().next() else {
        return Err(QueryError::new("group_by expression cannot be empty."));
    };
    let expression = as_array(expression, "group_by expression value")?;
    if expression.first().and_then(Value::as_str) != Some("ForEachUnique") || expression.len() != 2
    {
        return Err(QueryError::new(
            "only ForEachUnique group_by expressions are supported.",
        ));
    }
    let attribute = as_string(&expression[1], "ForEachUnique attribute")?;
    let Some(Value::Array(values)) = document_attr(document, attribute) else {
        return Ok(vec![(label.clone(), Value::Null)]);
    };
    let mut unique = BTreeMap::new();
    for value in values {
        unique
            .entry(value.to_string())
            .or_insert_with(|| value.clone());
    }
    Ok(unique
        .into_values()
        .map(|value| (label.clone(), value))
        .collect())
}

fn evaluate_aggregate(aggregate: &Value, documents: &[&Document]) -> Result<Value, QueryError> {
    let array = as_array(aggregate, "aggregate function")?;
    let op = array
        .first()
        .and_then(Value::as_str)
        .ok_or_else(|| QueryError::new("aggregate function requires an operator."))?;
    match op {
        "Count" => {
            validate_count_aggregate(array)?;
            Ok(Value::Number(Number::from(documents.len())))
        }
        "Sum" => {
            let attribute = array
                .get(1)
                .and_then(Value::as_str)
                .ok_or_else(|| QueryError::new("Sum aggregate requires an attribute name."))?;
            let total: f64 = documents
                .iter()
                .filter_map(|document| document_attr(document, attribute))
                .filter_map(value_as_f64)
                .sum();
            Ok(number_value(total))
        }
        _ => Err(QueryError::new(format!(
            "Unsupported aggregate function '{op}'."
        ))),
    }
}

fn validate_count_aggregate(array: &[Value]) -> Result<(), QueryError> {
    match array {
        [_] => Ok(()),
        [_, Value::String(attribute)] if attribute == "id" => Ok(()),
        [_, Value::String(_)] => Err(QueryError::new(
            "💔 aggregate_by with attributes other than \"id\" not yet supported",
        )),
        _ => Err(QueryError::new(
            "💔 aggregate_by currently requires exactly one function",
        )),
    }
}

fn compare_f64(left: f64, right: f64) -> Ordering {
    left.partial_cmp(&right).unwrap_or(Ordering::Equal)
}

fn stable_id_compare(left: &Value, right: &Value) -> Ordering {
    left.to_string().cmp(&right.to_string())
}

fn compare_values_for_order(left: Option<&Value>, right: Option<&Value>) -> Ordering {
    match (left, right) {
        (None | Some(Value::Null), None | Some(Value::Null)) => Ordering::Equal,
        (None | Some(Value::Null), _) => Ordering::Less,
        (_, None | Some(Value::Null)) => Ordering::Greater,
        (Some(left), Some(right)) => {
            compare_scalar(left, right).unwrap_or_else(|| left.to_string().cmp(&right.to_string()))
        }
    }
}

fn compare_scalar(left: &Value, right: &Value) -> Option<Ordering> {
    if let (Some(left), Some(right)) = (value_as_f64(left), value_as_f64(right)) {
        return left.partial_cmp(&right);
    }
    if let (Some(left), Some(right)) = (value_as_datetime(left), value_as_datetime(right)) {
        return Some(left.cmp(&right));
    }
    if let (Some(left), Some(right)) = (left.as_str(), right.as_str()) {
        return Some(left.cmp(right));
    }
    if let (Some(left), Some(right)) = (left.as_bool(), right.as_bool()) {
        return Some(left.cmp(&right));
    }
    None
}

fn value_as_datetime(value: &Value) -> Option<DateTime<Utc>> {
    value.as_str().and_then(parse_datetime_text)
}

fn parse_datetime_text(text: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(text)
        .map(|datetime| datetime.with_timezone(&Utc))
        .ok()
        .or_else(|| {
            NaiveDate::parse_from_str(text, "%Y-%m-%d")
                .ok()
                .and_then(|date| date.and_hms_opt(0, 0, 0))
                .map(|datetime| DateTime::from_naive_utc_and_offset(datetime, Utc))
        })
}

fn format_datetime(datetime: DateTime<Utc>) -> String {
    datetime.format("%Y-%m-%dT%H:%M:%S%.9fZ").to_string()
}

fn parse_midpoint(value: &Value) -> Result<f64, QueryError> {
    if let Some(number) = value_as_f64(value) {
        return Ok(number);
    }
    let text = value
        .as_str()
        .ok_or_else(|| QueryError::new("Decay midpoint must be numeric or a duration string."))?;
    let split_at = text
        .find(|character: char| !character.is_ascii_digit() && character != '.')
        .ok_or_else(|| QueryError::new("Decay duration midpoint requires a unit."))?;
    let amount = text[..split_at]
        .parse::<f64>()
        .map_err(|_| QueryError::new("Decay duration midpoint has an invalid number."))?;
    let unit = &text[split_at..];
    let multiplier = match unit {
        "ms" => 1.0,
        "s" => 1_000.0,
        "m" => 60_000.0,
        "h" => 3_600_000.0,
        "d" => 86_400_000.0,
        "w" => 604_800_000.0,
        _ => {
            return Err(QueryError::new(
                "Decay duration midpoint unit must be one of ms, s, m, h, d, w.",
            ));
        }
    };
    Ok(amount * multiplier)
}

fn numeric_or_datetime_distance(left: &Value, right: &Value) -> Result<f64, QueryError> {
    if let (Some(left), Some(right)) = (value_as_f64(left), value_as_f64(right)) {
        return Ok((left - right).abs());
    }
    if let (Some(left), Some(right)) = (value_as_datetime(left), value_as_datetime(right)) {
        return Ok((left - right).num_milliseconds().unsigned_abs() as f64);
    }
    Err(QueryError::new(
        "Dist values must both be numeric or RFC3339 datetimes.",
    ))
}

fn string_attr_tokens_with_config(
    document: &Document,
    field: &str,
    config: &FtsConfig,
) -> Vec<String> {
    document
        .attributes
        .get(field)
        .map(|value| value_tokens(value, config))
        .unwrap_or_default()
}

fn query_tokens_with_config(
    value: &Value,
    label: &str,
    config: &FtsConfig,
) -> Result<Vec<String>, QueryError> {
    if let Some(text) = value.as_str() {
        if config.tokenizer == Tokenizer::PreTokenizedArray {
            return Err(QueryError::new(format!(
                "{label} must be an array of strings for pre_tokenized_array."
            )));
        }
        return Ok(tokenize(text, config));
    }
    if let Some(tokens) = value.as_array() {
        return Ok(normalize_tokens(
            tokens
                .iter()
                .map(|token| as_string(token, label).map(ToString::to_string))
                .collect::<Result<Vec<_>, _>>()?,
            config,
        ));
    }
    Err(QueryError::new(format!(
        "{label} must be a string or array of strings."
    )))
}

fn value_tokens(value: &Value, config: &FtsConfig) -> Vec<String> {
    match (config.tokenizer, value) {
        (Tokenizer::PreTokenizedArray, Value::Array(tokens)) => normalize_tokens(
            tokens
                .iter()
                .filter_map(Value::as_str)
                .map(ToString::to_string)
                .collect(),
            config,
        ),
        (_, Value::String(text)) => tokenize(text, config),
        (_, Value::Array(texts)) => texts
            .iter()
            .filter_map(Value::as_str)
            .flat_map(|text| tokenize(text, config))
            .collect(),
        _ => Vec::new(),
    }
}

fn tokenize(text: &str, config: &FtsConfig) -> Vec<String> {
    let text = if config.ascii_folding {
        deunicode(text)
    } else {
        text.to_string()
    };
    let raw_tokens = match config.tokenizer {
        Tokenizer::PreTokenizedArray => Vec::new(),
        Tokenizer::WordV0 => tokenize_word_v2(&text, true, false),
        Tokenizer::WordV1 => tokenize_word_v2(&text, true, true),
        Tokenizer::WordV2 => tokenize_word_v2(&text, false, true),
        Tokenizer::WordV3 => text
            .unicode_words()
            .filter(|part| !part.is_empty())
            .map(ToString::to_string)
            .collect(),
    };
    normalize_tokens(raw_tokens, config)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum LegacyTokenKind {
    Word,
    Emoji,
}

fn tokenize_word_v2(text: &str, merge_ideographs: bool, include_emoji: bool) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut current_kind: Option<LegacyTokenKind> = None;

    for character in text.chars() {
        if is_ideographic(character) && merge_ideographs {
            flush_token(
                &mut tokens,
                &mut current,
                &mut current_kind,
                LegacyTokenKind::Word,
            );
            current.push(character);
        } else if is_ideographic(character) {
            flush_token(
                &mut tokens,
                &mut current,
                &mut current_kind,
                LegacyTokenKind::Word,
            );
            tokens.push(character.to_string());
        } else if character.is_alphanumeric() {
            flush_token(
                &mut tokens,
                &mut current,
                &mut current_kind,
                LegacyTokenKind::Word,
            );
            current.push(character);
        } else if include_emoji && is_emoji_token_character(character) {
            flush_token(
                &mut tokens,
                &mut current,
                &mut current_kind,
                LegacyTokenKind::Emoji,
            );
            current.push(character);
        } else {
            if is_emoji_joiner(character) && current_kind == Some(LegacyTokenKind::Emoji) {
                current.push(character);
                continue;
            }
            if !current.is_empty() {
                tokens.push(std::mem::take(&mut current));
            }
            current_kind = None;
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

fn flush_token(
    tokens: &mut Vec<String>,
    current: &mut String,
    current_kind: &mut Option<LegacyTokenKind>,
    next_kind: LegacyTokenKind,
) {
    if current_kind.is_some_and(|kind| kind != next_kind) && !current.is_empty() {
        tokens.push(std::mem::take(current));
    }
    *current_kind = Some(next_kind);
}

fn is_ideographic(character: char) -> bool {
    matches!(
        character as u32,
        0x3400..=0x4DBF
            | 0x4E00..=0x9FFF
            | 0xF900..=0xFAFF
            | 0x20000..=0x2A6DF
            | 0x2A700..=0x2B73F
            | 0x2B740..=0x2B81F
            | 0x2B820..=0x2CEAF
            | 0x2CEB0..=0x2EBEF
            | 0x30000..=0x3134F
            | 0x31350..=0x323AF
    )
}

fn is_emoji_token_character(character: char) -> bool {
    matches!(
        character as u32,
        0x1F000..=0x1FAFF | 0x2600..=0x27BF | 0x2300..=0x23FF
    )
}

fn is_emoji_joiner(character: char) -> bool {
    matches!(
        character as u32,
        0x200D | 0xFE0E | 0xFE0F | 0x1F3FB..=0x1F3FF
    )
}

fn normalize_tokens(tokens: Vec<String>, config: &FtsConfig) -> Vec<String> {
    let stemmer = config
        .stemming
        .then(|| Stemmer::create(stemmer_algorithm(config.language)));
    tokens
        .into_iter()
        .map(|token| {
            if config.case_sensitive {
                token
            } else {
                token.to_lowercase()
            }
        })
        .filter(|token| token.len() <= config.max_token_length)
        .filter(|token| !config.remove_stopwords || !is_stopword(config.language, token))
        .map(|token| {
            stemmer
                .as_ref()
                .map(|stemmer| stemmer.stem(&token).into_owned())
                .unwrap_or(token)
        })
        .collect()
}

fn stemmer_algorithm(language: FtsLanguage) -> Algorithm {
    match language {
        FtsLanguage::Arabic => Algorithm::Arabic,
        FtsLanguage::Danish => Algorithm::Danish,
        FtsLanguage::Dutch => Algorithm::Dutch,
        FtsLanguage::English => Algorithm::English,
        FtsLanguage::Finnish => Algorithm::Finnish,
        FtsLanguage::French => Algorithm::French,
        FtsLanguage::German => Algorithm::German,
        FtsLanguage::Greek => Algorithm::Greek,
        FtsLanguage::Hungarian => Algorithm::Hungarian,
        FtsLanguage::Italian => Algorithm::Italian,
        FtsLanguage::Norwegian => Algorithm::Norwegian,
        FtsLanguage::Portuguese => Algorithm::Portuguese,
        FtsLanguage::Romanian => Algorithm::Romanian,
        FtsLanguage::Russian => Algorithm::Russian,
        FtsLanguage::Spanish => Algorithm::Spanish,
        FtsLanguage::Swedish => Algorithm::Swedish,
        FtsLanguage::Tamil => Algorithm::Tamil,
        FtsLanguage::Turkish => Algorithm::Turkish,
    }
}

fn is_stopword(language: FtsLanguage, token: &str) -> bool {
    stop_words::get(stopword_language_code(language)).contains(&token)
}

fn stopword_language_code(language: FtsLanguage) -> &'static str {
    match language {
        FtsLanguage::Arabic => "ar",
        FtsLanguage::Danish => "da",
        FtsLanguage::Dutch => "nl",
        FtsLanguage::English => "en",
        FtsLanguage::Finnish => "fi",
        FtsLanguage::French => "fr",
        FtsLanguage::German => "de",
        FtsLanguage::Greek => "el",
        FtsLanguage::Hungarian => "hu",
        FtsLanguage::Italian => "it",
        FtsLanguage::Norwegian => "no",
        FtsLanguage::Portuguese => "pt",
        FtsLanguage::Romanian => "ro",
        FtsLanguage::Russian => "ru",
        FtsLanguage::Spanish => "es",
        FtsLanguage::Swedish => "sv",
        FtsLanguage::Tamil => "ta",
        FtsLanguage::Turkish => "tr",
    }
}

fn sparse_map(value: &Value, label: &str) -> Result<HashMap<String, f64>, QueryError> {
    let object = as_object(value, label)?;
    object
        .iter()
        .map(|(key, value)| {
            value_as_f64(value)
                .map(|number| (key.clone(), number))
                .ok_or_else(|| {
                    QueryError::new(format!("{label} value for '{key}' must be numeric."))
                })
        })
        .collect()
}

fn dense_vector_from_value(value: &Value) -> Option<Vec<f64>> {
    value.as_array().and_then(|items| {
        if items.is_empty() {
            return None;
        }
        items.iter().map(value_as_f64).collect::<Option<Vec<_>>>()
    })
}

fn numeric_array(value: &Value, label: &str) -> Result<Vec<f64>, QueryError> {
    if let Some(encoded) = value.as_str() {
        let bytes = STANDARD
            .decode(encoded)
            .map_err(|error| QueryError::new(format!("{label} base64 is invalid: {error}")))?;
        if bytes.len() % std::mem::size_of::<f32>() != 0 {
            return Err(QueryError::new(format!(
                "{label} base64 length must be a multiple of 4 bytes."
            )));
        }
        return bytes
            .chunks_exact(std::mem::size_of::<f32>())
            .map(|chunk| {
                let raw = [chunk[0], chunk[1], chunk[2], chunk[3]];
                Ok(f32::from_le_bytes(raw) as f64)
            })
            .collect();
    }
    as_array(value, label)?
        .iter()
        .enumerate()
        .map(|(index, item)| {
            value_as_f64(item)
                .ok_or_else(|| QueryError::new(format!("{label}[{index}] must be numeric.")))
        })
        .collect()
}

fn is_numeric_array(value: &Value) -> bool {
    value
        .as_array()
        .map(|items| !items.is_empty() && items.iter().all(|item| value_as_f64(item).is_some()))
        .unwrap_or(false)
}

fn value_as_f64(value: &Value) -> Option<f64> {
    value.as_f64()
}

fn value_as_u64(value: &Value) -> Option<u64> {
    value.as_u64()
}

fn number_value(value: f64) -> Value {
    Number::from_f64(value)
        .map(Value::Number)
        .unwrap_or(Value::Null)
}

fn as_object<'a>(value: &'a Value, label: &str) -> Result<&'a Map<String, Value>, QueryError> {
    value
        .as_object()
        .ok_or_else(|| QueryError::new(format!("{label} must be an object.")))
}

fn as_array<'a>(value: &'a Value, label: &str) -> Result<&'a Vec<Value>, QueryError> {
    value
        .as_array()
        .ok_or_else(|| QueryError::new(format!("{label} must be an array.")))
}

fn as_string<'a>(value: &'a Value, label: &str) -> Result<&'a str, QueryError> {
    value
        .as_str()
        .ok_or_else(|| QueryError::new(format!("{label} must be a string.")))
}

#[cfg(test)]
mod tests;
