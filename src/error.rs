use serde_json::{Map, Value};
use std::error::Error;
use std::fmt::{Display, Formatter};

#[derive(Debug, Clone, PartialEq)]
pub struct QueryError {
    message: String,
    status_code: u16,
    body_fields: Map<String, Value>,
}

impl QueryError {
    pub(crate) fn new(message: impl Into<String>) -> Self {
        Self::with_status(400, message)
    }

    pub(crate) fn unprocessable(message: impl Into<String>) -> Self {
        Self::with_status(422, message)
    }

    pub(crate) fn not_found(message: impl Into<String>) -> Self {
        Self::with_status(404, message)
    }

    pub(crate) fn schema_type_change(
        attribute: &str,
        current_type: &str,
        incoming_type: &str,
    ) -> Self {
        Self::new(format!(
            "🙅 invalid schema update for attribute '{attribute}': cannot change attribute type from {current_type} to {incoming_type}"
        ))
        .with_body_field("attribute", Value::String(attribute.to_string()))
    }

    fn with_status(status_code: u16, message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            status_code,
            body_fields: Map::new(),
        }
    }

    fn with_body_field(mut self, key: &str, value: Value) -> Self {
        self.body_fields.insert(key.to_string(), value);
        self
    }

    pub fn status_code(&self) -> u16 {
        self.status_code
    }

    pub fn body_fields(&self) -> &Map<String, Value> {
        &self.body_fields
    }
}

impl Display for QueryError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for QueryError {}
