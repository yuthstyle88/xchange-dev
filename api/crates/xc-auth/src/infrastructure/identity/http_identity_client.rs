use futures::future::BoxFuture;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue};
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::domain::identity::identity_client::{
    IdentityClient, IdentityError, IdentityLoginResponse, IdentityOtpRequestResponse,
    IdentityOtpVerifyResponse, IdentityRegisterRequest, IdentityRegisterResponse,
};
use crate::infrastructure::identity::identity_settings::IdentitySettings;

/// Identity's server-side create (#1157).
const REGISTER_PATH: &str = "/auth/register";

/// Identity's error code: `{"error":"phone_taken"}` (its `ErrorResponse`), or an object's
/// `code`, in either case style; normalised to snake_case.
fn error_code(v: &Value) -> Option<String> {
    let e = v.get("error")?;
    let raw = e
        .as_str()
        .or_else(|| e.get("code").and_then(Value::as_str))?;
    let mut out = String::with_capacity(raw.len() + 4);
    for ch in raw.chars() {
        if ch.is_ascii_uppercase() {
            out.push('_');
            out.push(ch.to_ascii_lowercase());
        } else {
            out.push(ch);
        }
    }
    Some(out)
}

pub struct HttpIdentityClient {
    client: reqwest::Client,
    settings: IdentitySettings,
    address_header: Option<HeaderName>,
}

impl HttpIdentityClient {
    pub fn new(settings: IdentitySettings, address_header: impl AsRef<str>) -> Self {
        let address_header = HeaderName::from_bytes(address_header.as_ref().as_bytes()).ok();
        Self {
            client: reqwest::Client::new(),
            settings,
            address_header,
        }
    }

    fn headers(&self, client_ip: Option<&str>) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        if let Ok(v) = HeaderValue::from_str(&self.settings.client_id) {
            headers.insert(HeaderName::from_static("x-client-id"), v);
        }
        if let Ok(v) = HeaderValue::from_str(&self.settings.client_secret) {
            headers.insert(HeaderName::from_static("x-client-secret"), v);
        }
        if let Some(ref api_key) = self.settings.api_key {
            if let Ok(v) = HeaderValue::from_str(api_key) {
                headers.insert(HeaderName::from_static("x-api-key"), v);
            }
        }
        if let (Some(ip), Some(header_name)) = (client_ip, &self.address_header) {
            if let Ok(v) = HeaderValue::from_str(ip) {
                headers.insert(header_name.clone(), v);
            }
        }
        headers
    }

    async fn send_request<T: DeserializeOwned>(
        &self,
        path: &str,
        body: Value,
        client_ip: Option<&str>,
    ) -> Result<T, IdentityError> {
        self.send_request_with_bearer(path, body, client_ip, None)
            .await
    }

    async fn send_request_with_bearer<T: DeserializeOwned>(
        &self,
        path: &str,
        body: Value,
        client_ip: Option<&str>,
        bearer: Option<&str>,
    ) -> Result<T, IdentityError> {
        let url = format!("{}{path}", self.settings.base_url.trim_end_matches('/'));
        let mut headers = self.headers(client_ip);
        if let Some(token) = bearer {
            if let Ok(v) = HeaderValue::from_str(&format!("Bearer {token}")) {
                headers.insert(AUTHORIZATION, v);
            }
        }

        let response = match self
            .client
            .post(&url)
            .headers(headers)
            .json(&body)
            .send()
            .await
        {
            Ok(resp) => resp,
            Err(_) => {
                return Err(IdentityError::unavailable(path, 0, "connection_failed"));
            }
        };

        let status = response.status();
        let bytes = match response.bytes().await {
            Ok(b) => b,
            Err(_) => {
                return Err(IdentityError::unavailable(
                    path,
                    status.as_u16(),
                    "read_failed",
                ));
            }
        };

        if status.is_server_error() {
            let code_str = serde_json::from_slice::<Value>(&bytes)
                .ok()
                .and_then(|v| v.get("error").and_then(|e| e.as_str()).map(str::to_string))
                .unwrap_or_else(|| "none".to_string());
            return Err(IdentityError::unavailable(path, status.as_u16(), code_str));
        }

        if status.is_success() {
            match serde_json::from_slice::<T>(&bytes) {
                Ok(parsed) => Ok(parsed),
                Err(_) => Err(IdentityError::unavailable(
                    path,
                    status.as_u16(),
                    "unreadable_body",
                )),
            }
        } else {
            let json_val = serde_json::from_slice::<Value>(&bytes).ok();
            let code = json_val.as_ref().and_then(error_code);

            match code.as_deref() {
                Some("invalid_credentials") => Err(IdentityError::InvalidCredentials),
                Some("account_locked") => Err(IdentityError::AccountLocked),
                Some("rate_limited") | Some("too_many_attempts") => {
                    Err(IdentityError::TooManyAttempts)
                }
                Some("identity_disabled") => Err(IdentityError::IdentityDisabled),
                Some("invalid_code") => Err(IdentityError::InvalidCode),
                Some("challenge_expired")
                | Some("challenge_not_found")
                | Some("invalid_challenge_state") => Err(IdentityError::CodeExpired),
                Some("invalid_refresh_token")
                | Some("refresh_token_expired")
                | Some("refresh_token_reuse")
                | Some("session_inactive")
                | Some("session_not_found") => Err(IdentityError::SignIn),
                // The server-side create (#1157) names which one is taken; every other call keeps
                // the one sentence it had.
                Some("phone_taken") if path == REGISTER_PATH => Err(IdentityError::PhoneTaken),
                Some("username_taken") if path == REGISTER_PATH => {
                    Err(IdentityError::UsernameTaken)
                }
                Some("email_taken") if path == REGISTER_PATH => Err(IdentityError::EmailTaken),
                Some("username_taken") | Some("email_taken") | Some("phone_taken") => {
                    Err(IdentityError::AlreadyRegistered)
                }
                Some("phone_required") => Err(IdentityError::PhoneRequired),
                Some("invalid_phone") | Some("unsupported_country_code") => {
                    Err(IdentityError::InvalidPhone)
                }
                Some("password_policy_violation") => Err(IdentityError::WeakPassword),
                Some("missing_identifier") => Err(IdentityError::InvalidRequest),
                Some(other) => Err(IdentityError::unavailable(path, status.as_u16(), other)),
                None => Err(IdentityError::unavailable(
                    path,
                    status.as_u16(),
                    "bare_or_no_code",
                )),
            }
        }
    }
}

