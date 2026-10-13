//! DTO types for the customer app's assistant routes (APP-6 #997, design `customer-app.md` §4.4):
//! `assistantAsk` and the view `my_assistant_conversation`.

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use xc_app::application::assistant::assistant_service::AssistantAnswer;
use xc_app::domain::assistant::conversation::{AssistantConversation, AssistantMessage, MessageRole};

use crate::presentation::http::v3::view::ViewFilter;

// ------------------------------------------------------------ assistantAsk

#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "assistant_ask.ts")]
pub struct AssistantAskInput {
    #[serde(default)]
    #[ts(optional)]
    pub branch: Option<String>,
    pub text: String,
    #[serde(default)]
    #[ts(optional)]
    pub locale: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "assistant_ask.ts")]
pub struct AssistantAskAnswerDto {
    pub conversation_id: String,
    pub customer_message: AssistantMessageDto,
    pub assistant_reply: AssistantMessageDto,
}

impl AssistantAskAnswerDto {
    pub fn of(answer: &AssistantAnswer) -> Self {
        Self {
            conversation_id: answer.conversation_id.to_string(),
            customer_message: AssistantMessageDto::of(&answer.customer_message),
            assistant_reply: AssistantMessageDto::of(&answer.assistant_reply),
        }
    }
}

// ------------------------------------------------------------ Shared / Conversation

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "my_assistant_conversation.ts")]
pub struct AssistantMessageDto {
    pub id: String,
    pub role: String,
    pub redacted_text: String,
    #[serde(default)]
    #[ts(optional)]
    pub quote_id: Option<String>,
    pub created_at: String,
}

impl AssistantMessageDto {
    pub fn of(msg: &AssistantMessage) -> Self {
        Self {
            id: msg.id.to_string(),
            role: match msg.role {
                MessageRole::Customer => "CUSTOMER".to_string(),
                MessageRole::Assistant => "ASSISTANT".to_string(),
            },
            redacted_text: msg.redacted_text.clone(),
            quote_id: msg.quote_id.map(|q| q.to_string()),
            created_at: msg.created_at.to_rfc3339(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "my_assistant_conversation.ts")]
pub struct AssistantConversationDto {
    pub id: String,
    pub locale: String,
    pub created_at: String,
    #[serde(default)]
    #[ts(optional)]
    pub expires_at: Option<String>,
}

impl AssistantConversationDto {
    pub fn of(conv: &AssistantConversation) -> Self {
        Self {
            id: conv.id.to_string(),
            locale: conv.locale.clone(),
            created_at: conv.created_at.to_rfc3339(),
            expires_at: conv.expires_at.map(|e| e.to_rfc3339()),
        }
    }
}

// ------------------------------------------------------------ my_assistant_conversation

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "my_assistant_conversation.ts")]
pub struct MyAssistantConversationData {
    pub conversation: Option<AssistantConversationDto>,
    pub messages: Vec<AssistantMessageDto>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "my_assistant_conversation.ts")]
pub struct MyAssistantConversationFilter {
    #[serde(default)]
    #[ts(optional)]
    pub conversation_id: Option<String>,
}

impl ViewFilter for MyAssistantConversationFilter {}
