use base64::{Engine, engine::general_purpose::STANDARD};
use chrono::{DateTime, NaiveDate, Utc};
use deunicode::deunicode;
use globset::GlobBuilder;
use regex::Regex;
use rust_stemmers::{Algorithm, Stemmer};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Number, Value, json};
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::error::Error;
use std::fmt::{Display, Formatter};
use unicode_segmentation::UnicodeSegmentation;

const PATCH_BY_FILTER_LIMIT: usize = 50_000;
const DELETE_BY_FILTER_LIMIT: usize = 5_000_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueryError {
    message: String,
}

impl QueryError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl Display for QueryError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for QueryError {}

#[cfg(target_arch = "wasm32")]
mod wasm_api {
    use crate::{Micropuffer, MiniStore, QueryError};
    use serde::Serialize;
    use serde_json::{Value, json};
    use std::convert::TryFrom;
    use std::fmt::Display;
    use wasm_bindgen::prelude::*;

    fn js_error(error: impl Display) -> JsValue {
        JsValue::from_str(&error.to_string())
    }

    fn read_store(store_json: &str) -> Result<MiniStore, JsValue> {
        serde_json::from_str(store_json)
            .map_err(|error| JsValue::from_str(&format!("store must be valid JSON: {error}")))
    }

    fn read_request(request_json: &str) -> Result<Value, JsValue> {
        serde_json::from_str(request_json)
            .map_err(|error| JsValue::from_str(&format!("request must be valid JSON: {error}")))
    }

    fn write_json(value: impl Serialize) -> Result<String, JsValue> {
        serde_json::to_string(&value).map_err(js_error)
    }

    fn write_mutation(engine: &Micropuffer, response: Value) -> Result<String, JsValue> {
        write_json(json!({
            "response": response,
            "store": engine.store()
        }))
    }

    fn page_size_from_request(request: &Value) -> Result<usize, JsValue> {
        match request.get("page_size").and_then(Value::as_u64) {
            Some(page_size) => usize::try_from(page_size)
                .map_err(|_| JsValue::from_str("page_size is too large for this runtime.")),
            None => Ok(1000),
        }
    }

    fn query_error(error: QueryError) -> JsValue {
        JsValue::from_str(&error.to_string())
    }

    #[wasm_bindgen]
    pub fn micropuffer_query(
        store_json: &str,
        namespace_name: &str,
        request_json: &str,
    ) -> Result<String, JsValue> {
        let store = read_store(store_json)?;
        let request = read_request(request_json)?;
        let engine = Micropuffer::from_store(store);
        write_json(
            engine
                .query(namespace_name, &request)
                .map_err(query_error)?,
        )
    }

    #[wasm_bindgen]
    pub fn micropuffer_write(
        store_json: &str,
        namespace_name: &str,
        request_json: &str,
    ) -> Result<String, JsValue> {
        let store = read_store(store_json)?;
        let request = read_request(request_json)?;
        let mut engine = Micropuffer::from_store(store);
        let response = engine
            .write(namespace_name, &request)
            .map_err(query_error)?;
        write_mutation(&engine, response)
    }

    #[wasm_bindgen]
    pub fn micropuffer_delete_namespace(
        store_json: &str,
        namespace_name: &str,
    ) -> Result<String, JsValue> {
        let store = read_store(store_json)?;
        let mut engine = Micropuffer::from_store(store);
        let response = engine
            .delete_namespace(namespace_name)
            .map_err(query_error)?;
        write_mutation(&engine, response)
    }

    #[wasm_bindgen]
    pub fn micropuffer_list_namespaces(
        store_json: &str,
        request_json: &str,
    ) -> Result<String, JsValue> {
        let store = read_store(store_json)?;
        let request = read_request(request_json)?;
        let prefix = request.get("prefix").and_then(Value::as_str);
        let cursor = request.get("cursor").and_then(Value::as_str);
        let page_size = page_size_from_request(&request)?;
        let engine = Micropuffer::from_store(store);
        write_json(
            engine
                .list_namespaces(prefix, cursor, page_size)
                .map_err(query_error)?,
        )
    }

    #[wasm_bindgen]
    pub fn micropuffer_metadata(store_json: &str, namespace_name: &str) -> Result<String, JsValue> {
        let store = read_store(store_json)?;
        let engine = Micropuffer::from_store(store);
        write_json(engine.metadata(namespace_name).map_err(query_error)?)
    }

    #[wasm_bindgen]
    pub fn micropuffer_schema(store_json: &str, namespace_name: &str) -> Result<String, JsValue> {
        let store = read_store(store_json)?;
        let engine = Micropuffer::from_store(store);
        write_json(engine.schema(namespace_name).map_err(query_error)?)
    }

    #[wasm_bindgen]
    pub fn micropuffer_update_schema(
        store_json: &str,
        namespace_name: &str,
        request_json: &str,
    ) -> Result<String, JsValue> {
        let store = read_store(store_json)?;
        let request = read_request(request_json)?;
        let mut engine = Micropuffer::from_store(store);
        let response = engine
            .update_schema(namespace_name, &request)
            .map_err(query_error)?;
        write_mutation(&engine, response)
    }

