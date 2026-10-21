//! Pages presentation.

use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::str::FromStr;

use actix_web::http::Method;
use actix_web::{HttpRequest, HttpResponse, web};
use rust_decimal::{Decimal, RoundingStrategy};
use value_objects::actor::Actor;
use xc_auth::domain::policy::value_objects;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{ExchangeArithmetic, ForeignAmount, Rate, UnitSize};

use crate::bootstrap::{AppState, BuildInfo};

/// The sessions one request carries. A browser can hold an owner session and a
/// staff session at the same time.
pub struct Sessions {
    /// `Actor::owner(id)` when the request carries a valid owner session.
    pub owner: Option<Actor>,
    /// `Actor::staff(id, role, branch)` when it carries a valid staff session.
    pub staff: Option<Actor>,
}

/// The callers one request carries, including their mapped Blueprint actors.
#[derive(Clone, Debug)]
pub struct Callers {
    pub user: Option<xc_auth::domain::access::Actor>,
    pub staff_user: Option<xc_auth::domain::access::Actor>,
}

impl Callers {
    /// The owner's standing when the request carries an owner (OR-7, #1026): replaces `owner`.
    pub fn owner_standing(&self) -> Option<xc_auth::domain::access::Standing> {
        self.user
            .as_ref()
            .and_then(|user| xc_auth::domain::access::Standing::of(user, chrono::Utc::now()))
            .filter(|standing| standing.is_owner())
    }

    /// The staff standing when the request carries a staff member (OR-7, #1026): replaces `staff`.
    pub fn staff_standing(&self) -> Option<xc_auth::domain::access::Standing> {
        self.staff_user
            .as_ref()
            .and_then(|user| xc_auth::domain::access::Standing::of(user, chrono::Utc::now()))
            .filter(|standing| !standing.is_owner())
    }
}

/// Organization ID seeded in migration 20261011100000_organizations.sql:9-10.
pub const SEEDED_XC_ORG_ID: &str = "00000000-0000-0000-0000-000000000001";

/// Builds `Callers` from `Sessions` using the resolved old-cookie organisation ID.
///
/// Shared builder (lane controller ruling, 2026-10-12, #1463 issuecomment-6126805322):
/// both `AccountSessionReader::callers` in production and test callers build old-cookie actors
/// through this one shared builder that uses the startup-resolved organisation id rather than
/// the literal code 'XC'.
pub fn old_cookie_callers(sessions: Sessions, org_id: &str) -> Callers {
    let epoch = chrono::DateTime::<chrono::Utc>::UNIX_EPOCH;
    let staff_user = sessions
        .staff
        .as_ref()
        .map(|a| xc_auth::domain::access::v3_roles::actor_from_v3(a, org_id, epoch));
    let user = sessions
        .owner
        .as_ref()
        .map(|a| xc_auth::domain::access::v3_roles::actor_from_v3(a, org_id, epoch))
        .or_else(|| staff_user.clone());
    Callers { user, staff_user }
}

/// Reads the sessions of a request. Declared here until ticket T4c, which
/// implements it in `presentation/extractors/actor.rs`; it may move there then.
///
/// OR-7.m (#1027): `sessions` has a default (no session) so a reader can override `callers` alone,
/// built with `presentation::test_callers`. Both go when password login is deleted.
pub trait SessionReader: Send + Sync {
    fn sessions<'a>(
        &'a self,
        _req: &'a HttpRequest,
    ) -> Pin<Box<dyn Future<Output = Sessions> + 'a>> {
        Box::pin(async {
            Sessions {
                owner: None,
                staff: None,
            }
        })
    }

    fn callers<'a>(&'a self, req: &'a HttpRequest) -> Pin<Box<dyn Future<Output = Callers> + 'a>> {
        Box::pin(async move {
            let s = self.sessions(req).await;
            old_cookie_callers(s, SEEDED_XC_ORG_ID)
        })
    }
}

