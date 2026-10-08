//! The `me` view (§7.6).

use std::future::Future;
use std::path::Path;
use std::sync::Arc;

use actix_web::web;
use pos_error::AppResult;
use ts_rs::TS;
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_auth::domain::policy::value_objects::actor_kind::ActorKind;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;

use crate::bootstrap::AppState;
use crate::presentation::dto::v3::common::Me;
use crate::presentation::dto::v3::me::MeView;
use crate::presentation::http::v3::view::{BranchView, ViewEndpoint};

struct MeBranchView;

impl BranchView for MeBranchView {
    const NAME: &'static str = "me";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "adminSnapshot");
    type Data = MeView;

    fn load(
        &self,
        branch: &BranchId,
        who: &Actor,
    ) -> impl Future<Output = AppResult<Self::Data>> + Send {
        let me = Me {
            kind: match who.kind() {
                ActorKind::Owner => "owner",
                ActorKind::Staff => "staff",
            }
            .to_owned(),
            id: who.id().to_owned(),
            role: who.role().as_str().to_owned(),
            branch: branch.as_str().to_owned(),
        };
        async move { Ok(MeView(me)) }
    }
}

pub fn register(cfg: &mut web::ServiceConfig, _state: &AppState) {
    ViewEndpoint::register(cfg, Arc::new(MeBranchView));
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    MeView::export_all(&cfg)?;
    Ok(())
}
