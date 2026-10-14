//! The customer's kiosk: rates, quote, confirm, status and the published receipt.
//! Methods of `KioskService` (design §10 S0); the type lives in `service.rs`.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use pos_error::{AppError, AppResult};
use rust_decimal::Decimal;
use xc_kernel::ids::BranchId;
use xc_kernel::money::ForeignAmount;
use xc_kernel::queue_tickets::{QueueTicketSource, QueueTickets};

use crate::domain::kiosk::commands::kiosk_command::KioskCommand;
use crate::domain::kiosk::commands::new_quote::NewQuote;
use crate::domain::kiosk::entities::kiosk_machine::KioskMachine;
use crate::domain::kiosk::entities::payment_gate::PaymentGate;
use crate::domain::kiosk::repositories::kiosk_repository::KioskRepository;
use crate::domain::kiosk::value_objects::audit_line::CUSTOMER_KIOSK;
use crate::domain::kiosk::value_objects::customer_receipt::CustomerReceipt;
use crate::domain::kiosk::value_objects::identified_request::IdentifiedRequest;
use crate::domain::kiosk::value_objects::kiosk_actor::KioskActor;
use crate::domain::kiosk::value_objects::kiosk_currencies::KioskCurrencies;
use crate::domain::kiosk::value_objects::kiosk_error::KioskError;
use crate::domain::kiosk::value_objects::kiosk_service_error::KioskServiceError;
use crate::domain::kiosk::value_objects::kiosk_status::KioskStatus;
use crate::domain::kiosk::value_objects::kiosk_time::KioskTime;
use crate::domain::kiosk::value_objects::public_rates::PublicRates;
use crate::domain::kiosk::value_objects::quote_limit::QuoteLimit;
use crate::domain::kiosk::value_objects::request_identity::RequestIdentity;

use super::service::{
    KioskService, NO_COUNTER, cut_to_millis, line_for, no_such_branch, refuse, time,
};

impl KioskService {
    /// `rates`: what the kiosk offers now.
    pub async fn public_rates(&self, branch: &BranchId) -> AppResult<PublicRates> {
        let view = self
            .ports
            .counter(branch)
            .await?
            .ok_or_else(no_such_branch)?;
        let currencies = KioskCurrencies::new(&self.repo.switches(branch).await?);
        let payment = self.ports.payment_rules(branch).await?;
        Ok(PublicRates {
            branch_id: branch.clone(),
            rates: view.offered(&currencies),
            branch_name: view.branch_name,
            open: view.open,
            payment,
        })
    }