    #[wasm_bindgen]
    pub fn micropuffer_patch_metadata(
        store_json: &str,
        namespace_name: &str,
        request_json: &str,
    ) -> Result<String, JsValue> {
        let store = read_store(store_json)?;
        let request = read_request(request_json)?;
        let mut engine = Micropuffer::from_store(store);
        let response = engine
            .patch_metadata(namespace_name, &request)
            .map_err(query_error)?;
        write_mutation(&engine, response)
    }

    #[wasm_bindgen]
    pub fn micropuffer_export_namespace(
        store_json: &str,
        namespace_name: &str,
        request_json: &str,
    ) -> Result<String, JsValue> {
        let store = read_store(store_json)?;
        let request = read_request(request_json)?;
        let engine = Micropuffer::from_store(store);
        write_json(
            engine
                .export_namespace(namespace_name, &request)
                .map_err(query_error)?,
        )
    }

    #[wasm_bindgen]
    pub fn micropuffer_warm_cache(
        store_json: &str,
        namespace_name: &str,
    ) -> Result<String, JsValue> {
        let store = read_store(store_json)?;
        let engine = Micropuffer::from_store(store);
        write_json(engine.warm_cache(namespace_name).map_err(query_error)?)
    }

    #[wasm_bindgen]
    pub fn micropuffer_recall(
        store_json: &str,
        namespace_name: &str,
        request_json: &str,
    ) -> Result<String, JsValue> {
        let store = read_store(store_json)?;
        let request = read_request(request_json)?;
        let engine = Micropuffer::from_store(store);
        write_json(
            engine
                .recall(namespace_name, &request)
                .map_err(query_error)?,
        )
    }

    #[wasm_bindgen]
    pub fn micropuffer_explain_query(
        store_json: &str,
        namespace_name: &str,
        request_json: &str,
    ) -> Result<String, JsValue> {
        let store = read_store(store_json)?;
        let request = read_request(request_json)?;
        let engine = Micropuffer::from_store(store);
        write_json(
            engine
                .explain_query(namespace_name, &request)
                .map_err(query_error)?,
        )
    }
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct MiniStore {
    pub namespaces: Vec<Namespace>,
}

impl MiniStore {
    pub fn namespace(&self, name: &str) -> Result<&Namespace, QueryError> {
        self.namespaces
            .iter()
            .find(|namespace| namespace.name == name)
            .ok_or_else(|| QueryError::new(format!("Namespace '{name}' was not found.")))
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
        self.store
            .namespaces
            .retain(|namespace| namespace.name != namespace_name);
        Ok(json!({}))
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
        namespace_metadata(self.store.namespace(namespace_name)?)
    }

    pub fn schema(&self, namespace_name: &str) -> Result<Value, QueryError> {
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
        export_namespace(self.store.namespace(namespace_name)?, request)
    }

    pub fn warm_cache(&self, namespace_name: &str) -> Result<Value, QueryError> {
        self.store.namespace(namespace_name)?;
        Ok(json!({
            "status": "ACCEPTED",
            "message": "micropuffer cache is already memory-resident"
        }))
    }

    pub fn recall(&self, namespace_name: &str, request: &Value) -> Result<Value, QueryError> {
        recall_namespace(self.store.namespace(namespace_name)?, request)
    }

