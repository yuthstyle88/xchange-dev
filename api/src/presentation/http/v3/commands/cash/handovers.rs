//! `handoverStart`, `handoverAccept` and `handoverReject` (design §4.4, the G3 ruling and design PR 1269). Only the
//! hand-over's receiver decides it (`outOfScope`), and a decided hand-over is `staleVersion` (Q-B). There is no reason
//! field on a reject.

use actix_web::http::{Method, StatusCode};
use chrono::Utc;
use pos_error::AppError;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_cash::application::handover::handover_service::INVALID_RECEIVER;
use xc_cash::application::handover::{HandoverAccepted, HandoverService, StartHandover};
use xc_cash::domain::error::CashError;
use xc_cash::domain::handover::{Handover, HandoverError};
use xc_kernel::ids::BranchId;
use xc_shift::domain::shift::value_objects::shift_error::ShiftError;

use super::{caller_in, count_lines, from_cash_error, shift_lifecycle_hook, validation_failed};
use crate::infrastructure::idempotency::store::IdempotencyKey;
use crate::presentation::dto::v3::cash::{
    CashRecordInput, HandoverRecordOutput, HandoverStartInput,
};
use crate::presentation::http::v3::command::CommandAction;
use crate::presentation::http::v3::error::V3Error;

fn answer(h: Handover) -> HandoverRecordOutput {
    HandoverRecordOutput {
        id: h.id,
        status: h.status.as_str().to_owned(),
    }
}

pub struct HandoverStart {
    handovers: HandoverService,
}

impl HandoverStart {
    pub fn new(pool: PgPool) -> Self {
        let hook = shift_lifecycle_hook(&pool);
        Self {
            handovers: HandoverService::pg(pool).with_shift_lifecycle_hook(hook),
        }
    }
}

impl CommandAction for HandoverStart {
    const NAME: &'static str = "handoverStart";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "handoverStart");
    type Input = HandoverStartInput;
    type Output = HandoverRecordOutput;
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
            .map_err(|e| match e {
                AppError::Validation(ref msg) if msg == INVALID_RECEIVER => {
                    validation_failed("receiverId", msg.clone())
                }
                AppError::Domain(ref msg) if msg == &HandoverError::AlreadyPending.to_string() => {
                    V3Error::new("handoverPending", None, msg.clone(), StatusCode::CONFLICT)
                }
                other => from_cash_error(other),
            })?;
        Ok(answer(h))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HandoverAcceptInput {
    #[serde(default)]
    pub branch: Option<String>,
    pub id: String,
    #[serde(default)]
    pub shift_template_id: Option<String>,
}

pub struct HandoverAccept {
    handovers: HandoverService,
}

impl HandoverAccept {
    pub fn new(pool: PgPool) -> Self {
        let hook = shift_lifecycle_hook(&pool);
        Self {
            handovers: HandoverService::pg(pool).with_shift_lifecycle_hook(hook),
        }
    }
}

impl CommandAction for HandoverAccept {
    const NAME: &'static str = "handoverAccept";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "handoverAccept");
    type Input = HandoverAcceptInput;
    type Output = HandoverRecordOutput;
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
        // §4.9: awaiting FINANCE is an answer, not an error, so its approval request commits with the seam
        match self
            .handovers
            .accept_chosen_or_await_in(
                tx,
                &user,
                &input.id,
                input.shift_template_id.as_deref(),
                now,
            )
            .await
            .map_err(|e| match e {
                AppError::Domain(ref msg) if msg == &ShiftError::OutsideShiftWindow.to_string() => {
                    V3Error::new(
                        "outsideShiftWindow",
                        None,
                        msg.clone(),
                        StatusCode::CONFLICT,
                    )
                }
                AppError::Domain(ref msg) if msg == &ShiftError::ShiftNotChosen.to_string() => {
                    V3Error::new(
                        "shiftNotChosen",
                        None,
                        msg.clone(),
                        StatusCode::UNPROCESSABLE_ENTITY,
                    )
                }
                other => from_cash_error(other),
            })? {
            HandoverAccepted::Accepted(h) => Ok(answer(*h)),
            HandoverAccepted::AwaitingFinance(_) => Ok(HandoverRecordOutput {
                id: input.id,
                status: "AWAITING_FINANCE".to_owned(),
            }),
        }
    }
}

pub struct HandoverReject {
    handovers: HandoverService,
}

impl HandoverReject {
    pub fn new(pool: PgPool) -> Self {
        let hook = shift_lifecycle_hook(&pool);
        Self {
            handovers: HandoverService::pg(pool).with_shift_lifecycle_hook(hook),
        }
    }
}

impl CommandAction for HandoverReject {
    const NAME: &'static str = "handoverReject";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "handoverReject");
    type Input = CashRecordInput;
    type Output = HandoverRecordOutput;
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

pub struct HandoverCancel {
    handovers: HandoverService,
}

impl HandoverCancel {
    pub fn new(pool: PgPool) -> Self {
        let hook = shift_lifecycle_hook(&pool);
        Self {
            handovers: HandoverService::pg(pool).with_shift_lifecycle_hook(hook),
        }
    }
}

impl CommandAction for HandoverCancel {
    const NAME: &'static str = "handoverCancel";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "handoverReject");
    type Input = CashRecordInput;
    type Output = HandoverRecordOutput;
    const ROUTE: &'static str = "/api/handovers/{id}/cancel";
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
            .cancel_in(tx, &user, &input.id, now)
            .await
            .map_err(|e| match e {
                AppError::Domain(ref msg) if msg == &CashError::NotOwnShift.to_string() => {
                    V3Error::new("outOfScope", None, msg.clone(), StatusCode::FORBIDDEN)
                }
                other => from_cash_error(other),
            })?;
        Ok(answer(h))
    }
}
