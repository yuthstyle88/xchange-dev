//! SnapshotBuilder.
//!
//! `adminSnapshot` is the whole of one branch as the v2 screens read it, and
//! `adminDeskConfig` is five values of it. Six write actions answer with the
//! same snapshot minus `operations` and the two member fields
//! ([`write_answer`]). This file loads through the services of each context and
//! through [`SnapshotSources`] for what has no service yet, and assembles with
//! the printing functions of `v2_json`. It holds no SQL and decides nothing:
//! who may ask, and for which branch, is settled before it is called.
//!
//! Known differences from v2, each decided before this file was written:
//!
//! - a transaction carries no `row` and no `rawStatus`, a shift no `row`,
//!   `startRow`, `endRow` (spreadsheet leftovers nothing reads);
//! - a transaction's `status` is always `COMPLETED`: v3 stores an exchange only
//!   once it is saved;
//! - the differences listed at the top of `v2_json` (a request's `hash`, an
//!   event's `details`, names inside custody rows).
//!
//! The reads are separate statements, not one transaction: a write that lands
//! between two of them can show in one list and not yet in another. v2 read the
//! same way, and every screen asks again after its own write.

use std::collections::BTreeMap;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use pos_error::AppResult;
use xc_exchange::application::customer::service::CustomerService;
use xc_kernel::ids::BranchId;
use xc_kernel::money::Side;
use xc_rate::application::branch::service::BranchService;
use xc_rate::application::rate::service::RateService;
use xc_shift::application::custody::service::CustodyService;
use xc_shift::application::shift::service::ShiftService;

use crate::application::compat::snapshot_sources::{
    MemberMark, SnapshotSources, SnapshotTransaction,
};
use crate::application::compat::v2_json::{
    V2Json, branches_json, customers_json, money_json, movements_json, operations_json,
    rate_history_json, rate_json, rates_json, shifts_json, time_json, time_or_empty_json,
};

/// The method v2 wrote before it had `Bank Transfer`, and read back as that.
const OLD_TRANSFER_METHOD: &str = "SCB Transfer";
const TRANSFER_METHOD: &str = "Bank Transfer";

/// Builds the answers of `adminSnapshot` and `adminDeskConfig`.
#[derive(Clone)]
pub struct SnapshotBuilder {
    branches: BranchService,
    rates: RateService,
    customers: CustomerService,
    shifts: ShiftService,
    custody: CustodyService,
    sources: Arc<dyn SnapshotSources>,
}

impl SnapshotBuilder {
    pub fn new(
        branches: BranchService,
        rates: RateService,
        customers: CustomerService,
        shifts: ShiftService,
        custody: CustodyService,
        sources: Arc<dyn SnapshotSources>,
    ) -> Self {
        Self {
            branches,
            rates,
            customers,
            shifts,
            custody,
            sources,
        }
    }

