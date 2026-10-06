//! RpcRequest and RpcRefusal: the envelope of both RPC routes.
//!
//! What v2 does with the text of a request to `/api/rpc` or `/api/staff/rpc`
//! before any action runs, and how it words a refusal. No database and no
//! HTTP types: the route layer hands the body in as text and turns an
//! [`RpcResponse`] into its own response.
//!
//! The order of v2's checks, which the dispatcher has to keep:
//!
//! 1. [`RpcRequest::parse`] — the size, the JSON, and on `/api/rpc` whether
//!    there is a name at all;
//! 2. the dispatcher's own check of the name against the caller's role;
//! 3. [`RpcRequest::payload`] — on `/api/rpc` a payload that is not an object
//!    is refused only here, after the role check, as v2 does for an employee.
//!
//! Known differences from v2, all on bodies no screen sends:
//!
//! - a body that is not JSON, or is JSON `null`, is answered
//!   `คำขอไม่ถูกต้อง`; v2 answers the JavaScript engine's English sentence;
//! - what `serde_json` refuses and the JavaScript engine reads is refused the
//!   same way: a `\u` escape of half a surrogate pair, a number too large for
//!   a double, more than 128 levels of nesting;
//! - on the staff route a name written as a fraction is printed as Rust
//!   prints it, and a payload written as text with half a surrogate pair
//!   cannot occur (the text was refused as JSON).

use serde_json::{Map, Value};

/// The longest body either route reads, in UTF-16 units (v2 compares the
/// `length` of a JavaScript string).
pub const MAX_BODY_UTF16_UNITS: usize = 120_000;

const INVALID_REQUEST: &str = "คำขอไม่ถูกต้อง";
const TOO_LARGE_RPC: &str = "ข้อมูลคำขอใหญ่เกินกำหนด";
const TOO_LARGE_STAFF: &str = "ข้อมูลมากเกินไป";
const ROLE_NOT_ALLOWED_RPC: &str = "บทบาทนี้ไม่มีสิทธิ์ทำรายการนี้";
const ROLE_NOT_ALLOWED_STAFF: &str = "บทบาทนี้ไม่มีสิทธิ์ทำรายการ";
const SIGN_IN: &str = "กรุณาเข้าสู่ระบบ";
const EMPLOYEE_AUTH_REQUIRED: &str = "EMPLOYEE_AUTH_REQUIRED";
const CHANGE_FIRST_PASSWORD: &str = "กรุณาเปลี่ยนรหัสผ่านก่อนเริ่มงาน";

/// Which of the two RPC routes a request came in on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RpcRoute {
    /// `POST /api/rpc`: the owner, and an employee through the admin host.
    Rpc,
    /// `POST /api/staff/rpc`: employees only.
    StaffRpc,
}

/// A request body that was read: the action's name and the payload.
#[derive(Debug, Clone, PartialEq)]
pub struct RpcRequest {
    route: RpcRoute,
    action: String,
    /// `None`: `/api/rpc` was sent a payload that is not an object.
    payload: Option<Map<String, Value>>,
}

impl RpcRequest {
    /// Reads the body of a request to `route`.
    ///
    /// `caller_is_staff` matters on `/api/rpc` only, and only for the
    /// sentence: a body without a usable name is a role refusal for an
    /// employee and an invalid request for the owner. The staff route never
    /// refuses a name here; it makes whatever was sent into text, as v2 does,
    /// and leaves the judging to the dispatcher.
    ///
    /// A name that is text is handed on as written on both routes: whether
    /// the caller may use it is the dispatcher's check.
    pub fn parse(
        route: RpcRoute,
        body: &str,
        caller_is_staff: bool,
    ) -> Result<RpcRequest, RpcRefusal> {
        if longer_than_the_limit(body) {
            return Err(RpcRefusal::too_large(route));
        }
        match route {
            RpcRoute::Rpc => parse_rpc(body, caller_is_staff),
            RpcRoute::StaffRpc => parse_staff(body),
        }
    }

    /// The route the request came in on.
    pub fn route(&self) -> RpcRoute {
        self.route
    }

    /// The name of the action asked for, not yet judged against any role.
    pub fn action(&self) -> &str {
        &self.action
    }

