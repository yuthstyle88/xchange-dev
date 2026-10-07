//! The documents family: `documents`, the links to the papers of one exchange.
//!
//! v2 (`xchange-online@3ffc5a5`, `server.mjs`) knows the name on
//! `/api/staff/rpc` only — on `/api/rpc` it is not an action for anybody, and
//! `Policy` refuses it there before a family is asked — so this family handles
//! the staff route and no other.
//!
//! What v2's engine does, and this file after it:
//!
//! - it looks the exchange up with `row[0] === input.ref` in the caller's own
//!   branch: `ref` counts only when it is a text, and exactly as sent. No
//!   `ref`, `null`, a number, a list — nothing is found;
//! - nothing found is `ไม่พบเอกสารรายการ` ([`NO_DOCUMENTS`]);
//! - the answer is `{"ref": …, "links": [[label, url], …]}`, `ref` first, the
//!   links in the order passport, payment proof (when there is one), receipt.
//!
//! The lookup itself — the branch's LIVE exchanges only, the labels, the
//! `/files/<key>` urls — is [`DocumentsLookup::find`]. The urls stay the
//! owner's here: the dispatcher turns them to the staff paths, as it does for
//! every answer of the staff route. Who may ask (every role but RATES and
//! SYSTEM) is `Policy`'s, judged before this file is reached.

use pos_error::AppError;
use serde_json::Value;

use crate::application::compat::actions::{ActionFuture, RpcAction, RpcCall};
use crate::application::compat::envelope::RpcRoute;
use crate::application::compat::v2_json::V2Json;
use crate::application::workflows::documents::{Documents, DocumentsLookup, NO_DOCUMENTS};

/// The one name of the family.
const DOCUMENTS: &str = "documents";

/// The documents family: `documents`, the links to the papers of one exchange.
#[derive(Clone)]
pub struct DocumentsAction {
    lookup: DocumentsLookup,
}

impl DocumentsAction {
    /// `lookup` finds the documents of an exchange of a branch (`DocumentsLookup::find`).
    pub fn new(lookup: DocumentsLookup) -> Self {
        Self { lookup }
    }
}

impl RpcAction for DocumentsAction {
    fn handles(&self, route: RpcRoute, action: &str) -> bool {
        route == RpcRoute::StaffRpc && action == DOCUMENTS
    }

    fn run<'a>(&'a self, call: RpcCall<'a>) -> ActionFuture<'a> {
        Box::pin(async move {
            let Some(document_ref) = sent_ref(call.payload.get("ref")) else {
                return Err(AppError::Domain(NO_DOCUMENTS.to_owned()).into());
            };
            let found = self.lookup.find(call.branch, document_ref).await?;
            Ok(answer(&found))
        })
    }
}

/// The `ref` v2's `row[0] === input.ref` can match: a text, as sent. A text
/// holding a NUL names no exchange either (and PostgreSQL would not take it).
fn sent_ref(value: Option<&Value>) -> Option<&str> {
    value
        .and_then(Value::as_str)
        .filter(|text| !text.contains('\0'))
}

/// v2's `{ref, links}` with the keys in the order v2 prints them.
fn answer(found: &Documents) -> V2Json {
    let links = found
        .links
        .iter()
        .map(|link| {
            V2Json::Array(vec![
                V2Json::text(link.label.as_str()),
                V2Json::text(link.url.as_str()),
            ])
        })
        .collect();
    V2Json::object(vec![
        ("ref", V2Json::text(found.document_ref.as_str())),
        ("links", V2Json::Array(links)),
    ])
}