    /// v2's `adminSnapshot` for `branch`: `{version, rateHistory, customers,
    /// branchId, branches, transactions, rates, stock, updated, ready, actor,
    /// syncedAt, sheetUrl, deskUrl, operations}`, in that order.
    ///
    /// `caller` is the username of who asks (printed as `actor`); `now` is the
    /// server clock at the request (printed as `syncedAt`, cut to milliseconds).
    /// A branch that does not exist is refused with `ไม่พบสาขาที่เลือก กรุณารีเฟรช`.
    pub async fn admin_snapshot(
        &self,
        branch: &BranchId,
        caller: &str,
        now: DateTime<Utc>,
    ) -> AppResult<V2Json> {
        let here = self.branches.require(branch).await?;
        let branches = self.branches.list().await?;
        let history = self.rates.history(branch).await?;
        let rates = self.rates.current(branch).await?;
        let customers = self.customers.list_live(branch).await?;
        let members = self.sources.members(branch).await?;
        let transactions = self.sources.transactions(branch).await?;
        let shifts = self.shifts.shifts(branch).await?;
        let movements = self.shifts.movements(branch).await?;
        let operations = self.operations(branch).await?;

        // the newest rate row of the branch, whatever its currency
        let updated = history.first().map(|change| change.updated_at);
        let branch_id = here.id.as_str();

        Ok(V2Json::object(vec![
            ("version", V2Json::int(1)),
            ("rateHistory", rate_history_json(&history)),
            (
                "customers",
                with_members(customers_json(&customers), &members),
            ),
            ("branchId", V2Json::text(branch_id)),
            ("branches", branches_json(&branches)),
            ("transactions", transactions_json(&transactions)),
            ("rates", rates_json(&rates)),
            (
                "stock",
                V2Json::object(vec![
                    ("shifts", shifts_json(&shifts)),
                    ("movements", movements_json(&movements)),
                ]),
            ),
            ("updated", time_or_empty_json(updated)),
            ("ready", V2Json::Bool(true)),
            ("actor", V2Json::text(caller)),
            ("syncedAt", time_json(now)),
            ("sheetUrl", V2Json::text("/#transactions")),
            (
                "deskUrl",
                V2Json::text(format!("/desk?branch={}", uri_component(branch_id))),
            ),
            ("operations", operations),
        ]))
    }

    /// v2's `adminDeskConfig` for `branch`: `{branchId, branchName, rates,
    /// activeShift, adminUrl, operations}` — `rates` and `operations` are the
    /// snapshot's, `activeShift` is the open shift's id or `null`.
    ///
    /// A branch that does not exist is refused with `ไม่พบสาขาที่เลือก กรุณารีเฟรช`.
    pub async fn desk_config(&self, branch: &BranchId) -> AppResult<V2Json> {
        let here = self.branches.require(branch).await?;
        let rates = self.rates.current(branch).await?;
        let active = self.shifts.active(branch).await?;
        let operations = self.operations(branch).await?;
        let branch_id = here.id.as_str();

        Ok(V2Json::object(vec![
            ("branchId", V2Json::text(branch_id)),
            ("branchName", V2Json::text(here.name.as_str())),
            ("rates", rates_json(&rates)),
            (
                "activeShift",
                active.map_or(V2Json::Null, |shift| V2Json::text(shift.id)),
            ),
            (
                "adminUrl",
                V2Json::text(format!("/?branch={}", uri_component(branch_id))),
            ),
            ("operations", operations),
        ]))
    }

    /// v2's `operations`: the custody state and log of the branch, with the
    /// names of its people joined in.
    async fn operations(&self, branch: &BranchId) -> AppResult<V2Json> {
        let state = self.custody.state(branch).await?;
        let events = self.custody.events(branch).await?;
        let people = self.sources.people(branch).await?;
        Ok(operations_json(&state, &events, &people))
    }
}

/// A snapshot as the six write actions answer it (`adminSaveRate`,
/// `adminCreateBranch`, `adminOpenShift`, `adminMoveStock`, `adminCloseShift`,
/// `adminAbandonRequest`): without `operations`, and each customer without
/// `member` and `memberVerifiedAt`. Everything else is kept, in order.
pub fn write_answer(snapshot: &V2Json) -> V2Json {
    let V2Json::Object(pairs) = snapshot else {
        return snapshot.clone();
    };
    V2Json::Object(
        pairs
            .iter()
            .filter(|(key, _)| key != "operations")
            .map(|(key, value)| {
                let value = if key == "customers" {
                    without_member_fields(value)
                } else {
                    value.clone()
                };
                (key.clone(), value)
            })
            .collect(),
    )
}

fn without_member_fields(customers: &V2Json) -> V2Json {
    let V2Json::Array(rows) = customers else {
        return customers.clone();
    };
    V2Json::Array(
        rows.iter()
            .map(|row| match row {
                V2Json::Object(pairs) => V2Json::Object(
                    pairs
                        .iter()
                        .filter(|(key, _)| key != "member" && key != "memberVerifiedAt")
                        .cloned()
                        .collect(),
                ),
                other => other.clone(),
            })
            .collect(),
    )
}

