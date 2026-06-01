use std::error::Error;
use std::fmt::{Display, Formatter};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueryError {
    message: String,
    status_code: u16,
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

    fn with_status(status_code: u16, message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            status_code,
        }
    }

    pub fn status_code(&self) -> u16 {
        self.status_code
    }
}

impl Display for QueryError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for QueryError {}
