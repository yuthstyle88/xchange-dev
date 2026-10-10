//! v3 HTTP presentation module (§7.7).

pub mod action;
pub mod command;
pub mod commands;
pub mod desk;
pub mod error;
pub mod export_gate;
pub mod org_view;
pub mod system;
pub mod view;

use actix_web::web;

use crate::bootstrap::AppState;

pub struct ViewEntry {
    pub name: &'static str,
    pub register: fn(&mut web::ServiceConfig, &AppState),
    pub export_types: fn(&std::path::Path) -> Result<(), ts_rs::ExportError>,
}

include!(concat!(env!("OUT_DIR"), "/v3_views.rs"));
include!(concat!(env!("OUT_DIR"), "/v3_actions.rs"));
include!(concat!(env!("OUT_DIR"), "/v3_commands.rs"));

pub fn configure_v3(cfg: &mut web::ServiceConfig, state: &AppState) {
    for v in VIEWS {
        (v.register)(cfg, state);
    }
    for a in ACTIONS {
        (a.register)(cfg, state);
    }
    for c in COMMANDS {
        (c.register)(cfg, state);
    }
    system::broker_exchanges::register(cfg, state);
}
