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
pub fn micropuffer_warm_cache(store_json: &str, namespace_name: &str) -> Result<String, JsValue> {
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
