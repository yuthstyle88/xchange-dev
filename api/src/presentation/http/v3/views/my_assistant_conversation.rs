//! The view `my_assistant_conversation` (APP-6 #997, design `customer-app.md` §4.4): the caller's
//! assistant conversation and messages.
//!
//! The owner is the signed-in customer, else the `xc_app_sid` session the request carries; a caller
//! with neither has no conversation.

use std::future::Future;
use std::path::Path;
use std::sync::Arc;

use actix_web::web;
use pos_error::{AppError, AppResult};
use ts_rs::TS;
use uuid::Uuid;
use xc_app::application::assistant::assistant_service::AssistantService;
use xc_app::domain::app_error::CustomerAppError;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;

use crate::bootstrap::AppState;
use crate::presentation::dto::v3::assistant::{
    AssistantConversationDto, AssistantMessageDto, MyAssistantConversationData,
    MyAssistantConversationFilter,
};
use crate::presentation::http::v3::caller::Caller;
use crate::presentation::http::v3::planner_draft_save::draft_owner;
use crate::presentation::http::v3::view::{BranchView, ViewEndpoint};

pub struct MyAssistantConversationView {
    service: Arc<AssistantService>,
}

impl BranchView for MyAssistantConversationView {
    const NAME: &'static str = "my_assistant_conversation";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "my_assistant_conversation");
    const VERSIONED: bool = false;
    type Data = MyAssistantConversationData;
    type Filter = MyAssistantConversationFilter;

    fn load(
        &self,
        branch: &BranchId,
        who: &Actor,
        filter: &Self::Filter,
    ) -> impl Future<Output = AppResult<Self::Data>> + Send {
        let caller = Caller {
            user: Some(who.clone()),
            app_session: None,
        };
        let branch = branch.clone();
        let filter = filter.clone();
        async move { self.load_as(&branch, &caller, &filter).await }
    }

    fn load_as(
        &self,
        _branch: &BranchId,
        caller: &Caller,
        filter: &Self::Filter,
    ) -> impl Future<Output = AppResult<Self::Data>> + Send {
        let owner = draft_owner(caller);
        let service = self.service.clone();
        let filter = filter.clone();
        async move {
            let Some(owner) = owner else {
                return Ok(MyAssistantConversationData {
                    conversation: None,
                    messages: Vec::new(),
                });
            };

            if let Some(id_str) = filter.conversation_id.as_deref() {
                let id = Uuid::parse_str(id_str)
                    .map_err(|_| AppError::Validation("invalid conversation_id".to_string()))?;
                match service.get_conversation(&owner, id).await {
                    Ok((conv, msgs)) => Ok(MyAssistantConversationData {
                        conversation: Some(AssistantConversationDto::of(&conv)),
                        messages: msgs.iter().map(AssistantMessageDto::of).collect(),
                    }),
                    Err(CustomerAppError::NotFound) => Ok(MyAssistantConversationData {
                        conversation: None,
                        messages: Vec::new(),
                    }),
                    Err(CustomerAppError::OutOfScope) => Err(AppError::Forbidden),
                    Err(e) => {
                        let why = match e {
                            CustomerAppError::Server(why) => why,
                            other => other.code().to_owned(),
                        };
                        Err(AppError::Internal(format!("my_assistant_conversation: {why}")))
                    }
                }
            } else {
                Ok(MyAssistantConversationData {
                    conversation: None,
                    messages: Vec::new(),
                })
            }
        }
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    ViewEndpoint::register(
        cfg,
        Arc::new(MyAssistantConversationView {
            service: Arc::new(AssistantService::new(state.db.clone())),
        }),
    );
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    MyAssistantConversationData::export_all(&cfg)?;
    MyAssistantConversationFilter::export_all(&cfg)?;
    AssistantConversationDto::export_all(&cfg)?;
    AssistantMessageDto::export_all(&cfg)?;
    Ok(())
}
