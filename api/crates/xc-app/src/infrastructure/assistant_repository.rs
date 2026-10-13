//! PostgreSQL implementation of `AssistantRepository`.

use chrono::{DateTime, Utc};
use futures::future::BoxFuture;
use sqlx::postgres::PgRow;
use sqlx::{PgPool, Postgres, Row, Transaction};
use uuid::Uuid;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, CustomerSide, ForeignAmount, Money};

use crate::domain::app_error::CustomerAppError;
use crate::domain::app_session::SessionHash;
use crate::domain::assistant::conversation::{
    AssistantConversation, AssistantMessage, MessageRole,
};
use crate::domain::assistant::intent::{Intent, IntentKind};
use crate::domain::assistant::repository::AssistantRepository;
use crate::domain::owner::DraftOwner;

#[derive(Clone)]
pub struct PgAssistantRepository {
    #[allow(dead_code)]
    pool: PgPool,
}

impl PgAssistantRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

fn corrupt(what: &str) -> CustomerAppError {
    CustomerAppError::Server(format!("assistant storage: {what}"))
}

fn conversation_of(row: &PgRow) -> Result<AssistantConversation, CustomerAppError> {
    let customer_id: Option<String> = row.try_get("customer_id")?;
    let session_hash: Option<String> = row.try_get("session_hash")?;
    let owner = match (customer_id, session_hash) {
        (Some(c), None) => DraftOwner::Customer(c),
        (None, Some(s)) => DraftOwner::Session(SessionHash::from_stored(s)),
        _ => return Err(corrupt("not exactly one conversation owner")),
    };
    Ok(AssistantConversation {
        id: row.try_get("id")?,
        organization_id: row.try_get("organization_id")?,
        owner,
        locale: row.try_get("locale")?,
        created_at: row.try_get("created_at")?,
        expires_at: row.try_get("expires_at")?,
    })
}

fn intent_to_json(intent: &Intent) -> serde_json::Value {
    serde_json::json!({
        "kind": match intent.kind {
            IntentKind::Quote => "QUOTE",
            IntentKind::Queue => "QUEUE",
            IntentKind::Alert => "ALERT",
            IntentKind::Branch => "BRANCH",
            IntentKind::Unknown => "UNKNOWN",
        },
        "currency": intent.currency.as_ref().map(|c| c.as_str()),
        "customerSide": intent.customer_side.as_ref().map(|s| s.as_str()),
        "quantity": intent.quantity.as_ref().map(|q| q.to_string()),
        "budgetThb": intent.budget_thb.as_ref().map(|b| b.to_string()),
        "branch": intent.branch.as_ref().map(|b| b.as_str()),
    })
}

fn json_to_intent(val: &serde_json::Value) -> Option<Intent> {
    let kind_str = val.get("kind")?.as_str()?;
    let kind = match kind_str {
        "QUOTE" => IntentKind::Quote,
        "QUEUE" => IntentKind::Queue,
        "ALERT" => IntentKind::Alert,
        "BRANCH" => IntentKind::Branch,
        _ => IntentKind::Unknown,
    };
    let currency = val
        .get("currency")
        .and_then(|v| v.as_str())
        .and_then(|s| CurrencyCode::parse(s).ok());
    let customer_side = val
        .get("customerSide")
        .and_then(|v| v.as_str())
        .and_then(CustomerSide::parse);
    let quantity = val
        .get("quantity")
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse::<rust_decimal::Decimal>().ok())
        .map(ForeignAmount::from_decimal);
    let budget_thb = val
        .get("budgetThb")
        .and_then(|v| v.as_str())
        .and_then(|s| Money::parse(s).ok());
    let branch = val
        .get("branch")
        .and_then(|v| v.as_str())
        .map(BranchId::new);

    Some(Intent {
        kind,
        currency,
        customer_side,
        quantity,
        budget_thb,
        branch,
    })
}

fn message_of(row: &PgRow) -> Result<AssistantMessage, CustomerAppError> {
    let role_str: String = row.try_get("role")?;
    let role = match role_str.as_str() {
        "CUSTOMER" => MessageRole::Customer,
        "ASSISTANT" => MessageRole::Assistant,
        _ => return Err(corrupt("unknown message role")),
    };
    let parsed_val: Option<serde_json::Value> = row.try_get("parsed_intent")?;
    let parsed_intent = parsed_val.as_ref().and_then(json_to_intent);

    Ok(AssistantMessage {
        id: row.try_get("id")?,
        organization_id: row.try_get("organization_id")?,
        conversation_id: row.try_get("conversation_id")?,
        role,
        redacted_text: row.try_get("redacted_text")?,
        parsed_intent,
        quote_id: row.try_get("quote_id")?,
        idempotency_key: row.try_get("idempotency_key")?,
        created_at: row.try_get("created_at")?,
    })
}

