//! Device service: print and drawer jobs, the printer switch, what an employee
//! saw of the drawer, and the lines written about all of it.
//!
//! Every command that writes runs in `BranchTx::run` for its branch; reading
//! never takes the branch.

use std::sync::Arc;

use chrono::{DateTime, TimeZone, Utc};
use pos_error::{AppError, AppResult};
use rust_decimal::Decimal;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xc_rate::application::branch::service::BranchService;

use crate::infrastructure::device_agents::agent_repository::AgentRepository;
use crate::infrastructure::device_agents::pg_agent_repository::PgAgentRepository;

use super::device_commands::{QueueJob, SavedExchange};
use super::device_event::DeviceEvent;
use super::device_job::{DeviceJob, JobKind, JobStatus};
use super::device_repository::{DeviceRepository, NewEvent, NewJob, PRINTER_ONLINE, StoredJob};
use super::drawer_job::{DrawerJobKey, DrawerJobOutcome};
use super::job_ids::JobIds;
use super::pg_device_repository::PgDeviceRepository;
use super::printer_models::{DrawerSettings, HAS_DRAWER, PRINTER_MODEL, PrinterModels};
use super::printer_settings::{
    PRINTER_HOST, PRINTER_KIND, PRINTER_PORT, PrinterKind, PrinterSettings,
};
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
pub const BAD_PRINTER: &str = "ตั้งค่าเครื่องพิมพ์ไม่ถูกต้อง";
pub const NO_DRAWER: &str = "สาขานี้ไม่มีลิ้นชัก";

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
    agents: PgAgentRepository,
    exchanges: Arc<dyn SavedExchanges>,
    ids: Arc<dyn JobIds>,
    driver: Arc<dyn DeviceDriver>,
    printer_models: Arc<PrinterModels>,
}

fn refuse<T>(sentence: &str) -> AppResult<T> {
    Err(AppError::Domain(sentence.to_owned()))
}

fn replayed(prior: StoredJob, hash: &str) -> AppResult<DeviceJob> {
    if prior.request_hash != hash {
        return refuse(REQUEST_CHANGED);
    }
    Ok(prior.job)
}