    pub fn explain_query(
        &self,
        namespace_name: &str,
        request: &Value,
    ) -> Result<Value, QueryError> {
        explain_query(self.store.namespace(namespace_name)?, request)
    }
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
}

fn default_created_at() -> String {
    "1970-01-01T00:00:00Z".to_string()
}

fn default_encryption() -> Value {
    json!({ "mode": "default" })
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DistanceMetric {
    CosineDistance,
    #[default]
    EuclideanSquared,
}

impl DistanceMetric {
    fn parse(value: &Value) -> Result<Self, QueryError> {
        match value.as_str() {
            Some("cosine_distance") => Ok(Self::CosineDistance),
            Some("euclidean_squared") => Ok(Self::EuclideanSquared),
            _ => Err(QueryError::new(
                "distance_metric must be 'cosine_distance' or 'euclidean_squared'.",
            )),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Document {
    pub id: Value,
    #[serde(flatten)]
    pub attributes: Map<String, Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
struct Bm25FieldStats {
    doc_count: usize,
    avg_len: f64,
    doc_freqs: HashMap<String, usize>,
    config: FtsConfig,
}

#[derive(Debug, Clone)]
struct Bm25Stats {
    fields: HashMap<String, Bm25FieldStats>,
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
    let namespace = store.namespace(namespace_name)?;
    query_namespace(namespace, request)
}

pub fn namespace_metadata(namespace: &Namespace) -> Result<Value, QueryError> {
    let mut response = Map::new();
    let schema = schema_with_id(namespace);
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
        .map(|(attribute, definition)| normalize_schema_definition(&attribute, &definition))
        .collect::<Result<Map<_, _>, _>>()?;
    Ok(Value::Object(schema))
}

pub fn update_namespace_schema(
    store: &mut MiniStore,
    namespace_name: &str,
    request: &Value,
) -> Result<Value, QueryError> {
    let schema = as_object(request, "schema update request")?;
    let namespace = store
        .namespace_mut(namespace_name)
        .ok_or_else(|| QueryError::new(format!("Namespace '{namespace_name}' was not found.")))?;
    merge_schema(namespace, schema)?;
    namespace.updated_at = logical_now();
    namespace_schema(namespace)
}

pub fn patch_namespace_metadata(
    store: &mut MiniStore,
    namespace_name: &str,
    request: &Value,
) -> Result<Value, QueryError> {
    let request = as_object(request, "metadata patch request")?;
    let namespace = store
        .namespace_mut(namespace_name)
        .ok_or_else(|| QueryError::new(format!("Namespace '{namespace_name}' was not found.")))?;
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
    query_namespace(namespace, &Value::Object(query))
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
        .filter(|document| document.attributes.get("vector").is_some())
        .collect::<Vec<_>>();
    let sample = candidates.iter().take(num).copied().collect::<Vec<_>>();
    let mut ground_truth = Vec::new();
    for query_document in &sample {
        let query_vector = query_document
            .attributes
            .get("vector")
            .ok_or_else(|| QueryError::new("recall query document is missing vector."))?;
        let mut neighbors = candidates
            .iter()
            .map(|document| {
                dense_distance(document, "vector", query_vector, namespace.distance_metric)
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
    if let Some(vector) = document.attributes.get("vector") {
        row.insert("vector".to_string(), vector.clone());
    }
    Value::Object(row)
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
    ensure_namespace(store, namespace_name);
    if let Some(distance_metric) = request.get("distance_metric") {
        let metric = DistanceMetric::parse(distance_metric)?;
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

    let mut upserts =
        collect_write_rows(request.get("upsert_rows"), request.get("upsert_columns"))?;
    let mut patches = collect_write_rows(request.get("patch_rows"), request.get("patch_columns"))?;
    let deletes = collect_delete_ids(request.get("deletes"))?;
    reject_duplicate_write_ids(&upserts, &patches, &deletes)?;

    let upsert_condition = request.get("upsert_condition");
    let patch_condition = request.get("patch_condition");
    let delete_condition = request.get("delete_condition");
    let namespace = store
        .namespace_mut(namespace_name)
        .ok_or_else(|| QueryError::new("namespace disappeared during write."))?;
    validate_write_rows_against_schema(namespace, &upserts, &patches)?;
    normalize_write_rows_against_schema(namespace, &mut upserts, &mut patches)?;
    for id in deletes {
        if delete_document(namespace, &id, delete_condition)? {
            summary.deleted_ids.push(id);
        }
    }
    for row in patches {
        summary.billable_logical_bytes_written += write_row_logical_bytes(&row);
        let id = row.id.clone();
        if patch_document(namespace, row, patch_condition)? {
            summary.patched_ids.push(id);
        }
    }
    for row in upserts {
        summary.billable_logical_bytes_written += write_row_logical_bytes(&row);
        let id = row.id.clone();
        if upsert_document(namespace, row, upsert_condition)? {
            summary.upserted_ids.push(id);
        }
    }
    refresh_inferred_schema(namespace)?;
    if !summary.upserted_ids.is_empty()
        || !summary.patched_ids.is_empty()
        || !summary.deleted_ids.is_empty()
    {
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
    let bm25_stats = Bm25Stats::new(namespace);
    let mut ranked = rank_documents(namespace, rank_plan.clone(), filters, &bm25_stats)?;
    sort_ranked_documents(&mut ranked, rank_plan.kind.clone());
    let ranked = apply_limit(ranked, &limit, rank_plan.kind.clone())?;
    let mut rows = Vec::with_capacity(ranked.len());
    for ranked_doc in ranked {
        rows.push(project_document(
            ranked_doc.doc,
            object,
            options.vector_encoding,
            rank_plan.kind.clone(),
            ranked_doc.score,
        )?);
    }
    Ok(with_metrics(json!({ "rows": rows }), namespace))
}

fn with_metrics(mut response: Value, namespace: &Namespace) -> Value {
    if let Value::Object(object) = &mut response {
        object.insert(
            "billing".to_string(),
            json!({
                "billable_logical_bytes_queried": rough_namespace_bytes(namespace),
                "billable_logical_bytes_returned": rough_namespace_bytes(namespace).min(4096)
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
        let incoming_type = schema_type_name(definition)?;
        validate_schema_definition(attribute, definition, incoming_type)?;
        if namespace.schema.contains_key(attribute) {
            let current_type = schema_type_name(
                namespace
                    .schema
                    .get(attribute)
                    .ok_or_else(|| QueryError::new("schema changed unexpectedly."))?,
            )?;
            if current_type != incoming_type {
                return Err(QueryError::new(format!(
                    "Changing the type of attribute '{attribute}' is not supported."
                )));
            }
        } else if inferred_schema(namespace).contains_key(attribute) {
            let current = inferred_schema(namespace);
            let current_type = schema_type_name(
                current
                    .get(attribute)
                    .ok_or_else(|| QueryError::new("schema inference changed unexpectedly."))?,
            )?;
            if current_type != incoming_type {
                return Err(QueryError::new(format!(
                    "Changing the type of attribute '{attribute}' is not supported."
                )));
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
            .insert(attribute.clone(), definition.clone());
    }
    validate_vector_column_count(&namespace.schema)
}

fn refresh_inferred_schema(namespace: &mut Namespace) -> Result<(), QueryError> {
    let mut schema = namespace.schema.clone();
    for document in &namespace.documents {
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

fn normalize_schema_definition(
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
    if attribute == "vector" && is_dense_vector_type(&schema_type) && !config.contains_key("ann") {
        config.insert(
            "ann".to_string(),
            json!({ "distance_metric": "euclidean_squared" }),
        );
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
    for key in object.keys() {
        match key.as_str() {
            "type" | "filterable" | "regex" | "glob" | "fuzzy" | "full_text_search" | "ann"
            | "sparse_knn" => {}
            _ => {
                return Err(QueryError::new(format!(
                    "schema.{attribute}.{key} is not supported."
                )));
            }
        }
    }
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
    if name.is_empty() || name.len() > 128 {
        return Err(QueryError::new(
            "namespace names must be between 1 and 128 characters.",
        ));
    }
    if !name
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || "-_.".contains(character))
    {
        return Err(QueryError::new(
            "namespace names must match [A-Za-z0-9-_.]{1,128}.",
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
        });
    }
    Ok(json!({
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
                "{operation} cannot be combined with '{key}'."
            )));
        }
    }
    Ok(())
}

fn collect_write_rows(
    rows_value: Option<&Value>,
    columns_value: Option<&Value>,
) -> Result<Vec<WriteRow>, QueryError> {
    let mut rows = Vec::new();
    if let Some(rows_value) = rows_value {
        for row in as_array(rows_value, "write rows")? {
            rows.push(write_row_from_object(as_object(row, "write row")?)?);
        }
    }
    if let Some(columns_value) = columns_value {
        rows.extend(write_rows_from_columns(columns_value)?);
    }
    Ok(rows)
}

fn write_row_from_object(object: &Map<String, Value>) -> Result<WriteRow, QueryError> {
    let id = object
        .get("id")
        .cloned()
        .ok_or_else(|| QueryError::new("write row id is required."))?;
    validate_document_id(&id)?;
    let attributes = object
        .iter()
        .filter(|(key, _)| key.as_str() != "id")
        .map(|(key, value)| validate_attribute_name(key).map(|()| (key.clone(), value.clone())))
        .collect::<Result<Map<_, _>, _>>()?;
    Ok(WriteRow { id, attributes })
}

fn write_rows_from_columns(columns_value: &Value) -> Result<Vec<WriteRow>, QueryError> {
    let columns = as_object(columns_value, "write columns")?;
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
    as_array(deletes, "deletes")?
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
    for id in upserts
        .iter()
        .map(|row| &row.id)
        .chain(patches.iter().map(|row| &row.id))
        .chain(deletes.iter())
    {
        let key = id_key(id);
        if !seen.insert(key) {
            return Err(QueryError::new(
                "the same document ID cannot appear multiple times in one write request.",
            ));
        }
    }
    Ok(())
}

fn upsert_document(
    namespace: &mut Namespace,
    row: WriteRow,
    condition: Option<&Value>,
) -> Result<bool, QueryError> {
    let existing_index = document_index(namespace, &row.id);
    if let Some(index) = existing_index {
        if !condition
            .map(|filter| eval_filter_with_new(&namespace.documents[index], filter, Some(&row)))
            .transpose()?
            .unwrap_or(true)
        {
            return Ok(false);
        }
        namespace.documents[index] = Document {
            id: row.id,
            attributes: row.attributes,
        };
    } else {
        namespace.documents.push(Document {
            id: row.id,
            attributes: row.attributes,
        });
    }
    Ok(true)
}

fn patch_document(
    namespace: &mut Namespace,
    row: WriteRow,
    condition: Option<&Value>,
) -> Result<bool, QueryError> {
    let Some(index) = document_index(namespace, &row.id) else {
        return Ok(false);
    };
    if !condition
        .map(|filter| eval_filter_with_new(&namespace.documents[index], filter, Some(&row)))
        .transpose()?
        .unwrap_or(true)
    {
        return Ok(false);
    }
    for (key, value) in row.attributes {
        namespace.documents[index].attributes.insert(key, value);
    }
    Ok(true)
}

fn delete_document(
    namespace: &mut Namespace,
    id: &Value,
    condition: Option<&Value>,
) -> Result<bool, QueryError> {
    let Some(index) = document_index(namespace, id) else {
        return Ok(false);
    };
    if !condition
        .map(|filter| eval_filter_with_new(&namespace.documents[index], filter, None))
        .transpose()?
        .unwrap_or(true)
    {
        return Ok(false);
    }
    namespace.documents.remove(index);
    Ok(true)
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
    for (index, document) in namespace.documents.iter_mut().enumerate() {
        if selected.contains(&index) {
            for (key, value) in patch {
                document.attributes.insert(key.clone(), value.clone());
            }
            patched.push(document.id.clone());
        }
    }
    Ok(FilterWriteOutcome {
        ids: patched,
        rows_remaining,
    })
}

fn document_index(namespace: &Namespace, id: &Value) -> Option<usize> {
    namespace
        .documents
        .iter()
        .position(|document| values_equal(&document.id, id))
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
    id.to_string()
}

impl Bm25Stats {
    fn new(namespace: &Namespace) -> Self {
        let mut fields: BTreeSet<String> = BTreeSet::new();
        for document in &namespace.documents {
            for (field, value) in &document.attributes {
                if value.is_string()
                    || value
                        .as_array()
                        .is_some_and(|items| items.iter().all(Value::is_string))
                {
                    fields.insert(field.clone());
                }
            }
        }
        let mut stats = HashMap::new();
        for field in fields {
            let config = fts_config_for_field(namespace, &field);
            let mut doc_freqs: HashMap<String, usize> = HashMap::new();
            let mut total_len = 0usize;
            let mut doc_count = 0usize;
            for document in &namespace.documents {
                let tokens = string_attr_tokens_with_config(document, &field, &config);
                if tokens.is_empty() {
                    continue;
                }
                doc_count += 1;
                total_len += tokens.len();
                let unique: BTreeSet<String> = tokens.into_iter().collect();
                for token in unique {
                    let count = doc_freqs.entry(token).or_insert(0);
                    *count += 1;
                }
            }
            let avg_len = if doc_count == 0 {
                0.0
            } else {
                total_len as f64 / doc_count as f64
            };
            stats.insert(
                field,
                Bm25FieldStats {
                    doc_count,
                    avg_len,
                    doc_freqs,
                    config,
                },
            );
        }
        Self { fields: stats }
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
        Some(_) => Err(QueryError::new(
            "vector_encoding must be either 'float' or 'base64'.",
        )),
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

fn rank_documents<'a>(
    namespace: &'a Namespace,
    rank_plan: RankPlan<'a>,
    filters: Option<&Value>,
    bm25_stats: &Bm25Stats,
) -> Result<Vec<RankedDocument<'a>>, QueryError> {
    let mut ranked = Vec::new();
    for document in &namespace.documents {
        if !filters
            .map(|filter| eval_filter_with_schema(document, filter, Some(&namespace.schema)))
            .transpose()?
            .unwrap_or(true)
        {
            continue;
        }
        let score = match rank_plan.kind {
            RankKind::AttributeOrder { .. } | RankKind::MultiAttributeOrder { .. } => 0.0,
            RankKind::SmallerIsBetter | RankKind::LargerIsBetter => eval_rank_expr(
                document,
                rank_plan.rank_by,
                bm25_stats,
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

fn sort_ranked_documents(ranked: &mut [RankedDocument<'_>], kind: RankKind) {
    ranked.sort_by(|left, right| match kind {
        RankKind::SmallerIsBetter => compare_f64(left.score, right.score)
            .then_with(|| stable_id_compare(&left.doc.id, &right.doc.id)),
        RankKind::LargerIsBetter => compare_f64(right.score, left.score)
            .then_with(|| stable_id_compare(&left.doc.id, &right.doc.id)),
        RankKind::AttributeOrder {
            ref attribute,
            direction,
        } => compare_order_attr(left.doc, right.doc, attribute, direction)
            .then_with(|| stable_id_compare(&left.doc.id, &right.doc.id)),
        RankKind::MultiAttributeOrder { ref attributes } => {
            compare_order_attrs(left.doc, right.doc, attributes)
                .then_with(|| stable_id_compare(&left.doc.id, &right.doc.id))
        }
    });
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

fn sort_ranked_by_attribute(
    ranked: &mut [RankedDocument<'_>],
    attr: &str,
    direction: SortDirection,
) {
    ranked.sort_by(|left, right| {
        let base = compare_values_for_order(
            document_attr(left.doc, attr),
            document_attr(right.doc, attr),
        );
        let directed = match direction {
            SortDirection::Asc => base,
            SortDirection::Desc => base.reverse(),
        };
        directed.then_with(|| stable_id_compare(&left.doc.id, &right.doc.id))
    });
}

fn apply_limit<'a>(
    mut ranked: Vec<RankedDocument<'a>>,
    limit: &Limit,
    kind: RankKind,
) -> Result<Vec<RankedDocument<'a>>, QueryError> {
    if let RankKind::AttributeOrder {
        ref attribute,
        direction,
    } = kind
    {
        sort_ranked_by_attribute(&mut ranked, attribute, direction);
    }
    let Some(per) = &limit.per else {
        ranked.truncate(limit.total);
        return Ok(ranked);
    };
    if !matches!(
        kind,
        RankKind::AttributeOrder { .. } | RankKind::MultiAttributeOrder { .. }
    ) {
        return Err(QueryError::new(
            "limit.per is only supported for order by attribute queries.",
        ));
    }
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

fn eval_rank_expr(
    document: &Document,
    expression: &Value,
    bm25_stats: &Bm25Stats,
    distance_metric: DistanceMetric,
) -> Result<f64, QueryError> {
    let array = as_array(expression, "rank expression")?;
    if array.is_empty() {
        return Err(QueryError::new("rank expression cannot be empty."));
    }
    if let Some(op) = array[0].as_str() {
        match op {
            "Sum" => return eval_sum(document, array, bm25_stats, distance_metric),
            "Max" => return eval_max(document, array, bm25_stats, distance_metric),
            "Product" => return eval_product(document, array, bm25_stats, distance_metric),
            "Attribute" => return eval_attribute_score(document, array),
            "Saturate" => return eval_saturate(document, array, bm25_stats, distance_metric),
            "Decay" => return eval_decay(document, array, bm25_stats, distance_metric),
            "Dist" => return eval_dist(document, array, bm25_stats, distance_metric),
            "And" | "Or" | "Not" => {
                return Ok(if eval_filter(document, expression)? {
                    1.0
                } else {
                    0.0
                });
            }
            _ => {}
        }
    }
    if array.len() >= 3 {
        let attr = as_string(&array[0], "rank_by attribute")?;
        let op = as_string(&array[1], "rank_by operator")?;
        return match op {
            "ANN" | "kNN" => dense_distance(document, attr, &array[2], distance_metric),
            "SparseKNN" => sparse_dot_product(document, attr, &array[2]),
            "BM25" => bm25_score(document, attr, &array[2], array.get(3), bm25_stats),
            _ => Ok(if eval_filter(document, expression)? {
                1.0
            } else {
                0.0
            }),
        };
    }
    Err(QueryError::new("unsupported rank expression."))
}

fn eval_sum(
    document: &Document,
    array: &[Value],
    bm25_stats: &Bm25Stats,
    distance_metric: DistanceMetric,
) -> Result<f64, QueryError> {
    let terms = array
        .get(1)
        .ok_or_else(|| QueryError::new("Sum requires terms."))?;
    let terms = as_array(terms, "Sum terms")?;
    let mut total = 0.0;
    for term in terms {
        total += eval_rank_expr(document, term, bm25_stats, distance_metric)?;
    }
    Ok(total)
}

fn eval_max(
    document: &Document,
    array: &[Value],
    bm25_stats: &Bm25Stats,
    distance_metric: DistanceMetric,
) -> Result<f64, QueryError> {
    let terms: Vec<&Value> = if array.len() == 2 {
        as_array(&array[1], "Max terms")?.iter().collect()
    } else {
        array.iter().skip(1).collect()
    };
    let mut best = 0.0;
    for term in terms {
        let score = if let Some(number) = term.as_f64() {
            number
        } else {
            eval_rank_expr(document, term, bm25_stats, distance_metric)?
        };
        if score > best {
            best = score;
        }
    }
    Ok(best)
}

fn eval_product(
    document: &Document,
    array: &[Value],
    bm25_stats: &Bm25Stats,
    distance_metric: DistanceMetric,
) -> Result<f64, QueryError> {
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
    Ok(weight * eval_rank_expr(document, &array[2], bm25_stats, distance_metric)?)
}

fn eval_attribute_score(document: &Document, array: &[Value]) -> Result<f64, QueryError> {
    if array.len() != 2 {
        return Err(QueryError::new("Attribute requires one attribute name."));
    }
    let attr = as_string(&array[1], "Attribute name")?;
    Ok(document_attr(document, attr)
        .and_then(value_as_f64)
        .unwrap_or(0.0))
}

fn eval_saturate(
    document: &Document,
    array: &[Value],
    bm25_stats: &Bm25Stats,
    distance_metric: DistanceMetric,
) -> Result<f64, QueryError> {
    if array.len() < 2 {
        return Err(QueryError::new("Saturate requires an expression."));
    }
    let score = eval_rank_expr(document, &array[1], bm25_stats, distance_metric)?.max(0.0);
    let options = array.get(2).and_then(Value::as_object);
    let midpoint = options
        .and_then(|object| object.get("midpoint"))
        .and_then(value_as_f64)
        .unwrap_or(1.0);
    let exponent = options
        .and_then(|object| object.get("exponent"))
        .and_then(value_as_f64)
        .unwrap_or(1.0);
    if score <= 0.0 || midpoint <= 0.0 || exponent <= 0.0 {
        return Ok(0.0);
    }
    let powered = score.powf(exponent);
    Ok(powered / (powered + midpoint.powf(exponent)))
}

fn eval_decay(
    document: &Document,
    array: &[Value],
    bm25_stats: &Bm25Stats,
    distance_metric: DistanceMetric,
) -> Result<f64, QueryError> {
    if array.len() < 2 {
        return Err(QueryError::new("Decay requires an expression."));
    }
    let distance = eval_rank_expr(document, &array[1], bm25_stats, distance_metric)?.abs();
    let options = array.get(2).and_then(Value::as_object);
    let midpoint = options
        .and_then(|object| object.get("midpoint"))
        .map(parse_midpoint)
        .transpose()?
        .unwrap_or(1.0);
    let exponent = options
        .and_then(|object| object.get("exponent"))
        .and_then(value_as_f64)
        .unwrap_or(1.0);
    if midpoint <= 0.0 || exponent <= 0.0 {
        return Ok(0.0);
    }
    let midpoint = midpoint.powf(exponent);
    Ok(midpoint / (distance.powf(exponent) + midpoint))
}

fn eval_dist(
    document: &Document,
    array: &[Value],
    bm25_stats: &Bm25Stats,
    distance_metric: DistanceMetric,
) -> Result<f64, QueryError> {
    if array.len() != 3 {
        return Err(QueryError::new(
            "Dist requires an expression and an origin.",
        ));
    }
    let value = if let Some(attribute_expr) = array[1].as_array() {
        if attribute_expr.first().and_then(Value::as_str) == Some("Attribute") {
            let attr = attribute_expr
                .get(1)
                .and_then(Value::as_str)
                .ok_or_else(|| QueryError::new("Dist Attribute requires a name."))?;
            document_attr(document, attr)
                .cloned()
                .unwrap_or(Value::Null)
        } else {
            number_value(eval_rank_expr(
                document,
                &array[1],
                bm25_stats,
                distance_metric,
            )?)
        }
    } else {
        number_value(eval_rank_expr(
            document,
            &array[1],
            bm25_stats,
            distance_metric,
        )?)
    };
    numeric_or_datetime_distance(&value, &array[2])
}

fn dense_distance(
    document: &Document,
    attr: &str,
    query: &Value,
    distance_metric: DistanceMetric,
) -> Result<f64, QueryError> {
    let left = document
        .attributes
        .get(attr)
        .ok_or_else(|| QueryError::new(format!("Vector attribute '{attr}' is missing.")))?;
    let left = numeric_array(left, attr)?;
    let right = numeric_array(query, "query vector")?;
    if left.len() != right.len() {
        return Err(QueryError::new(format!(
            "Vector dimension mismatch for '{attr}': document has {}, query has {}.",
            left.len(),
            right.len()
        )));
    }
    match distance_metric {
        DistanceMetric::EuclideanSquared => Ok(left
            .iter()
            .zip(right.iter())
            .map(|(doc_value, query_value)| {
                let delta = doc_value - query_value;
                delta * delta
            })
            .sum()),
        DistanceMetric::CosineDistance => {
            let dot = left
                .iter()
                .zip(right.iter())
                .map(|(doc_value, query_value)| doc_value * query_value)
                .sum::<f64>();
            let left_norm = left.iter().map(|value| value * value).sum::<f64>().sqrt();
            let right_norm = right.iter().map(|value| value * value).sum::<f64>().sqrt();
            if left_norm == 0.0 || right_norm == 0.0 {
                return Ok(1.0);
            }
            Ok(1.0 - dot / (left_norm * right_norm))
        }
    }
}

fn sparse_dot_product(document: &Document, attr: &str, query: &Value) -> Result<f64, QueryError> {
    let doc_vector = document
        .attributes
        .get(attr)
        .map(|value| sparse_map(value, attr))
        .transpose()?
        .unwrap_or_default();
    let query_vector = sparse_map(query, "query sparse vector")?;
    let mut score = 0.0;
    for (key, query_value) in query_vector {
        let doc_value = doc_vector.get(&key).copied().unwrap_or(0.0);
        score += doc_value * query_value;
    }
    Ok(score)
}

fn bm25_score(
    document: &Document,
    field: &str,
    query: &Value,
    options: Option<&Value>,
    stats: &Bm25Stats,
) -> Result<f64, QueryError> {
    let field_stats = match stats.fields.get(field) {
        Some(stats) if stats.doc_count > 0 && stats.avg_len > 0.0 => stats,
        _ => return Ok(0.0),
    };
    let query_tokens = query_tokens_with_config(query, "BM25 query", &field_stats.config)?;
    if query_tokens.is_empty() {
        return Ok(0.0);
    }
    let document_tokens = string_attr_tokens_with_config(document, field, &field_stats.config);
    if document_tokens.is_empty() {
        return Ok(0.0);
    }
    let mut term_counts: HashMap<String, usize> = HashMap::new();
    for token in &document_tokens {
        let count = term_counts.entry(token.clone()).or_insert(0);
        *count += 1;
    }
    let last_as_prefix = options
        .and_then(Value::as_object)
        .and_then(|object| object.get("last_as_prefix"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let mut score = 0.0;
    let mut query_term_counts: HashMap<&String, usize> = HashMap::new();
    for token in &query_tokens {
        let count = query_term_counts.entry(token).or_insert(0);
        *count += 1;
    }
    for (index, token) in query_tokens.iter().enumerate() {
        if query_tokens[..index].contains(token) {
            continue;
        }
        let prefix = last_as_prefix && index + 1 == query_tokens.len();
        if prefix {
            if document_tokens
                .iter()
                .any(|doc_token| doc_token.starts_with(token))
            {
                score += 1.0;
            }
            continue;
        }
        let Some(tf) = term_counts.get(token).copied() else {
            continue;
        };
        let doc_freq = field_stats.doc_freqs.get(token).copied().unwrap_or(0);
        if doc_freq == 0 {
            continue;
        }
        let idf = ((field_stats.doc_count as f64 - doc_freq as f64 + 0.5)
            / (doc_freq as f64 + 0.5)
            + 1.0)
            .ln();
        let tf = tf as f64;
        let doc_len = document_tokens.len() as f64;
        let k1 = field_stats.config.k1();
        let b = field_stats.config.b();
        let k3 = field_stats.config.k3();
        let qtf = query_term_counts.get(token).copied().unwrap_or(1) as f64;
        let query_weight = (qtf * (k3 + 1.0)) / (qtf + k3);
        score += idf * (tf * (k1 + 1.0))
            / (tf + k1 * (1.0 - b + b * doc_len / field_stats.avg_len))
            * query_weight;
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
        (Some(Value::Bool(true)), None) => {
            for (key, value) in &document.attributes {
                row.insert(key.clone(), maybe_encode_vector(value, vector_encoding)?);
            }
        }
        (Some(Value::Array(attributes)), None) => {
            for attribute in attributes {
                let name = as_string(attribute, "include_attributes item")?;
                if let Some(value) = document.attributes.get(name) {
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
    let filters = request.get("filters");
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

fn query_grouped_aggregations(
    namespace: &Namespace,
    request: &Map<String, Value>,
    group_by: &Value,
    aggregations: &Map<String, Value>,
    documents: &[&Document],
) -> Result<Value, QueryError> {
    let group_exprs = as_array(group_by, "group_by")?;
    let limit = parse_limit(request)?;
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
    for (_, (mut group_key, docs)) in groups.into_iter().take(limit.total) {
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
        "Count" => Ok(Value::Number(Number::from(documents.len()))),
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
mod tests {
    use crate::{
        DistanceMetric, Document, Micropuffer, MiniStore, Namespace, PATCH_BY_FILTER_LIMIT,
        default_created_at, default_encryption, parse_fts_config, query_namespace, query_store,
        tokenize, write_store,
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
            response_rows[0]["$dist"].as_f64().unwrap()
                > response_rows[1]["$dist"].as_f64().unwrap()
        );

        let token_array = query_namespace(
            &namespace(),
            &json!({
                "rank_by": ["body", "BM25", ["rust", "hybrid"]],
                "limit": 3
            }),
        )
        .unwrap();
        assert_eq!(rows(&token_array)[0]["id"], 3);
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

        let encoded_query =
            STANDARD.encode([1.0_f32.to_le_bytes(), 0.0_f32.to_le_bytes()].concat());
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
    fn aggregates_and_grouped_aggregates_apply_filters() {
        let response = query_namespace(
            &namespace(),
            &json!({
                "aggregate_by": {"count": ["Count"], "score_sum": ["Sum", "score"]},
                "filters": ["public", "Eq", true],
                "limit": 10
            }),
        )
        .unwrap();
        assert_eq!(response["aggregations"]["count"], 2);
        assert_eq!(response["aggregations"]["score_sum"], 17.0);

        let grouped = query_namespace(
            &namespace(),
            &json!({
                "aggregate_by": {"count": ["Count"]},
                "group_by": ["tenant_id", {"tag": ["ForEachUnique", "tags"]}],
                "limit": {"total": 10}
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
    }

    #[test]
    fn multi_query_preserves_result_order() {
        let response = query_namespace(
            &namespace(),
            &json!({
                "queries": [
                    {"rank_by": ["vector", "ANN", [0.0, 0.0]], "limit": 1},
                    {"aggregate_by": {"count": ["Count"]}, "limit": 1}
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
    fn writes_upsert_patch_delete_and_query_in_memory() {
        let mut store = MiniStore::default();
        write_store(
            &mut store,
            "local-test",
            &json!({
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
    fn writes_enforce_schema_types_and_vector_invariants() {
        let mut clone = Micropuffer::new();
        clone
            .write(
                "schema-demo",
                &json!({
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
    fn patch_by_filter_respects_partial_limit_and_rows_remaining() {
        let mut documents = Vec::with_capacity(PATCH_BY_FILTER_LIMIT + 1);
        for index in 0..=PATCH_BY_FILTER_LIMIT {
            documents.push(Document {
                id: Value::Number(Number::from(index as u64)),
                attributes: Map::from_iter([
                    ("group".to_string(), Value::String("all".to_string())),
                    ("patched".to_string(), Value::Bool(false)),
                ]),
            });
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
        assert!(type_change.to_string().contains("Changing the type"));

        let vector_count = clone
            .write(
                "too-many-vectors",
                &json!({
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
        clone
            .write("demo-copy", &json!({"copy_from_namespace": "demo"}))
            .unwrap();
        let listed = clone.list_namespaces(Some("demo"), None, 10).unwrap();
        let namespaces = listed.get("namespaces").and_then(Value::as_array).unwrap();
        assert_eq!(namespaces.len(), 2);
        let copied = clone
            .query(
                "demo-copy",
                &json!({"aggregate_by": {"count": ["Count"]}, "limit": 1}),
            )
            .unwrap();
        assert_eq!(copied["aggregations"]["count"], 3);
        clone.delete_namespace("demo-copy").unwrap();
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
        assert!(
            invalid
                .to_string()
                .contains("copy_from_namespace cannot be combined")
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
        assert_eq!(metadata["schema"]["published_at"], "datetime");
        assert_eq!(metadata["schema"]["vector"], "[2]f32");
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
        assert_eq!(rows(&export).len(), 1);
        assert_eq!(rows(&export)[0]["id"], 2);
        assert!(rows(&export)[0].get("$dist").is_none());
    }

    #[test]
    fn schema_update_and_warm_cache_match_workspace_shapes() {
        let mut clone = Micropuffer::new();
        clone
            .write(
                "schema-api",
                &json!({
                    "upsert_rows": [
                        {"id": 1, "vector": [0.0, 0.0], "title": "hello"}
                    ]
                }),
            )
            .unwrap();
        let schema = clone.schema("schema-api").unwrap();
        assert_eq!(schema["id"]["type"], "uint");
        assert_eq!(schema["vector"]["type"], "[2]f32");
        assert_eq!(schema["title"]["type"], "string");

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
        assert_eq!(updated["title"]["regex"], true);

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

        let v0 =
            parse_fts_config(&json!({"tokenizer": "word_v0", "remove_stopwords": false})).unwrap();
        let v1 =
            parse_fts_config(&json!({"tokenizer": "word_v1", "remove_stopwords": false})).unwrap();
        let v2 =
            parse_fts_config(&json!({"tokenizer": "word_v2", "remove_stopwords": false})).unwrap();
        let v3 =
            parse_fts_config(&json!({"tokenizer": "word_v3", "remove_stopwords": false})).unwrap();
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
        assert!(
            invalid_extra_field
                .to_string()
                .contains("branch_from_namespace cannot be combined")
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
}