impl AssistantRepository for PgAssistantRepository {
    fn get_conversation_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        id: Uuid,
    ) -> BoxFuture<'a, Result<Option<AssistantConversation>, CustomerAppError>> {
        Box::pin(async move {
            let row = sqlx::query(
                "SELECT id, organization_id, customer_id, session_hash, locale, created_at, expires_at \
                 FROM assistant_conversations WHERE id = $1",
            )
            .bind(id)
            .fetch_optional(&mut **tx)
            .await?;

            row.as_ref().map(conversation_of).transpose()
        })
    }

    fn find_active_conversation_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        organization_id: Uuid,
        owner: &'a DraftOwner,
        now: DateTime<Utc>,
    ) -> BoxFuture<'a, Result<Option<AssistantConversation>, CustomerAppError>> {
        Box::pin(async move {
            let row = match owner {
                DraftOwner::Customer(customer_id) => {
                    sqlx::query(
                        "SELECT id, organization_id, customer_id, session_hash, locale, created_at, expires_at \
                         FROM assistant_conversations \
                         WHERE organization_id = $1 \
                           AND customer_id = $2 \
                           AND (expires_at IS NULL OR expires_at > $3) \
                         ORDER BY created_at DESC \
                         LIMIT 1",
                    )
                    .bind(organization_id)
                    .bind(customer_id)
                    .bind(now)
                    .fetch_optional(&mut **tx)
                    .await?
                }
                DraftOwner::Session(session_hash) => {
                    sqlx::query(
                        "SELECT id, organization_id, customer_id, session_hash, locale, created_at, expires_at \
                         FROM assistant_conversations \
                         WHERE organization_id = $1 \
                           AND session_hash = $2 \
                           AND (expires_at IS NULL OR expires_at > $3) \
                         ORDER BY created_at DESC \
                         LIMIT 1",
                    )
                    .bind(organization_id)
                    .bind(session_hash.as_str())
                    .bind(now)
                    .fetch_optional(&mut **tx)
                    .await?
                }
            };

            row.as_ref().map(conversation_of).transpose()
        })
    }

    fn insert_conversation_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        conversation: &'a AssistantConversation,
    ) -> BoxFuture<'a, Result<(), CustomerAppError>> {
        Box::pin(async move {
            let (customer_id, session_hash) = match &conversation.owner {
                DraftOwner::Customer(c) => (Some(c.as_str()), None),
                DraftOwner::Session(s) => (None, Some(s.as_str())),
            };

            sqlx::query(
                "INSERT INTO assistant_conversations (id, organization_id, customer_id, session_hash, locale, created_at, expires_at) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7)",
            )
            .bind(conversation.id)
            .bind(conversation.organization_id)
            .bind(customer_id)
            .bind(session_hash)
            .bind(&conversation.locale)
            .bind(conversation.created_at)
            .bind(conversation.expires_at)
            .execute(&mut **tx)
            .await?;

            Ok(())
        })
    }

    fn insert_message_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        message: &'a AssistantMessage,
    ) -> BoxFuture<'a, Result<(), CustomerAppError>> {
        Box::pin(async move {
            let parsed_json = message.parsed_intent.as_ref().map(intent_to_json);

            sqlx::query(
                "INSERT INTO assistant_messages (id, organization_id, conversation_id, role, redacted_text, parsed_intent, quote_id, idempotency_key, created_at) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
            )
            .bind(message.id)
            .bind(message.organization_id)
            .bind(message.conversation_id)
            .bind(message.role.as_str())
            .bind(&message.redacted_text)
            .bind(parsed_json)
            .bind(message.quote_id)
            .bind(&message.idempotency_key)
            .bind(message.created_at)
            .execute(&mut **tx)
            .await?;

            Ok(())
        })
    }

    fn find_message_by_idempotency_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        conversation_id: Uuid,
        idempotency_key: &'a str,
    ) -> BoxFuture<'a, Result<Option<AssistantMessage>, CustomerAppError>> {
        Box::pin(async move {
            let row = sqlx::query(
                "SELECT id, organization_id, conversation_id, role, redacted_text, parsed_intent, quote_id, idempotency_key, created_at \
                 FROM assistant_messages \
                 WHERE conversation_id = $1 AND idempotency_key = $2",
            )
            .bind(conversation_id)
            .bind(idempotency_key)
            .fetch_optional(&mut **tx)
            .await?;

            row.as_ref().map(message_of).transpose()
        })
    }

    fn list_messages_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        conversation_id: Uuid,
    ) -> BoxFuture<'a, Result<Vec<AssistantMessage>, CustomerAppError>> {
        Box::pin(async move {
            let rows = sqlx::query(
                "SELECT id, organization_id, conversation_id, role, redacted_text, parsed_intent, quote_id, idempotency_key, created_at \
                 FROM assistant_messages \
                 WHERE conversation_id = $1 \
                 ORDER BY created_at ASC, (CASE role WHEN 'CUSTOMER' THEN 0 ELSE 1 END) ASC",
            )
            .bind(conversation_id)
            .fetch_all(&mut **tx)
            .await?;

            rows.iter().map(message_of).collect()
        })
    }

    fn claim_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        session: &'a SessionHash,
        customer_id: &'a str,
        _now: DateTime<Utc>,
    ) -> BoxFuture<'a, Result<u64, CustomerAppError>> {
        Box::pin(async move {
            let res = sqlx::query(
                "UPDATE assistant_conversations \
                 SET customer_id = $1, session_hash = NULL \
                 WHERE session_hash = $2",
            )
            .bind(customer_id)
            .bind(session.as_str())
            .execute(&mut **tx)
            .await?;

            Ok(res.rows_affected())
        })
    }
}
