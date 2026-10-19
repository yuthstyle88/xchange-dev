//! `AssistantService` (design §4.3, §5): process customer messages, parse intent,
//! return previews without making commitments, and claim session conversations on sign-in.

use chrono::Duration;
use chrono::{DateTime, Utc};
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

use crate::application::planner::planner_preview::{PlannerEstimate, PlannerPreview};
use crate::domain::app_error::CustomerAppError;
use crate::domain::app_session::SessionHash;
use crate::domain::assistant::conversation::MessageRole;
use crate::domain::assistant::conversation::{AssistantConversation, AssistantMessage};
use crate::domain::assistant::intent::{Intent, IntentKind, IntentParser};
use crate::domain::assistant::repository::AssistantRepository;
use crate::domain::owner::DraftOwner;
use crate::domain::planner::planner_draft::{DraftAmount, DraftPlan};
use crate::infrastructure::assistant_repository::PgAssistantRepository;

/// The answer returned by `AssistantService::ask`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssistantAnswer {
    pub conversation_id: Uuid,
    pub customer_message: AssistantMessage,
    pub assistant_reply: AssistantMessage,
    pub intent: Intent,
    pub preview: Option<PlannerEstimate>,
}

pub struct AssistantService {
    pool: PgPool,
    repo: PgAssistantRepository,
    preview: PlannerPreview,
}

impl AssistantService {
    pub fn new(pool: PgPool) -> Self {
        Self {
            repo: PgAssistantRepository::new(pool.clone()),
            preview: PlannerPreview::new(pool.clone()),
            pool,
        }
    }

