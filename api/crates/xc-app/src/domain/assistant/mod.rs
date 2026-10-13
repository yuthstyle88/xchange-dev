//! Domain model of the customer assistant (design §4.1, §7).

pub mod conversation;
pub mod intent;
pub mod repository;

pub use conversation::{AssistantConversation, AssistantMessage, MessageRole};
pub use intent::{Intent, IntentKind, IntentParser};
pub use repository::AssistantRepository;
