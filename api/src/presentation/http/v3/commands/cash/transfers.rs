//! `transferCreate`, `transferSubmit`, `transferDispatch`, `transferReceive` and `transferReconcile` (design §4.4,
//! §4.9; G1 bodies on #624). Every one keeps the seam's default `Locks::One(<session branch>)`: dispatch runs at the
//! source, receive at the destination, which `receive_in` checks and refuses with `Forbidden` (`outOfScope`) otherwise
//! (G9 ruling).

use actix_web::http::Method;
use chrono::Utc;
use sqlx::PgPool;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_cash::application::transfer::TransferFlow;
use xc_cash::application::transfer::transfer_flow::TRANSFER_TOTAL_EXCEEDS_COLUMN;
use xc_cash::domain::transfer::cash_transfer::{
    CashTransfer, NewTransfer, NewTransferLine, TransferKind,
};
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, IntBound};

use super::{caller_in, from_cash_error, validation_failed, whole_number};
use crate::infrastructure::idempotency::store::IdempotencyKey;
use crate::presentation::dto::v3::cash::{
    CashRecordInput, CashRecordOutput, TransferCreateInput, TransferReceiveInput,
    TransferReconcileInput,
};
use crate::presentation::http::v3::command::CommandAction;
use crate::presentation::http::v3::error::V3Error;
use crate::presentation::http::v3::input;

fn answer(t: CashTransfer) -> CashRecordOutput {
    CashRecordOutput {
        id: t.id,
        status: t.status.as_str().to_owned(),
    }
}

fn line(
    currency: &str,
    denomination_id: i64,
    field: &'static str,
    qty: &str,
) -> Result<NewTransferLine, V3Error> {
    let q = whole_number(field, qty)?;
    if field == "qty" {
        input::integer("qty", q.into(), IntBound::integer(1, i32::MAX.into()))?;
    }
    Ok(NewTransferLine {
        currency: CurrencyCode::parse(currency)
            .map_err(|_| validation_failed("currency", "unknown currency"))?,
        denomination_id,
        qty: q,
    })
}

pub struct TransferCreate {
    flow: TransferFlow,
}

impl TransferCreate {
    pub fn new(pool: PgPool) -> Self {
        Self {
            flow: TransferFlow::pg(pool),
        }
    }
}

impl CommandAction for TransferCreate {
    const NAME: &'static str = "transferCreate";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "transferCreate");
    type Input = TransferCreateInput;
    type Output = CashRecordOutput;
    const ROUTE: &'static str = "/api/cash-transfers";
    const METHOD: Method = Method::POST;

    async fn run(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        who: &Actor,
        input: Self::Input,
        key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        let kind = TransferKind::parse(&input.kind)
            .ok_or_else(|| validation_failed("kind", "unknown transfer kind"))?;
        let lines = input
            .lines
            .iter()
            .map(|l| line(&l.currency, l.denomination_id, "qty", &l.qty))
            .collect::<Result<Vec<_>, _>>()?;
        let now = Utc::now();
        let user = caller_in(tx, who, now).await?;
        let t = self
            .flow
            .create_in(
                tx,
                &user,
                NewTransfer {
                    kind,
                    from_branch: branch.clone(),
                    to_branch: input.to_branch_id.map(BranchId::new),
                    from_shift: input.from_shift_id,
                    to_shift: input.to_shift_id,
                    lines,
                    note: Some(input.note),
                    idempotency_key: key.as_str().to_owned(),
                },
                now,
            )
            .await
            .map_err(|e| match e {
                pos_error::AppError::Domain(ref m) | pos_error::AppError::Validation(ref m)
                    if m == TRANSFER_TOTAL_EXCEEDS_COLUMN || m.contains("overflows i64") =>
                {
                    validation_failed("lines", m.clone())
                }
                other => from_cash_error(other),
            })?;
        Ok(answer(t))
    }
}

pub struct TransferSubmit {
    flow: TransferFlow,
}

impl TransferSubmit {
    pub fn new(pool: PgPool) -> Self {
        Self {
            flow: TransferFlow::pg(pool),
        }
    }
}

impl CommandAction for TransferSubmit {
    const NAME: &'static str = "transferSubmit";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "transferSubmit");
    type Input = CashRecordInput;
    type Output = CashRecordOutput;
    const ROUTE: &'static str = "/api/cash-transfers/{id}/submit";
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
        let t = self
            .flow
            .submit_in(tx, &user, &input.id, now)
            .await
            .map_err(from_cash_error)?;
        Ok(answer(t))
    }
}

pub struct TransferDispatch {
    flow: TransferFlow,
}

impl TransferDispatch {
    pub fn new(pool: PgPool) -> Self {
        Self {
            flow: TransferFlow::pg(pool),
        }
    }
}

impl CommandAction for TransferDispatch {
    const NAME: &'static str = "transferDispatch";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "transferDispatch");
    type Input = CashRecordInput;
    type Output = CashRecordOutput;
    const ROUTE: &'static str = "/api/cash-transfers/{id}/dispatch";
    const METHOD: Method = Method::POST;

    async fn run(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        who: &Actor,
        input: Self::Input,
        _key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        let now = Utc::now();
        let user = caller_in(tx, who, now).await?;
        let t = self
            .flow
            .dispatch_in(tx, branch, &user, &input.id, now)
            .await
            .map_err(from_cash_error)?;
        Ok(answer(t))
    }
}

pub struct TransferReceive {
    flow: TransferFlow,
}

impl TransferReceive {
    pub fn new(pool: PgPool) -> Self {
        Self {
            flow: TransferFlow::pg(pool),
        }
    }
}

impl CommandAction for TransferReceive {
    const NAME: &'static str = "transferReceive";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "transferReceive");
    type Input = TransferReceiveInput;
    type Output = CashRecordOutput;
    const ROUTE: &'static str = "/api/cash-transfers/{id}/receive";
    const METHOD: Method = Method::POST;

    async fn run(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        who: &Actor,
        input: Self::Input,
        _key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        let counted = input
            .lines
            .iter()
            .map(|l| {
                line(
                    &l.currency,
                    l.denomination_id,
                    "qtyReceived",
                    &l.qty_received,
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        let now = Utc::now();
        let user = caller_in(tx, who, now).await?;
        let t = self
            .flow
            .receive_in(tx, branch, &user, &input.id, counted, now)
            .await
            .map_err(from_cash_error)?;
        Ok(answer(t))
    }
}

pub struct TransferReconcile {
    flow: TransferFlow,
}

impl TransferReconcile {
    pub fn new(pool: PgPool) -> Self {
        Self {
            flow: TransferFlow::pg(pool),
        }
    }
}

impl CommandAction for TransferReconcile {
    const NAME: &'static str = "transferReconcile";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "transferReconcile");
    type Input = TransferReconcileInput;
    type Output = CashRecordOutput;
    const ROUTE: &'static str = "/api/cash-transfers/{id}/reconcile";
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
        let t = self
            .flow
            .reconcile_in(tx, &user, &input.id, &input.note, now)
            .await
            .map_err(from_cash_error)?;
        Ok(answer(t))
    }
}
