use crate::{Micropuffer as CoreMicropuffer, MiniStore, QueryError};
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

#[derive(Serialize)]
struct HttpResponse {
    status: u16,
    body: Value,
}

fn error_body(error: &QueryError) -> Value {
    let mut body = json!({
        "status": "error",
        "error": error.to_string()
    });
    let Some(object) = body.as_object_mut() else {
        return body;
    };
    for (key, value) in error.body_fields() {
        object.insert(key.clone(), value.clone());
    }
    body
}

fn http_response(result: Result<Value, QueryError>) -> Result<String, JsValue> {
    let response = match result {
        Ok(body) => HttpResponse { status: 200, body },
        Err(error) => HttpResponse {
            status: error.status_code(),
            body: error_body(&error),
        },
    };
    write_json(response)
}

#[wasm_bindgen]
pub struct Micropuffer {
    engine: CoreMicropuffer,
}

#[wasm_bindgen]
impl Micropuffer {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        Self {
            engine: CoreMicropuffer::new(),
        }
    }

    #[wasm_bindgen(js_name = fromStore)]
    pub fn from_store(store_json: &str) -> Result<Micropuffer, JsValue> {
        Ok(Self {
            engine: CoreMicropuffer::from_store(read_store(store_json)?),
        })
    }

    #[wasm_bindgen(js_name = replaceStore)]
    pub fn replace_store(&mut self, store_json: &str) -> Result<(), JsValue> {
        self.engine = CoreMicropuffer::from_store(read_store(store_json)?);
        Ok(())
    }

    #[wasm_bindgen(js_name = exportStore)]
    pub fn export_store(&self) -> Result<String, JsValue> {
        write_json(self.engine.store())
    }

    pub fn query(&self, namespace_name: &str, request_json: &str) -> Result<String, JsValue> {
        let request = read_request(request_json)?;
        write_json(
            self.engine
                .query(namespace_name, &request)
                .map_err(query_error)?,
        )
    }

    #[wasm_bindgen(js_name = queryResponse)]
    pub fn query_response(
        &self,
        namespace_name: &str,
        request_json: &str,
    ) -> Result<String, JsValue> {
        let request = read_request(request_json)?;
        http_response(self.engine.query(namespace_name, &request))
    }

    pub fn write(&mut self, namespace_name: &str, request_json: &str) -> Result<String, JsValue> {
        let request = read_request(request_json)?;
        write_json(
            self.engine
                .write(namespace_name, &request)
                .map_err(query_error)?,
        )
    }

    #[wasm_bindgen(js_name = writeResponse)]
    pub fn write_response(
        &mut self,
        namespace_name: &str,
        request_json: &str,
    ) -> Result<String, JsValue> {
        let request = read_request(request_json)?;
        http_response(self.engine.write(namespace_name, &request))
    }

    #[wasm_bindgen(js_name = deleteNamespace)]
    pub fn delete_namespace(&mut self, namespace_name: &str) -> Result<String, JsValue> {
        write_json(
            self.engine
                .delete_namespace(namespace_name)
                .map_err(query_error)?,
        )
    }

    #[wasm_bindgen(js_name = deleteNamespaceResponse)]
    pub fn delete_namespace_response(&mut self, namespace_name: &str) -> Result<String, JsValue> {
        http_response(self.engine.delete_namespace(namespace_name))
    }

    #[wasm_bindgen(js_name = listNamespaces)]
    pub fn list_namespaces(&self, request_json: &str) -> Result<String, JsValue> {
        let request = read_request(request_json)?;
        let prefix = request.get("prefix").and_then(Value::as_str);
        let cursor = request.get("cursor").and_then(Value::as_str);
        let page_size = page_size_from_request(&request)?;
        write_json(
            self.engine
                .list_namespaces(prefix, cursor, page_size)
                .map_err(query_error)?,
        )
    }

    #[wasm_bindgen(js_name = listNamespacesResponse)]
    pub fn list_namespaces_response(&self, request_json: &str) -> Result<String, JsValue> {
        let request = read_request(request_json)?;
        let prefix = request.get("prefix").and_then(Value::as_str);
        let cursor = request.get("cursor").and_then(Value::as_str);
        let page_size = page_size_from_request(&request)?;
        http_response(self.engine.list_namespaces(prefix, cursor, page_size))
    }

    pub fn metadata(&self, namespace_name: &str) -> Result<String, JsValue> {
        write_json(self.engine.metadata(namespace_name).map_err(query_error)?)
    }

    #[wasm_bindgen(js_name = metadataResponse)]
    pub fn metadata_response(&self, namespace_name: &str) -> Result<String, JsValue> {
        http_response(self.engine.metadata(namespace_name))
    }

    pub fn schema(&self, namespace_name: &str) -> Result<String, JsValue> {
        write_json(self.engine.schema(namespace_name).map_err(query_error)?)
    }

    #[wasm_bindgen(js_name = schemaResponse)]
    pub fn schema_response(&self, namespace_name: &str) -> Result<String, JsValue> {
        http_response(self.engine.schema(namespace_name))
    }

    #[wasm_bindgen(js_name = updateSchema)]
    pub fn update_schema(
        &mut self,
        namespace_name: &str,
        request_json: &str,
    ) -> Result<String, JsValue> {
        let request = read_request(request_json)?;
        write_json(
            self.engine
                .update_schema(namespace_name, &request)
                .map_err(query_error)?,
        )
    }

    #[wasm_bindgen(js_name = updateSchemaResponse)]
    pub fn update_schema_response(
        &mut self,
        namespace_name: &str,
        request_json: &str,
    ) -> Result<String, JsValue> {
        let request = read_request(request_json)?;
        http_response(self.engine.update_schema(namespace_name, &request))
    }

    #[wasm_bindgen(js_name = patchMetadata)]
    pub fn patch_metadata(
        &mut self,
        namespace_name: &str,
        request_json: &str,
    ) -> Result<String, JsValue> {
        let request = read_request(request_json)?;
        write_json(
            self.engine
                .patch_metadata(namespace_name, &request)
                .map_err(query_error)?,
        )
    }

    #[wasm_bindgen(js_name = patchMetadataResponse)]
    pub fn patch_metadata_response(
        &mut self,
        namespace_name: &str,
        request_json: &str,
    ) -> Result<String, JsValue> {
        let request = read_request(request_json)?;
        http_response(self.engine.patch_metadata(namespace_name, &request))
    }

    #[wasm_bindgen(js_name = exportNamespace)]
    pub fn export_namespace(
        &self,
        namespace_name: &str,
        request_json: &str,
    ) -> Result<String, JsValue> {
        let request = read_request(request_json)?;
        write_json(
            self.engine
                .export_namespace(namespace_name, &request)
                .map_err(query_error)?,
        )
    }

    #[wasm_bindgen(js_name = exportNamespaceResponse)]
    pub fn export_namespace_response(
        &self,
        namespace_name: &str,
        request_json: &str,
    ) -> Result<String, JsValue> {
        let request = read_request(request_json)?;
        http_response(self.engine.export_namespace(namespace_name, &request))
    }

    #[wasm_bindgen(js_name = warmCache)]
    pub fn warm_cache(&self, namespace_name: &str) -> Result<String, JsValue> {
        write_json(
            self.engine
                .warm_cache(namespace_name)
                .map_err(query_error)?,
        )
    }

    #[wasm_bindgen(js_name = warmCacheResponse)]
    pub fn warm_cache_response(&self, namespace_name: &str) -> Result<String, JsValue> {
        http_response(self.engine.warm_cache(namespace_name))
    }

    pub fn recall(&self, namespace_name: &str, request_json: &str) -> Result<String, JsValue> {
        let request = read_request(request_json)?;
        write_json(
            self.engine
                .recall(namespace_name, &request)
                .map_err(query_error)?,
        )
    }

    #[wasm_bindgen(js_name = recallResponse)]
    pub fn recall_response(
        &self,
        namespace_name: &str,
        request_json: &str,
    ) -> Result<String, JsValue> {
        let request = read_request(request_json)?;
        http_response(self.engine.recall(namespace_name, &request))
    }

    #[wasm_bindgen(js_name = explainQuery)]
    pub fn explain_query(
        &self,
        namespace_name: &str,
        request_json: &str,
    ) -> Result<String, JsValue> {
        let request = read_request(request_json)?;
        write_json(
            self.engine
                .explain_query(namespace_name, &request)
                .map_err(query_error)?,
        )
    }

    #[wasm_bindgen(js_name = explainQueryResponse)]
    pub fn explain_query_response(
        &self,
        namespace_name: &str,
        request_json: &str,
    ) -> Result<String, JsValue> {
        let request = read_request(request_json)?;
        http_response(self.engine.explain_query(namespace_name, &request))
    }
}
