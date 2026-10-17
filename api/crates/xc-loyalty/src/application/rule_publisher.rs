//! Loyalty rule version publisher port and types (LY-6 #1411, design §6, §6.4 R4).

use std::str::FromStr;

use futures::future::BoxFuture;
use rust_decimal::Decimal;
use serde_json::{Map, Value};
use uuid::Uuid;

const POINT_ALLOWED_KEYS: &[&str] = &[
    "id",
    "version",
    "basis",
    "pointsRatio",
    "rounding",
    "minimumBasisAmount",
    "expiryMonths",
    "validFrom",
    "validTo",
];

const TIER_ALLOWED_KEYS: &[&str] = &[
    "id",
    "version",
    "rollingMonths",
    "tiers",
    "grace",
    "validFrom",
    "validTo",
];

const NON_CONTENT_KEYS: &[&str] = &["status", "approvedRef", "approvedAt", "createdAt"];

/// Which loyalty rule family a version belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleKind {
    Point,
    Tier,
}

impl RuleKind {
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "POINT" => Some(Self::Point),
            "TIER" => Some(Self::Tier),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Point => "POINT",
            Self::Tier => "TIER",
        }
    }

    pub fn route_segment(&self) -> &'static str {
        match self {
            Self::Point => "point-rule-versions",
            Self::Tier => "tier-rule-versions",
        }
    }
}

/// A rule version as returned by Loyalty.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleVersion {
    pub id: Uuid,
    pub kind: RuleKind,
    pub status: String,
    pub approved_ref: Option<String>,
    pub content: Value,
}

impl RuleVersion {
    pub fn from_dto(kind: RuleKind, dto: Value) -> Result<Self, LoyaltyRuleError> {
        let Some(obj) = dto.as_object() else {
            return Err(LoyaltyRuleError::Invalid(
                "rule version response is not a JSON object".to_owned(),
            ));
        };
        let id = obj
            .get("id")
            .and_then(Value::as_str)
            .and_then(|s| Uuid::parse_str(s).ok())
            .ok_or_else(|| {
                LoyaltyRuleError::Invalid("missing or invalid rule version id".to_owned())
            })?;
        if obj
            .get("version")
            .and_then(Value::as_i64)
            .and_then(|n| i32::try_from(n).ok())
            .is_none()
        {
            return Err(LoyaltyRuleError::Invalid(
                "missing or invalid rule version number".to_owned(),
            ));
        }
        let status = obj
            .get("status")
            .and_then(Value::as_str)
            .ok_or_else(|| LoyaltyRuleError::Invalid("missing rule version status".to_owned()))?
            .to_owned();
        let approved_ref = obj
            .get("approvedRef")
            .and_then(Value::as_str)
            .map(str::to_owned);

        let mut content_map = obj.clone();
        for k in NON_CONTENT_KEYS {
            content_map.remove(*k);
        }

        Ok(Self {
            id,
            kind,
            status,
            approved_ref,
            content: Value::Object(content_map),
        })
    }
}

/// Errors produced by `LoyaltyRuleWriter`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoyaltyRuleError {
    Unavailable(String),
    Invalid(String),
    NotFound,
    InvalidTransition(String),
}

impl std::fmt::Display for LoyaltyRuleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unavailable(m) => write!(f, "loyalty unavailable: {m}"),
            Self::Invalid(m) => write!(f, "invalid loyalty rule: {m}"),
            Self::NotFound => write!(f, "loyalty rule version not found"),
            Self::InvalidTransition(m) => write!(f, "invalid loyalty rule transition: {m}"),
        }
    }
}

impl std::error::Error for LoyaltyRuleError {}

/// Prepares draft fields for sending to Loyalty: keeps only the allowed fields of
/// `CreatePointRuleVersionRequest` / `CreateTierRuleVersionRequest` (never `status`, `current`,
/// or `isCurrent`) and ensures numeric decimal fields are formatted as strings on the wire.
pub fn prepare_draft_fields(kind: RuleKind, fields: &Map<String, Value>) -> Map<String, Value> {
    let allowed = match kind {
        RuleKind::Point => POINT_ALLOWED_KEYS,
        RuleKind::Tier => TIER_ALLOWED_KEYS,
    };
    let mut out = Map::new();
    for (k, v) in fields {
        if !allowed.contains(&k.as_str()) {
            continue;
        }
        let mapped = if kind == RuleKind::Point
            && (k == "pointsRatio" || k == "minimumBasisAmount")
            && let Value::Number(n) = v
            && let Ok(dec) = Decimal::from_str(&n.to_string())
        {
            Value::String(dec.to_string())
        } else {
            v.clone()
        };
        out.insert(k.clone(), mapped);
    }
    out
}

/// Port for creating, submitting, approving, rejecting, and reading rule versions in Loyalty.
pub trait LoyaltyRuleWriter: Send + Sync {
    fn create_draft<'a>(
        &'a self,
        kind: RuleKind,
        fields: &'a Map<String, Value>,
    ) -> BoxFuture<'a, Result<RuleVersion, LoyaltyRuleError>>;

    fn submit<'a>(
        &'a self,
        kind: RuleKind,
        id: Uuid,
    ) -> BoxFuture<'a, Result<RuleVersion, LoyaltyRuleError>>;

    fn approve<'a>(
        &'a self,
        kind: RuleKind,
        id: Uuid,
        approved_ref: &'a str,
    ) -> BoxFuture<'a, Result<RuleVersion, LoyaltyRuleError>>;

    fn reject<'a>(
        &'a self,
        kind: RuleKind,
        id: Uuid,
        reason: &'a str,
    ) -> BoxFuture<'a, Result<RuleVersion, LoyaltyRuleError>>;

    fn get<'a>(
        &'a self,
        kind: RuleKind,
        id: Uuid,
    ) -> BoxFuture<'a, Result<Option<RuleVersion>, LoyaltyRuleError>>;

    fn list<'a>(
        &'a self,
        kind: RuleKind,
    ) -> BoxFuture<'a, Result<Vec<RuleVersion>, LoyaltyRuleError>>;
}
