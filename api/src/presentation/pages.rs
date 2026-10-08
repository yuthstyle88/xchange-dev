//! Pages presentation.

use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::str::FromStr;

use actix_web::http::Method;
use actix_web::{HttpRequest, HttpResponse, web};
use rust_decimal::{Decimal, RoundingStrategy};
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_auth::domain::policy::value_objects::role::Role;
use xc_kernel::ids::BranchId;

use crate::bootstrap::{AppState, BuildInfo};

/// The sessions one request carries. A browser can hold an owner session and a
/// staff session at the same time.
pub struct Sessions {
    /// `Actor::owner(id)` when the request carries a valid owner session.
    pub owner: Option<Actor>,
    /// `Actor::staff(id, role, branch)` when it carries a valid staff session.
    pub staff: Option<Actor>,
}

/// Reads the sessions of a request. Declared here until ticket T4c, which
/// implements it in `presentation/extractors/actor.rs`; it may move there then.
pub trait SessionReader: Send + Sync {
    fn sessions<'a>(&'a self, req: &'a HttpRequest)
    -> Pin<Box<dyn Future<Output = Sessions> + 'a>>;
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PageKind {
    Branch,
    Plain,
    DeskVariant,
}

struct PageSpec {
    path: &'static str,
    file: &'static str,
    kind: PageKind,
    who_signed_in: bool,
    scripts: &'static [&'static str],
}

const DESK_SCRIPTS: &[&str] = &[
    "/desk-skip.js",
    "/desk-devices.js",
    "/desk-kiosk.js",
    "/account-bar.js",
    "/_xc/desk.js",
];

const STAFF_DESK_SCRIPT: &str = "<script>let staffDraftId='';const staffGasOriginal=gasCall;gasCall=async function(name,payload){if(name==='reserveCustomer'&&!staffDraftId){const draft=await staffGasOriginal('opsBeginTransaction',{type:s.type,currency:s.currency,amount:Number(s.amount)});staffDraftId=draft.id;}if(name==='saveTransaction')payload={...payload,staffDraftId};return staffGasOriginal(name,payload);};</script>";

const PAGE_SPECS: &[PageSpec] = &[
    PageSpec {
        path: "/",
        file: "admin.html",
        kind: PageKind::Branch,
        who_signed_in: true,
        scripts: &["/admin-devices.js", "/account-bar.js"],
    },
    PageSpec {
        path: "/desk",
        file: "desk.html",
        kind: PageKind::Branch,
        who_signed_in: true,
        scripts: DESK_SCRIPTS,
    },
    PageSpec {
        path: "/receipt-sample",
        file: "receipt-sample.html",
        kind: PageKind::Branch,
        who_signed_in: true,
        scripts: &[],
    },
    PageSpec {
        path: "/staff",
        file: "staff.html",
        kind: PageKind::Plain,
        who_signed_in: false,
        scripts: &[],
    },
    PageSpec {
        path: "/staff/report",
        file: "staff-report.html",
        kind: PageKind::Plain,
        who_signed_in: false,
        scripts: &[],
    },
    PageSpec {
        path: "/login",
        file: "login.html",
        kind: PageKind::Plain,
        who_signed_in: false,
        scripts: &[],
    },
    PageSpec {
        path: "/kiosk",
        file: "kiosk.html",
        kind: PageKind::Plain,
        who_signed_in: false,
        scripts: &[],
    },
    PageSpec {
        path: "/devices",
        file: "devices.html",
        kind: PageKind::Plain,
        who_signed_in: true,
        scripts: &["/account-bar.js"],
    },
    PageSpec {
        path: "/kiosk-queue.html",
        file: "kiosk-queue.html",
        kind: PageKind::Plain,
        who_signed_in: true,
        scripts: &["/account-bar.js"],
    },
    PageSpec {
        path: "/kiosk-currencies.html",
        file: "kiosk-currencies.html",
        kind: PageKind::Plain,
        who_signed_in: true,
        scripts: &["/account-bar.js"],
    },
    PageSpec {
        path: "/staff-guide",
        file: "staff-guide.html",
        kind: PageKind::Plain,
        who_signed_in: true,
        scripts: &["/account-bar.js"],
    },
    PageSpec {
        path: "/staff-guide.html",
        file: "staff-guide.html",
        kind: PageKind::Plain,
        who_signed_in: true,
        scripts: &["/account-bar.js"],
    },
    PageSpec {
        path: "/staff/desk",
        file: "desk.html",
        kind: PageKind::DeskVariant,
        who_signed_in: false,
        scripts: DESK_SCRIPTS,
    },
];