impl IdentityClient for HttpIdentityClient {
    fn login<'a>(
        &'a self,
        identifier: &'a str,
        password: &'a str,
        client_ip: Option<&'a str>,
    ) -> BoxFuture<'a, Result<IdentityLoginResponse, IdentityError>> {
        Box::pin(async move {
            let body = serde_json::json!({
                "identifier": identifier,
                "password": password,
            });
            let resp: IdentityLoginResponse =
                self.send_request("/auth/login", body, client_ip).await?;
            if resp.access_token.is_empty() {
                return Err(IdentityError::unavailable(
                    "/auth/login",
                    200,
                    "missing_access_token",
                ));
            }
            Ok(resp)
        })
    }

    fn refresh<'a>(
        &'a self,
        refresh_token: &'a str,
        client_ip: Option<&'a str>,
    ) -> BoxFuture<'a, Result<IdentityLoginResponse, IdentityError>> {
        Box::pin(async move {
            let body = serde_json::json!({
                "refresh_token": refresh_token,
            });
            let resp: IdentityLoginResponse =
                self.send_request("/auth/refresh", body, client_ip).await?;
            if resp.access_token.is_empty() {
                return Err(IdentityError::unavailable(
                    "/auth/refresh",
                    200,
                    "missing_access_token",
                ));
            }
            Ok(resp)
        })
    }

    fn logout<'a>(
        &'a self,
        refresh_token: &'a str,
        client_ip: Option<&'a str>,
    ) -> BoxFuture<'a, Result<(), IdentityError>> {
        Box::pin(async move {
            let body = serde_json::json!({
                "refresh_token": refresh_token,
            });
            let path = "/auth/logout";
            let url = format!("{}{path}", self.settings.base_url.trim_end_matches('/'));
            let headers = self.headers(client_ip);

            let response = match self
                .client
                .post(&url)
                .headers(headers)
                .json(&body)
                .send()
                .await
            {
                Ok(resp) => resp,
                Err(_) => {
                    return Err(IdentityError::unavailable(path, 0, "connection_failed"));
                }
            };

            if response.status().is_server_error() {
                return Err(IdentityError::unavailable(
                    path,
                    response.status().as_u16(),
                    "server_error",
                ));
            }

            Ok(())
        })
    }

    fn otp_request<'a>(
        &'a self,
        destination: &'a str,
        channel: &'a str,
        client_ip: Option<&'a str>,
    ) -> BoxFuture<'a, Result<IdentityOtpRequestResponse, IdentityError>> {
        Box::pin(async move {
            let body = serde_json::json!({
                "destination": destination,
                "channel": channel,
            });
            self.send_request("/auth/otp/request", body, client_ip)
                .await
        })
    }

    fn otp_verify<'a>(
        &'a self,
        challenge_id: &'a str,
        code: &'a str,
        client_ip: Option<&'a str>,
    ) -> BoxFuture<'a, Result<IdentityOtpVerifyResponse, IdentityError>> {
        Box::pin(async move {
            let body = serde_json::json!({
                "challengeId": challenge_id,
                "code": code,
                "registerIfAbsent": false,
            });
            self.send_request("/auth/otp/verify", body, client_ip).await
        })
    }

    fn register<'a>(
        &'a self,
        request: &'a IdentityRegisterRequest<'a>,
    ) -> BoxFuture<'a, Result<IdentityRegisterResponse, IdentityError>> {
        Box::pin(async move {
            let mut body = serde_json::json!({
                "phone": request.phone,
                "password": request.password,
                "mustChangePassword": request.must_change_password,
            });
            if let Some(username) = request.username {
                body["username"] = serde_json::json!(username);
            }
            if let Some(email) = request.email {
                body["email"] = serde_json::json!(email);
            }
            self.send_request(REGISTER_PATH, body, None).await
        })
    }

    fn change_password<'a>(
        &'a self,
        access_token: &'a str,
        current_password: &'a str,
        new_password: &'a str,
        client_ip: Option<&'a str>,
    ) -> BoxFuture<'a, Result<(), IdentityError>> {
        Box::pin(async move {
            let body = serde_json::json!({
                "currentPassword": current_password,
                "newPassword": new_password,
            });
            let _: Value = self
                .send_request_with_bearer(
                    "/auth/password/change",
                    body,
                    client_ip,
                    Some(access_token),
                )
                .await?;
            Ok(())
        })
    }
}
