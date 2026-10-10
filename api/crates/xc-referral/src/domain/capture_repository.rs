//! `CaptureRepository` (design §8): the store of `referral_attributions`. Its methods are
//! `[proposed]` (#979 §7); every one runs in the caller's transaction.

use futures::future::BoxFuture;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use crate::application::capture_service::CaptureError;
use crate::domain::capture::{Capture, CaptureId, CaptureTarget};

#[derive(Debug, Clone)]
pub struct TargetInfo {
    pub organization_id: Uuid,
    pub branch_id: String,
    pub customer_id: String,
}

pub trait CaptureRepository: Send + Sync {
    /// Resolves target info (customer_id, branch_id, organization_id).
    fn resolve_target<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        target: &'a CaptureTarget,
    ) -> BoxFuture<'a, Result<TargetInfo, CaptureError>>;

    /// Finds any ACTIVE capture on `target`, returning (id, code).
    fn find_active_for_target<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        target: &'a CaptureTarget,
    ) -> BoxFuture<'a, Result<Option<(CaptureId, String)>, CaptureError>>;

    /// Inserts a new `Capture` row.
    fn insert<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        capture: &'a Capture,
    ) -> BoxFuture<'a, Result<(), CaptureError>>;

    /// Finds a capture by id.
    fn find_by_id<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        id: &'a CaptureId,
    ) -> BoxFuture<'a, Result<Option<Capture>, CaptureError>>;

    /// Marks the capture as SUPERSEDED if currently ACTIVE. Returns true if updated.
    fn mark_superseded<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        id: &'a CaptureId,
    ) -> BoxFuture<'a, Result<bool, CaptureError>>;

    /// Finds ACTIVE, unpushed captures for customer, updates their customer_sub and pushed_at,
    /// and returns their (id, organization_id).
    fn mark_pushed_for_customer<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        customer: &'a str,
        sub: &'a str,
    ) -> BoxFuture<'a, Result<Vec<(CaptureId, Uuid)>, CaptureError>>;

    /// Sets transaction_id on the ACTIVE capture for booking_id.
    fn link_transaction<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        booking_id: &'a str,
        transaction_id: Uuid,
    ) -> BoxFuture<'a, Result<(), CaptureError>>;
}
