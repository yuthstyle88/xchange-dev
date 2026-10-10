//! Documents routes: the two pages that list the documents of one exchange.

use std::sync::Arc;

use actix_web::http::header;
use actix_web::{HttpRequest, HttpResponse, web};
use pos_error::AppError;
use pos_infra::transactions::Transactor;
use serde_json::{Value, json};
use xc_auth::domain::policy::entities::policy::Policy;
use xc_auth::domain::policy::value_objects::action::Action;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_exchange::application::exchange::service::TransactionService;
use xc_exchange::infrastructure::exchange::transaction_repository::PgTransactionRepository;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xc_rate::application::branch::service::BranchService;

use crate::application::compat::staff_view::rewrite_urls;
use crate::application::workflows::documents::{Documents, DocumentsLookup, PASSPORT, RECEIPT};
use crate::bootstrap::AppState;
use crate::infrastructure::files::file_service::{BAD_BRANCH, FileService};
use crate::presentation::pages;

const SERVER_ERROR: &str = "เซิร์ฟเวอร์ขัดข้อง";

/// Routes of this module; registered through the one route table.
pub fn configure(cfg: &mut web::ServiceConfig, state: &AppState) {
    let branch_tx = || BranchTx::new(Transactor::new(state.db.clone()));
    let lookup = DocumentsLookup::new(
        TransactionService::new(
            Arc::new(PgTransactionRepository::new(state.db.clone())),
            branch_tx(),
        ),
        BranchService::new(branch_tx()),
        FileService::new(state.db.clone(), state.settings.files.dir.clone()),
    );
    cfg.app_data(web::Data::new(lookup));
    cfg.service(
        web::resource("/documents")
            .route(web::get().to(documents))
            .default_service(web::to(pages::fallback)),
    );
    cfg.service(
        web::resource("/staff/documents")
            .route(web::get().to(staff_documents))
            .default_service(web::to(pages::fallback)),
    );
}

fn own_origin(req: &HttpRequest) -> String {
    let https = req
        .headers()
        .get("x-forwarded-proto")
        .is_some_and(|v| v.as_bytes() == b"https");
    let host = req
        .headers()
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("localhost");
    format!("{}://{}", if https { "https" } else { "http" }, host)
}

fn redirect(req: &HttpRequest, status: u16, path: &str) -> HttpResponse {
    HttpResponse::build(actix_web::http::StatusCode::from_u16(status).unwrap())
        .insert_header((header::LOCATION, format!("{}{path}", own_origin(req))))
        .finish()
}

/// Of a repeated query name the first counts, as v2's `searchParams.get` reads it.
fn query(req: &HttpRequest, name: &str) -> Option<String> {
    web::Query::<Vec<(String, String)>>::from_query(req.query_string())
        .ok()
        .and_then(|pairs| {
            pairs
                .into_inner()
                .into_iter()
                .find(|(k, _)| k == name)
                .map(|(_, v)| v)
        })
}

