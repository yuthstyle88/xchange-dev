//! v3 HTTP refusal errors (§7.3).

use actix_web::http::{StatusCode, header};
use actix_web::{HttpResponse, ResponseError};
use xc_auth::domain::policy::value_objects::policy_error::PolicyError;

/// A v3 refusal response error.
#[derive(Debug, Clone)]
pub struct V3Error {
    pub code: &'static str,
    pub field: Option<&'static str>,
    pub field_path: Option<String>,
    pub message: String,
    pub status: StatusCode,
}

impl V3Error {
    /// Creates a new `V3Error`.
    pub fn new(
        code: &'static str,
        field: Option<&'static str>,
        message: impl Into<String>,
        status: StatusCode,
    ) -> Self {
        Self {
            code,
            field,
            field_path: None,
            message: message.into(),
            status,
        }
    }

    /// Creates a new `V3Error` with an owned field path (e.g. `lines[0].qty`, `limits.JPY`).
    pub fn with_field_path(
        code: &'static str,
        field_path: impl Into<String>,
        message: impl Into<String>,
        status: StatusCode,
    ) -> Self {
        Self {
            code,
            field: None,
            field_path: Some(field_path.into()),
            message: message.into(),
            status,
        }
    }
}

impl std::fmt::Display for V3Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for V3Error {}

const SERVER_ERROR: &str = "เซิร์ฟเวอร์ขัดข้อง";

impl ResponseError for V3Error {
    fn status_code(&self) -> StatusCode {
        self.status
    }

    fn error_response(&self) -> HttpResponse {
        let message: &str = if self.status.is_server_error() {
            tracing::error!(code = self.code, "{}", self.message);
            SERVER_ERROR
        } else {
            &self.message
        };

        let body = if let Some(field) = self.field_path.as_deref().or(self.field) {
            serde_json::json!({
                "error": {
                    "code": self.code,
                    "field": field,
                    "message": message,
                }
            })
        } else {
            serde_json::json!({
                "error": {
                    "code": self.code,
                    "message": message,
                }
            })
        };

        HttpResponse::build(self.status)
            .insert_header((header::CONTENT_TYPE, "application/json"))
            .insert_header((header::CACHE_CONTROL, "no-store"))
            .json(body)
    }
}

impl From<PolicyError> for V3Error {
    fn from(err: PolicyError) -> Self {
        let code = err.code();
        let status =
            StatusCode::from_u16(err.status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        Self {
            code,
            field: None,
            field_path: None,
            message: err.to_string(),
            status,
        }
    }
}
