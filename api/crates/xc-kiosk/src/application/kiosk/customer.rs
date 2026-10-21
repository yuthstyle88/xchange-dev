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
use crate::domain::kiosk::entities::kiosk_request::KioskRequest;
use crate::domain::kiosk::entities::payment_gate::PaymentGate;
use crate::domain::kiosk::repositories::kiosk_ports::KioskPorts;
use crate::domain::kiosk::repositories::kiosk_repository::KioskRepository;
use crate::domain::kiosk::value_objects::audit_line::CUSTOMER_KIOSK;
use crate::domain::kiosk::value_objects::cancel_reason::CancelReason;
use crate::domain::kiosk::value_objects::customer_quote::CustomerQuote;
use crate::domain::kiosk::value_objects::customer_receipt::CustomerReceipt;
use crate::domain::kiosk::value_objects::identified_request::IdentifiedRequest;
use crate::domain::kiosk::value_objects::issued_request::IssuedRequest;
use crate::domain::kiosk::value_objects::kiosk_actor::KioskActor;
use crate::domain::kiosk::value_objects::kiosk_currencies::KioskCurrencies;
use crate::domain::kiosk::value_objects::kiosk_error::KioskError;
use crate::domain::kiosk::value_objects::kiosk_service_error::KioskServiceError;
use crate::domain::kiosk::value_objects::kiosk_status::KioskStatus;
use crate::domain::kiosk::value_objects::kiosk_time::KioskTime;
use crate::domain::kiosk::value_objects::public_rates::PublicRates;
use crate::domain::kiosk::value_objects::quote_limit::QuoteLimit;
use crate::domain::kiosk::value_objects::request_identity::RequestIdentity;
use crate::domain::kiosk::value_objects::transition::Transition;