/// Page routes; registers the page routes of §8.1.
pub fn configure(cfg: &mut web::ServiceConfig, _state: &AppState) {
    cfg.service(
        web::resource("/_xc/forms.js")
            .route(web::get().to(forms_script))
            .default_service(web::to(fallback)),
    );
    compiled_script(cfg, "/_xc/desk.js", include_str!("../../static/xc-desk.js"));

    cfg.service(
        web::resource("/devices/receipt")
            .route(web::get().to(devices_receipt_handler))
            .default_service(web::to(fallback)),
    );

    for spec in PAGE_SPECS {
        cfg.service(
            web::resource(spec.path)
                .route(web::get().to(page_handler))
                .default_service(web::to(fallback)),
        );
    }
}

struct ReceiptQueryParams {
    branch: Option<String>,
    reference: Option<String>,
}

fn parse_receipt_query(query_str: &str) -> Result<ReceiptQueryParams, HttpResponse> {
    if query_str.is_empty() {
        return Ok(ReceiptQueryParams {
            branch: None,
            reference: None,
        });
    }
    let pairs: Vec<(String, String)> = web::Query::from_query(query_str)
        .map(|q: web::Query<Vec<(String, String)>>| q.into_inner())
        .map_err(|_| {
            HttpResponse::BadRequest()
                .content_type("text/plain; charset=utf-8")
                .body("Invalid branch")
        })?;

    let mut branch = None;
    let mut reference = None;
    for (k, v) in pairs {
        if k == "branch" {
            branch = Some(v);
        } else if k == "ref" {
            reference = Some(v);
        }
    }

    Ok(ReceiptQueryParams { branch, reference })
}

fn format_rate(rate: Decimal) -> String {
    rate.normalize().to_string()
}

fn to_locale_string(d: Decimal) -> String {
    let rounded = d.round_dp_with_strategy(3, RoundingStrategy::MidpointAwayFromZero);
    let normalized = rounded.normalize();
    let s = normalized.to_string();
    let (int_part, frac_part) = match s.split_once('.') {
        Some((i, f)) => (i, Some(f)),
        None => (s.as_str(), None),
    };

    let is_negative = int_part.starts_with('-');
    let digits = if is_negative {
        &int_part[1..]
    } else {
        int_part
    };

    let mut formatted_int = String::with_capacity(digits.len() + digits.len() / 3);
    let rem = digits.len() % 3;
    if rem > 0 {
        formatted_int.push_str(&digits[..rem]);
    }
    for chunk in digits.as_bytes()[rem..].chunks(3) {
        if !formatted_int.is_empty() {
            formatted_int.push(',');
        }
        if let Ok(s) = std::str::from_utf8(chunk) {
            formatted_int.push_str(s);
        }
    }
    if is_negative {
        formatted_int.insert(0, '-');
    }
    if formatted_int.is_empty() {
        formatted_int.push('0');
    }

    match frac_part {
        Some(f) if !f.is_empty() => format!("{formatted_int}.{f}"),
        _ => formatted_int,
    }
}

fn server_error() -> HttpResponse {
    HttpResponse::InternalServerError()
        .content_type("application/json; charset=utf-8")
        .body("{\"error\":\"เซิร์ฟเวอร์ขัดข้อง\"}")
}