    /// Ask the assistant a question or request in an existing transaction.
    #[allow(clippy::too_many_arguments)]
    pub async fn ask_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        organization_id: Uuid,
        owner: &DraftOwner,
        locale: &str,
        text: &str,
        idempotency_key: Option<&str>,
        now: DateTime<Utc>,
    ) -> Result<AssistantAnswer, CustomerAppError> {
        let redacted = xc_kernel::pii::redact_text(text);
        let parsed_intent = IntentParser::parse_in(text, &redacted, locale);

        // Find active conversation or create a new one
        let conversation = match self
            .repo
            .find_active_conversation_in(tx, organization_id, owner, now)
            .await?
        {
            Some(c) => c,
            None => {
                // Check retention policy for ASSISTANT_CONVERSATION
                let period_days: Option<i32> = sqlx::query_scalar(
                    "SELECT period_days FROM retention_policies \
                     WHERE organization_id = $1 AND data_type = 'ASSISTANT_CONVERSATION' \
                       AND period_days IS NOT NULL",
                )
                .bind(organization_id)
                .fetch_optional(&mut **tx)
                .await?;

                let expires_at = period_days.map(|d| now + Duration::days(d as i64));
                let new_conv = AssistantConversation {
                    id: Uuid::new_v4(),
                    organization_id,
                    owner: owner.clone(),
                    locale: locale.to_string(),
                    created_at: now,
                    expires_at,
                };
                self.repo.insert_conversation_in(tx, &new_conv).await?;
                new_conv
            }
        };

        // Idempotency check
        if let Some(key) = idempotency_key {
            if let Some(existing_customer_msg) = self
                .repo
                .find_message_by_idempotency_in(tx, conversation.id, key)
                .await?
            {
                // Find corresponding assistant reply
                let reply_messages = self.repo.list_messages_in(tx, conversation.id).await?;
                let assistant_reply = reply_messages.into_iter().find(|m| {
                    m.role == MessageRole::Assistant
                        && m.created_at >= existing_customer_msg.created_at
                });

                if let Some(reply) = assistant_reply {
                    let intent = existing_customer_msg
                        .parsed_intent
                        .clone()
                        .unwrap_or(parsed_intent);
                    return Ok(AssistantAnswer {
                        conversation_id: conversation.id,
                        customer_message: existing_customer_msg,
                        assistant_reply: reply,
                        intent,
                        preview: None,
                    });
                }
            }
        }

        // Create and insert customer message
        let customer_message = AssistantMessage {
            id: Uuid::new_v4(),
            organization_id,
            conversation_id: conversation.id,
            role: MessageRole::Customer,
            redacted_text: redacted,
            parsed_intent: Some(parsed_intent.clone()),
            quote_id: None,
            idempotency_key: idempotency_key.map(|k| k.to_string()),
            created_at: now,
        };
        self.repo.insert_message_in(tx, &customer_message).await?;

        // Create and insert assistant reply
        let reply_text = match locale {
            "th" => "สวัสดีครับ มีอะไรให้ผู้ช่วย XCHANGE ดูแลคุณในวันนี้ได้บ้างครับ",
            "zh" => "您好！我是 XCHANGE 助手，请问有什么可以帮助您的？",
            _ => "Hello! How can I help you today with your currency exchange?",
        };
        let assistant_reply = AssistantMessage {
            id: Uuid::new_v4(),
            organization_id,
            conversation_id: conversation.id,
            role: MessageRole::Assistant,
            redacted_text: reply_text.to_string(),
            parsed_intent: None,
            quote_id: None,
            idempotency_key: None,
            created_at: now + Duration::milliseconds(1),
        };
        self.repo.insert_message_in(tx, &assistant_reply).await?;

        let preview = if parsed_intent.kind == IntentKind::Quote {
            if let (Some(currency), Some(customer_side), Some(quantity)) = (
                parsed_intent.currency,
                parsed_intent.customer_side,
                parsed_intent.quantity,
            ) {
                let plan = DraftPlan {
                    branch_id: parsed_intent.branch.clone(),
                    currency,
                    customer_side,
                    amount: DraftAmount::Quantity(quantity),
                };
                self.preview.estimate(tx, &plan, now).await.ok()
            } else {
                None
            }
        } else {
            None
        };

        Ok(AssistantAnswer {
            conversation_id: conversation.id,
            customer_message,
            assistant_reply,
            intent: parsed_intent,
            preview,
        })
    }

    /// Ask the assistant a question or request (manages its own transaction).
    #[allow(clippy::too_many_arguments)]
    pub async fn ask(
        &self,
        organization_id: Uuid,
        owner: &DraftOwner,
        locale: &str,
        text: &str,
        idempotency_key: Option<&str>,
        now: DateTime<Utc>,
    ) -> Result<AssistantAnswer, CustomerAppError> {
        let mut tx = self.pool.begin().await?;
        let answer = self
            .ask_in(
                &mut tx,
                organization_id,
                owner,
                locale,
                text,
                idempotency_key,
                now,
            )
            .await?;
        tx.commit().await?;
        Ok(answer)
    }

    /// Read a conversation and its messages: another owner → `outOfScope`, none → `notFound`.
    pub async fn get_conversation(
        &self,
        owner: &DraftOwner,
        id: Uuid,
    ) -> Result<(AssistantConversation, Vec<AssistantMessage>), CustomerAppError> {
        let mut tx = self.pool.begin().await?;
        let conv = self
            .repo
            .get_conversation_in(&mut tx, id)
            .await?
            .ok_or(CustomerAppError::NotFound)?;

        if conv.owner != *owner {
            return Err(CustomerAppError::OutOfScope);
        }

        let messages = self.repo.list_messages_in(&mut tx, conv.id).await?;
        tx.commit().await?;

        Ok((conv, messages))
    }

    /// Sign-in (P6): every conversation of `session` becomes `customer_id`'s in one statement.
    /// Answers how many moved; a second call answers 0.
    pub async fn claim_session_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        session: &SessionHash,
        customer_id: &str,
        now: DateTime<Utc>,
    ) -> Result<u64, CustomerAppError> {
        self.repo.claim_in(tx, session, customer_id, now).await
    }
}