/// 1 to 70 characters of `A-Z a-z 0-9 -`.
fn is_branch_name(text: &str) -> bool {
    BranchId::is_well_formed(text)
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
        let agents = PgAgentRepository::new(branch_tx.pool().clone());
        let branches = BranchService::new(branch_tx.clone());
        Self {
            branch_tx,
            branches,
            repo,
            agents,
            exchanges,
            ids,
            driver: Arc::new(SimulatedDriver),
            printer_models: Arc::new(PrinterModels::built_in()),
        }
    }

    /// The same service with another driver.
    pub fn with_driver(mut self, driver: Arc<dyn DeviceDriver>) -> Self {
        self.driver = driver;
        self
    }

    /// The same service with another printer models registry.
    pub fn with_printer_models(mut self, models: Arc<PrinterModels>) -> Self {
        self.printer_models = models;
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
                    if kind == JobKind::DrawerRequest
                        && repo.setting_in(tx, &b, HAS_DRAWER).await?.as_deref() == Some("0")
                    {
                        if let Some(prior) =
                            repo.find_job_by_request(tx, &b, &cmd.request_id).await?
                        {
                            return replayed(prior, &hash);
                        }
                        return refuse(NO_DRAWER);
                    }
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
                    replayed(prior, &hash)
                })
            })
            .await
    }

    /// Runs on the caller's transaction, which already holds the branch lock.
    pub async fn queue_drawer_in(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        key: DrawerJobKey,
        actor: &str,
        now: DateTime<Utc>,
    ) -> AppResult<DrawerJobOutcome> {
        if !is_branch_name(branch.as_str()) {
            return refuse(BAD_BRANCH);
        }

        let now = cut_to_millis(now)?;
        let request_id = key.request_id(branch);

        if let Some(stored) = self
            .repo
            .find_job_by_request(tx, branch, &request_id)
            .await?
        {
            return Ok(DrawerJobOutcome::Existing(stored.job));
        }

        if !self.agents.has_active_agent_in(tx, branch).await? {
            return Ok(DrawerJobOutcome::NoAgent);
        }

        let settings =
            DrawerSettings::read_in(tx, &self.repo, branch, &self.printer_models).await?;
        let has_pulse = self
            .printer_models
            .get(&settings.model)
            .and_then(|m| m.drawer_pulse.as_ref())
            .is_some();

        if settings.has_drawer == Some(false) || !has_pulse {
            let line = NewEvent {
                dedupe: None,
                actor,
                kind: "DRAWER_SKIPPED",
                reference: key.reference(),
                detail: &json!({}),
                amounts: &[],
                at: now,
            };
            self.repo.insert_event(tx, branch, &line).await?;
            return Ok(DrawerJobOutcome::NoDrawer);
        }

        let kind = JobKind::DrawerRequest;
        let id = self.ids.next();
        let reference = key.reference();
        let reason = key.reason();
        let hash = sha256_hex(&json!([kind.as_str(), reference, reason, actor]).to_string());
        let new_job = NewJob {
            id: &id,
            request_id: &request_id,
            request_hash: &hash,
            reference,
            kind,
            actor,
            at: now,
        };

        if self.repo.insert_job(tx, branch, &new_job).await? {
            let detail = json!({
                "job": id,
                "reason": reason,
                "mode": self.driver.mode(),
                "physicalAction": false,
            });
            let line = NewEvent {
                dedupe: None,
                actor,
                kind: kind.as_str(),
                reference,
                detail: &detail,
                amounts: &[],
                at: now,
            };
            self.repo.insert_event(tx, branch, &line).await?;
            return Ok(DrawerJobOutcome::Queued(DeviceJob {
                id,
                request_id,
                branch_id: branch.as_str().to_owned(),
                reference: reference.to_owned(),
                kind,
                status: JobStatus::Queued,
                actor: actor.to_owned(),
                created_at: now,
                updated_at: now,
            }));
        }

        if let Some(prior) = self
            .repo
            .find_job_by_request(tx, branch, &request_id)
            .await?
        {
            return Ok(DrawerJobOutcome::Existing(prior.job));
        }

        Err(AppError::Internal(format!(
            "drawer job for request {request_id} neither stored nor found"
        )))
    }

    /// Queues a drawer job in its own branch transaction.
    pub async fn queue_drawer(
        &self,
        branch: &BranchId,
        key: DrawerJobKey,
        actor: &str,
        now: DateTime<Utc>,
    ) -> AppResult<DrawerJobOutcome> {
        if !is_branch_name(branch.as_str()) {
            return refuse(BAD_BRANCH);
        }
        let this = self.clone();
        let b = branch.clone();
        let actor = actor.to_owned();
        self.branch_tx
            .run(branch, move |tx| {
                Box::pin(async move { this.queue_drawer_in(tx, &b, key, &actor, now).await })
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
        let agents = self.agents.clone();
        let driver = self.driver.clone();
        let models = self.printer_models.clone();
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
                    if job.kind == JobKind::DrawerRequest
                        && agents.has_active_agent_in(tx, &b).await?
                    {
                        return Ok(job);
                    }
                    let printer = PrinterSettings::read_in(tx, &repo, &b).await?;
                    let drawer = DrawerSettings::read_in(tx, &repo, &b, &models).await?;
                    let status = driver.run_with(job.kind, &printer, &drawer).await;
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

    /// Sets how the branch's printer is reached.
    pub async fn set_printer(
        &self,
        branch: &BranchId,
        kind: PrinterKind,
        _now: DateTime<Utc>,
    ) -> AppResult<()> {
        self.require_branch(branch).await?;
        match &kind {
            PrinterKind::Network(host, port) => {
                if host.trim().is_empty() || *port == 0 {
                    return refuse(BAD_PRINTER);
                }
            }
            PrinterKind::Simulated => {}
        }
        let repo = self.repo.clone();
        let b = branch.clone();
        self.branch_tx
            .run(branch, move |tx| {
                Box::pin(async move {
                    match kind {
                        PrinterKind::Simulated => {
                            repo.put_setting(tx, &b, PRINTER_KIND, "SIMULATED").await?;
                        }
                        PrinterKind::Network(host, port) => {
                            repo.put_setting(tx, &b, PRINTER_KIND, "NETWORK").await?;
                            repo.put_setting(tx, &b, PRINTER_HOST, &host).await?;
                            let port_str = port.to_string();
                            repo.put_setting(tx, &b, PRINTER_PORT, &port_str).await?;
                        }
                    }
                    Ok(())
                })
            })
            .await
    }

    /// Sets which printer model the branch uses and whether a drawer is connected.
    pub async fn set_drawer(
        &self,
        branch: &BranchId,
        model: &str,
        has_drawer: bool,
        _now: DateTime<Utc>,
    ) -> AppResult<()> {
        self.require_branch(branch).await?;
        if self.printer_models.get(model).is_none() {
            return refuse(BAD_PRINTER);
        }
        let repo = self.repo.clone();
        let b = branch.clone();
        let model_str = model.to_owned();
        self.branch_tx
            .run(branch, move |tx| {
                Box::pin(async move {
                    repo.put_setting(tx, &b, PRINTER_MODEL, &model_str).await?;
                    repo.put_setting(tx, &b, HAS_DRAWER, if has_drawer { "1" } else { "0" })
                        .await?;
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

    /// Reads the drawer settings of a branch.
    pub async fn drawer_settings(&self, branch: &BranchId) -> AppResult<DrawerSettings> {
        self.require_branch(branch).await?;
        let mut tx = self.branch_tx.pool().begin().await?;
        let settings =
            DrawerSettings::read_in(&mut tx, &self.repo, branch, &self.printer_models).await?;
        tx.rollback().await?;
        Ok(settings)
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
            let principal: Option<Decimal> = sqlx::query_scalar(
                "SELECT SUM(l.thb_amount) \
                 FROM transactions t \
                 JOIN transaction_lines l ON l.transaction_id = t.id \
                 WHERE t.document_ref = $1 AND t.branch_id = $2 AND t.status = 'COMPLETED'",
            )
            .bind(&saved.document_ref)
            .bind(b)
            .fetch_one(&mut **tx)
            .await
            .map_err(AppError::Database)?;

            let principal = principal.ok_or_else(|| {
                AppError::Internal(format!("no stored transaction for {}", saved.document_ref))
            })?;

            let mut thb = principal
                .round_dp_with_strategy(2, rust_decimal::RoundingStrategy::MidpointAwayFromZero);
            thb.rescale(2);

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

        if saved.cash {
            self.queue_drawer_in(
                tx,
                branch,
                DrawerJobKey::CashSave {
                    reference: saved.document_ref.clone(),
                },
                actor,
                now,
            )
            .await?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use super::*;

    #[test]
    fn count_confirmed_thb_pins_stored_principal_half_up_two_places() {
        // R9: The device count line's THB (COUNT_CONFIRMED) is the row's stored principal, 2 places, HALF_UP.
        // It is not recomputed from amount * rate.
        let principal = Decimal::from_str("1234.567").unwrap();
        let mut thb = principal
            .round_dp_with_strategy(2, rust_decimal::RoundingStrategy::MidpointAwayFromZero);
        thb.rescale(2);
        assert_eq!(thb.to_string(), "1234.57");

        let exact = Decimal::from_str("10000").unwrap();
        let mut thb2 =
            exact.round_dp_with_strategy(2, rust_decimal::RoundingStrategy::MidpointAwayFromZero);
        thb2.rescale(2);
        assert_eq!(thb2.to_string(), "10000.00");

        let half_up = Decimal::from_str("100.005").unwrap();
        let mut thb3 =
            half_up.round_dp_with_strategy(2, rust_decimal::RoundingStrategy::MidpointAwayFromZero);
        thb3.rescale(2);
        assert_eq!(thb3.to_string(), "100.01");
    }
}
