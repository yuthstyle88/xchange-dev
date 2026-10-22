//! Shared by the kiosk web queue's up-front tests (KWQ slice a, design
//! `docs/design/blueprint/kiosk-shared-phone.md` §4): a queue port that issues one ticket per
//! request, answers the same ticket on a repeat, and records every call.
//! Written by the slice controller (claude/mbp-xc-ks). Do not edit, delete or weaken.

use std::collections::BTreeMap;
use std::sync::Mutex;

use chrono::{DateTime, Utc};
use futures::future::BoxFuture;
use pos_error::AppResult;
use xc_kernel::ids::BranchId;
use xc_kernel::queue_tickets::{IssuedTicket, KioskTicketOutcome, QueueTicketSource, QueueTickets};

#[derive(Default)]
struct Calls {
    /// request id → the ticket issued for it.
    tickets: BTreeMap<String, IssuedTicket>,
    /// Every `issue_in` call: (branch, source, idempotency key).
    issues: Vec<(String, QueueTicketSource, String)>,
    /// Every `link_kiosk_customer_in` call: (branch, request id, customer id).
    links: Vec<(String, String, String)>,
    /// Every `cancel_active_for_kiosk_in` call: (branch, request id). Added for slice c.
    cancels: Vec<(String, String)>,
}

/// A queue port that numbers tickets `W001`, `W002`, … in the order requests are first issued.
#[derive(Default)]
pub struct RecordingQueue {
    calls: Mutex<Calls>,
}

impl RecordingQueue {
    pub fn issues(&self) -> Vec<(String, QueueTicketSource, String)> {
        self.calls.lock().expect("lock").issues.clone()
    }

    pub fn links(&self) -> Vec<(String, String, String)> {
        self.calls.lock().expect("lock").links.clone()
    }

    pub fn cancels(&self) -> Vec<(String, String)> {
        self.calls.lock().expect("lock").cancels.clone()
    }

    /// The distinct tickets issued so far.
    pub fn tickets(&self) -> Vec<IssuedTicket> {
        self.calls
            .lock()
            .expect("lock")
            .tickets
            .values()
            .cloned()
            .collect()
    }
}

impl QueueTickets for RecordingQueue {
    fn issue_in<'a>(
        &'a self,
        _tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &'a BranchId,
        source: &'a QueueTicketSource,
        key: &'a str,
        now: DateTime<Utc>,
    ) -> BoxFuture<'a, AppResult<IssuedTicket>> {
        Box::pin(async move {
            let QueueTicketSource::Kiosk { request_id } = source else {
                panic!("the kiosk issues only kiosk tickets, got {source:?}");
            };
            let mut calls = self.calls.lock().expect("lock");
            calls
                .issues
                .push((branch.as_str().to_owned(), source.clone(), key.to_owned()));
            let next = calls.tickets.len() as i32 + 1;
            let ticket = calls
                .tickets
                .entry(request_id.clone())
                .or_insert_with(|| IssuedTicket {
                    id: format!("ticket-{request_id}"),
                    service_date: now.date_naive(),
                    service_type: "WALK_IN".to_owned(),
                    sequence: next,
                    display_number: format!("W{next:03}"),
                    issued_at: now,
                })
                .clone();
            Ok(ticket)
        })
    }

    fn start_for_kiosk_in<'a>(
        &'a self,
        _tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        _branch: &'a BranchId,
        _request_id: &'a str,
        _now: DateTime<Utc>,
    ) -> BoxFuture<'a, AppResult<bool>> {
        unreachable!("the token scan starts no ticket")
    }

    fn cancel_active_for_booking_in<'a>(
        &'a self,
        _tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        _branch: &'a BranchId,
        _booking_id: &'a str,
        _now: DateTime<Utc>,
    ) -> BoxFuture<'a, AppResult<bool>> {
        unreachable!("the token scan cancels no booking ticket")
    }

    fn cancel_active_for_kiosk_in<'a>(
        &'a self,
        _tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &'a BranchId,
        request_id: &'a str,
        _outcome: &'a KioskTicketOutcome,
        _now: DateTime<Utc>,
    ) -> BoxFuture<'a, AppResult<bool>> {
        Box::pin(async move {
            self.calls
                .lock()
                .expect("lock")
                .cancels
                .push((branch.as_str().to_owned(), request_id.to_owned()));
            Ok(true)
        })
    }

    fn link_kiosk_customer_in<'a>(
        &'a self,
        _tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &'a BranchId,
        request_id: &'a str,
        customer_id: &'a str,
    ) -> BoxFuture<'a, AppResult<bool>> {
        Box::pin(async move {
            self.calls.lock().expect("lock").links.push((
                branch.as_str().to_owned(),
                request_id.to_owned(),
                customer_id.to_owned(),
            ));
            Ok(true)
        })
    }
}