    /// The payload, always an object.
    ///
    /// Call it after the check of the name: on `/api/rpc` this is where a
    /// payload that is not an object is refused. On the staff route it never
    /// refuses.
    pub fn payload(&self) -> Result<&Map<String, Value>, RpcRefusal> {
        self.payload
            .as_ref()
            .ok_or_else(RpcRefusal::invalid_request)
    }

    /// The action and the payload, under the same rule as [`Self::payload`].
    pub fn into_parts(self) -> Result<(String, Map<String, Value>), RpcRefusal> {
        match self.payload {
            Some(payload) => Ok((self.action, payload)),
            None => Err(RpcRefusal::invalid_request()),
        }
    }
}

/// `text.length > 120000` of v2. A text of no more bytes than the limit
/// cannot hold more units, so the common case does not count.
fn longer_than_the_limit(body: &str) -> bool {
    body.len() > MAX_BODY_UTF16_UNITS && body.encode_utf16().count() > MAX_BODY_UTF16_UNITS
}

/// `const { action, payload = {} } = JSON.parse(text)` of v2's `/api/rpc`.
fn parse_rpc(body: &str, caller_is_staff: bool) -> Result<RpcRequest, RpcRefusal> {
    let value: Value = serde_json::from_str(body).map_err(|_| RpcRefusal::invalid_request())?;
    let (action, payload) = match value {
        // v2 fails to take `action` out of `null`.
        Value::Null => return Err(RpcRefusal::invalid_request()),
        Value::Object(mut body) => (body.remove("action"), body.remove("payload")),
        // An array, a text, a number: read, and found to hold neither key.
        _ => (None, None),
    };
    // Both of v2's lists hold text only, so anything else is in neither.
    let Some(Value::String(action)) = action else {
        return Err(if caller_is_staff {
            RpcRefusal::role_not_allowed(RpcRoute::Rpc)
        } else {
            RpcRefusal::invalid_request()
        });
    };
    let payload = match payload {
        None => Some(Map::new()),
        Some(Value::Object(payload)) => Some(payload),
        Some(_) => None,
    };
    Ok(RpcRequest {
        route: RpcRoute::Rpc,
        action,
        payload,
    })
}

/// `body = JSON.parse(raw || "{}")`, `String(body.action || "")` and
/// `{ ...(body.payload || {}) }` of v2's staff route.
fn parse_staff(body: &str) -> Result<RpcRequest, RpcRefusal> {
    let text = if body.is_empty() { "{}" } else { body };
    let value: Value = serde_json::from_str(text).map_err(|_| RpcRefusal::invalid_request())?;
    let (action, payload) = match value {
        // v2 fails to read `payload` of `null`.
        Value::Null => return Err(RpcRefusal::invalid_request()),
        Value::Object(mut body) => (body.remove("action"), body.remove("payload")),
        _ => (None, None),
    };
    let action = match action {
        Some(action) if js_truthy(&action) => js_string(&action),
        _ => String::new(),
    };
    let payload = match payload {
        Some(payload) if js_truthy(&payload) => js_spread(payload),
        _ => Map::new(),
    };
    Ok(RpcRequest {
        route: RpcRoute::StaffRpc,
        action,
        payload: Some(payload),
    })
}

/// What JavaScript's `||` keeps: everything but `null`, `false`, zero and
/// the empty text.
fn js_truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(flag) => *flag,
        Value::Number(number) => number.as_f64().is_some_and(|n| n != 0.0),
        Value::String(text) => !text.is_empty(),
        Value::Array(_) | Value::Object(_) => true,
    }
}

/// JavaScript's `String(value)` for what JSON can hold. An array is its
/// members joined by commas, `null` among them printing as nothing — which is
/// how an array of one name becomes that name.
fn js_string(value: &Value) -> String {
    match value {
        Value::Null => "null".to_string(),
        Value::Bool(flag) => flag.to_string(),
        Value::Number(number) => number.to_string(),
        Value::String(text) => text.clone(),
        Value::Array(members) => members
            .iter()
            .map(|member| match member {
                Value::Null => String::new(),
                other => js_string(other),
            })
            .collect::<Vec<_>>()
            .join(","),
        Value::Object(_) => "[object Object]".to_string(),
    }
}

