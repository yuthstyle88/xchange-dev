//! The `my_notifications` view (§4.4, §7.7).

use std::future::Future;
use std::path::Path;
use std::sync::Arc;

use actix_web::web;
use pos_error::{AppError, AppResult};
use sqlx::PgPool;
use ts_rs::TS;
use uuid::Uuid;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;
use xc_notify::application::notification_service::{
    MyNotifications, MyNotificationsQuery, NotificationService,
};
use xc_notify::application::template_registry::TemplateRegistry;

use crate::bootstrap::AppState;
use crate::presentation::dto::v3::my_notifications::{
    MyNotificationItem, MyNotificationObject, MyNotificationsData, MyNotificationsFilter,
};
use crate::presentation::http::v3::view::{BranchView, ViewEndpoint};

pub struct MyNotificationsBranchView {
    service: NotificationService,
    pool: PgPool,
}

impl MyNotificationsBranchView {
    pub fn new(pool: PgPool) -> Self {
        let registry = Arc::new(TemplateRegistry::generated());
        Self {
            service: NotificationService::new(pool.clone(), registry),
            pool,
        }
    }
}

impl BranchView for MyNotificationsBranchView {
    const NAME: &'static str = "my_notifications";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "myNotifications");
    const VERSIONED: bool = true;
    type Data = MyNotificationsData;
    type Filter = MyNotificationsFilter;

    fn own_version(
        &self,
        _branch: &BranchId,
        who: &Actor,
    ) -> impl Future<Output = AppResult<Option<i64>>> + Send {
        let pool = self.pool.clone();
        let user_id = who.user_id.clone();
        async move {
            let user_uuid = Uuid::parse_str(&user_id)
                .map_err(|e| AppError::Validation(format!("invalid user_id: {e}")))?;
            let version: i64 = sqlx::query_scalar(
                "SELECT COALESCE(MAX(seq), 0) + COUNT(read_at) FROM notifications WHERE recipient_user_id = $1",
            )
            .bind(user_uuid)
            .fetch_one(&pool)
            .await
            .map_err(AppError::Database)?;
            Ok(Some(version))
        }
    }

    fn load(
        &self,
        _branch: &BranchId,
        who: &Actor,
        filter: &Self::Filter,
    ) -> impl Future<Output = AppResult<Self::Data>> + Send {
        let service = self.service.clone();
        let user_id = who.user_id.clone();
        let unread_only = filter.unread.unwrap_or(false);

        async move {
            let res = service
                .my_notifications(
                    &user_id,
                    &MyNotificationsQuery {
                        unread_only,
                        since: None,
                    },
                )
                .await
                .map_err(|e| match e {
                    xc_notify::domain::error::NotifyError::Store(err) => err,
                    other => AppError::Domain(other.to_string()),
                })?;

            let items = match res {
                MyNotifications::Changed { items, .. } => items,
                MyNotifications::Unchanged { .. } => Vec::new(),
            };

            let mapped: Vec<MyNotificationItem> = items
                .into_iter()
                .map(|item| MyNotificationItem {
                    id: item.id.0,
                    seq: item.seq,
                    locale: item.locale.as_str().to_string(),
                    title: item.title,
                    body: item.body,
                    object: item.object.map(|o| MyNotificationObject {
                        object_type: o.object_type,
                        object_id: o.object_id,
                    }),
                    read: item.read,
                    created_at: item.created_at.to_rfc3339(),
                })
                .collect();

            Ok(MyNotificationsData { items: mapped })
        }
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    let view = MyNotificationsBranchView::new(state.db.clone());
    ViewEndpoint::register(cfg, Arc::new(view));
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    MyNotificationObject::export_all(&cfg)?;
    MyNotificationItem::export_all(&cfg)?;
    MyNotificationsData::export_all(&cfg)?;
    MyNotificationsFilter::export_all(&cfg)?;
    Ok(())
}