async fn devices_receipt_handler(req: HttpRequest) -> HttpResponse {
    if req.method() != Method::GET {
        return plain_404();
    }

    let sessions = sessions_of(&req).await;
    if sessions.owner.is_none() && sessions.staff.is_none() {
        return to_login(&req);
    }

    let query = match parse_receipt_query(req.query_string()) {
        Ok(q) => q,
        Err(resp) => return resp,
    };

    let url_branch = match query.branch.as_deref() {
        Some("") | None => "MAIN",
        Some(b) => b,
    };

    if !is_valid_branch(url_branch) {
        return HttpResponse::BadRequest()
            .content_type("text/plain; charset=utf-8")
            .body("Invalid branch");
    }

    let ref_str = match query.reference.as_deref() {
        Some(r) if !r.is_empty() => r,
        _ => {
            return HttpResponse::NotFound()
                .content_type("text/plain; charset=utf-8")
                .body("ไม่พบรายการ");
        }
    };

    let target_branch = if sessions.owner.is_some() {
        url_branch
    } else if let Some(staff) = &sessions.staff {
        staff.branch_id().map(|b| b.as_str()).unwrap_or("MAIN")
    } else {
        url_branch
    };

    let Some(state) = req.app_data::<web::Data<AppState>>() else {
        tracing::error!("missing AppState in devices_receipt_handler");
        return server_error();
    };

    let branch_row: Option<(String,)> =
        match sqlx::query_as("SELECT name FROM branches WHERE id = $1")
            .bind(target_branch)
            .fetch_optional(&state.db)
            .await
        {
            Ok(row) => row,
            Err(e) => {
                tracing::error!("database error querying branch {target_branch}: {e}");
                return server_error();
            }
        };

    let Some((branch_name,)) = branch_row else {
        return HttpResponse::NotFound()
            .content_type("text/plain; charset=utf-8")
            .body("ไม่พบรายการ");
    };

    let tx_row: Option<(String, String, String, String, String, String)> = match sqlx::query_as(
        "SELECT customer_id, type, code, amount::text, rate::text, fee::text \
         FROM transactions \
         WHERE branch_id = $1 AND document_ref = $2 AND mode = 'LIVE'",
    )
    .bind(target_branch)
    .bind(ref_str)
    .fetch_optional(&state.db)
    .await
    {
        Ok(row) => row,
        Err(e) => {
            tracing::error!("database error querying transaction {ref_str}: {e}");
            return server_error();
        }
    };

    let Some((customer_id, tx_type, code, amount_str, rate_str, fee_str)) = tx_row else {
        return HttpResponse::NotFound()
            .content_type("text/plain; charset=utf-8")
            .body("ไม่พบรายการ");
    };

    let amount = Decimal::from_str(&amount_str).unwrap_or_default();
    let rate = Decimal::from_str(&rate_str).unwrap_or_default();
    let fee = Decimal::from_str(&fee_str).unwrap_or_default();

    let base = amount * rate;
    let (verb, thb) = if tx_type == "BUY" {
        ("จ่าย", base - fee)
    } else {
        ("รับ", base + fee)
    };

    let amount_formatted = to_locale_string(amount);
    let rate_formatted = escape_html(&format_rate(rate));
    let fee_formatted = to_locale_string(fee);
    let thb_formatted = to_locale_string(thb);

    const STYLE: &str = "<style>@page{size:80mm 285mm;margin:4mm}body{box-sizing:border-box;width:72mm;\
    margin:0 auto;overflow-wrap:anywhere;font:14px/1.7 Tahoma,sans-serif}h1{text-align:center;\
    font-size:20px}hr{border:0;border-top:1px dashed #333}</style>";
    const FOOTER: &str = "<hr><p>ตัวอย่างก่อนพิมพ์ · พื้นที่พิมพ์ 72 มม. / กระดาษ 80 มม.</p>";

    let escaped_ref = escape_html(ref_str);
    let escaped_branch_name = escape_html(&branch_name);
    let escaped_customer = escape_html(&customer_id);
    let escaped_type = escape_html(&tx_type);
    let escaped_currency = escape_html(code.trim());

    let lines = format!(
        "ลูกค้า {escaped_customer}<br>{escaped_type} {amount_formatted} {escaped_currency}<br>เรท {rate_formatted}<br>ค่าธรรมเนียม {fee_formatted} THB<br>{verb} {thb_formatted} THB"
    );
    let html = format!(
        "<!doctype html><html lang=\"th\"><meta charset=\"utf-8\"><title>ตัวอย่างสลิป {escaped_ref}</title>{STYLE}\
        <h1>XCHANGE × T.K.</h1><hr><p>สาขา {escaped_branch_name}<br>เลขรายการ {escaped_ref}<br>{lines}</p>{FOOTER}</html>"
    );

    HttpResponse::Ok()
        .content_type("text/html; charset=utf-8")
        .insert_header(("Cache-Control", "no-store"))
        .body(html)
}

