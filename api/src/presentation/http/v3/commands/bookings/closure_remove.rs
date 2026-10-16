//! `closureRemove` command (`POST /api/branches/{id}/closures/{date}/remove`, design §4.4, §4.5, BK-6 #1388 / #1913).

use std::sync::Arc;

use actix_web::http::{Method, StatusCode};
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;

use super::{BookingOkOutput, BookingServices};
use crate::infrastructure::idempotency::store::IdempotencyKey;
use crate::presentation::http::v3::command::{CommandAction, Locks};
use crate::presentation::http::v3::error::V3Error;

pub struct ClosureRemove {
    services: Arc<BookingServices>,
}

impl ClosureRemove {
    pub fn new(services: Arc<BookingServices>) -> Self {
        Self { services }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "bookings.ts")]
pub struct ClosureRemoveInput {
    pub id: String,
    #[ts(type = "string")]
    pub date: NaiveDate,
}

impl CommandAction for ClosureRemove {
    const NAME: &'static str = "closureRemove";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "closureRemove");
    type Input = ClosureRemoveInput;
    type Output = BookingOkOutput;
    const ROUTE: &'static str = "/api/branches/{id}/closures/{date}/remove";
    const METHOD: Method = Method::POST;

    fn locks(&self, _branch: &BranchId, input: &Self::Input) -> Locks {
        Locks::One(BranchId::new(&input.id))
    }

    async fn run(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        _branch: &BranchId,
        _who: &Actor,
        input: Self::Input,
        _key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        let services = self.services.services()?;

        let branch = BranchId::new(&input.id);
        BranchTx::lock_also(tx, &branch).await.map_err(|e| {
            V3Error::new(
                "serverError",
                None,
                e.to_string(),
                StatusCode::INTERNAL_SERVER_ERROR,
            )
        })?;

        services
            .slot_service
            .remove_closure_in(tx, &branch, input.date)
            .await
            .map_err(|e| {
                V3Error::new(
                    "badRequest",
                    None,
                    e.to_string(),
                    StatusCode::BAD_REQUEST,
                )
            })?;

        Ok(BookingOkOutput { ok: true })
    }
}
