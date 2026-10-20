//! View `pricing_board` (pricing-tiers.md §6): one branch's pricing board. A page load or a click, not polled.
//!
//! SKELETON (PT-0): `load` is PT-7's. The view's name, permission (`ReferenceRatesRead`, through the reference-rates
//! route as `rates_reference` does) and data type stay.

use std::future::Future;
use std::path::Path;
use std::sync::Arc;

use actix_web::web;
use pos_error::AppResult;
use sqlx::PgPool;
use ts_rs::TS;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;

use crate::bootstrap::AppState;
use crate::presentation::dto::v3::common::NoFilter;
use crate::presentation::dto::v3::pricing_board::{
    PricingBandDto, PricingBoardData, PricingQuoteDto, PricingRecommendationDto, PricingRowDto,
    PricingSourceDto, PricingTierMarginDto, PricingTierRateDto,
};
use crate::presentation::http::v3::view::{BranchView, ViewEndpoint};

pub struct PricingBoardView {
    pool: PgPool,
}

impl PricingBoardView {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl BranchView for PricingBoardView {
    const NAME: &'static str = "pricing_board";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::OwnerRoute, "/api/reference-rates");
    const VERSIONED: bool = false;
    type Data = PricingBoardData;
    type Filter = NoFilter;

    fn load(
        &self,
        _branch: &BranchId,
        _who: &Actor,
        _filter: &Self::Filter,
    ) -> impl Future<Output = AppResult<Self::Data>> + Send {
        let _pool = self.pool.clone();
        async move { todo!("PT-7: pricing_board load") }
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    ViewEndpoint::register(cfg, Arc::new(PricingBoardView::new(state.db.clone())));
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    PricingBandDto::export_all(&cfg)?;
    PricingSourceDto::export_all(&cfg)?;
    PricingQuoteDto::export_all(&cfg)?;
    PricingTierRateDto::export_all(&cfg)?;
    PricingTierMarginDto::export_all(&cfg)?;
    PricingRowDto::export_all(&cfg)?;
    PricingRecommendationDto::export_all(&cfg)?;
    PricingBoardData::export_all(&cfg)?;
    Ok(())
}