/// `customers_json` prints nobody as a member; this sets `member` and
/// `memberVerifiedAt` on the customers the source names.
fn with_members(customers: V2Json, members: &[MemberMark]) -> V2Json {
    if members.is_empty() {
        return customers;
    }
    let by_id: BTreeMap<&str, &MemberMark> = members
        .iter()
        .map(|mark| (mark.customer_id.as_str(), mark))
        .collect();
    let V2Json::Array(rows) = customers else {
        return customers;
    };
    V2Json::Array(
        rows.into_iter()
            .map(|row| {
                let V2Json::Object(pairs) = row else {
                    return row;
                };
                let mark = pairs.iter().find_map(|(key, value)| match value {
                    V2Json::Text(id) if key == "id" => by_id.get(id.as_str()).copied(),
                    _ => None,
                });
                let Some(mark) = mark else {
                    return V2Json::Object(pairs);
                };
                V2Json::Object(
                    pairs
                        .into_iter()
                        .map(|(key, value)| match key.as_str() {
                            "member" => (key, V2Json::Bool(true)),
                            "memberVerifiedAt" => {
                                (key, mark.verified_at.map_or(V2Json::Null, time_json))
                            }
                            _ => (key, value),
                        })
                        .collect(),
                )
            })
            .collect(),
    )
}

/// v2's `transactions`, in the order given (the source gives the oldest
/// first): `{id, status, created, date, type, currency, amount, rate,
/// referenceRate, fee, bankFee, method, slipUrl, paymentRef, customerId,
/// customer, staff, notes, shiftId}`.
fn transactions_json(transactions: &[SnapshotTransaction]) -> V2Json {
    V2Json::Array(transactions.iter().map(transaction_json).collect())
}

fn transaction_json(tx: &SnapshotTransaction) -> V2Json {
    let side = match tx.side {
        Side::Buy => "BUY",
        Side::Sell => "SELL",
    };
    let method = if tx.method == OLD_TRANSFER_METHOD {
        TRANSFER_METHOD
    } else {
        tx.method.as_str()
    };
    V2Json::object(vec![
        ("id", V2Json::text(tx.document_ref.as_str())),
        ("status", V2Json::text("COMPLETED")),
        ("created", time_json(tx.recorded_at)),
        (
            "date",
            V2Json::text(tx.value_date.format("%Y-%m-%d").to_string()),
        ),
        ("type", V2Json::text(side)),
        ("currency", V2Json::text(tx.currency.as_str())),
        ("amount", money_json(tx.amount)),
        ("rate", rate_json(tx.rate)),
        (
            "referenceRate",
            tx.reference_rate.map_or(V2Json::Null, rate_json),
        ),
        ("fee", money_json(tx.fee)),
        ("bankFee", money_json(tx.bank_fee)),
        ("method", V2Json::text(method)),
        (
            "slipUrl",
            tx.slip_key
                .as_deref()
                .map_or(V2Json::Null, |key| V2Json::text(format!("/files/{key}"))),
        ),
        ("paymentRef", V2Json::text(tx.payment_ref.as_str())),
        ("customerId", V2Json::text(tx.customer_id.as_str())),
        ("customer", V2Json::text(tx.customer_name.as_str())),
        ("staff", V2Json::text(tx.staff.as_str())),
        ("notes", V2Json::text(tx.notes.as_str())),
        (
            "shiftId",
            tx.shift_id.as_deref().map_or(V2Json::Null, V2Json::text),
        ),
    ])
}

/// JavaScript's `encodeURIComponent`: everything but letters, digits and
/// `- _ . ! ~ * ' ( )` becomes `%XX` of its UTF-8 bytes.
fn uri_component(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        let kept = byte.is_ascii_alphanumeric()
            || matches!(
                byte,
                b'-' | b'_' | b'.' | b'!' | b'~' | b'*' | b'\'' | b'(' | b')'
            );
        if kept {
            out.push(char::from(byte));
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}
