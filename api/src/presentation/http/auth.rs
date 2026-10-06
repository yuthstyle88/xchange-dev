//! Auth routes: `/api/auth` and the staff door.

use std::sync::Arc;
use std::time::SystemTime;

use actix_web::{HttpRequest, HttpResponse, web};
use serde_json::{Value, json};
use xc_auth::application::account::service::AccountService;
use xc_auth::domain::account::entities::employee::Employee;
use xc_auth::domain::account::entities::session::SignedIn;
use xc_auth::domain::account::value_objects::password_hasher::PasswordHasher;
use xc_auth::infrastructure::account::account_repository::PgAccountRepository;

use crate::bootstrap::AppState;
use crate::presentation::extractors::actor::{
    AccountSessionReader, Accounts, BAD_REQUEST, OWNER_COOKIE, OWNER_ID, SESSION_SECONDS,
    STAFF_COOKIE, accounts, answer, cookie, failure, payload_of, read, refuse, same_origin,
    server_error, set_cookie, signed_in,
};
use crate::presentation::http::client_address::client_address;
use crate::presentation::pages::{self, SessionReader};

pub fn configure(cfg: &mut web::ServiceConfig, state: &AppState) {
    let service: Accounts = AccountService::new(
        Arc::new(PgAccountRepository::new(state.db.clone())),
        PasswordHasher::new(),
    );
    let reader: Arc<dyn SessionReader> = Arc::new(AccountSessionReader {
        accounts: service.clone(),
    });
    cfg.app_data(web::Data::new(service));
    cfg.app_data(web::Data::from(reader));
    let body = || web::PayloadConfig::new(1 << 20);
    cfg.service(
        web::resource("/api/auth")
            .app_data(body())
            .route(web::post().to(auth))
            .default_service(web::to(pages::fallback)),
    );
    for (path, op) in [
        ("/api/staff/login", Op::Login),
        ("/api/staff/logout", Op::Logout),
        ("/api/staff/password", Op::Password),
        ("/api/staff/me", Op::Me),
    ] {
        cfg.service(
            web::resource(path)
                .app_data(body())
                .route(
                    web::post()
                        .to(move |req: HttpRequest, bytes: web::Bytes| staff(req, bytes, op)),
                )
                .default_service(web::to(pages::fallback)),
        );
    }
}

/// v2's `safe(employee)`.
pub fn employee_json(e: &Employee) -> Value {
    json!({ "id": e.id, "username": e.username, "name": e.name, "role": e.role,
            "branchId": e.branch_id, "active": e.active, "mustChange": e.must_change })
}

/// v2's `publicUser`.
fn user_json(who: &SignedIn) -> Value {
    match who {
        SignedIn::Owner(o) => {
            json!({ "id": OWNER_ID, "username": o.username, "name": o.name, "role": "OWNER", "branchId": null })
        }
        SignedIn::Employee(e) => {
            json!({ "id": e.id, "username": e.username, "name": e.name, "role": e.role, "branchId": e.branch_id })
        }
    }
}

async fn auth(req: HttpRequest, bytes: web::Bytes) -> HttpResponse {
    if !same_origin(&req) {
        return refuse(400, BAD_REQUEST);
    }
    let body = match read(&bytes, 4000, "ข้อมูลคำขอใหญ่เกินกำหนด")
    {
        Ok(body) => body,
        Err(response) => return response,
    };
    let Some(accounts) = accounts(&req) else {
        return server_error();
    };
    let payload = payload_of(&body);
    let ip = client_address(&req);
    let (owner, staff) = (cookie(&req, OWNER_COOKIE), cookie(&req, STAFF_COOKIE));
    let now = SystemTime::now().into();
    match body["action"].as_str() {
        Some("status") => match signed_in(accounts, &owner, &staff, SystemTime::now()).await {
            Ok(who) => answer(
                200,
                json!({ "setupRequired": false, "user": who.as_ref().map(user_json) }),
                true,
            ),
            Err(e) => failure(e),
        },
        Some("login") => match accounts.login(&payload, ip, now).await {
            Ok(login) => {
                let mut body = json!({ "user": user_json(&login.who) });
                let (set, clear) = match &login.who {
                    SignedIn::Owner(_) => (OWNER_COOKIE, STAFF_COOKIE),
                    SignedIn::Employee(e) => {
                        body["mustChange"] = json!(e.must_change);
                        (STAFF_COOKIE, OWNER_COOKIE)
                    }
                };
                let mut response = answer(200, body, true);
                append(
                    &mut response,
                    set_cookie(set, &login.token, SESSION_SECONDS),
                );
                append(&mut response, set_cookie(clear, "", 0));
                response
            }
            Err(e) => failure(e),
        },
        Some("logout") => match accounts.logout(&owner, &staff).await {
            Ok(()) => {
                let mut response = answer(200, json!({ "saved": true }), true);
                append(&mut response, set_cookie(OWNER_COOKIE, "", 0));
                append(&mut response, set_cookie(STAFF_COOKIE, "", 0));
                response
            }
            Err(e) => failure(e),
        },
        _ => refuse(400, BAD_REQUEST),
    }
}

fn append(
    response: &mut HttpResponse,
    (name, value): (actix_web::http::header::HeaderName, String),
) {
    response
        .headers_mut()
        .append(name, value.parse().expect("an ASCII cookie"));
}

#[derive(Clone, Copy)]
enum Op {
    Login,
    Logout,
    Password,
    Me,
}

async fn staff(req: HttpRequest, bytes: web::Bytes, op: Op) -> HttpResponse {
    if !same_origin(&req) {
        return refuse(400, BAD_REQUEST);
    }
    let body = match read(&bytes, 120_000, "ข้อมูลมากเกินไป") {
        Ok(body) => body,
        Err(response) => return response,
    };
    let Some(accounts) = accounts(&req) else {
        return server_error();
    };
    let payload = payload_of(&body);
    let token = cookie(&req, STAFF_COOKIE);
    let now = SystemTime::now().into();
    match op {
        Op::Login => {
            let ip = client_address(&req);
            match accounts.login_employee(&payload, ip, now).await {
                Ok(login) => {
                    let SignedIn::Employee(e) = &login.who else {
                        return server_error();
                    };
                    let mut response = answer(200, json!({ "employee": employee_json(e) }), true);
                    append(
                        &mut response,
                        set_cookie(STAFF_COOKIE, &login.token, SESSION_SECONDS),
                    );
                    response
                }
                Err(e) => failure(e),
            }
        }
        Op::Logout => match accounts.logout("", &token).await {
            Ok(()) => {
                let mut response = answer(200, json!({ "saved": true }), true);
                append(&mut response, set_cookie(STAFF_COOKIE, "", 0));
                response
            }
            Err(e) => failure(e),
        },
        Op::Password => match accounts.change_password(&token, &payload, now).await {
            Ok(()) => {
                let mut response = answer(200, json!({ "saved": true, "loginAgain": true }), true);
                append(&mut response, set_cookie(STAFF_COOKIE, "", 0));
                response
            }
            Err(e) => failure(e),
        },
        Op::Me => match accounts.employee_session(&token, true, now).await {
            Ok(e) => answer(200, json!({ "employee": employee_json(&e) }), true),
            Err(e) => failure(e),
        },
    }
}
