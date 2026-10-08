//! The `kiosk_queue` view (§7.7, §12.18.2).

use std::future::Future;
use std::path::Path;
use std::sync::Arc;

use actix_web::web;
use chrono::Utc;
use pos_error::AppResult;
use pos_infra::transactions::Transactor;
use sqlx::PgPool;
use xc_auth::domain::policy::entities::policy::Policy;
use xc_auth::domain::policy::value_objects::action::Action;
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xc_kernel::money::Side;
use xc_kiosk::application::kiosk::service::KioskService;
use xc_kiosk::domain::kiosk::value_objects::kiosk_status::KioskStatus;
use xc_kiosk::domain::kiosk::value_objects::queue_filter::QueueFilter;
use xc_rate::domain::rate::entities::rate_source::RateSource;
use xc_rate::infrastructure::rate::bot_rate_source::BotRateSource;

use crate::bootstrap::AppState;
use crate::infrastructure::devices::pg_device_repository::PgDeviceRepository;
use crate::infrastructure::kiosk::kiosk_ports::{KioskAuditLog, PgKioskAuditLog, PgKioskPorts};
use crate::presentation::dto::kiosk::kiosk_time_iso;
use crate::presentation::dto::v3::common::Money;
use crate::presentation::dto::v3::kiosk_queue::{
    KioskQueueFilter, KioskQueueRow, KioskQueueState, KioskQueueType,
};
use crate::presentation::http::kiosk::{username_or_id, usernames};
use crate::presentation::http::v3::view::{PagedView, RowId, ViewEndpoint};

struct KioskQueuePagedView {
    kiosk: KioskService,
    db: PgPool,
}

impl PagedView for KioskQueuePagedView {
    const NAME: &'static str = "kiosk_queue";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Kiosk, "list");
    type Row = KioskQueueRow;
    type Filter = KioskQueueFilter;

    fn page(
        &self,
        branch: &BranchId,
        who: &Actor,
        filter: &Self::Filter,
        before: Option<RowId>,
        limit: u16,
    ) -> impl Future<Output = AppResult<Vec<Self::Row>>> + Send {
        let branch = branch.clone();
        let who = who.clone();
        let filter_state = filter.state;
        let kiosk = self.kiosk.clone();
        let db = self.db.clone();
        async move {
            let queue_filter = match filter_state {
                None | Some(KioskQueueState::Waiting) => QueueFilter::Waiting,
                Some(KioskQueueState::Working) => QueueFilter::Working,
                Some(KioskQueueState::Completed) => QueueFilter::Completed,
            };

            let now = Utc::now();
            let items = kiosk
                .queue_before(
                    &branch,
                    queue_filter,
                    before.as_deref(),
                    limit as i64,
                    now,
                )
                .await?;

            let can_claim =
                Policy::allow(Some(&who), EntryPath::Kiosk, &Action::new("accept")).is_ok();

            let names = usernames(
                &db,
                items.iter().filter_map(|item| item.request.staff.as_deref()),
            )
            .await;

            let rows = items
                .into_iter()
                .map(|item| {
                    let request = &item.request;
                    let status = item.identity.status;
                    let held_by_caller = request.staff.as_deref() == Some(who.id());
                    let can_accept = can_claim && status == KioskStatus::Waiting;
                    let can_continue = can_claim
                        && held_by_caller
                        && matches!(status, KioskStatus::Accepted | KioskStatus::IdChecked);

                    let transaction_type = match request.quote.side {
                        Side::Buy => KioskQueueType::BUY,
                        Side::Sell => KioskQueueType::SELL,
                    };

                    let staff = request
                        .staff
                        .as_deref()
                        .map_or_else(String::new, |id| username_or_id(&names, id));

                    KioskQueueRow {
                        id: item.identity.id,
                        queue: item.identity.queue,
                        order_no: item.identity.order_no,
                        transaction_no: item
                            .identity
                            .transaction_no
                            .as_ref()
                            .map(|t| t.as_str().to_string()),
                        status: status.as_str().to_string(),
                        transaction_type,
                        currency: request.quote.currency.as_str().to_string(),
                        foreign: Money(request.quote.foreign.to_string()),
                        thb: Money(request.quote.thb.to_string()),
                        staff,
                        started_at: item.times.started_at.and_then(kiosk_time_iso),
                        completed_at: item.times.completed_at.and_then(kiosk_time_iso),
                        duration_seconds: item.times.duration_seconds(),
                        can_accept,
                        can_continue,
                    }
                })
                .collect();

            Ok(rows)
        }
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    let branch_tx = BranchTx::new(Transactor::new(state.db.clone()));
    let audit: Arc<dyn KioskAuditLog> =
        Arc::new(PgKioskAuditLog::new(PgDeviceRepository::new(state.db.clone())));
    let source: Arc<dyn RateSource> = Arc::new(BotRateSource::bot());
    let ports = Arc::new(PgKioskPorts::new(state.db.clone(), audit, source));
    let kiosk = KioskService::new(branch_tx, ports);

    let view = KioskQueuePagedView {
        kiosk,
        db: state.db.clone(),
    };
    ViewEndpoint::register_paged(cfg, Arc::new(view));
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    crate::presentation::dto::v3::kiosk_queue::export_types(dir)
}
