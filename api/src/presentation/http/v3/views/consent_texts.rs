//! Consent texts view (§4.4).

use std::future::Future;
use std::path::Path;
use std::sync::Arc;

use actix_web::web;
use chrono::{DateTime, Utc};
use pos_error::AppResult;
use sqlx::{PgPool, Row};
use ts_rs::TS;
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;

use crate::bootstrap::AppState;
use crate::presentation::dto::v3::common::NoFilter;
use crate::presentation::dto::v3::consent_texts::{ConsentTextViewItem, ConsentTextsData};
use crate::presentation::http::v3::view::{BranchView, ViewEndpoint};

pub struct ConsentTextsBranchView {
    pool: PgPool,
}

impl ConsentTextsBranchView {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl BranchView for ConsentTextsBranchView {
    const NAME: &'static str = "consent_texts";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "consentPolicy");
    const VERSIONED: bool = false;
    type Data = ConsentTextsData;
    type Filter = NoFilter;

    fn load(
        &self,
        branch: &BranchId,
        _who: &Actor,
        _filter: &Self::Filter,
    ) -> impl Future<Output = AppResult<Self::Data>> + Send {
        let pool = self.pool.clone();
        let branch_id = branch.as_str().to_string();

        async move {
            let rows = sqlx::query(
                "SELECT ct.purpose, ct.lang, ct.version, ct.body, ct.published_at \
                 FROM consent_texts ct \
                 JOIN branches b ON b.organization_id = ct.organization_id \
                 WHERE b.id = $1 \
                   AND ct.version = ( \
                       SELECT MAX(inner_ct.version) \
                       FROM consent_texts inner_ct \
                       WHERE inner_ct.organization_id = ct.organization_id \
                         AND inner_ct.purpose = ct.purpose \
                         AND inner_ct.lang = ct.lang \
                   ) \
                 ORDER BY ct.purpose ASC, ct.lang ASC",
            )
            .bind(&branch_id)
            .fetch_all(&pool)
            .await
            .map_err(pos_error::AppError::Database)?;

            let mut texts = Vec::with_capacity(rows.len());
            for row in rows {
                let purpose: String = row.get("purpose");
                let lang: String = row.get("lang");
                let version: i32 = row.get("version");
                let body: String = row.get("body");
                let published_at: Option<DateTime<Utc>> = row.get("published_at");
                texts.push(ConsentTextViewItem {
                    purpose,
                    lang,
                    version,
                    body,
                    published_at: published_at
                        .map(|dt| dt.to_rfc3339())
                        .unwrap_or_default(),
                });
            }

            Ok(ConsentTextsData { texts })
        }
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    ViewEndpoint::register(cfg, Arc::new(ConsentTextsBranchView::new(state.db.clone())));
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    ConsentTextViewItem::export_all(&cfg)?;
    ConsentTextsData::export_all(&cfg)?;
    Ok(())
}
