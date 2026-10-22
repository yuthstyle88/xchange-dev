//! HTTP route handlers.
pub mod app_receipts;
pub mod app_verification_upload;
pub mod auth;
pub mod branch_tablets;
pub mod branch_verification_intake;
pub mod client_address;
pub mod customer_register;
pub mod customers;
pub mod device_agent;
pub mod device_setup;
pub mod devices;
pub mod drawer_passport;
pub mod files;
pub mod health;
pub mod kiosk;
pub mod payment_methods;
pub mod public;
pub mod reference_rates;
pub mod rpc;
pub mod users;
pub mod v3;
pub mod verification_content;

use actix_web::web;

use crate::bootstrap::AppState;

/// The one route table. No other place registers a route.
pub fn configure(cfg: &mut web::ServiceConfig, state: &AppState) {
    health::configure(cfg);
    auth::configure(cfg, state);
    customer_register::configure(cfg);
    app_receipts::configure(cfg, state);
    branch_tablets::configure(cfg, state);
    branch_verification_intake::configure(cfg, state);
    customers::configure(cfg, state);
    device_agent::configure(cfg, state);
    device_setup::configure(cfg, state);
    devices::configure(cfg, state);
    drawer_passport::configure(cfg, state);
    files::configure(cfg, state);
    verification_content::configure(cfg, state);
    app_verification_upload::configure(cfg, state);
    kiosk::configure(cfg, state);
    payment_methods::configure(cfg, state);
    public::configure(cfg, state);
    users::configure(cfg, state);
    reference_rates::configure(cfg, state);
    rpc::configure(cfg, state);
    v3::configure_v3(cfg, state);
}