use super::desk::cancel_in;
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
        self.quote_for(branch, cmd, None, now).await
    }

    /// `quote` for a web request (design `kiosk-shared-phone.md` §4.2 D1, D2): the same rules and
    /// the same transaction as `quote`, plus, when `customer` is given, the member
    /// (`kiosk_requests.customer_id`) and the hash of its request token (`cancel_token_hash`).
    /// `customer: None` is exactly `quote` (B1); `quote` delegates here.
    pub async fn quote_for(
        &self,
        branch: &BranchId,
        cmd: NewQuote,
        customer: Option<CustomerQuote>,
        now: DateTime<Utc>,
    ) -> AppResult<IdentifiedRequest> {
        self.branch_name(branch).await?;
        let (this, b) = (self.clone(), branch.clone());
        self.branch_tx
            .run(branch, move |tx| {
                Box::pin(async move { this.quote_for_in(tx, &b, cmd, customer, now).await })
            })
            .await
    }

    /// `quote_for` on the caller's transaction, which already holds `branch`'s lock (the v3
    /// command seam's, design `kiosk-shared-phone.md` §4.3 slice c): the same rules, the same
    /// statements, the same answer. `quote_for` becomes `branch_tx.run(|tx| quote_for_in(tx, …))`,
    /// so the two can never drift (B1).
    pub async fn quote_for_in(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        cmd: NewQuote,
        customer: Option<CustomerQuote>,
        now: DateTime<Utc>,
    ) -> AppResult<IdentifiedRequest> {
        let at = cut_to_millis(now);
        let now = time(now);
        let (repo, ports, b) = (&self.repo, &self.ports, branch);
        let view = ports.counter_in(tx, b).await?.ok_or_else(no_such_branch)?;
        let mut facts = match (cmd.input.currency, cmd.input.side) {
            (Some(currency), Some(side)) => {
                let currencies = KioskCurrencies::new(&repo.switches_in(tx, b).await?);
                view.facts(&currencies, currency, side)
            }
            _ => view.facts_without_rate(),
        };
        let mut request =
            KioskMachine::quote(&cmd.input, &facts, &cmd.new_id, now).map_err(refuse)?;

        if let (Some(currency), Some(side)) = (cmd.input.currency, cmd.input.side) {
            if facts.line.is_some() {
                let mut settled = false;
                for _round in 0..4 {
                    let foreign_minor = request
                        .quote
                        .foreign
                        .try_minor()
                        .map_err(|_| refuse(KioskError::AmountOrCurrencyInvalid))?;
                    let foreign = ForeignAmount::from_decimal(Decimal::new(foreign_minor, 2));
                    match ports.tier_line_in(tx, b, currency, side, foreign).await? {
                        None => {
                            settled = true;
                            break;
                        }
                        Some(tier_line) => {
                            if facts.line.as_ref().map(|l| l.line_id) == Some(tier_line.line_id) {
                                settled = true;
                                break;
                            }
                            facts.line = Some(tier_line);
                            request = KioskMachine::quote(&cmd.input, &facts, &cmd.new_id, now)
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
        let rules = ports.payment_rules_in(tx, b).await?;
        PaymentGate::quote(&request, &rules).map_err(refuse)?;

        // Check staleness under R5
        if let Some(line_id) = request.quote.line_id {
            let admitted = ports
                .check_stale_in(
                    tx,
                    b,
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

        for old in repo.quoted_in(tx, b).await? {
            if KioskMachine::is_purgeable_for(
                &old,
                repo.customer_in(tx, b, &old.id).await?.is_some(),
                now,
            ) {
                repo.delete_in(tx, b, &old.id).await?;
            }
        }

        let since = KioskTime::from_epoch_millis(now.epoch_millis() - QuoteLimit::WINDOW_MS);
        let recent = repo.quotes_from_in(tx, b, &cmd.client_ip, since).await?;
        if QuoteLimit::reached(recent) {
            return Err(refuse(KioskServiceError::TooManyQuotes));
        }

        repo.insert_in(tx, b, &request, &cmd.client_ip).await?;
        if let Some(customer) = &customer {
            repo.set_customer_in(tx, b, &request.id, &customer.customer_id)
                .await?;
            repo.set_cancel_token_in(tx, b, &request.id, &customer.token)
                .await?;
        }

        let quote_minor = request
            .quote
            .foreign
            .try_minor()
            .map_err(|_| refuse(KioskError::AmountOrCurrencyInvalid))?;
        let quote_qty = ForeignAmount::from_decimal(Decimal::new(quote_minor, 2));
        if let Some(quote_id) = ports
            .quote_in(
                tx,
                b,
                request.quote.currency,
                request.quote.side,
                quote_qty,
                at,
            )
            .await
            .map_err(|err| match err {
                pos_error::AppError::Domain(_) => refuse(KioskError::AmountOrCurrencyInvalid),
                other => other,
            })?
        {
            repo.set_quote_id_in(tx, b, &request.id, &quote_id).await?;
        }

        let identity = KioskMachine::identity(Some(&request), now).map_err(refuse)?;
        Ok(IdentifiedRequest { request, identity })
    }

    /// `issueByToken`'s core (§4.2 D5, §4.4 B6–B8): in one transaction of `branch`, the request
    /// whose token is `token`. A `QUOTED` one is confirmed exactly as `confirm` does (WAITING,
    /// one `IssueSource::Kiosk` ticket through `queue`, key `kiosk-confirm:<id>`), and its member
    /// is linked to the ticket (`QueueTickets::link_kiosk_customer_in`). A `WAITING` one answers
    /// the same ticket again (B7), never a second number. Unknown, another branch's, past its
    /// purge window (`KioskMachine::is_purgeable`) or in any other status ⇒ `token_not_found()`,
    /// one answer for all (B8). The cancel token is not minted here: the request token stays the
    /// request's only key.
    pub async fn issue_by_token(
        &self,
        branch: &BranchId,
        token: &str,
        now: DateTime<Utc>,
        queue: Arc<dyn QueueTickets>,
    ) -> AppResult<IssuedRequest> {
        let branch_name = self.branch_name(branch).await?;
        let at = cut_to_millis(now);
        let now = time(now);
        let (repo, ports, b, token) = (
            self.repo.clone(),
            self.ports.clone(),
            branch.clone(),
            token.to_owned(),
        );
        self.branch_tx
            .run(branch, move |tx| {
                Box::pin(async move {
                    let req = find_live_by_token_in(tx, &repo, &b, &token, now).await?;
                    if req.status == KioskStatus::Quoted {
                        // `find_live_by_token_in` already enforced the D4 purge window (`is_purgeable_for`);
                        // `KioskMachine::next(Confirm)` checks the 3s–5m kiosk confirm window against `now`.
                        let confirm_now =
                            KioskTime::from_epoch_millis(req.created.epoch_millis() + 3_000);
                        let moved =
                            confirm_in(tx, &repo, &*ports, &b, Some(&req), at, confirm_now).await?;
                        // As `confirm_with`: a ticket only for a request that is now WAITING.
                        if moved.request.status != KioskStatus::Waiting {
                            return Err(token_not_found());
                        }
                    }
                    let id = req.id;
                    let source = QueueTicketSource::Kiosk {
                        request_id: id.clone(),
                    };
                    let ticket = queue
                        .issue_in(tx, &b, &source, &format!("kiosk-confirm:{id}"), at)
                        .await
                        .map_err(queue_refusal)?;
                    if let Some(customer_id) = repo.customer_in(tx, &b, &id).await? {
                        queue
                            .link_kiosk_customer_in(tx, &b, &id, &customer_id)
                            .await?;
                    }
                    let timezone = repo
                        .branch_timezone_in(tx, &b)
                        .await?
                        .ok_or_else(no_such_branch)?;
                    Ok(IssuedRequest {
                        request_id: id,
                        ticket,
                        branch_name,
                        timezone,
                    })
                })
            })
            .await
    }

    /// `findByToken`'s core (§4.2 D7, B13): the id of this branch's request whose token is
    /// `token`, under the same rule and the same single refusal as `issue_by_token`.
    pub async fn find_by_token(
        &self,
        branch: &BranchId,
        token: &str,
        now: DateTime<Utc>,
    ) -> AppResult<String> {
        self.branch_name(branch).await?;
        let now = time(now);
        let (repo, b, token) = (self.repo.clone(), branch.clone(), token.to_owned());
        self.branch_tx
            .run(branch, move |tx| {
                Box::pin(async move {
                    let req = find_live_by_token_in(tx, &repo, &b, &token, now).await?;
                    Ok(req.id)
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
        Ok(self.confirm_with(branch, id, now, None, None).await?.0)
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
        Ok(self
            .confirm_with(branch, id, now, Some(queue), None)
            .await?
            .0)
    }

    /// `confirm` that also stores the hash of `cancel_token` on the move to `WAITING` and returns
    /// `(identity, true)` on that first confirm, or `(identity, false)` on a repeat (`kiosk-cancel.md` §3.4).
    pub async fn confirm_with_token(
        &self,
        branch: &BranchId,
        id: &str,
        now: DateTime<Utc>,
        queue: Arc<dyn QueueTickets>,
        cancel_token: &str,
    ) -> AppResult<(RequestIdentity, bool)> {
        self.confirm_with(branch, id, now, Some(queue), Some(cancel_token))
            .await
    }

    async fn confirm_with(
        &self,
        branch: &BranchId,
        id: &str,
        now: DateTime<Utc>,
        queue: Option<Arc<dyn QueueTickets>>,
        cancel_token: Option<&str>,
    ) -> AppResult<(RequestIdentity, bool)> {
        self.branch_name(branch).await?;
        let at = cut_to_millis(now);
        let now = time(now);
        let (repo, ports, b, id, cancel_token) = (
            self.repo.clone(),
            self.ports.clone(),
            branch.clone(),
            id.to_owned(),
            cancel_token.map(str::to_owned),
        );
        self.branch_tx
            .run(branch, move |tx| {
                Box::pin(async move {
                    let stored = repo.find_in(tx, &b, &id).await?;
                    let moved =
                        confirm_in(tx, &repo, &*ports, &b, stored.as_ref(), at, now).await?;
                    let mut minted = false;
                    if moved.event.is_some() && moved.request.status == KioskStatus::Waiting {
                        if let Some(token) = &cancel_token {
                            repo.set_cancel_token_in(tx, &b, &id, token).await?;
                            minted = true;
                        }
                        if let Some(queue) = &queue {
                            let source = QueueTicketSource::Kiosk {
                                request_id: id.clone(),
                            };
                            queue
                                .issue_in(tx, &b, &source, &format!("kiosk-confirm:{id}"), at)
                                .await
                                .map_err(queue_refusal)?;
                        }
                    }
                    let identity =
                        KioskMachine::identity(Some(&moved.request), now).map_err(refuse)?;
                    Ok((identity, minted))
                })
            })
            .await
    }

    /// `customerCancel`: the customer cancels their own `WAITING` request at the kiosk using its
    /// `cancel_token` from the same `client_ip` (`kiosk-cancel.md` §3.4).
    pub async fn customer_cancel(
        &self,
        branch: &BranchId,
        id: &str,
        cancel_token: &str,
        client_ip: &str,
        now: DateTime<Utc>,
        queue: Arc<dyn QueueTickets>,
    ) -> AppResult<RequestIdentity> {
        self.branch_name(branch).await?;
        if cancel_token.is_empty() {
            return Err(refuse(KioskError::CustomerCancelRefused));
        }
        let at = cut_to_millis(now);
        let now_kiosk = time(now);
        let (repo, ports, b, id, cancel_token, client_ip) = (
            self.repo.clone(),
            self.ports.clone(),
            branch.clone(),
            id.to_owned(),
            cancel_token.to_owned(),
            client_ip.to_owned(),
        );
        self.branch_tx
            .run(branch, move |tx| {
                Box::pin(async move {
                    let stored = repo.find_in(tx, &b, &id).await?;
                    let Some(req) = &stored else {
                        return Err(refuse(KioskError::CustomerCancelRefused));
                    };
                    if req.status != KioskStatus::Waiting {
                        return Err(refuse(KioskError::CustomerCancelRefused));
                    }
                    let stored_ip = repo.client_ip_in(tx, &b, &id).await?;
                    if stored_ip.as_deref() != Some(client_ip.as_str()) {
                        return Err(refuse(KioskError::CustomerCancelRefused));
                    }
                    if !repo
                        .matches_cancel_token_in(tx, &b, &id, &cancel_token)
                        .await?
                    {
                        return Err(refuse(KioskError::CustomerCancelRefused));
                    }
                    cancel_in(
                        tx,
                        &repo,
                        &*ports,
                        &*queue,
                        &b,
                        stored,
                        &KioskActor::Customer,
                        CancelReason::CustomerCancelled,
                        at,
                        now_kiosk,
                    )
                    .await
                    .map_err(|err| match err {
                        AppError::Domain(_) => refuse(KioskError::CustomerCancelRefused),
                        other => other,
                    })
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

/// The one answer for a request token that does not lead to a request of this branch (§4.2 D5):
/// unknown, another branch's, expired, reissued-away. Answered as `404 notFound`.
pub fn token_not_found() -> AppError {
    AppError::NotFound {
        entity: "kioskRequest",
        id: String::new(),
    }
}

async fn find_live_by_token_in(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    repo: &impl KioskRepository,
    branch: &BranchId,
    token: &str,
    now: KioskTime,
) -> AppResult<KioskRequest> {
    let req = repo
        .find_by_token_in(tx, branch, token)
        .await?
        .ok_or_else(token_not_found)?;
    if KioskMachine::is_purgeable_for(
        &req,
        repo.customer_in(tx, branch, &req.id).await?.is_some(),
        now,
    ) || !matches!(req.status, KioskStatus::Quoted | KioskStatus::Waiting)
    {
        return Err(token_not_found());
    }
    Ok(req)
}

async fn confirm_in(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    repo: &impl KioskRepository,
    ports: &dyn KioskPorts,
    branch: &BranchId,
    stored: Option<&KioskRequest>,
    at: DateTime<Utc>,
    now: KioskTime,
) -> AppResult<Transition> {
    let facts = match stored {
        Some(request) => {
            let view = ports
                .counter_in(tx, branch)
                .await?
                .ok_or_else(no_such_branch)?;
            let currencies = KioskCurrencies::new(&repo.switches_in(tx, branch).await?);
            let mut f = view.facts(&currencies, request.quote.currency, request.quote.side);
            if f.line.is_some() {
                let foreign_minor = request.quote.foreign.try_minor().map_err(|_| {
                    AppError::Internal(format!(
                        "stored quote foreign amount {} overflows hundredths",
                        request.quote.foreign
                    ))
                })?;
                let foreign = ForeignAmount::from_decimal(Decimal::new(foreign_minor, 2));
                if let Some(tier_line) = ports
                    .tier_line_in(
                        tx,
                        branch,
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
        stored,
        &KioskCommand::Confirm,
        &KioskActor::Customer,
        &facts,
        now,
    );
    let rules = ports.payment_rules_in(tx, branch).await?;
    let moved = PaymentGate::confirm(machine, stored, &rules).map_err(refuse)?;
    if let Some(event) = moved.event {
        repo.update_in(tx, branch, &moved.request).await?;
        let line = line_for(event, &moved.request, CUSTOMER_KIOSK);
        ports.record_in(tx, branch, &line, at).await?;
    }
    Ok(moved)
}

/// A refusal of the queue's port (`queueClosed`, `idempotencyConflict`) as the kiosk's own refusal,
/// so the kiosk answers it as it answers its others (400 with the code), not as a server error.
fn queue_refusal(err: AppError) -> AppError {
    match err {
        AppError::Validation(code) | AppError::Conflict(code) => AppError::Domain(code),
        other => other,
    }
}
