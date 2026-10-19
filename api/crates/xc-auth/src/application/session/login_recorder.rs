use std::net::IpAddr;

use sqlx::PgPool;

use super::session_service::sha256_hex;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoginResult {
    Ok,
    BadPassword,
    UnknownUser,
    NotActive,
    TempExpired,
    RateLimited,
    Unavailable,
}

impl LoginResult {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "OK",
            Self::BadPassword => "BAD_PASSWORD",
            Self::UnknownUser => "UNKNOWN_USER",
            Self::NotActive => "NOT_ACTIVE",
            Self::TempExpired => "TEMP_EXPIRED",
            Self::RateLimited => "RATE_LIMITED",
            Self::Unavailable => "UNAVAILABLE",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct LoginAttempt<'a> {
    pub identifier_sent: &'a str,
    pub ip: Option<&'a str>,
    pub result: LoginResult,
    pub user_id: Option<&'a str>,
    pub organization_id: Option<&'a str>,
}

pub struct LoginRecorder<'a> {
    pool: &'a PgPool,
}

impl<'a> LoginRecorder<'a> {
    pub fn new(pool: &'a PgPool) -> Self {
        Self { pool }
    }

    pub async fn record(&self, attempt: LoginAttempt<'_>) -> Result<(), sqlx::Error> {
        let identifier_hash = sha256_hex(attempt.identifier_sent);
        let ip = attempt
            .ip
            .and_then(|raw| raw.parse::<IpAddr>().ok())
            .map(|addr| addr.to_string());
        sqlx::query(
            "INSERT INTO login_history (organization_id, user_id, identifier_hash, ip, result) \
             VALUES ($1::uuid, $2::uuid, $3, $4::inet, $5)",
        )
        .bind(attempt.organization_id)
        .bind(attempt.user_id)
        .bind(identifier_hash)
        .bind(ip)
        .bind(attempt.result.as_str())
        .execute(self.pool)
        .await?;
        Ok(())
    }
}
