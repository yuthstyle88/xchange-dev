//! The customer's kiosk: rates, quote, confirm, status and the published receipt.
//! Methods of `KioskService` (design §10 S0); the type lives in `service.rs`.

use chrono::{DateTime, Utc};
use pos_error::AppResult;
use rust_decimal::Decimal;
use xc_kernel::ids::BranchId;
use xc_kernel::money::ForeignAmount;

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
                    let facts = match (cmd.input.currency, cmd.input.side) {
                        (Some(currency), Some(side)) => {
                            let currencies = KioskCurrencies::new(&repo.switches_in(tx, &b).await?);
                            view.facts(&currencies, currency, side)
                        }
                        _ => view.facts_without_rate(),
                    };
                    let request = KioskMachine::quote(&cmd.input, &facts, &cmd.new_id, now)
                        .map_err(refuse)?;
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
                            view.facts(&currencies, request.quote.currency, request.quote.side)
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
