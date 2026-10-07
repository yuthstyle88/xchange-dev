//! The two hooks the exchange save calls inside its branch transaction.
//! Methods of `KioskService` (design §10 S0); the type lives in `service.rs`.

use chrono::{DateTime, Utc};
use pos_error::AppResult;
use xc_kernel::ids::BranchId;

use crate::domain::kiosk::entities::kiosk_machine::KioskMachine;
use crate::domain::kiosk::repositories::kiosk_repository::KioskRepository;
use crate::domain::kiosk::value_objects::kiosk_actor::KioskActor;
use crate::domain::kiosk::value_objects::kiosk_error::KioskError;
use crate::domain::kiosk::value_objects::save_attempt::SaveAttempt;
use crate::domain::kiosk::value_objects::transaction_link::TransactionLink;

use super::service::{KioskService, cut_to_millis, line_for, refuse, username};

impl KioskService {
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
