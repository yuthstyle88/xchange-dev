//! Kiosk repository trait.
//!
//! Every statement of the kiosk goes through here. A method ending in `_in`
//! runs on the transaction it is handed; the others read committed data.

use std::future::Future;

use pos_error::AppResult;
use xc_kernel::ids::BranchId;
use xc_kernel::money::CurrencyCode;

use crate::domain::kiosk::entities::kiosk_request::KioskRequest;
use crate::domain::kiosk::value_objects::currency_switch::CurrencySwitch;
use crate::domain::kiosk::value_objects::kiosk_time::KioskTime;
use crate::domain::kiosk::value_objects::receipt_image::ReceiptImage;

/// A stored receipt picture and when it was published.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredReceipt {
    pub image: String,
    pub published: KioskTime,
}

type Tx<'a, 'c> = &'a mut sqlx::Transaction<'c, sqlx::Postgres>;

pub trait KioskRepository: Send + Sync {
    /// The request with this id in this branch.
    fn find(
        &self,
        branch: &BranchId,
        id: &str,
    ) -> impl Future<Output = AppResult<Option<KioskRequest>>> + Send;

    fn find_in(
        &self,
        tx: Tx<'_, '_>,
        branch: &BranchId,
        id: &str,
    ) -> impl Future<Output = AppResult<Option<KioskRequest>>> + Send;

    fn insert_in(
        &self,
        tx: Tx<'_, '_>,
        branch: &BranchId,
        request: &KioskRequest,
        client_ip: &str,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Stores the status, the holder and the transaction link of `request`.
    fn update_in(
        &self,
        tx: Tx<'_, '_>,
        branch: &BranchId,
        request: &KioskRequest,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Every request of the branch nobody confirmed yet.
    fn quoted_in(
        &self,
        tx: Tx<'_, '_>,
        branch: &BranchId,
    ) -> impl Future<Output = AppResult<Vec<KioskRequest>>> + Send;

    fn delete_in(
        &self,
        tx: Tx<'_, '_>,
        branch: &BranchId,
        id: &str,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// How many requests of the branch came from `client_ip` after `since`.
    fn quotes_from_in(
        &self,
        tx: Tx<'_, '_>,
        branch: &BranchId,
        client_ip: &str,
        since: KioskTime,
    ) -> impl Future<Output = AppResult<i64>> + Send;

    /// How many requests of the branch are waiting, and the newest of them.
    fn waiting(
        &self,
        branch: &BranchId,
    ) -> impl Future<Output = AppResult<(i64, Option<String>)>> + Send;

    /// The confirmed requests of the branch, newest first, at most `limit`.
    fn queue(
        &self,
        branch: &BranchId,
        limit: i64,
    ) -> impl Future<Output = AppResult<Vec<KioskRequest>>> + Send;

    fn switches(
        &self,
        branch: &BranchId,
    ) -> impl Future<Output = AppResult<Vec<CurrencySwitch>>> + Send;

    fn switches_in(
        &self,
        tx: Tx<'_, '_>,
        branch: &BranchId,
    ) -> impl Future<Output = AppResult<Vec<CurrencySwitch>>> + Send;

    /// One row per branch and currency; a second call replaces the first.
    fn set_switch_in(
        &self,
        tx: Tx<'_, '_>,
        branch: &BranchId,
        code: CurrencyCode,
        enabled: bool,
    ) -> impl Future<Output = AppResult<()>> + Send;

    fn receipt(
        &self,
        branch: &BranchId,
        id: &str,
    ) -> impl Future<Output = AppResult<Option<StoredReceipt>>> + Send;

    /// One receipt per request; publishing again replaces picture and time.
    fn put_receipt_in(
        &self,
        tx: Tx<'_, '_>,
        branch: &BranchId,
        id: &str,
        image: &ReceiptImage,
        at: KioskTime,
    ) -> impl Future<Output = AppResult<()>> + Send;
}
