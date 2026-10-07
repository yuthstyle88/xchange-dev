//! The desk's side of the queue: notifications, list, accept, identity check, get and publish the receipt.
//! Methods of `KioskService` (design §10 S0); the type lives in `service.rs`.

use chrono::{DateTime, Utc};
use pos_error::AppResult;
use xc_kernel::ids::BranchId;

use crate::domain::kiosk::commands::kiosk_command::KioskCommand;
use crate::domain::kiosk::entities::kiosk_machine::KioskMachine;
use crate::domain::kiosk::entities::kiosk_request::KioskRequest;
use crate::domain::kiosk::repositories::kiosk_repository::KioskRepository;
use crate::domain::kiosk::value_objects::identified_request::IdentifiedRequest;
use crate::domain::kiosk::value_objects::kiosk_actor::KioskActor;
use crate::domain::kiosk::value_objects::kiosk_error::KioskError;
use crate::domain::kiosk::value_objects::kiosk_service_error::KioskServiceError;
use crate::domain::kiosk::value_objects::notifications::Notifications;
use crate::domain::kiosk::value_objects::receipt_image::ReceiptImage;
use crate::domain::kiosk::value_objects::request_identity::RequestIdentity;

use super::service::{
    KioskService, NO_COUNTER, QUEUE_LIMIT, cut_to_millis, line_for, refuse, time, username,
};

impl KioskService {
    /// `notifications`: how many requests wait, for anyone signed in.
    pub async fn notifications(
        &self,
        branch: &BranchId,
        _actor: &KioskActor,
    ) -> AppResult<Notifications> {
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
        _actor: &KioskActor,
        now: DateTime<Utc>,
    ) -> AppResult<Vec<IdentifiedRequest>> {
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
}