/// Resolves the old-cookie organisation id (code 'XC') from `organizations` at startup.
///
/// Lookup (lane controller ruling, 2026-10-12): an organisation's code-to-id mapping never
/// changes, so the id of the old cookie's organisation (code XC) is resolved once at startup
/// from `organizations` and held by `SessionReader`. If the row is missing at startup, boot
/// fails with a clear message rather than serving refused sessions.
pub async fn resolve_old_cookie_org_id(pool: &sqlx::PgPool) -> Result<String, String> {
    let id: Option<String> =
        sqlx::query_scalar("SELECT id::text FROM organizations WHERE code = 'XC'")
            .fetch_optional(pool)
            .await
            .map_err(|e| format!("failed to query organization row for code 'XC': {e}"))?;
    match id {
        Some(id) => Ok(id),
        None => Err("organization row with code 'XC' not found in organizations table".to_string()),
    }
}

pub async fn get_callers(req: &HttpRequest) -> Callers {
    if let Some(reader) = req.app_data::<web::Data<dyn SessionReader>>() {
        reader.callers(req).await
    } else {
        Callers {
            user: None,
            staff_user: None,
        }
    }
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

const STAFF_DESK_SCRIPT: &str = "<script>let staffDraftId='';const staffGasOriginal=gasCall;gasCall=async function(name,payload){if(name==='reserveCustomer'&&!staffDraftId){const r=await fetch('/api/tx-drafts',{method:'POST',credentials:'same-origin',headers:{'content-type':'application/json','x-xchange-request':'1','idempotency-key':crypto.randomUUID()},body:JSON.stringify({branch:window.XC_BRANCH||'',side:s.type,currency:s.currency,amount:String(s.amount)})});const answer=await r.json();if(!r.ok)throw new Error((answer.error&&answer.error.message)||answer.error||'draftBegin');staffDraftId=answer.id;}if(name==='saveTransaction')payload={...payload,staffDraftId};return staffGasOriginal(name,payload);};</script>";

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

fn stored_decimal(column: &str, text: &str, reference: &str) -> Result<Decimal, HttpResponse> {
    match Decimal::from_str(text) {
        Ok(v) => Ok(v),
        Err(e) => {
            tracing::error!("invalid {column} {text:?} in transaction {reference}: {e}");
            Err(server_error())
        }
    }
}

async fn devices_receipt_handler(req: HttpRequest) -> HttpResponse {
    if req.method() != Method::GET {
        return plain_404();
    }

    let callers = get_callers(&req).await;
    let owner = callers.owner_standing();
    let staff = callers.staff_standing();
    if owner.is_none() && staff.is_none() {
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

    let target_branch = if owner.is_some() {
        url_branch
    } else if let Some(staff) = &staff {
        staff.branch.as_ref().map(|b| b.as_str()).unwrap_or("MAIN")
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

    let tx_row: Option<(String, String, String, String, String, String, String)> =
        match sqlx::query_as(
            "SELECT t.customer_id, tl.shop_side, tl.currency, tl.quantity::text, \
         tl.rate_snapshot::text, tl.fee_thb::text, tl.unit_size::text \
         FROM transactions t \
         JOIN transaction_lines tl ON tl.transaction_id = t.id \
         WHERE t.branch_id = $1 AND t.document_ref = $2 AND t.mode = 'LIVE' AND t.status = 'COMPLETED' \
         ORDER BY tl.line_no LIMIT 1",
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

    let Some((customer_id, tx_type, code, amount_str, rate_str, fee_str, unit_size_str)) = tx_row
    else {
        return HttpResponse::NotFound()
            .content_type("text/plain; charset=utf-8")
            .body("ไม่พบรายการ");
    };

    let amount = match stored_decimal("quantity", &amount_str, ref_str) {
        Ok(v) => v,
        Err(r) => return r,
    };
    let rate = match stored_decimal("rate_snapshot", &rate_str, ref_str) {
        Ok(v) => v,
        Err(r) => return r,
    };
    let fee = match stored_decimal("fee_thb", &fee_str, ref_str) {
        Ok(v) => v,
        Err(r) => return r,
    };
    let unit_val = match UnitSize::parse(unit_size_str.trim()) {
        Ok(u) => u,
        Err(e) => {
            tracing::error!("invalid unit_size {unit_size_str:?} in transaction {ref_str}: {e}");
            return server_error();
        }
    };
    let rate_val = match Rate::parse(&rate.normalize().to_string()) {
        Ok(r) => r,
        Err(e) => {
            tracing::error!("invalid rate {rate:?} in transaction {ref_str}: {e}");
            return server_error();
        }
    };

    let foreign = ForeignAmount::from_decimal(amount);
    let base_money = match ExchangeArithmetic::try_thb(foreign, rate_val, unit_val) {
        Ok(m) => m,
        Err(e) => {
            tracing::error!("thb overflow in transaction {ref_str}: {e}");
            return server_error();
        }
    };
    let base = Decimal::from_i128_with_scale(base_money.minor_i128(), 2);
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

    if let Some(table) = req.app_data::<web::Data<crate::presentation::screens::ScreenTable>>()
        && table.holds(req.path())
    {
        return table.serve(&req).await;
    }

    let Some(spec) = PAGE_SPECS.iter().find(|s| s.path == req.path()) else {
        return plain_404();
    };

    if spec.kind == PageKind::DeskVariant {
        let callers = get_callers(&req).await;
        let staff = match callers.staff_standing() {
            Some(s) if s.staff_role() == Some(xc_auth::domain::access::Role::Teller) => s,
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

        let branch_name = staff.branch.as_ref().map(|b| b.as_str()).unwrap_or("");
        let json_branch = serde_json::to_string(branch_name).unwrap_or_else(|_| "\"\"".to_string());
        if let Some(pos) = content.find("<head>") {
            content.replace_range(
                pos..pos + 6,
                &format!("<head><script>window.XC_BRANCH={json_branch};</script>"),
            );
        }
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

    let callers = if spec.who_signed_in || spec.kind == PageKind::Branch {
        Some(get_callers(&req).await)
    } else {
        None
    };

    if spec.who_signed_in {
        let c = callers.as_ref().unwrap();
        if c.owner_standing().is_none() && c.staff_standing().is_none() {
            return to_login(&req);
        }
    }

    let branch = if spec.kind == PageKind::Branch {
        let c = callers.as_ref().unwrap();
        match page_branch(&req, c) {
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

pub(crate) fn page_branch(req: &HttpRequest, c: &Callers) -> Result<String, HttpResponse> {
    let name = if c.owner_standing().is_none() {
        if let Some(staff) = c.staff_standing() {
            staff
                .branch
                .as_ref()
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

fn encode_component(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(*b as char);
            }
            _ => {
                use std::fmt::Write;
                let _ = write!(out, "%{:02X}", b);
            }
        }
    }
    out
}

pub(crate) fn to_login(req: &HttpRequest) -> HttpResponse {
    let origin = redirect_origin(req);
    let path = req.uri().path();
    let target = match req.uri().query() {
        Some(q) if !q.is_empty() => format!("{path}?{q}"),
        _ => path.to_string(),
    };
    let next = encode_component(&target);
    HttpResponse::SeeOther()
        .insert_header(("Location", format!("{origin}/login?next={next}")))
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

    #[actix_web::test]
    async fn r1_quantity_unreadable_text_answers_500_server_error() {
        let resp = stored_decimal("quantity", "abc", "TX-001").expect_err("must refuse abc");
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

    #[actix_web::test]
    async fn r2_rate_snapshot_empty_text_answers_500_server_error() {
        let resp = stored_decimal("rate_snapshot", "", "TX-001").expect_err("must refuse empty");
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

    #[actix_web::test]
    async fn r3_fee_thb_malformed_scientific_text_answers_500_server_error() {
        let resp = stored_decimal("fee_thb", "1e5x", "TX-001").expect_err("must refuse 1e5x");
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

    #[test]
    fn k1_valid_decimal_strings_including_zero_fee_return_exact_values() {
        assert_eq!(
            stored_decimal("quantity", "100.0000", "TX-001").expect("valid quantity"),
            Decimal::from_str("100.0000").unwrap()
        );
        assert_eq!(
            stored_decimal("rate_snapshot", "32.12345678", "TX-001").expect("valid rate_snapshot"),
            Decimal::from_str("32.12345678").unwrap()
        );
        assert_eq!(
            stored_decimal("fee_thb", "0.00", "TX-001").expect("valid zero fee"),
            Decimal::from_str("0.00").unwrap()
        );
    }
}
