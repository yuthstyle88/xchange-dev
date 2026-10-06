//! Kiosk service.
//!
//! Every method names the branch it works on and checks that the branch exists
//! before anything else it reads. Who may call what is `KioskMachine::authorize`,
//! asked first of all. A refusal is `AppError::Domain(<the sentence>)`. Every
//! write runs inside `BranchTx::run` of that branch; `check_save_in` and
//! `complete_in` run inside the transaction of the save that calls them.

use std::fmt::Display;
use std::sync::Arc;

use chrono::{DateTime, TimeZone, Utc};
use pos_error::{AppError, AppResult};
use rust_decimal::Decimal;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xc_kernel::money::CurrencyCode;

use crate::domain::kiosk::commands::configure_currency::ConfigureCurrency;
use crate::domain::kiosk::commands::kiosk_command::KioskCommand;
use crate::domain::kiosk::commands::new_quote::NewQuote;
use crate::domain::kiosk::commands::save_counter_rate::SaveCounterRate;
use crate::domain::kiosk::entities::kiosk_machine::KioskMachine;
use crate::domain::kiosk::entities::kiosk_request::KioskRequest;
use crate::domain::kiosk::repositories::kiosk_ports::KioskPorts;
use crate::domain::kiosk::repositories::kiosk_repository::KioskRepository;
use crate::domain::kiosk::value_objects::audit_line::{AuditLine, CUSTOMER_KIOSK, KioskAudit};
use crate::domain::kiosk::value_objects::counter_facts::CounterFacts;
use crate::domain::kiosk::value_objects::currency_switch::CurrencySwitch;
use crate::domain::kiosk::value_objects::customer_receipt::CustomerReceipt;
use crate::domain::kiosk::value_objects::identified_request::IdentifiedRequest;
use crate::domain::kiosk::value_objects::kiosk_action::KioskAction;
use crate::domain::kiosk::value_objects::kiosk_actor::KioskActor;
use crate::domain::kiosk::value_objects::kiosk_currencies::KioskCurrencies;
use crate::domain::kiosk::value_objects::kiosk_error::KioskError;
use crate::domain::kiosk::value_objects::kiosk_event_kind::KioskEventKind;
use crate::domain::kiosk::value_objects::kiosk_service_error::KioskServiceError;
use crate::domain::kiosk::value_objects::kiosk_time::KioskTime;
use crate::domain::kiosk::value_objects::notifications::Notifications;
use crate::domain::kiosk::value_objects::public_rates::PublicRates;
use crate::domain::kiosk::value_objects::quote_limit::QuoteLimit;
use crate::domain::kiosk::value_objects::receipt_image::ReceiptImage;
use crate::domain::kiosk::value_objects::request_identity::RequestIdentity;
use crate::domain::kiosk::value_objects::save_attempt::SaveAttempt;
use crate::domain::kiosk::value_objects::transaction_link::TransactionLink;
use crate::infrastructure::kiosk::kiosk_repository::PgKioskRepository;

/// How many requests the queue shows.
pub const QUEUE_LIMIT: i64 = 100;

/// A counter nobody needs to look at: closed, no rate.
const NO_COUNTER: CounterFacts = CounterFacts {
    open: false,
    rate: None,
};

fn refuse(sentence: impl Display) -> AppError {
    AppError::Domain(sentence.to_string())
}

fn no_such_branch() -> AppError {
    refuse(KioskServiceError::NoSuchBranch)
}

fn allow(action: KioskAction, actor: &KioskActor) -> AppResult<()> {
    KioskMachine::authorize(action, actor).map_err(refuse)
}

/// The instant as the kiosk keeps it: whole milliseconds.
fn time(now: DateTime<Utc>) -> KioskTime {
    KioskTime::from_epoch_millis(now.timestamp_millis())
}

fn cut_to_millis(now: DateTime<Utc>) -> DateTime<Utc> {
    Utc.timestamp_millis_opt(now.timestamp_millis())
        .single()
        .unwrap_or(now)
}

fn username(actor: &KioskActor) -> &str {
    match actor {
        KioskActor::Staff { username, .. } => username,
        KioskActor::Customer => CUSTOMER_KIOSK,
    }
}

/// The audit line of a transition: who is named and what is kept with it.
fn line_for(event: KioskEventKind, request: &KioskRequest, actor: &str) -> AuditLine {
    let (actor, what) = match event {
        KioskEventKind::Request => (
            CUSTOMER_KIOSK,
            KioskAudit::Request {
                quote: request.quote,
            },
        ),
        KioskEventKind::Accepted => (actor, KioskAudit::Accepted),
        KioskEventKind::IdChecked => (actor, KioskAudit::IdChecked),
        KioskEventKind::Completed => (
            actor,
            KioskAudit::Completed {
                document_ref: request
                    .transaction
                    .as_ref()
                    .map(|link| link.document_ref.clone())
                    .unwrap_or_default(),
            },
        ),
    };
    AuditLine {
        actor: actor.to_owned(),
        subject: request.id.clone(),
        what,
    }
}

