//! Builds the passport drawer the server talks to (design §5.5.8). Row S0 lands the builder
//! answering `None`; row AG-PD reads the switch in `Settings` and builds the device on the pool.

use std::collections::HashSet;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use pos_error::AppResult;
use pos_infra::transactions::Transactor;
use sqlx::PgPool;
use xc_exchange::application::exchange::service::TransactionService;
use xc_exchange::infrastructure::exchange::transaction_repository::PgTransactionRepository;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;

use crate::application::workflows::passport_drawer_device::{
    PassportDrawerDevice, PassportDrawerOutcome, PassportDrawerState,
};
use crate::config::Settings;
use crate::infrastructure::devices::device_job::{DeviceJob, JobStatus};
use crate::infrastructure::devices::device_repository::DeviceRepository;
use crate::infrastructure::devices::device_service::DeviceService;
use crate::infrastructure::devices::drawer_job::{DrawerJobKey, DrawerJobOutcome, PassportSource};
use crate::infrastructure::devices::job_ids::UuidJobIds;
use crate::infrastructure::devices::pg_device_repository::PgDeviceRepository;
use crate::infrastructure::devices::pg_saved_exchanges::PgSavedExchanges;
use crate::infrastructure::devices::printer_models::PrinterModels;

pub struct DevicePassportDrawer {
    branches: HashSet<String>,
    device_service: DeviceService,
    branch_tx: BranchTx,
    pool: PgPool,
    read_window: std::time::Duration,
}

fn word(status: JobStatus) -> PassportDrawerState {
    match status {
        JobStatus::Queued | JobStatus::Leased => PassportDrawerState::Requested,
        JobStatus::DrawerOpened => PassportDrawerState::SubmittedToCups,
        JobStatus::Failed
        | JobStatus::Expired
        | JobStatus::CommandSimulated
        | JobStatus::SimulatedPrinted => PassportDrawerState::Uncertain,
    }
}

impl PassportDrawerDevice for DevicePassportDrawer {
    fn switched_on(&self, branch: &BranchId) -> bool {
        self.branches.contains(branch.as_str())
    }

    fn request<'a>(
        &'a self,
        branch: &'a BranchId,
        source: PassportSource,
        order_id: &'a str,
        actor: &'a str,
        now: DateTime<Utc>,
    ) -> Pin<Box<dyn Future<Output = AppResult<PassportDrawerOutcome>> + Send + 'a>> {
        Box::pin(async move {
            let key = DrawerJobKey::Passport {
                source,
                order_id: order_id.to_string(),
            };
            let request_id = key.request_id(branch);

            enum ImmediateOrQueued {
                Immediate(PassportDrawerOutcome),
                Queued(DeviceJob),
            }

            let b = branch.clone();
            let actor_str = actor.to_string();
            let order_id_str = order_id.to_string();
            let service = self.device_service.clone();

            let queued_or_immediate = self
                .branch_tx
                .run(branch, move |tx| {
                    Box::pin(async move {
                        if source == PassportSource::Kiosk {
                            let row: Option<(Option<String>, String)> = sqlx::query_as(
                                "SELECT staff, status FROM kiosk_requests WHERE branch_id = $1 AND id = $2",
                            )
                            .bind(b.as_str())
                            .bind(&order_id_str)
                            .fetch_optional(&mut **tx)
                            .await?;

                            match row {
                                Some((Some(ref staff), ref status))
                                    if staff == &actor_str
                                        && (status == "ACCEPTED" || status == "ID_CHECKED") => {}
                                _ => {
                                    return Ok(ImmediateOrQueued::Immediate(
                                        PassportDrawerOutcome::NotHolder,
                                    ));
                                }
                            }
                        }

                        let outcome =
                            service.queue_drawer_in(tx, &b, key, &actor_str, now).await?;
                        match outcome {
                            DrawerJobOutcome::NoAgent => {
                                Ok(ImmediateOrQueued::Immediate(PassportDrawerOutcome::State {
                                    state: PassportDrawerState::Simulated,
                                    duplicate: false,
                                }))
                            }
                            DrawerJobOutcome::NoDrawer => {
                                Ok(ImmediateOrQueued::Immediate(PassportDrawerOutcome::State {
                                    state: PassportDrawerState::Uncertain,
                                    duplicate: false,
                                }))
                            }
                            DrawerJobOutcome::Existing(job) => {
                                if job.actor != actor_str {
                                    Ok(ImmediateOrQueued::Immediate(
                                        PassportDrawerOutcome::OtherEmployee,
                                    ))
                                } else {
                                    Ok(ImmediateOrQueued::Immediate(PassportDrawerOutcome::State {
                                        state: word(job.status),
                                        duplicate: true,
                                    }))
                                }
                            }
                            DrawerJobOutcome::Queued(job) => Ok(ImmediateOrQueued::Queued(job)),
                        }
                    })
                })
                .await?;

            let job = match queued_or_immediate {
                ImmediateOrQueued::Immediate(outcome) => return Ok(outcome),
                ImmediateOrQueued::Queued(job) => job,
            };

            let repo = PgDeviceRepository::new(self.pool.clone());
            let deadline = tokio::time::Instant::now() + self.read_window;
            let mut status = job.status;

            while status == JobStatus::Queued || status == JobStatus::Leased {
                let now_inst = tokio::time::Instant::now();
                if now_inst >= deadline {
                    break;
                }
                let remaining = deadline - now_inst;
                let sleep_dur = std::time::Duration::from_millis(250).min(remaining);
                tokio::time::sleep(sleep_dur).await;

                let mut tx = self.pool.begin().await?;
                if let Some(stored) = repo
                    .find_job_by_request(&mut tx, branch, &request_id)
                    .await?
                {
                    status = stored.job.status;
                }
                let _ = tx.commit().await;
            }

            Ok(PassportDrawerOutcome::State {
                state: word(status),
                duplicate: false,
            })
        })
    }
}

/// The branch-independent passport drawer, or `None` while there is none to build.
pub fn passport_drawer_device(
    settings: &Settings,
    db: &PgPool,
) -> Option<Arc<dyn PassportDrawerDevice>> {
    if settings.devices.passport_drawer_branches.is_empty() {
        return None;
    }

    let printer_models = match PrinterModels::new(settings.devices.printer_models.clone()) {
        Ok(models) => Arc::new(models),
        Err(err) => {
            tracing::error!(error = %err, "failed to initialize configured printer models");
            return None;
        }
    };

    let branch_tx = BranchTx::new(Transactor::new(db.clone()));
    let transactions = TransactionService::new(
        Arc::new(PgTransactionRepository::new(db.clone())),
        branch_tx.clone(),
    );
    let device_service = DeviceService::new(
        branch_tx.clone(),
        Arc::new(PgSavedExchanges::new(transactions)),
        Arc::new(UuidJobIds),
    )
    .with_printer_models(printer_models);

    let branches: HashSet<String> = settings
        .devices
        .passport_drawer_branches
        .iter()
        .cloned()
        .collect();
    let read_window =
        std::time::Duration::from_millis(settings.devices.passport_drawer_read_window_ms);

    Some(Arc::new(DevicePassportDrawer {
        branches,
        device_service,
        branch_tx,
        pool: db.clone(),
        read_window,
    }))
}
