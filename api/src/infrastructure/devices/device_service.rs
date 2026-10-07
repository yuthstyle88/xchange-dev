//! Device service: print and drawer jobs, the printer switch, what an employee
//! saw of the drawer, and the lines written about all of it.
//!
//! Every command that writes runs in `BranchTx::run` for its branch; reading
//! never takes the branch.

use std::sync::Arc;

use chrono::{DateTime, TimeZone, Utc};
use pos_error::{AppError, AppResult};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xc_rate::application::branch::service::BranchService;

use super::device_commands::{QueueJob, SavedExchange};
use super::device_event::DeviceEvent;
use super::device_job::{DeviceJob, JobKind, JobStatus};
use super::device_repository::{DeviceRepository, NewEvent, NewJob, PRINTER_ONLINE};
use super::job_ids::JobIds;
use super::pg_device_repository::PgDeviceRepository;
use super::saved_exchanges::{ExchangeState, SavedExchanges};
use super::simulated_driver::{DeviceDriver, SimulatedDriver};

pub const BAD_BRANCH: &str = "สาขาไม่ถูกต้อง";
pub const BAD_KIND: &str = "ประเภทอุปกรณ์ไม่ถูกต้อง";
pub const BAD_REQUEST_ID: &str = "รหัสคำขอไม่ถูกต้อง";
pub const PRINT_NEEDS_REF: &str = "เลือกรายการก่อนพิมพ์";
pub const NOT_COMPLETED_HERE: &str = "ไม่พบรายการสำเร็จในสาขานี้";
pub const REASON_TOO_LONG: &str = "เหตุผลยาวเกินกำหนด";
pub const DRAWER_NEEDS_REASON: &str = "ระบุเหตุผลเปิดลิ้นชัก";
pub const REQUEST_CHANGED: &str = "คำขอเดิมมีข้อมูลเปลี่ยน";
pub const NO_SUCH_JOB: &str = "ไม่พบงาน";
pub const RETRY_ONLY_FAILED: &str = "ลองใหม่ได้เฉพาะงานที่ล้มเหลว";
pub const NOT_HERE: &str = "ไม่พบรายการในสาขานี้";
pub const BAD_STATE: &str = "สถานะไม่ถูกต้อง";
pub const COUNT_FIRST: &str = "ยืนยันการนับเงินก่อนบันทึกรายการเงินสด";

/// How many jobs and lines the overview carries.
pub const OVERVIEW_JOBS: i64 = 100;
pub const OVERVIEW_EVENTS: i64 = 200;

/// The longest reason, counted the way the screens count it (UTF-16 units).
const REASON_MAX: usize = 200;

/// What the devices screen shows of one branch.
#[derive(Debug, Clone, PartialEq)]
pub struct DeviceOverview {
    /// Newest first.
    pub jobs: Vec<DeviceJob>,
    /// Newest first.
    pub events: Vec<DeviceEvent>,
    pub printer_online: bool,
}

#[derive(Clone)]
pub struct DeviceService {
    branch_tx: BranchTx,
    branches: BranchService,
    repo: PgDeviceRepository,
    exchanges: Arc<dyn SavedExchanges>,
    ids: Arc<dyn JobIds>,
    driver: Arc<dyn DeviceDriver>,
}

fn refuse<T>(sentence: &str) -> AppResult<T> {
    Err(AppError::Domain(sentence.to_owned()))
}

/// 1 to 70 characters of `A-Z a-z 0-9 -`.
fn is_branch_name(text: &str) -> bool {
    (1..=70).contains(&text.len()) && text.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

/// Exactly 36 characters of `0-9 a-f -`.
fn is_request_id(text: &str) -> bool {
    text.len() == 36
        && text
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b) || b == b'-')
}

/// What JavaScript's `trim` removes: its white space and its line terminators.
fn is_js_blank(c: char) -> bool {
    matches!(
        c,
        '\u{0009}'..='\u{000D}'
            | '\u{0020}'
            | '\u{00A0}'
            | '\u{1680}'
            | '\u{2000}'..='\u{200A}'
            | '\u{2028}'
            | '\u{2029}'
            | '\u{202F}'
            | '\u{205F}'
            | '\u{3000}'
            | '\u{FEFF}'
    )
}

fn cut_to_millis(t: DateTime<Utc>) -> AppResult<DateTime<Utc>> {
    Utc.timestamp_millis_opt(t.timestamp_millis())
        .single()
        .ok_or_else(|| AppError::Internal(format!("instant out of range: {t}")))
}