/// JavaScript's `{ ...value }`: an object as it is, an array and a text by
/// position, a number or `true` as nothing.
fn js_spread(value: Value) -> Map<String, Value> {
    match value {
        Value::Object(map) => map,
        Value::Array(members) => members
            .into_iter()
            .enumerate()
            .map(|(at, member)| (at.to_string(), member))
            .collect(),
        Value::String(text) => text
            .encode_utf16()
            .enumerate()
            .map(|(at, unit)| {
                (
                    at.to_string(),
                    Value::String(String::from_utf16_lossy(&[unit])),
                )
            })
            .collect(),
        Value::Null | Value::Bool(_) | Value::Number(_) => Map::new(),
    }
}

/// A request that is answered with a sentence instead of a result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RpcRefusal {
    /// The HTTP status.
    pub status: u16,
    /// The sentence the screens show: the `error` of the body.
    pub sentence: String,
    /// Whether the answer carries `Cache-Control: no-store`.
    pub no_store: bool,
}

impl RpcRefusal {
    /// A refusal by a use case, in the use case's sentence: 400, `no-store`.
    pub fn refused(sentence: impl Into<String>) -> RpcRefusal {
        RpcRefusal {
            status: 400,
            sentence: sentence.into(),
            no_store: true,
        }
    }

    /// `คำขอไม่ถูกต้อง`: the same-origin headers, a body that cannot be read,
    /// a payload that is not an object, a name the owner's route does not
    /// know.
    pub fn invalid_request() -> RpcRefusal {
        RpcRefusal::refused(INVALID_REQUEST)
    }

    /// A body longer than [`MAX_BODY_UTF16_UNITS`]: each route has its own
    /// sentence.
    pub fn too_large(route: RpcRoute) -> RpcRefusal {
        RpcRefusal::refused(match route {
            RpcRoute::Rpc => TOO_LARGE_RPC,
            RpcRoute::StaffRpc => TOO_LARGE_STAFF,
        })
    }

    /// An action the employee's role may not ask for on `route`: each route
    /// has its own sentence.
    pub fn role_not_allowed(route: RpcRoute) -> RpcRefusal {
        RpcRefusal::refused(match route {
            RpcRoute::Rpc => ROLE_NOT_ALLOWED_RPC,
            RpcRoute::StaffRpc => ROLE_NOT_ALLOWED_STAFF,
        })
    }

    /// Nobody the route accepts is signed in: 401. On `/api/rpc` this is the
    /// one answer v2 sends without `Cache-Control`.
    pub fn not_signed_in(route: RpcRoute) -> RpcRefusal {
        match route {
            RpcRoute::Rpc => RpcRefusal {
                status: 401,
                sentence: SIGN_IN.to_string(),
                no_store: false,
            },
            RpcRoute::StaffRpc => RpcRefusal {
                status: 401,
                sentence: EMPLOYEE_AUTH_REQUIRED.to_string(),
                no_store: true,
            },
        }
    }

    /// An employee whose first password is unchanged: not signed in as far
    /// as `/api/rpc` goes, told to change it on the staff route.
    pub fn first_password_unchanged(route: RpcRoute) -> RpcRefusal {
        match route {
            RpcRoute::Rpc => RpcRefusal::not_signed_in(route),
            RpcRoute::StaffRpc => RpcRefusal::refused(CHANGE_FIRST_PASSWORD),
        }
    }

    /// The answer as it goes out: `{"error": sentence}`, letters unescaped
    /// as v2 prints them.
    pub fn response(&self) -> RpcResponse {
        let mut headers = Vec::with_capacity(2);
        if self.no_store {
            headers.push(("cache-control", "no-store"));
        }
        headers.push(("content-type", "application/json"));
        RpcResponse {
            status: self.status,
            headers,
            body: format!("{{\"error\":{}}}", Value::String(self.sentence.clone())),
        }
    }
}

/// An answer of either route, free of any HTTP library's types.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RpcResponse {
    /// The HTTP status.
    pub status: u16,
    /// Header names in lower case.
    pub headers: Vec<(&'static str, &'static str)>,
    /// The JSON text of the body.
    pub body: String,
}
