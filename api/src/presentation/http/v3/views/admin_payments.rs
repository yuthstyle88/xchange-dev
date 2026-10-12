//! The `admin_payments` view (§7, §12.3).

use std::future::Future;
use std::path::Path;
use std::sync::Arc;

use actix_web::web;
use pos_error::AppResult;
use pos_infra::transactions::Transactor;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xc_rate::infrastructure::branch::branch_repository::PgBranchRepository;

use crate::application::settings::payment_settings::PaymentSettingsService;
use crate::bootstrap::AppState;
use crate::infrastructure::settings::branch_setting_store::BranchSettingStore;
use crate::presentation::dto::v3::admin_payments::{
    AdminPaymentsData, PaymentHistoryItem, PaymentMethodItem, export_types as export_dto_types,
};
use crate::presentation::dto::v3::common::NoFilter;
use crate::presentation::http::v3::view::{BranchView, ViewEndpoint};

struct AdminPaymentsBranchView {
    service: PaymentSettingsService,
}

impl AdminPaymentsBranchView {
    fn new(service: PaymentSettingsService) -> Self {
        Self { service }
    }
}

impl BranchView for AdminPaymentsBranchView {
    const NAME: &'static str = "admin_payments";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::PaymentMethods, "list");
    type Data = AdminPaymentsData;
    type Filter = NoFilter;

    fn load(
        &self,
        branch: &BranchId,
        who: &Actor,
        _filter: &Self::Filter,
    ) -> impl Future<Output = AppResult<Self::Data>> + Send {
        let service = self.service.clone();
        let branch = branch.clone();
        let who = who.clone();
        async move {
            let view = service.view(&branch, &who).await?;
            let methods: Vec<PaymentMethodItem> = serde_json::from_value(view.methods)
                .map_err(|e| pos_error::AppError::Domain(e.to_string()))?;
            let history: Vec<PaymentHistoryItem> = view
                .history
                .into_iter()
                .map(|e| {
                    let before_methods =
                        serde_json::from_value(e.before_methods).unwrap_or_default();
                    let after_methods =
                        serde_json::from_value(e.after_methods).unwrap_or_default();
                    PaymentHistoryItem {
                        version: e.version,
                        actor: e.actor,
                        created_at: e.created_at,
                        before_methods,
                        after_methods,
                    }
                })
                .collect();

            Ok(AdminPaymentsData {
                branch_id: view.branch_id,
                settings_version: view.version,
                methods,
                updated_at: view.updated_at,
                updated_by: view.updated_by,
                can_edit: view.can_edit,
                history,
            })
        }
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    let branch_tx = BranchTx::new(Transactor::new(state.db.clone()));
    let store = BranchSettingStore::new(state.db.clone());
    let branches = PgBranchRepository::new(state.db.clone());
    let service = PaymentSettingsService::new(branch_tx, store, branches);
    ViewEndpoint::register(cfg, Arc::new(AdminPaymentsBranchView::new(service)));
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    export_dto_types(dir)
}