fn sha256_hex(text: &str) -> String {
    Sha256::digest(text.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

impl DeviceService {
    /// A service with the simulated driver. Touches nothing.
    pub fn new(
        branch_tx: BranchTx,
        exchanges: Arc<dyn SavedExchanges>,
        ids: Arc<dyn JobIds>,
    ) -> Self {
        let repo = PgDeviceRepository::new(branch_tx.pool().clone());
        let branches = BranchService::new(branch_tx.clone());
        Self {
            branch_tx,
            branches,
            repo,
            exchanges,
            ids,
            driver: Arc::new(SimulatedDriver),
        }
    }

    /// The same service with another driver.
    pub fn with_driver(mut self, driver: Arc<dyn DeviceDriver>) -> Self {
        self.driver = driver;
        self
    }

    /// A cash exchange is counted before it is saved.
    pub fn require_count_confirmed(cash: bool, confirmed: bool) -> AppResult<()> {
        if cash && !confirmed {
            return refuse(COUNT_FIRST);
        }
        Ok(())
    }

    async fn require_branch(&self, branch: &BranchId) -> AppResult<()> {
        if !is_branch_name(branch.as_str()) {
            return refuse(BAD_BRANCH);
        }
        self.branches.require(branch).await?;
        Ok(())
    }

    /// Queues a print or drawer job. The same request again returns the job it
    /// made; the same request id with other content is refused.
    pub async fn queue(
        &self,
        branch: &BranchId,
        cmd: QueueJob,
        actor: &str,
        now: DateTime<Utc>,
    ) -> AppResult<DeviceJob> {
        self.require_branch(branch).await?;
        let Some(kind) = JobKind::parse(&cmd.kind) else {
            return refuse(BAD_KIND);
        };
        if !is_request_id(&cmd.request_id) {
            return refuse(BAD_REQUEST_ID);
        }
        if kind == JobKind::PrintReceipt && cmd.reference.is_empty() {
            return refuse(PRINT_NEEDS_REF);
        }
        if !cmd.reference.is_empty()
            && self.exchanges.find(branch, &cmd.reference).await? != Some(ExchangeState::Completed)
        {
            return refuse(NOT_COMPLETED_HERE);
        }
        if cmd.reason.encode_utf16().count() > REASON_MAX {
            return refuse(REASON_TOO_LONG);
        }
        if kind == JobKind::DrawerRequest && cmd.reason.chars().all(is_js_blank) {
            return refuse(DRAWER_NEEDS_REASON);
        }

        let now = cut_to_millis(now)?;
        let hash =
            sha256_hex(&json!([kind.as_str(), cmd.reference, cmd.reason, actor]).to_string());
        let id = self.ids.next();
        let detail = json!({
            "job": id, "reason": cmd.reason, "mode": self.driver.mode(), "physicalAction": false,
        });
        let repo = self.repo.clone();
        let b = branch.clone();
        let actor = actor.to_owned();
        self.branch_tx
            .run(branch, move |tx| {
                Box::pin(async move {
                    let new = NewJob {
                        id: &id,
                        request_id: &cmd.request_id,
                        request_hash: &hash,
                        reference: &cmd.reference,
                        kind,
                        actor: &actor,
                        at: now,
                    };
                    if repo.insert_job(tx, &b, &new).await? {
                        let line = NewEvent {
                            dedupe: None,
                            actor: &actor,
                            kind: kind.as_str(),
                            reference: &cmd.reference,
                            detail: &detail,
                            amounts: &[],
                            at: now,
                        };
                        repo.insert_event(tx, &b, &line).await?;
                        return Ok(DeviceJob {
                            id,
                            request_id: cmd.request_id,
                            branch_id: b.as_str().to_owned(),
                            reference: cmd.reference,
                            kind,
                            status: JobStatus::Queued,
                            actor,
                            created_at: now,
                            updated_at: now,
                        });
                    }
                    let Some(prior) = repo.find_job_by_request(tx, &b, &cmd.request_id).await?
                    else {
                        return Err(AppError::Internal(format!(
                            "device job for request {} neither stored nor found",
                            cmd.request_id
                        )));
                    };
                    if prior.request_hash != hash {
                        return refuse(REQUEST_CHANGED);
                    }
                    Ok(prior.job)
                })
            })
            .await
    }

    /// Carries a queued job out. A job that is not queued is returned as it is.
    pub async fn process(
        &self,
        branch: &BranchId,
        job_id: &str,
        actor: &str,
        now: DateTime<Utc>,
    ) -> AppResult<DeviceJob> {
        self.require_branch(branch).await?;
        let now = cut_to_millis(now)?;
        let repo = self.repo.clone();
        let driver = self.driver.clone();
        let b = branch.clone();
        let job_id = job_id.to_owned();
        let actor = actor.to_owned();
        self.branch_tx
            .run(branch, move |tx| {
                Box::pin(async move {
                    let Some(mut job) = repo.find_job(tx, &b, &job_id).await? else {
                        return refuse(NO_SUCH_JOB);
                    };
                    if job.status != JobStatus::Queued {
                        return Ok(job);
                    }
                    let online =
                        repo.setting_in(tx, &b, PRINTER_ONLINE).await?.as_deref() != Some("0");
                    let status = driver.run(job.kind, online);
                    repo.set_job_status(tx, &b, &job.id, status, now).await?;
                    let detail = json!({
                        "job": job.id, "physicalAction": false, "physicalOpenConfirmed": false,
                    });
                    let line = NewEvent {
                        dedupe: None,
                        actor: &actor,
                        kind: status.as_str(),
                        reference: &job.reference,
                        detail: &detail,
                        amounts: &[],
                        at: now,
                    };
                    repo.insert_event(tx, &b, &line).await?;
                    job.status = status;
                    job.updated_at = now;
                    Ok(job)
                })
            })
            .await
    }

    /// Queues a failed job again.
    pub async fn retry(
        &self,
        branch: &BranchId,
        job_id: &str,
        actor: &str,
        now: DateTime<Utc>,
    ) -> AppResult<()> {
        self.require_branch(branch).await?;
        let now = cut_to_millis(now)?;
        let repo = self.repo.clone();
        let b = branch.clone();
        let job_id = job_id.to_owned();
        let actor = actor.to_owned();
        self.branch_tx
            .run(branch, move |tx| {
                Box::pin(async move {
                    let job = match repo.find_job(tx, &b, &job_id).await? {
                        Some(job) if job.status == JobStatus::Failed => job,
                        _ => return refuse(RETRY_ONLY_FAILED),
                    };
                    repo.set_job_status(tx, &b, &job.id, JobStatus::Queued, now)
                        .await?;
                    let detail = json!({ "job": job.id });
                    let line = NewEvent {
                        dedupe: None,
                        actor: &actor,
                        kind: "JOB_RETRY",
                        reference: &job.reference,
                        detail: &detail,
                        amounts: &[],
                        at: now,
                    };
                    repo.insert_event(tx, &b, &line).await?;
                    Ok(())
                })
            })
            .await
    }

    /// Sets the branch's printer switch and writes that down, changed or not.
    pub async fn set_printer_online(
        &self,
        branch: &BranchId,
        online: bool,
        actor: &str,
        now: DateTime<Utc>,
    ) -> AppResult<()> {
        self.require_branch(branch).await?;
        let now = cut_to_millis(now)?;
        let repo = self.repo.clone();
        let b = branch.clone();
        let actor = actor.to_owned();
        self.branch_tx
            .run(branch, move |tx| {
                Box::pin(async move {
                    let value = if online { "1" } else { "0" };
                    repo.put_setting(tx, &b, PRINTER_ONLINE, value).await?;
                    let detail = json!({ "online": online });
                    let line = NewEvent {
                        dedupe: None,
                        actor: &actor,
                        kind: "SIMULATED_PRINTER_CONNECTION",
                        reference: "",
                        detail: &detail,
                        amounts: &[],
                        at: now,
                    };
                    repo.insert_event(tx, &b, &line).await?;
                    Ok(())
                })
            })
            .await
    }

    /// Writes down what an employee saw of the drawer. `reference` is empty or
    /// the document number of an exchange of the branch, in any state.
    pub async fn drawer_observation(
        &self,
        branch: &BranchId,
        reference: &str,
        state: &str,
        actor: &str,
        now: DateTime<Utc>,
    ) -> AppResult<()> {
        self.require_branch(branch).await?;
        if !reference.is_empty() && self.exchanges.find(branch, reference).await?.is_none() {
            return refuse(NOT_HERE);
        }
        if state != "OPEN" && state != "CLOSED" {
            return refuse(BAD_STATE);
        }
        let detail = json!({
            "state": state, "source": "EMPLOYEE_OBSERVATION", "sensorConfirmed": false,
        });
        self.write_line(
            branch,
            "DRAWER_MANUAL_OBSERVATION",
            reference,
            detail,
            actor,
            now,
        )
        .await
    }

    /// Writes down that an evidence file was uploaded. The upload route calls
    /// this after the file is stored.
    pub async fn scan_imported(
        &self,
        branch: &BranchId,
        upload_key: &str,
        mime: &str,
        actor: &str,
        now: DateTime<Utc>,
    ) -> AppResult<()> {
        self.require_branch(branch).await?;
        let detail = json!({ "mode": "MANUAL_IMPORT", "mime": mime });
        self.write_line(branch, "SCAN_IMPORTED", upload_key, detail, actor, now)
            .await
    }

    /// One line, in a branch transaction of its own.
    async fn write_line(
        &self,
        branch: &BranchId,
        kind: &'static str,
        reference: &str,
        detail: Value,
        actor: &str,
        now: DateTime<Utc>,
    ) -> AppResult<()> {
        let now = cut_to_millis(now)?;
        let repo = self.repo.clone();
        let b = branch.clone();
        let reference = reference.to_owned();
        let actor = actor.to_owned();
        self.branch_tx
            .run(branch, move |tx| {
                Box::pin(async move {
                    let line = NewEvent {
                        dedupe: None,
                        actor: &actor,
                        kind,
                        reference: &reference,
                        detail: &detail,
                        amounts: &[],
                        at: now,
                    };
                    repo.insert_event(tx, &b, &line).await?;
                    Ok(())
                })
            })
            .await
    }

    /// The newest jobs and lines of the branch and its printer switch.
    pub async fn overview(&self, branch: &BranchId) -> AppResult<DeviceOverview> {
        self.require_branch(branch).await?;
        Ok(DeviceOverview {
            jobs: self.repo.list_jobs(branch, OVERVIEW_JOBS).await?,
            events: self.repo.list_events(branch, OVERVIEW_EVENTS).await?,
            printer_online: self.repo.setting(branch, PRINTER_ONLINE).await?.as_deref()
                != Some("0"),
        })
    }

    /// The lines a saved exchange leaves, and its print job — written on the
    /// transaction that saves the exchange, which already holds the branch.
    /// Called twice for the same exchange it adds nothing the second time.
    pub async fn after_save_in(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        saved: &SavedExchange,
        actor: &str,
        now: DateTime<Utc>,
    ) -> AppResult<()> {
        let now = cut_to_millis(now)?;
        let b = branch.as_str();
        if saved.cash {
            let thb = saved.amount.checked_mul(saved.rate).ok_or_else(|| {
                AppError::Internal(format!("{} × {} is out of range", saved.amount, saved.rate))
            })?;
            let detail = json!({ "mode": "EMPLOYEE_CONFIRMATION", "currency": saved.currency });
            let amounts = [("foreignAmount", saved.amount), ("thb", thb)];
            let key = format!("COUNT:{b}:{}", saved.submission_id);
            let line = NewEvent {
                dedupe: Some(&key),
                actor,
                kind: "COUNT_CONFIRMED",
                reference: &saved.submission_id,
                detail: &detail,
                amounts: &amounts,
                at: now,
            };
            self.repo.insert_event(tx, branch, &line).await?;
        }

        let detail = json!({
            "submissionId": saved.submission_id, "passport": saved.passport_key, "slip": saved.slip_key,
        });
        let key = format!("LINKED:{b}:{}", saved.document_ref);
        let line = NewEvent {
            dedupe: Some(&key),
            actor,
            kind: "EXCHANGE_LINKED",
            reference: &saved.document_ref,
            detail: &detail,
            amounts: &[],
            at: now,
        };
        if !self.repo.insert_event(tx, branch, &line).await? {
            return Ok(());
        }

        let id = self.ids.next();
        let request_id = format!("auto:{b}:{}", saved.document_ref);
        let hash = sha256_hex(&saved.document_ref);
        let job = NewJob {
            id: &id,
            request_id: &request_id,
            request_hash: &hash,
            reference: &saved.document_ref,
            kind: JobKind::PrintReceipt,
            actor,
            at: now,
        };
        if self.repo.insert_job(tx, branch, &job).await? {
            let detail = json!({
                "job": id, "reason": "", "mode": self.driver.mode(), "physicalAction": false,
            });
            let line = NewEvent {
                dedupe: None,
                actor,
                kind: JobKind::PrintReceipt.as_str(),
                reference: &saved.document_ref,
                detail: &detail,
                amounts: &[],
                at: now,
            };
            self.repo.insert_event(tx, branch, &line).await?;
        }
        Ok(())
    }
}