    /// `quote`: stores a new request. Quotes of this branch nobody confirmed for
    /// more than ten minutes are deleted on the way.
    pub async fn quote(
        &self,
        branch: &BranchId,
        cmd: NewQuote,
        now: DateTime<Utc>,
    ) -> AppResult<IdentifiedRequest> {
        self.branch_name(branch).await?;
        let at = cut_to_millis(now);
        let now = time(now);
        let (repo, ports, b) = (self.repo.clone(), self.ports.clone(), branch.clone());
        self.branch_tx
            .run(branch, move |tx| {
                Box::pin(async move {
                    let view = ports.counter_in(tx, &b).await?.ok_or_else(no_such_branch)?;
                    let mut facts = match (cmd.input.currency, cmd.input.side) {
                        (Some(currency), Some(side)) => {
                            let currencies = KioskCurrencies::new(&repo.switches_in(tx, &b).await?);
                            view.facts(&currencies, currency, side)
                        }
                        _ => view.facts_without_rate(),
                    };
                    let mut request = KioskMachine::quote(&cmd.input, &facts, &cmd.new_id, now)
                        .map_err(refuse)?;

                    if let (Some(currency), Some(side)) = (cmd.input.currency, cmd.input.side) {
                        if facts.line.is_some() {
                            let mut settled = false;
                            for _round in 0..4 {
                                let foreign = ForeignAmount::from_decimal(Decimal::new(
                                    request.quote.foreign.minor(),
                                    2,
                                ));
                                match ports.tier_line_in(tx, &b, currency, side, foreign).await? {
                                    None => {
                                        settled = true;
                                        break;
                                    }
                                    Some(tier_line) => {
                                        if facts.line.as_ref().map(|l| l.line_id)
                                            == Some(tier_line.line_id)
                                        {
                                            settled = true;
                                            break;
                                        }
                                        facts.line = Some(tier_line);
                                        request = KioskMachine::quote(
                                            &cmd.input,
                                            &facts,
                                            &cmd.new_id,
                                            now,
                                        )
                                        .map_err(refuse)?;
                                    }
                                }
                            }
                            if !settled {
                                return Err(refuse(KioskError::AmountOrCurrencyInvalid));
                            }
                        }
                    }
                    // v2's last quote check: the method must be on for the direction.
                    let rules = ports.payment_rules_in(tx, &b).await?;
                    PaymentGate::quote(&request, &rules).map_err(refuse)?;

                    // Check staleness under R5
                    if let Some(line_id) = request.quote.line_id {
                        let admitted = ports
                            .check_stale_in(
                                tx,
                                &b,
                                line_id,
                                request.quote.currency,
                                request.quote.side,
                                request.quote.thb,
                                at,
                            )
                            .await?;
                        if !admitted {
                            return Err(refuse(KioskError::AmountOrCurrencyInvalid));
                        }
                    }

                    for old in repo.quoted_in(tx, &b).await? {
                        if KioskMachine::is_purgeable(&old, now) {
                            repo.delete_in(tx, &b, &old.id).await?;
                        }
                    }

                    let since =
                        KioskTime::from_epoch_millis(now.epoch_millis() - QuoteLimit::WINDOW_MS);
                    let recent = repo.quotes_from_in(tx, &b, &cmd.client_ip, since).await?;
                    if QuoteLimit::reached(recent) {
                        return Err(refuse(KioskServiceError::TooManyQuotes));
                    }

                    repo.insert_in(tx, &b, &request, &cmd.client_ip).await?;

                    let quote_qty =
                        ForeignAmount::from_decimal(Decimal::new(request.quote.foreign.minor(), 2));
                    if let Some(quote_id) = ports
                        .quote_in(
                            tx,
                            &b,
                            request.quote.currency,
                            request.quote.side,
                            quote_qty,
                            at,
                        )
                        .await
                        .map_err(|err| match err {
                            pos_error::AppError::Domain(_) => {
                                refuse(KioskError::AmountOrCurrencyInvalid)
                            }
                            other => other,
                        })?
                    {
                        repo.set_quote_id_in(tx, &b, &request.id, &quote_id).await?;
                    }

                    let identity = KioskMachine::identity(Some(&request), now).map_err(refuse)?;
                    Ok(IdentifiedRequest { request, identity })
                })
            })
            .await
    }

    /// `confirm`: the customer accepts the quote; the request joins the queue.
    pub async fn confirm(
        &self,
        branch: &BranchId,
        id: &str,
        now: DateTime<Utc>,
    ) -> AppResult<RequestIdentity> {
        self.confirm_with(branch, id, now, None).await
    }

    /// `confirm` that also issues the request's walk-in ticket through the queue's port, in the
    /// same transaction (design `queue-tickets.md` §4.5, QU-7 #1266): one ticket per request, and
    /// a closed queue refuses the confirm (`queueClosed`) so the request stays QUOTED.
    pub async fn confirm_with_queue(
        &self,
        branch: &BranchId,
        id: &str,
        now: DateTime<Utc>,
        queue: Arc<dyn QueueTickets>,
    ) -> AppResult<RequestIdentity> {
        self.confirm_with(branch, id, now, Some(queue)).await
    }

