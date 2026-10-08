//! The `kiosk_notifications` view (§7.7, §12.18.2).

use std::future::Future;
use std::path::Path;
use std::sync::Arc;

use actix_web::web;
use chrono::{SecondsFormat, Utc};
use pos_error::AppResult;
use pos_infra::transactions::Transactor;
use sqlx::PgPool;
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xc_kiosk::application::kiosk::service::KioskService;
use xc_kiosk::domain::kiosk::value_objects::kiosk_actor::KioskActor;
use xc_rate::domain::rate::entities::rate_source::RateSource;
use xc_rate::infrastructure::rate::bot_rate_source::BotRateSource;

use crate::bootstrap::AppState;
use crate::infrastructure::devices::pg_device_repository::PgDeviceRepository;
use crate::infrastructure::kiosk::kiosk_ports::{KioskAuditLog, PgKioskAuditLog, PgKioskPorts};
use crate::presentation::dto::kiosk::{kiosk_time_iso, queue_of};
use crate::presentation::dto::v3::common::NoFilter;
use crate::presentation::dto::v3::kiosk_notifications::{
    KioskNotifications, KioskNotificationsEmployee, KioskNotificationsWorking,
};
use crate::presentation::http::kiosk::{username_or_id, usernames};
use crate::presentation::http::v3::view::{BranchView, ViewEndpoint};

struct KioskNotificationsBranchView {
    kiosk: KioskService,
    db: PgPool,
}

impl BranchView for KioskNotificationsBranchView {
    const NAME: &'static str = "kiosk_notifications";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Kiosk, "notifications");
    type Data = KioskNotifications;
    type Filter = NoFilter;

    fn load(
        &self,
        branch: &BranchId,
        who: &Actor,
        _filter: &Self::Filter,
    ) -> impl Future<Output = AppResult<Self::Data>> + Send {
        let branch = branch.clone();
        let who = who.clone();
        let kiosk = self.kiosk.clone();
        let db = self.db.clone();
        async move {
            let kiosk_actor = KioskActor::Customer;
            let notifications = kiosk.notifications(&branch, &kiosk_actor).await?;
            let names = usernames(
                &db,
                notifications.working.iter().map(|w| w.staff.as_str()),
            )
            .await;
            let working = notifications
                .working
                .into_iter()
                .map(|item| KioskNotificationsWorking {
                    id: item.id.clone(),
                    queue: queue_of(&item.id),
                    staff: username_or_id(&names, &item.staff),
                    started_at: item.started_at.and_then(kiosk_time_iso),
                    status: item.status.as_str().to_string(),
                })
                .collect();

            let employee = KioskNotificationsEmployee {
                id: who.id().to_owned(),
                role: who.role().as_str().to_owned(),
            };

            let server_now = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);

            Ok(KioskNotifications {
                server_now,
                branch_id: branch.as_str().to_owned(),
                branch_name: notifications.branch_name,
                count: notifications.count,
                latest: notifications.latest,
                working_count: notifications.working_count,
                working,
                employee,
            })
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

    let view = KioskNotificationsBranchView {
        kiosk,
        db: state.db.clone(),
    };
    ViewEndpoint::register(cfg, Arc::new(view));
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    crate::presentation::dto::v3::kiosk_notifications::export_types(dir)
}
