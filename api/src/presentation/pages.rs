//! Pages presentation.

use std::future::Future;
use std::path::Path;
use std::pin::Pin;

use actix_web::http::Method;
use actix_web::{HttpRequest, HttpResponse, web};
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_auth::domain::policy::value_objects::role::Role;

use crate::bootstrap::AppState;

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
    for spec in PAGE_SPECS {
        cfg.service(
            web::resource(spec.path)
                .route(web::get().to(page_handler))
                .default_service(web::to(fallback)),
        );
    }
}

async fn page_handler(req: HttpRequest) -> HttpResponse {
    if req.method() != Method::GET {
        return plain_404();
    }

    let Some(spec) = PAGE_SPECS.iter().find(|s| s.path == req.path()) else {
        return plain_404();
    };

    if spec.kind == PageKind::DeskVariant {
        let sessions = get_sessions(&req).await;
        let staff = match &sessions.staff {
            Some(s) if s.role() == Role::Front => s,
            _ => {
                let origin = redirect_origin(&req);
                return HttpResponse::Found()
                    .insert_header(("Location", format!("{origin}/staff")))
                    .finish();
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

        return HttpResponse::Ok()
            .content_type("text/html; charset=utf-8")
            .insert_header(("Cache-Control", "private, no-store"))
            .body(content);
    }

    let sessions = if spec.who_signed_in || spec.kind == PageKind::Branch {
        Some(get_sessions(&req).await)
    } else {
        None
    };

    if spec.who_signed_in {
        let s = sessions.as_ref().unwrap();
        if s.owner.is_none() && s.staff.is_none() {
            let origin = redirect_origin(&req);
            return HttpResponse::SeeOther()
                .insert_header(("Location", format!("{origin}/login")))
                .finish();
        }
    }

    let branch = if spec.kind == PageKind::Branch {
        let s = sessions.as_ref().unwrap();
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
                        return HttpResponse::BadRequest()
                            .content_type("text/plain; charset=utf-8")
                            .body("Invalid branch");
                    }
                }
            }
        } else {
            match extract_branch_query(req.query_string()) {
                Some(name) => name,
                None => {
                    return HttpResponse::BadRequest()
                        .content_type("text/plain; charset=utf-8")
                        .body("Invalid branch");
                }
            }
        };
        if !is_valid_branch(&name) {
            return HttpResponse::BadRequest()
                .content_type("text/plain; charset=utf-8")
                .body("Invalid branch");
        }
        Some(name)
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

    let mut resp = HttpResponse::Ok();
    resp.content_type("text/html; charset=utf-8")
        .insert_header(("Cache-Control", "no-store"));

    if spec.kind == PageKind::Branch {
        resp.insert_header(("X-Content-Type-Options", "nosniff"))
            .insert_header(("Referrer-Policy", "same-origin"));
    }

    resp.body(content)
}

fn is_valid_branch(name: &str) -> bool {
    if name.is_empty() || name.len() > 70 {
        return false;
    }
    name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
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

fn read_page_file(web_dir: &Path, file_name: &str) -> Option<String> {
    let file_path = web_dir.join(file_name);
    std::fs::read_to_string(file_path).ok()
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
