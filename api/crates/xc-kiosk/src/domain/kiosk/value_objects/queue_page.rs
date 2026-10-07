use std::fmt;

use crate::domain::kiosk::entities::kiosk_request::KioskRequest;
use crate::domain::kiosk::value_objects::request_identity::RequestIdentity;
use crate::domain::kiosk::value_objects::work_times::WorkTimes;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueItem {
    pub request: KioskRequest,
    pub identity: RequestIdentity,
    pub times: WorkTimes,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueuePage {
    pub page: u32,
    pub has_more: bool,
    pub items: Vec<QueueItem>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueuePageError;

impl fmt::Display for QueuePageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "หน้าข้อมูลไม่ถูกต้อง")
    }
}

impl std::error::Error for QueuePageError {}

impl QueuePage {
    pub fn parse(value: Option<&serde_json::Value>) -> Result<u32, QueuePageError> {
        let val = match value {
            None => return Ok(0),
            Some(v) => v,
        };
        match val {
            serde_json::Value::Null => Ok(0),
            serde_json::Value::Bool(false) => Ok(0),
            serde_json::Value::Bool(true) => Ok(1),
            serde_json::Value::Number(n) => {
                if let Some(f) = n.as_f64() {
                    validate_number(f)
                } else {
                    Err(QueuePageError)
                }
            }
            serde_json::Value::String(s) => {
                let trimmed = s.trim();
                if trimmed.is_empty() {
                    return Ok(0);
                }
                if let Ok(f) = trimmed.parse::<f64>() {
                    validate_number(f)
                } else {
                    Err(QueuePageError)
                }
            }
            _ => Err(QueuePageError),
        }
    }
}

fn validate_number(f: f64) -> Result<u32, QueuePageError> {
    if f.is_finite() && (0.0..=100_000.0).contains(&f) && f.trunc() == f {
        Ok(f as u32)
    } else {
        Err(QueuePageError)
    }
}
