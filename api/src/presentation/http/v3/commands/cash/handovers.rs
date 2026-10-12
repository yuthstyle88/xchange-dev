//! `handoverStart`, `handoverAccept` and `handoverReject` (design §4.4, the G3 ruling and design PR 1269). Only the
//! hand-over's receiver decides it (`outOfScope`), and a decided hand-over is `staleVersion` (Q-B). There is no reason
//! field on a reject.

use actix_web::http::Method;
use chrono::Utc;
use sqlx::PgPool;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_cash::application::handover::{HandoverService, StartHandover};
use xc_cash::domain::handover::Handover;
use xc_kernel::ids::BranchId;

use super::{caller_in, count_lines, from_cash_error};
use crate::infrastructure::idempotency::store::IdempotencyKey;
use crate::presentation::dto::v3::cash::{CashRecordInput, CashRecordOutput, HandoverStartInput};
use crate::presentation::http::v3::command::CommandAction;
use crate::presentation::http::v3::error::V3Error;

fn answer(h: Handover) -> CashRecordOutput {
    CashRecordOutput {
        id: h.id,
        status: h.status.as_str().to_owned(),
    }
}

pub struct HandoverStart {
    handovers: HandoverService,
}

impl HandoverStart {
    pub fn new(pool: PgPool) -> Self {
        Self {
            handovers: HandoverService::pg(pool),
        }
    }
}

impl CommandAction for HandoverStart {
    const NAME: &'static str = "handoverStart";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "handoverStart");
    type Input = HandoverStartInput;
    type Output = CashRecordOutput;
    const ROUTE: &'static str = "/api/handovers";
    const METHOD: Method = Method::POST;

    async fn run(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        who: &Actor,
        input: Self::Input,
        key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        let lines = count_lines(&input.lines)?;
        let now = Utc::now();
        let user = caller_in(tx, who, now).await?;
        let h = self
            .handovers
            .start_in(
                tx,
                &user,
                StartHandover {
                    branch: branch.clone(),
                    from_shift_id: input.from_shift_id,
                    receiver_id: input.receiver_id,
                    lines,
                    note: input.note,
                    idempotency_key: key.as_str().to_owned(),
                },
                now,
            )
            .await
            .map_err(from_cash_error)?;
        Ok(answer(h))
    }
}

pub struct HandoverAccept {
    handovers: HandoverService,
}

impl HandoverAccept {
    pub fn new(pool: PgPool) -> Self {
        Self {
            handovers: HandoverService::pg(pool),
        }
    }
}

impl CommandAction for HandoverAccept {
    const NAME: &'static str = "handoverAccept";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "handoverAccept");
    type Input = CashRecordInput;
    type Output = CashRecordOutput;
    const ROUTE: &'static str = "/api/handovers/{id}/accept";
    const METHOD: Method = Method::POST;

    async fn run(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        _branch: &BranchId,
        who: &Actor,
        input: Self::Input,
        _key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        let now = Utc::now();
        let user = caller_in(tx, who, now).await?;
        let h = self
            .handovers
            .accept_in(tx, &user, &input.id, now)
            .await
            .map_err(from_cash_error)?;
        Ok(answer(h))
    }
}

pub struct HandoverReject {
    handovers: HandoverService,
}

impl HandoverReject {
    pub fn new(pool: PgPool) -> Self {
        Self {
            handovers: HandoverService::pg(pool),
        }
    }
}

impl CommandAction for HandoverReject {
    const NAME: &'static str = "handoverReject";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "handoverReject");
    type Input = CashRecordInput;
    type Output = CashRecordOutput;
    const ROUTE: &'static str = "/api/handovers/{id}/reject";
    const METHOD: Method = Method::POST;

    async fn run(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        _branch: &BranchId,
        who: &Actor,
        input: Self::Input,
        _key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        let now = Utc::now();
        let user = caller_in(tx, who, now).await?;
        let h = self
            .handovers
            .reject_in(tx, &user, &input.id, now)
            .await
            .map_err(from_cash_error)?;
        Ok(answer(h))
    }
}