#[derive(Clone)]
pub struct KioskService {
    branch_tx: BranchTx,
    repo: PgKioskRepository,
    ports: Arc<dyn KioskPorts>,
}

impl KioskService {
    pub fn new(branch_tx: BranchTx, ports: Arc<dyn KioskPorts>) -> Self {
        let repo = PgKioskRepository::new(branch_tx.pool().clone());
        Self {
            branch_tx,
            repo,
            ports,
        }
    }

    async fn branch_name(&self, branch: &BranchId) -> AppResult<String> {
        self.ports
            .branch_name(branch)
            .await?
            .ok_or_else(no_such_branch)
    }

    /// `rates`: what the kiosk offers now.
    pub async fn public_rates(&self, branch: &BranchId) -> AppResult<PublicRates> {
        let view = self
            .ports
            .counter(branch)
            .await?
            .ok_or_else(no_such_branch)?;
        let currencies = KioskCurrencies::new(&self.repo.switches(branch).await?);
        Ok(PublicRates {
            branch_id: branch.clone(),
            rates: view.offered(&currencies),
            branch_name: view.branch_name,
            open: view.open,
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
                    let moved = KioskMachine::next(
                        stored.as_ref(),
                        &KioskCommand::Confirm,
                        &KioskActor::Customer,
                        &facts,
                        now,
                    )
                    .map_err(refuse)?;
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

    /// `notifications`: how many requests wait, for anyone signed in.
    pub async fn notifications(
        &self,
        branch: &BranchId,
        actor: &KioskActor,
    ) -> AppResult<Notifications> {
        allow(KioskAction::Notifications, actor)?;
        let branch_name = self.branch_name(branch).await?;
        let (count, latest) = self.repo.waiting(branch).await?;
        Ok(Notifications {
            branch_name,
            count,
            latest,
        })
    }

    /// `list`: the queue of the branch, newest first, at most [`QUEUE_LIMIT`].
    pub async fn list(
        &self,
        branch: &BranchId,
        actor: &KioskActor,
        now: DateTime<Utc>,
    ) -> AppResult<Vec<IdentifiedRequest>> {
        allow(KioskAction::List, actor)?;
        self.branch_name(branch).await?;
        let now = time(now);
        self.repo
            .queue(branch, QUEUE_LIMIT)
            .await?
            .into_iter()
            .map(|request| {
                let identity = KioskMachine::identity(Some(&request), now).map_err(refuse)?;
                Ok(IdentifiedRequest { request, identity })
            })
            .collect()
    }

    /// `accept`: a desk employee takes a waiting request.
    pub async fn accept(
        &self,
        branch: &BranchId,
        actor: &KioskActor,
        id: &str,
        now: DateTime<Utc>,
    ) -> AppResult<()> {
        allow(KioskAction::Accept, actor)?;
        self.desk_step(branch, actor, id, KioskCommand::Accept, now)
            .await
            .map(|_| ())
    }

    /// `identityChecked`: the employee holding the request confirms the passport check.
    pub async fn identity_checked(
        &self,
        branch: &BranchId,
        actor: &KioskActor,
        id: &str,
        confirmed: bool,
        now: DateTime<Utc>,
    ) -> AppResult<RequestIdentity> {
        allow(KioskAction::IdentityChecked, actor)?;
        self.desk_step(
            branch,
            actor,
            id,
            KioskCommand::IdentityChecked { confirmed },
            now,
        )
        .await
    }

    async fn desk_step(
        &self,
        branch: &BranchId,
        actor: &KioskActor,
        id: &str,
        command: KioskCommand,
        now: DateTime<Utc>,
    ) -> AppResult<RequestIdentity> {
        self.branch_name(branch).await?;
        let at = cut_to_millis(now);
        let now = time(now);
        let (repo, ports, b, id, actor) = (
            self.repo.clone(),
            self.ports.clone(),
            branch.clone(),
            id.to_owned(),
            actor.clone(),
        );
        self.branch_tx
            .run(branch, move |tx| {
                Box::pin(async move {
                    let stored = repo.find_in(tx, &b, &id).await?;
                    let moved =
                        KioskMachine::next(stored.as_ref(), &command, &actor, &NO_COUNTER, now)
                            .map_err(refuse)?;
                    if let Some(event) = moved.event {
                        repo.update_in(tx, &b, &moved.request).await?;
                        let line = line_for(event, &moved.request, username(&actor));
                        ports.record_in(tx, &b, &line, at).await?;
                    }
                    KioskMachine::identity(Some(&moved.request), now).map_err(refuse)
                })
            })
            .await
    }

    /// `get`: the request the employee holds, to fill the transaction form from.
    pub async fn get(
        &self,
        branch: &BranchId,
        actor: &KioskActor,
        id: &str,
    ) -> AppResult<KioskRequest> {
        allow(KioskAction::Get, actor)?;
        self.branch_name(branch).await?;
        let stored = self.repo.find(branch, id).await?;
        KioskMachine::check_get(stored.as_ref(), actor).map_err(refuse)?;
        stored.ok_or_else(|| refuse(KioskError::AcceptFirst))
    }

    /// `publishReceipt`: the employee holding the request sends the receipt
    /// picture to the customer's screen.
    pub async fn publish_receipt(
        &self,
        branch: &BranchId,
        actor: &KioskActor,
        id: &str,
        image: &str,
        now: DateTime<Utc>,
    ) -> AppResult<()> {
        allow(KioskAction::PublishReceipt, actor)?;
        self.branch_name(branch).await?;
        let now = time(now);
        let image = ReceiptImage::parse(image);
        let (repo, b, id, actor) = (
            self.repo.clone(),
            branch.clone(),
            id.to_owned(),
            actor.clone(),
        );
        self.branch_tx
            .run(branch, move |tx| {
                Box::pin(async move {
                    let stored = repo.find_in(tx, &b, &id).await?;
                    KioskMachine::check_publish(stored.as_ref(), &actor).map_err(refuse)?;
                    let image =
                        image.ok_or_else(|| refuse(KioskServiceError::InvalidReceiptImage))?;
                    repo.put_receipt_in(tx, &b, &id, &image, now).await
                })
            })
            .await
    }

    /// `manage`: every currency with whether this branch's kiosk offers it.
    pub async fn manage(
        &self,
        branch: &BranchId,
        actor: &KioskActor,
    ) -> AppResult<Vec<CurrencySwitch>> {
        allow(KioskAction::Manage, actor)?;
        self.branch_name(branch).await?;
        Ok(KioskCurrencies::new(&self.repo.switches(branch).await?).all())
    }

    /// `configure`: saves the counter rate, then stores the switch.
    pub async fn configure(
        &self,
        branch: &BranchId,
        actor: &KioskActor,
        cmd: ConfigureCurrency,
        now: DateTime<Utc>,
    ) -> AppResult<()> {
        allow(KioskAction::Configure, actor)?;
        self.branch_name(branch).await?;
        let code = CurrencyCode::parse(&cmd.code)
            .ok()
            .filter(|code| !code.is_thb())
            .ok_or_else(|| refuse(KioskServiceError::InvalidCurrencySettings))?;
        let rates_valid = match (
            Decimal::from_str_exact(&cmd.buy),
            Decimal::from_str_exact(&cmd.sell),
        ) {
            (Ok(buy), Ok(sell)) => buy > Decimal::ZERO && sell >= buy,
            _ => false,
        };
        if !rates_valid {
            return Err(refuse(KioskServiceError::RatesFirst));
        }

        let actor = username(actor).to_owned();
        let rate = SaveCounterRate {
            code,
            buy: cmd.buy.clone(),
            sell: cmd.sell.clone(),
            previous_updated: cmd.previous_updated,
            partner_name: cmd.partner_name,
            partner_cost: cmd.partner_cost,
            request_id: cmd.request_id,
        };
        self.ports.save_rate(branch, &rate, &actor, now).await?;

        let at = cut_to_millis(now);
        let line = AuditLine {
            actor,
            subject: code.as_str().to_owned(),
            what: KioskAudit::CurrencyConfigured {
                enabled: cmd.enabled,
                buy: cmd.buy,
                sell: cmd.sell,
            },
        };
        let (repo, ports, b, enabled) = (
            self.repo.clone(),
            self.ports.clone(),
            branch.clone(),
            cmd.enabled,
        );
        self.branch_tx
            .run(branch, move |tx| {
                Box::pin(async move {
                    repo.set_switch_in(tx, &b, code, enabled).await?;
                    ports.record_in(tx, &b, &line, at).await
                })
            })
            .await
    }

    /// Asked by the transaction save before it saves for a kiosk request,
    /// inside the save's own transaction (which holds the branch lock).
    pub async fn check_save_in(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        id: &str,
        save: &SaveAttempt,
        actor: &KioskActor,
    ) -> AppResult<()> {
        let stored = self.repo.find_in(tx, branch, id).await?;
        KioskMachine::check_save(stored.as_ref(), save, actor).map_err(refuse)
    }

    /// Called by the transaction save after it saved, inside the same
    /// transaction: links the request to the document and completes it.
    pub async fn complete_in(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        id: &str,
        link: &TransactionLink,
        actor: &KioskActor,
        now: DateTime<Utc>,
    ) -> AppResult<()> {
        let stored = self
            .repo
            .find_in(tx, branch, id)
            .await?
            .ok_or_else(|| refuse(KioskError::NotReadyToSave))?;
        let moved = KioskMachine::complete(&stored, &link.submission_id, &link.document_ref)
            .map_err(refuse)?;
        if let Some(event) = moved.event {
            self.repo.update_in(tx, branch, &moved.request).await?;
            let line = line_for(event, &moved.request, username(actor));
            self.ports
                .record_in(tx, branch, &line, cut_to_millis(now))
                .await?;
        }
        Ok(())
    }
}
