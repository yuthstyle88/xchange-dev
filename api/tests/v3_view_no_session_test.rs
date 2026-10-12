//! Specification test for #430:
//! v3 views: a view inheriting a session-optional path would panic instead of answering 401.
//!
//! A view inheriting an entry path that Policy::allow permits without a session
//! (e.g. (EntryPath::Kiosk, "rates")) must answer 401 with {"error":{"code":"signIn","message":"กรุณาเข้าสู่ระบบ"}}
//! and Cache-Control: no-store when requested without a session, instead of panicking.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use actix_web::http::{StatusCode, header};
use actix_web::{App, HttpRequest, test as atest, web};
use pos_error::AppResult;
use serde::Serialize;
use serde_json::Value;
use ts_rs::TS;
use xc_auth::domain::access::Actor as AccessActor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;
use xchange::presentation::dto::v3::common::NoFilter;
use xchange::presentation::http::v3::view::{BranchView, PagedView, RowId, ViewEndpoint};
use xchange::presentation::pages::{SessionReader, Sessions};

#[derive(Serialize, TS)]
#[serde(rename_all = "camelCase")]
struct EchoData {
    field: String,
}

struct KioskRatesView;

impl BranchView for KioskRatesView {
    const NAME: &'static str = "test-kiosk-rates";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Kiosk, "rates");
    type Filter = NoFilter;
    type Data = EchoData;

    async fn load(
        &self,
        _branch: &BranchId,
        _who: &AccessActor,
        _filter: &NoFilter,
    ) -> AppResult<EchoData> {
        Ok(EchoData {
            field: "kiosk-rates".to_string(),
        })
    }
}

struct KioskRatesPagedView;

impl PagedView for KioskRatesPagedView {
    const NAME: &'static str = "test-kiosk-rates-paged";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Kiosk, "rates");
    type Filter = NoFilter;
    type Row = EchoData;

    async fn page(
        &self,
        _branch: &BranchId,
        _who: &AccessActor,
        _filter: &NoFilter,
        _before: Option<RowId>,
        _limit: u16,
    ) -> AppResult<Vec<EchoData>> {
        Ok(vec![EchoData {
            field: "paged-rates".to_string(),
        }])
    }
}

struct EmptySessionReader;

impl SessionReader for EmptySessionReader {
    fn sessions<'a>(
        &'a self,
        _req: &'a HttpRequest,
    ) -> Pin<Box<dyn Future<Output = Sessions> + 'a>> {
        Box::pin(async move {
            Sessions {
                owner: None,
                staff: None,
            }
        })
    }
}

#[actix_web::test]
async fn branch_view_inheriting_session_optional_path_answers_401_without_session() {
    let reader: Arc<dyn SessionReader> = Arc::new(EmptySessionReader);
    let app = atest::init_service(
        App::new()
            .configure(|cfg| {
                ViewEndpoint::register(cfg, Arc::new(KioskRatesView));
            })
            .app_data(web::Data::from(reader)),
    )
    .await;

    let req = atest::TestRequest::get()
        .uri("/api/v3/views/test-kiosk-rates")
        .insert_header((header::HOST, "xc.test"))
        .to_request();

    let resp = atest::call_service(&app, req).await;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

    let cache = resp
        .headers()
        .get(header::CACHE_CONTROL)
        .and_then(|v| v.to_str().ok());
    assert_eq!(cache, Some("no-store"));

    let content_type = resp
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok());
    assert_eq!(content_type, Some("application/json"));

    let bytes = atest::read_body(resp).await;
    let body: Value = serde_json::from_slice(&bytes).expect("valid JSON body");
    assert_eq!(
        body,
        serde_json::json!({
            "error": {
                "code": "signIn",
                "message": "กรุณาเข้าสู่ระบบ",
            }
        })
    );
}

#[actix_web::test]
async fn paged_view_inheriting_session_optional_path_answers_401_without_session() {
    let reader: Arc<dyn SessionReader> = Arc::new(EmptySessionReader);
    let app = atest::init_service(
        App::new()
            .configure(|cfg| {
                ViewEndpoint::register_paged(cfg, Arc::new(KioskRatesPagedView));
            })
            .app_data(web::Data::from(reader)),
    )
    .await;

    let req = atest::TestRequest::get()
        .uri("/api/v3/views/test-kiosk-rates-paged")
        .insert_header((header::HOST, "xc.test"))
        .to_request();

    let resp = atest::call_service(&app, req).await;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

    let cache = resp
        .headers()
        .get(header::CACHE_CONTROL)
        .and_then(|v| v.to_str().ok());
    assert_eq!(cache, Some("no-store"));

    let content_type = resp
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok());
    assert_eq!(content_type, Some("application/json"));

    let bytes = atest::read_body(resp).await;
    let body: Value = serde_json::from_slice(&bytes).expect("valid JSON body");
    assert_eq!(
        body,
        serde_json::json!({
            "error": {
                "code": "signIn",
                "message": "กรุณาเข้าสู่ระบบ",
            }
        })
    );
}

#[actix_web::test]
async fn view_inheriting_session_optional_path_without_session_reader_answers_401() {
    let app = atest::init_service(App::new().configure(|cfg| {
        ViewEndpoint::register(cfg, Arc::new(KioskRatesView));
    }))
    .await;

    let req = atest::TestRequest::get()
        .uri("/api/v3/views/test-kiosk-rates")
        .insert_header((header::HOST, "xc.test"))
        .to_request();

    let resp = atest::call_service(&app, req).await;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

    let cache = resp
        .headers()
        .get(header::CACHE_CONTROL)
        .and_then(|v| v.to_str().ok());
    assert_eq!(cache, Some("no-store"));

    let content_type = resp
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok());
    assert_eq!(content_type, Some("application/json"));

    let bytes = atest::read_body(resp).await;
    let body: Value = serde_json::from_slice(&bytes).expect("valid JSON body");
    assert_eq!(
        body,
        serde_json::json!({
            "error": {
                "code": "signIn",
                "message": "กรุณาเข้าสู่ระบบ",
            }
        })
    );
}
