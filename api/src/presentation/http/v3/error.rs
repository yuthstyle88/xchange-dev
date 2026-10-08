//! v3 HTTP refusal errors (§7.3).

use actix_web::http::{StatusCode, header};
use actix_web::{HttpResponse, ResponseError};
use xc_auth::domain::policy::value_objects::policy_error::PolicyError;

/// A v3 refusal response error.
#[derive(Debug, Clone)]
pub struct V3Error {
    pub code: &'static str,
    pub field: Option<&'static str>,
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

impl ResponseError for V3Error {
    fn status_code(&self) -> StatusCode {
        self.status
    }

    fn error_response(&self) -> HttpResponse {
        let body = if let Some(field) = self.field {
            serde_json::json!({
                "error": {
                    "code": self.code,
                    "field": field,
                    "message": self.message,
                }
            })
        } else {
            serde_json::json!({
                "error": {
                    "code": self.code,
                    "message": self.message,
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
        let code = match err {
            PolicyError::SignIn => "signIn",
            PolicyError::OwnerOnly => "ownerOnly",
            PolicyError::EmployeeAuthRequired => "employeeAuthRequired",
            PolicyError::InvalidRequest => "invalidRequest",
            PolicyError::RoleNotAllowed => "roleNotAllowed",
            PolicyError::RoleNotAllowedHere => "roleNotAllowedHere",
            PolicyError::OwnerEnables => "ownerEnables",
            PolicyError::FrontOnly => "frontOnly",
            PolicyError::DocumentsNotReadable => "documentsNotReadable",
            PolicyError::FilesNotReadable => "filesNotReadable",
            PolicyError::UploadNotAllowed => "uploadNotAllowed",
            PolicyError::NotFound => "notFound",
            PolicyError::DeviceStepNotAllowed => "deviceStepNotAllowed",
            PolicyError::KioskOwnerOrRates => "kioskOwnerOrRates",
            PolicyError::KioskStaff => "kioskStaff",
            PolicyError::KioskUnknown => "kioskUnknown",
            PolicyError::SignNotAllowed => "signNotAllowed",
            PolicyError::BackendUsesCommit => "backendUsesCommit",
            PolicyError::EmployeeAccount => "employeeAccount",
            PolicyError::ClockInFirst => "clockInFirst",
            PolicyError::PaymentOwnerOnly => "paymentOwnerOnly",
            PolicyError::DrawerRoleNotAllowed => "drawerRoleNotAllowed",
        };
        let status =
            StatusCode::from_u16(err.status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        Self {
            code,
            field: None,
            message: err.to_string(),
            status,
        }
    }
}
