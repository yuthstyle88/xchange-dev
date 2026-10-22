//! The branch tablet's routes (design `kiosk-shared-phone.md` §7.7):
//! - `POST /api/branch-tablets` — staff with `SystemSettings` at the named branch
//!   (`EntryPath::DeviceSetup`), same-origin, authorised exactly as `device_setup.rs:97-131`:
//!   `{action: "list" | "add" | "newCode" | "revoke", payload}`;
//! - `POST /api/branch-tablet/pair` — public, same-origin: `{code}` ⇒ `200 {tabletId, branchCode,
//!   name}` and the tablet cookie, the token never in the body;
//! - `GET /api/branch-tablet/me` — public, reads the cookie: `{paired: false}` or
//!   `{paired: true, tabletId, branchCode, name}`, `Cache-Control: no-store`.
//!
//! Filled by BT-4, which also adds `branch_tablets::configure(cfg, state);` to the one route table
//! (`http/mod.rs` `configure`). Not mounted until then: no route answers today, so
//! `api/tests/branch_tablet_http_test.rs` fails on the missing routes.

use actix_web::{HttpRequest, HttpResponse, web};

use crate::bootstrap::AppState;

/// Routes of this module; registered through the one route table.
pub fn configure(cfg: &mut web::ServiceConfig, state: &AppState) {
    let _ = (cfg, state);
    todo!("BT-4: branch_tablets::configure")
}

/// `POST /api/branch-tablets`.
pub async fn branch_tablets_handler(req: HttpRequest, bytes: web::Bytes) -> HttpResponse {
    let _ = (req, bytes);
    todo!("BT-4: the staff actions")
}

/// `POST /api/branch-tablet/pair`.
pub async fn pair_handler(req: HttpRequest, bytes: web::Bytes) -> HttpResponse {
    let _ = (req, bytes);
    todo!("BT-4: pair")
}

/// `GET /api/branch-tablet/me`.
pub async fn me_handler(req: HttpRequest) -> HttpResponse {
    let _ = req;
    todo!("BT-4: me")
}
