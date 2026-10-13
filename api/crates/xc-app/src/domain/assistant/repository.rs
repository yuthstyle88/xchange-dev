//! The assistant storage port (design §5 `AssistantRepository`).

use chrono::{DateTime, Utc};
use futures::future::BoxFuture;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use crate::domain::app_error::CustomerAppError;
use crate::domain::app_session::SessionHash;
use crate::domain::assistant::conversation::{AssistantConversation, AssistantMessage};
use crate::domain::owner::DraftOwner;

pub trait AssistantRepository: Send + Sync {
    /// Loads a conversation by id; `None` if no such row.
    fn get_conversation_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        id: Uuid,
    ) -> BoxFuture<'a, Result<Option<AssistantConversation>, CustomerAppError>>;

    /// Finds an active (not expired) conversation for `owner`, or `None`.
    fn find_active_conversation_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        organization_id: Uuid,
        owner: &'a DraftOwner,
        now: DateTime<Utc>,
    ) -> BoxFuture<'a, Result<Option<AssistantConversation>, CustomerAppError>>;

    /// Inserts a new conversation.
    fn insert_conversation_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        conversation: &'a AssistantConversation,
    ) -> BoxFuture<'a, Result<(), CustomerAppError>>;

    /// Inserts a new message.
    fn insert_message_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        message: &'a AssistantMessage,
    ) -> BoxFuture<'a, Result<(), CustomerAppError>>;

    /// Finds an existing message with the given idempotency key in `conversation_id`.
    fn find_message_by_idempotency_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        conversation_id: Uuid,
        idempotency_key: &'a str,
    ) -> BoxFuture<'a, Result<Option<AssistantMessage>, CustomerAppError>>;

    /// Lists messages in a conversation in chronological order.
    fn list_messages_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        conversation_id: Uuid,
    ) -> BoxFuture<'a, Result<Vec<AssistantMessage>, CustomerAppError>>;

    /// Sign-in claim: moves all conversations of `session` to `customer_id`. Answers how many moved.
    fn claim_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        session: &'a SessionHash,
        customer_id: &'a str,
        now: DateTime<Utc>,
    ) -> BoxFuture<'a, Result<u64, CustomerAppError>>;
}
