//! `closureAdd` command (`POST /api/branches/{id}/closures`, design §4.4, §4.5, BK-6 #1388 / #1913).

use std::sync::Arc;

use actix_web::http::{Method, StatusCode};
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_booking::domain::slot::branch_closure::BranchClosure;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;

use super::{BookingOkOutput, BookingServices};
use crate::infrastructure::idempotency::store::IdempotencyKey;
use crate::presentation::http::v3::command::{CommandAction, Locks};
use crate::presentation::http::v3::error::V3Error;

pub struct ClosureAdd {
    services: Arc<BookingServices>,
}

impl ClosureAdd {
    pub fn new(services: Arc<BookingServices>) -> Self {
        Self { services }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "bookings.ts")]
pub struct ClosureAddInput {
    pub id: String,
    #[ts(type = "string")]
    pub closed_on: NaiveDate,
    pub reason: String,
}

impl CommandAction for ClosureAdd {
    const NAME: &'static str = "closureAdd";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "closureAdd");
    type Input = ClosureAddInput;
    type Output = BookingOkOutput;
    const ROUTE: &'static str = "/api/branches/{id}/closures";
    const METHOD: Method = Method::POST;

    fn locks(&self, _branch: &BranchId, input: &Self::Input) -> Locks {
        Locks::One(BranchId::new(&input.id))
    }

    async fn run(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        _branch: &BranchId,
        who: &Actor,
        input: Self::Input,
        _key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        let services = self.services.services()?;
        let user_uuid = Uuid::parse_str(&who.user_id).unwrap_or_else(|_| Uuid::new_v4());

        let branch = BranchId::new(&input.id);
        BranchTx::lock_also(tx, &branch).await.map_err(|e| {
            V3Error::new(
                "serverError",
                None,
                e.to_string(),
                StatusCode::INTERNAL_SERVER_ERROR,
            )
        })?;

        let closure = BranchClosure {
            closed_on: input.closed_on,
            reason: input.reason,
        };

        services
            .slot_service
            .add_closure_in(tx, &branch, &closure, user_uuid)
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

