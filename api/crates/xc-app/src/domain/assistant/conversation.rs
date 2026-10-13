//! Assistant conversations and messages (design §4.1).

use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::domain::assistant::intent::Intent;
use crate::domain::owner::DraftOwner;

/// Role of the speaker in an assistant conversation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MessageRole {
    Customer,
    Assistant,
}

impl MessageRole {
    pub fn as_str(&self) -> &'static str {
        match self {
            MessageRole::Customer => "CUSTOMER",
            MessageRole::Assistant => "ASSISTANT",
        }
    }
}

/// A single message within an assistant conversation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssistantMessage {
    pub id: Uuid,
    pub organization_id: Uuid,
    pub conversation_id: Uuid,
    pub role: MessageRole,
    pub redacted_text: String,
    pub parsed_intent: Option<Intent>,
    pub quote_id: Option<Uuid>,
    pub idempotency_key: Option<String>,
    pub created_at: DateTime<Utc>,
}

/// A conversation thread between a customer/session and the assistant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssistantConversation {
    pub id: Uuid,
    pub organization_id: Uuid,
    pub owner: DraftOwner,
    pub locale: String,
    pub created_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
}