async fn page_handler(req: HttpRequest) -> HttpResponse {
    if req.method() != Method::GET {
        return plain_404();
    }

    if let Some(table) = req.app_data::<web::Data<crate::presentation::screens::ScreenTable>>() {
        if table.holds(req.path()) {
            return table.serve(&req).await;
        }
    }

    let Some(spec) = PAGE_SPECS.iter().find(|s| s.path == req.path()) else {
        return plain_404();
    };

    if spec.kind == PageKind::DeskVariant {
        let sessions = sessions_of(&req).await;
        let staff = match &sessions.staff {
            Some(s) if s.role() == Role::Front => s,
            _ => {
                return to_staff(&req);
            }
        };

        let Some(state) = req.app_data::<web::Data<AppState>>() else {
            return plain_404();
        };
        let web_dir = Path::new(&state.settings.web.dir);
        let Some(mut content) = read_page_file(web_dir, spec.file) else {
            return plain_404();
        };

        let branch_name = staff.branch_id().map(|b| b.as_str()).unwrap_or("");
        let json_branch = serde_json::to_string(branch_name).unwrap_or_else(|_| "\"\"".to_string());
        if let Some(pos) = content.find("<head>") {
            content.replace_range(
                pos..pos + 6,
                &format!("<head><script>window.XC_BRANCH={json_branch};</script>"),
            );
        }
        content = content.replace("/api/bookings", "/api/staff/bookings");
        content = content.replace("/documents", "/staff/documents");
        content = content.replace("/files/", "/api/staff/file/");
        content = content.replace("src=\"/transport.js\"", "src=\"/staff-transport.js\"");
        if let Some(pos) = content.find("</body>") {
            content.insert_str(pos, STAFF_DESK_SCRIPT);
        }
        content = add_scripts(content, spec.scripts);
        content = add_forms_helper(content);
        content = add_version_line(content, &state.build, state.settings.channel.as_str());

        return HttpResponse::Ok()
            .content_type("text/html; charset=utf-8")
            .insert_header(("Cache-Control", "private, no-store"))
            .body(content);
    }

    let sessions = if spec.who_signed_in || spec.kind == PageKind::Branch {
        Some(sessions_of(&req).await)
    } else {
        None
    };

    if spec.who_signed_in {
        let s = sessions.as_ref().unwrap();
        if s.owner.is_none() && s.staff.is_none() {
            return to_login(&req);
        }
    }

    let branch = if spec.kind == PageKind::Branch {
        let s = sessions.as_ref().unwrap();
        match page_branch(&req, s) {
            Ok(name) => Some(name),
            Err(resp) => return resp,
        }
    } else {
        None
    };

    let Some(state) = req.app_data::<web::Data<AppState>>() else {
        return plain_404();
    };
    let web_dir = Path::new(&state.settings.web.dir);
    let Some(mut content) = read_page_file(web_dir, spec.file) else {
        return plain_404();
    };

    if let Some(branch_name) = branch {
        let json_branch =
            serde_json::to_string(&branch_name).unwrap_or_else(|_| "\"\"".to_string());
        if let Some(pos) = content.find("<head>") {
            content.replace_range(
                pos..pos + 6,
                &format!("<head><script>window.XC_BRANCH={json_branch};</script>"),
            );
        }
    }

    content = add_scripts(content, spec.scripts);
    content = add_forms_helper(content);
    content = add_version_line(content, &state.build, state.settings.channel.as_str());

    let mut resp = HttpResponse::Ok();
    resp.content_type("text/html; charset=utf-8")
        .insert_header(("Cache-Control", "no-store"));

    if spec.kind == PageKind::Branch {
        resp.insert_header(("X-Content-Type-Options", "nosniff"))
            .insert_header(("Referrer-Policy", "same-origin"));
    }

    resp.body(content)
}

pub(crate) fn is_valid_branch(name: &str) -> bool {
    BranchId::is_well_formed(name)
}

