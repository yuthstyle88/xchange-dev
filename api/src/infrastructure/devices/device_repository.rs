//! Device repository trait: every statement on the three device tables.

use std::future::Future;

use chrono::{DateTime, Utc};
use pos_error::AppResult;
use rust_decimal::Decimal;
use serde_json::Value;
use xc_kernel::ids::BranchId;

use super::device_event::DeviceEvent;
use super::device_job::{DeviceJob, JobKind, JobStatus};

/// The key of the one setting a branch has.
pub const PRINTER_ONLINE: &str = "printerOnline";

/// A job about to be stored; it starts queued.
#[derive(Debug, Clone)]
pub struct NewJob<'a> {
    pub id: &'a str,
    pub request_id: &'a str,
    pub request_hash: &'a str,
    pub reference: &'a str,
    pub kind: JobKind,
    pub actor: &'a str,
    pub at: DateTime<Utc>,
}

/// The keys of a line's detail that hold an amount. They are stored as exact
/// JSON numbers (PostgreSQL keeps a `jsonb` number as a decimal) and handed
/// out as decimal text; no amount is ever held in a floating-point number.
pub const AMOUNT_KEYS: [&str; 2] = ["foreignAmount", "thb"];

/// A line about to be written.
#[derive(Debug, Clone)]
pub struct NewEvent<'a> {
    /// Lines with the same key in one branch are written once; `None` never collides.
    pub dedupe: Option<&'a str>,
    pub actor: &'a str,
    pub kind: &'a str,
    pub reference: &'a str,
    /// Everything of the detail that is not an amount.
    pub detail: &'a Value,
    /// The amounts of the detail, by key — each one of [`AMOUNT_KEYS`].
    pub amounts: &'a [(&'static str, Decimal)],
    pub at: DateTime<Utc>,
}

/// A stored job with the hash of the request that made it.
#[derive(Debug, Clone)]
pub struct StoredJob {
    pub job: DeviceJob,
    pub request_hash: String,
}

type Tx<'a, 'c> = &'a mut sqlx::Transaction<'c, sqlx::Postgres>;

pub trait DeviceRepository: Send + Sync {
    /// The newest `limit` jobs of the branch, newest first; jobs of one instant by id, greatest first.
    fn list_jobs(
        &self,
        branch: &BranchId,
        limit: i64,
    ) -> impl Future<Output = AppResult<Vec<DeviceJob>>> + Send;

    /// The newest `limit` lines of the branch, newest first. In each detail the
    /// amounts ([`AMOUNT_KEYS`]) are decimal text.
    fn list_events(
        &self,
        branch: &BranchId,
        limit: i64,
    ) -> impl Future<Output = AppResult<Vec<DeviceEvent>>> + Send;

    /// The value of a setting as committed, if the branch has it.
    fn setting(
        &self,
        branch: &BranchId,
        key: &str,
    ) -> impl Future<Output = AppResult<Option<String>>> + Send;

    /// The value of a setting as the transaction sees it.
    fn setting_in(
        &self,
        tx: Tx<'_, '_>,
        branch: &BranchId,
        key: &str,
    ) -> impl Future<Output = AppResult<Option<String>>> + Send;

    /// Stores or replaces one setting of the branch.
    fn put_setting(
        &self,
        tx: Tx<'_, '_>,
        branch: &BranchId,
        key: &str,
        value: &str,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Stores the job unless the branch already has one for that request id.
    /// `true` when it was stored.
    fn insert_job(
        &self,
        tx: Tx<'_, '_>,
        branch: &BranchId,
        job: &NewJob<'_>,
    ) -> impl Future<Output = AppResult<bool>> + Send;

    fn find_job_by_request(
        &self,
        tx: Tx<'_, '_>,
        branch: &BranchId,
        request_id: &str,
    ) -> impl Future<Output = AppResult<Option<StoredJob>>> + Send;

    fn find_job(
        &self,
        tx: Tx<'_, '_>,
        branch: &BranchId,
        id: &str,
    ) -> impl Future<Output = AppResult<Option<DeviceJob>>> + Send;

    fn set_job_status(
        &self,
        tx: Tx<'_, '_>,
        branch: &BranchId,
        id: &str,
        status: JobStatus,
        at: DateTime<Utc>,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Writes the line unless the branch already has one with its key.
    /// `true` when it was written.
    fn insert_event(
        &self,
        tx: Tx<'_, '_>,
        branch: &BranchId,
        event: &NewEvent<'_>,
    ) -> impl Future<Output = AppResult<bool>> + Send;
}
