use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueFilter {
    Waiting,
    Working,
    Completed,
    All,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueFilterError;

impl fmt::Display for QueueFilterError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ตัวกรองไม่ถูกต้อง")
    }
}

impl std::error::Error for QueueFilterError {}

impl QueueFilter {
    pub fn parse(value: Option<&serde_json::Value>) -> Result<Self, QueueFilterError> {
        let val = match value {
            None => return Ok(QueueFilter::All),
            Some(v) => v,
        };
        match val {
            serde_json::Value::Null => Ok(QueueFilter::All),
            serde_json::Value::Bool(false) => Ok(QueueFilter::All),
            serde_json::Value::Number(n) => {
                if n.as_i64() == Some(0) || n.as_f64() == Some(0.0) {
                    Ok(QueueFilter::All)
                } else {
                    Err(QueueFilterError)
                }
            }
            serde_json::Value::String(s) => match s.as_str() {
                "" | "all" => Ok(QueueFilter::All),
                "waiting" => Ok(QueueFilter::Waiting),
                "working" => Ok(QueueFilter::Working),
                "completed" => Ok(QueueFilter::Completed),
                _ => Err(QueueFilterError),
            },
            _ => Err(QueueFilterError),
        }
    }
}
