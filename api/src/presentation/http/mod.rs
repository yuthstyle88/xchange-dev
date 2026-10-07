//! HTTP route handlers.
pub mod auth;
pub mod bookings;
pub mod client_address;
pub mod customers;
pub mod device_agent;
pub mod device_setup;
pub mod devices;
pub mod documents;
pub mod employees;
pub mod files;
pub mod health;
pub mod kiosk;
pub mod reference_rates;
pub mod rpc;

use actix_web::web;

use crate::bootstrap::AppState;

/// The one route table. No other place registers a route.
pub fn configure(cfg: &mut web::ServiceConfig, state: &AppState) {
    health::configure(cfg);
    auth::configure(cfg, state);
    bookings::configure(cfg, state);
    customers::configure(cfg, state);
    device_agent::configure(cfg, state);
    device_setup::configure(cfg, state);
    devices::configure(cfg, state);
    documents::configure(cfg, state);
    employees::configure(cfg, state);
    files::configure(cfg, state);
    kiosk::configure(cfg, state);
    reference_rates::configure(cfg, state);
    rpc::configure(cfg, state);
}