/// v2's `esc3`.
fn esc(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

/// JavaScript's `encodeURIComponent`.
fn encode_component(text: &str) -> String {
    let mut out = String::new();
    for b in text.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn ending(mime: &str) -> &'static str {
    match mime {
        "image/svg+xml" => "svg",
        "image/jpeg" => "jpg",
        "image/png" => "png",
        "image/webp" => "webp",
        "application/pdf" => "pdf",
        "text/html" => "html",
        _ => "bin",
    }
}

fn owners_page(found: &Documents, branch: &str) -> String {
    let reference = esc(&found.document_ref);
    let mut items = String::new();
    for link in &found.links {
        let kind = match link.label.as_str() {
            PASSPORT => "PASSPORT",
            RECEIPT => "RECEIPT_FULL",
            _ => "PAYMENT",
        };
        let name = esc(&format!(
            "{}_{kind}.{}",
            found.document_ref,
            ending(&link.mime)
        ));
        let href = esc(&link.url);
        items.push_str(&format!(
            "<li>{} — <a href=\"{href}\" target=\"_blank\" rel=\"noopener\">เปิดดู</a> · <a href=\"{href}\" download=\"{name}\">ดาวน์โหลด {name}</a></li>",
            esc(&link.label)
        ));
    }
    format!(
        "<!doctype html><html lang=\"th\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>เอกสาร {reference}</title><style>body{{font:18px/1.7 Tahoma,sans-serif;max-width:700px;margin:40px auto;padding:20px}}a{{color:#ab0044}}li{{margin:20px 0}}</style></head><body><h1>เอกสารรายการ</h1><p>{reference} · ใบเสร็จและพาสปอร์ตผูกกับรหัสออเดอร์เดียวกัน</p><ul>{items}</ul><a href=\"/?branch={}#transactions\">กลับรายการแลกเงิน</a></body></html>",
        encode_component(branch)
    )
}

async fn documents(req: HttpRequest) -> HttpResponse {
    let callers = pages::get_callers(&req).await;
    let now = chrono::Utc::now();
    let is_staff =
        callers.owner.is_none() && (callers.staff.is_some() || callers.staff_user.is_some());
    let named_branch = if is_staff {
        None
    } else {
        query(&req, "branch")
            .filter(|b| !b.is_empty())
            .map(BranchId::new)
    };

    let grant = match Policy::authorize(
        callers.user.as_ref(),
        EntryPath::UserRoute,
        &Action::new("documents"),
        named_branch.as_ref(),
        now,
    ) {
        Ok(g) => g,
        Err(_) => return redirect(&req, 303, "/login"),
    };

    let branch = grant.branch.as_str().to_owned();

    // v2's `execute` refuses a name that cannot be a branch id before anything runs
    if !pages::is_valid_branch(&branch) {
        return HttpResponse::NotFound()
            .insert_header((header::CONTENT_TYPE, "text/plain;charset=UTF-8"))
            .body(BAD_BRANCH.to_owned());
    }
    let Some(lookup) = req.app_data::<web::Data<DocumentsLookup>>() else {
        return server_error(&AppError::Internal("no DocumentsLookup".into()));
    };
    let reference = query(&req, "ref").unwrap_or_default();
    match lookup.find(&grant.branch, &reference).await {
        Ok(found) => HttpResponse::Ok()
            .insert_header((header::CONTENT_TYPE, "text/html; charset=utf-8"))
            .insert_header((header::CACHE_CONTROL, "no-store"))
            .insert_header((header::X_CONTENT_TYPE_OPTIONS, "nosniff"))
            .body(owners_page(&found, &branch)),
        Err(AppError::Domain(sentence)) => HttpResponse::NotFound()
            .insert_header((header::CONTENT_TYPE, "text/plain;charset=UTF-8"))
            .body(sentence),
        Err(other) => server_error(&other),
    }
}

fn server_error(error: &AppError) -> HttpResponse {
    tracing::error!(error = %error, "documents request failed");
    HttpResponse::InternalServerError()
        .content_type("application/json; charset=utf-8")
        .body(json!({ "error": SERVER_ERROR }).to_string())
}

fn staff_page(found: &Documents) -> String {
    let mut links = String::new();
    for link in &found.links {
        // the staff way in to a file: the staff path's own rewriting of links (T11s)
        let href = match rewrite_urls(Value::String(link.url.clone())) {
            Value::String(href) => href,
            _ => link.url.clone(),
        };
        links.push_str(&format!(
            "<p><a href=\"{}\">{}</a></p>",
            esc(&href),
            esc(&link.label)
        ));
    }
    format!(
        "<!doctype html><html lang=\"th\"><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width\"><title>เอกสารรายการ</title><body style=\"font-family:sans-serif;padding:32px\"><h1>{}</h1>{links}<a href=\"/staff\">กลับหน้าพนักงาน</a></body></html>",
        esc(&found.document_ref)
    )
}

async fn staff_documents(req: HttpRequest) -> HttpResponse {
    let callers = pages::get_callers(&req).await;
    let now = chrono::Utc::now();
    let grant = match Policy::authorize(
        callers.staff_user.as_ref(),
        EntryPath::StaffRpc,
        &Action::new("documents"),
        None,
        now,
    ) {
        Ok(grant) => grant,
        Err(_) => return redirect(&req, 302, "/staff"),
    };
    let Some(lookup) = req.app_data::<web::Data<DocumentsLookup>>() else {
        return redirect(&req, 302, "/staff");
    };
    let reference = query(&req, "ref").unwrap_or_default();
    match lookup.find(&grant.branch, &reference).await {
        Ok(found) => HttpResponse::Ok()
            .insert_header((header::CONTENT_TYPE, "text/html; charset=utf-8"))
            .insert_header((header::CACHE_CONTROL, "private, no-store"))
            .body(staff_page(&found)),
        // v2 sends every failure of this page back to the staff page
        Err(_) => redirect(&req, 302, "/staff"),
    }
}
