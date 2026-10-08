//! v3 HTTP presentation module (§7.7).

pub mod error;
pub mod view;

use actix_web::web;

use crate::bootstrap::AppState;

pub struct ViewEntry {
    pub name: &'static str,
    pub register: fn(&mut web::ServiceConfig, &AppState),
    pub export_types: fn(&std::path::Path) -> Result<(), ts_rs::ExportError>,
}

include!(concat!(env!("OUT_DIR"), "/v3_views.rs"));

pub fn configure_v3(cfg: &mut web::ServiceConfig, state: &AppState) {
    for v in VIEWS {
        (v.register)(cfg, state);
    }
}