fn extract_branch_query(query_str: &str) -> Option<String> {
    if query_str.is_empty() {
        return Some("MAIN".to_string());
    }
    match web::Query::<Vec<(String, String)>>::from_query(query_str) {
        Ok(query) => {
            for (k, v) in query.into_inner() {
                if k == "branch" {
                    if v.is_empty() {
                        return Some("MAIN".to_string());
                    } else {
                        return Some(v);
                    }
                }
            }
            Some("MAIN".to_string())
        }
        Err(_) => None,
    }
}

async fn get_sessions(req: &HttpRequest) -> Sessions {
    if let Some(reader) = req.app_data::<web::Data<dyn SessionReader>>() {
        reader.sessions(req).await
    } else {
        Sessions {
            owner: None,
            staff: None,
        }
    }
}

fn redirect_origin(req: &HttpRequest) -> String {
    let scheme = match req
        .headers()
        .get("x-forwarded-proto")
        .and_then(|v| v.to_str().ok())
    {
        Some("https") => "https",
        _ => "http",
    };
    let host = req
        .headers()
        .get(actix_web::http::header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    format!("{scheme}://{host}")
}

pub(crate) fn page_branch(req: &HttpRequest, s: &Sessions) -> Result<String, HttpResponse> {
    let name = if s.owner.is_none() {
        if let Some(staff) = &s.staff {
            staff
                .branch_id()
                .map(|b| b.as_str())
                .unwrap_or("MAIN")
                .to_string()
        } else {
            match extract_branch_query(req.query_string()) {
                Some(name) => name,
                None => {
                    return Err(HttpResponse::BadRequest()
                        .content_type("text/plain; charset=utf-8")
                        .body("Invalid branch"));
                }
            }
        }
    } else {
        match extract_branch_query(req.query_string()) {
            Some(name) => name,
            None => {
                return Err(HttpResponse::BadRequest()
                    .content_type("text/plain; charset=utf-8")
                    .body("Invalid branch"));
            }
        }
    };
    if !is_valid_branch(&name) {
        return Err(HttpResponse::BadRequest()
            .content_type("text/plain; charset=utf-8")
            .body("Invalid branch"));
    }
    Ok(name)
}

pub(crate) async fn sessions_of(req: &HttpRequest) -> Sessions {
    get_sessions(req).await
}

pub(crate) fn to_login(req: &HttpRequest) -> HttpResponse {
    let origin = redirect_origin(req);
    HttpResponse::SeeOther()
        .insert_header(("Location", format!("{origin}/login")))
        .finish()
}

pub(crate) fn to_staff(req: &HttpRequest) -> HttpResponse {
    let origin = redirect_origin(req);
    HttpResponse::Found()
        .insert_header(("Location", format!("{origin}/staff")))
        .finish()
}

fn read_page_file(web_dir: &Path, file_name: &str) -> Option<String> {
    let file_path = web_dir.join(file_name);
    std::fs::read_to_string(file_path).ok()
}

/// The running build, on every page (#293): `v{version} · {build, 7} · {channel}`,
/// the values `/health/ready` reports, placed where `add_scripts` puts its tags.
fn add_version_line(mut content: String, build: &BuildInfo, channel: &str) -> String {
    let short: String = build.build.chars().take(7).collect();
    let line = format!(
        "<div id=\"xc-version\" style=\"position:fixed;right:8px;bottom:8px;\
         font:13px/1.4 system-ui,sans-serif;color:#333;\
         background:rgba(255,255,255,.85);padding:2px 8px;border-radius:4px;\
         pointer-events:none;z-index:2147483647\">v{} · {} · {}</div>",
        escape_html(&build.version),
        escape_html(&short),
        escape_html(channel),
    );
    if let Some(pos) = content.find("</body>") {
        content.insert_str(pos, &line);
    } else if let Some(pos) = content.find("</html>") {
        content.insert_str(pos, &line);
    } else {
        content.push_str(&line);
    }
    content
}

fn escape_html(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

fn add_scripts(mut content: String, scripts: &[&str]) -> String {
    if scripts.is_empty() {
        return content;
    }
    let tags: String = scripts
        .iter()
        .map(|src| format!("<script src=\"{src}\"></script>"))
        .collect();

    if let Some(pos) = content.find("</body>") {
        content.insert_str(pos, &tags);
    } else if let Some(pos) = content.find("</html>") {
        content.insert_str(pos, &tags);
    } else {
        content.push_str(&tags);
    }
    content
}

const FORMS_TAG: &str = "<script src=\"/_xc/forms.js\" defer></script>";

/// Adds the shared form helper tag immediately before `</body>` or `</html>`,
/// or appends it if neither is found, before `add_version_line` runs.
/// Known difference: every HTML page loads the shared form helper `/_xc/forms.js` (row FV).
fn add_forms_helper(mut content: String) -> String {
    if let Some(pos) = content.find("</body>") {
        content.insert_str(pos, FORMS_TAG);
    } else if let Some(pos) = content.find("</html>") {
        content.insert_str(pos, FORMS_TAG);
    } else {
        content.push_str(FORMS_TAG);
    }
    content
}

async fn forms_script() -> HttpResponse {
    HttpResponse::Ok()
        .content_type("text/javascript; charset=utf-8")
        .insert_header(("Cache-Control", "no-cache"))
        .insert_header(("X-Content-Type-Options", "nosniff"))
        .body(include_str!("../../static/xc-forms.js"))
}

fn compiled_script(cfg: &mut web::ServiceConfig, path: &'static str, body: &'static str) {
    cfg.service(
        web::resource(path)
            .route(web::get().to(move || async move {
                HttpResponse::Ok()
                    .content_type("text/javascript; charset=utf-8")
                    .insert_header(("Cache-Control", "no-cache"))
                    .insert_header(("X-Content-Type-Options", "nosniff"))
                    .body(body)
            }))
            .default_service(web::to(fallback)),
    );
}

/// What answers when no route matched. T1c serves static files here before the 404.
pub async fn fallback(req: HttpRequest) -> HttpResponse {
    if req.method() != Method::GET {
        return plain_404();
    }

    let Some(state) = req.app_data::<web::Data<AppState>>() else {
        return plain_404();
    };

    let web_dir = Path::new(&state.settings.web.dir);
    if let Some(resp) = serve_static_file(web_dir, req.uri().path()) {
        resp
    } else {
        plain_404()
    }
}

pub(crate) fn plain_404() -> HttpResponse {
    HttpResponse::NotFound()
        .content_type("text/plain")
        .body("Not found")
}

fn decode_raw_path(raw: &str) -> Option<String> {
    let bytes = raw.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            if i + 2 >= bytes.len() {
                return None;
            }
            let h1 = (bytes[i + 1] as char).to_digit(16)?;
            let h2 = (bytes[i + 2] as char).to_digit(16)?;
            let byte = ((h1 << 4) | h2) as u8;
            if byte == 0 {
                return None;
            }
            decoded.push(byte);
            i += 3;
        } else {
            if bytes[i] == 0 {
                return None;
            }
            decoded.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(decoded).ok()
}

fn static_content_type(file_name: &str) -> Option<&'static str> {
    if file_name.starts_with('.') && !file_name[1..].contains('.') {
        return None;
    }
    let dot_index = file_name.rfind('.')?;
    let ext = &file_name[dot_index..];
    match ext {
        ".js" => Some("text/javascript; charset=utf-8"),
        ".css" => Some("text/css; charset=utf-8"),
        ".svg" => Some("image/svg+xml"),
        ".jpeg" | ".jpg" => Some("image/jpeg"),
        ".png" => Some("image/png"),
        ".txt" => Some("text/plain; charset=utf-8"),
        ".json" => Some("application/json; charset=utf-8"),
        ".woff2" => Some("font/woff2"),
        _ => None,
    }
}

fn serve_static_file(web_dir: &Path, raw_path: &str) -> Option<HttpResponse> {
    let decoded_path = decode_raw_path(raw_path)?;
    if decoded_path.ends_with(".html") {
        return None;
    }

    let segments: Vec<&str> = decoded_path
        .split('/')
        .filter(|s| !s.is_empty() && *s != ".")
        .collect();

    if segments.is_empty() || segments.contains(&"..") {
        return None;
    }

    let last_segment = segments.last()?;
    let content_type = static_content_type(last_segment)?;

    let mut file_path = web_dir.to_path_buf();
    for seg in &segments {
        file_path.push(seg);
    }

    let meta = std::fs::metadata(&file_path).ok()?;
    if !meta.is_file() {
        return None;
    }

    let real_web_dir = web_dir.canonicalize().ok()?;
    let real_file_path = file_path.canonicalize().ok()?;
    if !real_file_path.starts_with(&real_web_dir) {
        return None;
    }

    let bytes = std::fs::read(&real_file_path).ok()?;

    Some(
        HttpResponse::Ok()
            .content_type(content_type)
            .insert_header(("Cache-Control", "no-cache"))
            .insert_header(("X-Content-Type-Options", "nosniff"))
            .body(bytes),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_4_decimal_rate_is_printed_as_stored_with_no_grouping() {
        let rate = Decimal::from_str("32.1234").unwrap();
        assert_eq!(format_rate(rate), "32.1234");
        let rate_with_zero = Decimal::from_str("32.1230").unwrap();
        assert_eq!(format_rate(rate_with_zero), "32.123");
        let rate_one_dp = Decimal::from_str("32.1").unwrap();
        assert_eq!(format_rate(rate_one_dp), "32.1");
    }

    #[test]
    fn a_rate_ge_1000_has_no_thousands_grouping() {
        let rate = Decimal::from_str("1234.5").unwrap();
        assert_eq!(format_rate(rate), "1234.5");
        let rate_large = Decimal::from_str("12345.6789").unwrap();
        assert_eq!(format_rate(rate_large), "12345.6789");
        let whole_rate = Decimal::from_str("1000.00").unwrap();
        assert_eq!(format_rate(whole_rate), "1000");
    }

    #[test]
    fn a_thb_amount_that_needs_rounding_to_3_decimals_rounds_half_away_from_zero() {
        // 100.50 at 32.1234 gives 3228.4017 -> 3,228.402
        let amount = Decimal::from_str("100.50").unwrap();
        let rate = Decimal::from_str("32.1234").unwrap();
        let thb = amount * rate;
        assert_eq!(to_locale_string(thb), "3,228.402");

        let half_up = Decimal::from_str("1234.5675").unwrap();
        assert_eq!(to_locale_string(half_up), "1,234.568");

        let round_down = Decimal::from_str("1234.5674").unwrap();
        assert_eq!(to_locale_string(round_down), "1,234.567");

        let half_down_away = Decimal::from_str("-1234.5675").unwrap();
        assert_eq!(to_locale_string(half_down_away), "-1,234.568");
    }

    #[test]
    fn a_whole_amount_has_thousands_grouping_and_no_fraction() {
        let whole = Decimal::from_str("1000").unwrap();
        assert_eq!(to_locale_string(whole), "1,000");
        let whole_with_zeros = Decimal::from_str("1000.00").unwrap();
        assert_eq!(to_locale_string(whole_with_zeros), "1,000");
        let hundred = Decimal::from_str("100").unwrap();
        assert_eq!(to_locale_string(hundred), "100");
        let million = Decimal::from_str("1000000").unwrap();
        assert_eq!(to_locale_string(million), "1,000,000");
        let zero = Decimal::from_str("0").unwrap();
        assert_eq!(to_locale_string(zero), "0");
    }

    #[test]
    fn an_amount_with_even_digit_before_midpoint_rounds_half_away_from_zero() {
        let even_midpoint = Decimal::from_str("1234.5665").unwrap();
        assert_eq!(to_locale_string(even_midpoint), "1,234.567");
        let neg_even_midpoint = Decimal::from_str("-1234.5665").unwrap();
        assert_eq!(to_locale_string(neg_even_midpoint), "-1,234.567");
    }

    #[actix_web::test]
    async fn server_error_answers_500_with_thai_message_and_json_content_type() {
        let resp = server_error();
        assert_eq!(
            resp.status(),
            actix_web::http::StatusCode::INTERNAL_SERVER_ERROR
        );
        assert_eq!(
            resp.headers().get("content-type").unwrap(),
            "application/json; charset=utf-8"
        );
        let body = actix_web::body::to_bytes(resp.into_body()).await.unwrap();
        assert_eq!(body, "{\"error\":\"เซิร์ฟเวอร์ขัดข้อง\"}");
    }
}