    async fn confirm_with(
        &self,
        branch: &BranchId,
        id: &str,
        now: DateTime<Utc>,
        queue: Option<Arc<dyn QueueTickets>>,
    ) -> AppResult<RequestIdentity> {
        self.branch_name(branch).await?;
        let at = cut_to_millis(now);
        let now = time(now);
        let (repo, ports, b, id) = (
            self.repo.clone(),
            self.ports.clone(),
            branch.clone(),
            id.to_owned(),
        );
        self.branch_tx
            .run(branch, move |tx| {
                Box::pin(async move {
                    let stored = repo.find_in(tx, &b, &id).await?;
                    let facts = match &stored {
                        Some(request) => {
                            let view =
                                ports.counter_in(tx, &b).await?.ok_or_else(no_such_branch)?;
                            let currencies = KioskCurrencies::new(&repo.switches_in(tx, &b).await?);
                            let mut f =
                                view.facts(&currencies, request.quote.currency, request.quote.side);
                            if f.line.is_some() {
                                let foreign = ForeignAmount::from_decimal(Decimal::new(
                                    request.quote.foreign.minor(),
                                    2,
                                ));
                                if let Some(tier_line) = ports
                                    .tier_line_in(
                                        tx,
                                        &b,
                                        request.quote.currency,
                                        request.quote.side,
                                        foreign,
                                    )
                                    .await?
                                {
                                    f.line = Some(tier_line);
                                }
                            }
                            f
                        }
                        None => NO_COUNTER,
                    };
                    let machine = KioskMachine::next(
                        stored.as_ref(),
                        &KioskCommand::Confirm,
                        &KioskActor::Customer,
                        &facts,
                        now,
                    );
                    let rules = ports.payment_rules_in(tx, &b).await?;
                    let moved =
                        PaymentGate::confirm(machine, stored.as_ref(), &rules).map_err(refuse)?;
                    if let Some(event) = moved.event {
                        repo.update_in(tx, &b, &moved.request).await?;
                        let line = line_for(event, &moved.request, CUSTOMER_KIOSK);
                        ports.record_in(tx, &b, &line, at).await?;
                        if let Some(queue) = &queue
                            && moved.request.status == KioskStatus::Waiting
                        {
                            let source = QueueTicketSource::Kiosk {
                                request_id: id.clone(),
                            };
                            queue
                                .issue_in(tx, &b, &source, &format!("kiosk-confirm:{id}"), at)
                                .await
                                .map_err(queue_refusal)?;
                        }
                    }
                    KioskMachine::identity(Some(&moved.request), now).map_err(refuse)
                })
            })
            .await
    }

    /// `status`: what the customer screen shows for a request.
    pub async fn status(
        &self,
        branch: &BranchId,
        id: &str,
        now: DateTime<Utc>,
    ) -> AppResult<RequestIdentity> {
        self.branch_name(branch).await?;
        let stored = self.repo.find(branch, id).await?;
        KioskMachine::identity(stored.as_ref(), time(now)).map_err(refuse)
    }

    /// `customerReceipt`: the same, with the published picture while it is readable.
    pub async fn customer_receipt(
        &self,
        branch: &BranchId,
        id: &str,
        now: DateTime<Utc>,
    ) -> AppResult<CustomerReceipt> {
        let now = time(now);
        self.branch_name(branch).await?;
        let stored = self.repo.find(branch, id).await?;
        let identity = KioskMachine::identity(stored.as_ref(), now).map_err(refuse)?;
        let image = self
            .repo
            .receipt(branch, id)
            .await?
            .filter(|receipt| KioskMachine::receipt_readable(receipt.published, now))
            .map(|receipt| receipt.image);
        Ok(CustomerReceipt { identity, image })
    }
}

/// A refusal of the queue's port (`queueClosed`, `idempotencyConflict`) as the kiosk's own refusal,
/// so the kiosk answers it as it answers its others (400 with the code), not as a server error.
fn queue_refusal(err: AppError) -> AppError {
    match err {
        AppError::Validation(code) | AppError::Conflict(code) => AppError::Domain(code),
        other => other,
    }
}
