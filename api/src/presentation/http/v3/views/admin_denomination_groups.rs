//! Banknote groups admin view (§10.2).

use std::future::Future;
use std::path::Path;
use std::sync::Arc;

use actix_web::web;
use pos_error::AppResult;
use sqlx::{PgPool, Row};
use ts_rs::TS;
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;

use crate::bootstrap::AppState;
use crate::presentation::dto::v3::admin_denomination_groups::{
    AdminDenominationGroupsData, GroupCurrencyDto, GroupDto,
};
use crate::presentation::dto::v3::common::NoFilter;
use crate::presentation::http::v3::view::{BranchView, ViewEndpoint};

pub struct AdminDenominationGroupsView {
    pool: PgPool,
}

impl AdminDenominationGroupsView {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl BranchView for AdminDenominationGroupsView {
    const NAME: &'static str = "admin_denomination_groups";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "rateDraft");
    const VERSIONED: bool = false;
    type Data = AdminDenominationGroupsData;
    type Filter = NoFilter;

    fn load(
        &self,
        _branch: &BranchId,
        _who: &Actor,
        _filter: &Self::Filter,
    ) -> impl Future<Output = AppResult<Self::Data>> + Send {
        let pool = self.pool.clone();

        async move {
            let rows = sqlx::query(
                "SELECT \
                     trim(d.currency_code) AS currency, \
                     d.group_code AS group_name, \
                     s.instrument_key, \
                     bool_or(d.face_value IS NOT NULL) AS has_face_value, \
                     EXISTS ( \
                         SELECT 1 \
                         FROM published_rate_lines l \
                         JOIN branches b ON l.publication_id = b.current_publication_id \
                         WHERE l.currency_code = d.currency_code \
                           AND l.denomination_group = d.group_code \
                           AND l.tier_lower_exclusive_thb IS NULL \
                     ) AS is_priced \
                 FROM denominations d \
                 LEFT JOIN denomination_group_sources s \
                     ON s.currency_code = d.currency_code \
                    AND s.group_code = d.group_code \
                    AND s.provider = 'RATE_BOARD' \
                 GROUP BY d.currency_code, d.group_code, s.instrument_key \
                 ORDER BY currency ASC, \
                          CASE WHEN d.group_code = 'ALL' THEN 0 ELSE 1 END, \
                          d.group_code ASC",
            )
            .fetch_all(&pool)
            .await
            .map_err(pos_error::AppError::Database)?;

            let mut currencies: Vec<GroupCurrencyDto> = Vec::new();
            for row in rows {
                let currency: String = row.get("currency");
                let group_name: String = row.get("group_name");
                let instrument_key: Option<String> = row.get("instrument_key");
                let has_face_value: bool = row.get("has_face_value");
                let is_priced: bool = row.get("is_priced");

                let removable = group_name != "ALL" && !has_face_value && !is_priced;

                let group_dto = GroupDto {
                    group: group_name,
                    instrument_key,
                    removable,
                };

                if let Some(last) = currencies.last_mut()
                    && last.currency == currency
                {
                    last.groups.push(group_dto);
                } else {
                    currencies.push(GroupCurrencyDto {
                        currency,
                        groups: vec![group_dto],
                    });
                }
            }

            Ok(AdminDenominationGroupsData {
                provider: "RATE_BOARD".to_string(),
                currencies,
            })
        }
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    ViewEndpoint::register(
        cfg,
        Arc::new(AdminDenominationGroupsView::new(state.db.clone())),
    );
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    GroupDto::export_all(&cfg)?;
    GroupCurrencyDto::export_all(&cfg)?;
    AdminDenominationGroupsData::export_all(&cfg)?;
    Ok(())
}
