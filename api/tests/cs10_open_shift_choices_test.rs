mod common;
mod quote_common;

use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use actix_web::http::{Method, StatusCode, header};
use actix_web::{App, HttpRequest, test as atest, web};
use chrono::{NaiveTime, TimeZone, Utc};
use chrono_tz::Asia::Bangkok;
use pos_infra::transactions::Transactor;
use quote_common::*;
use serde_json::Value;
use xc_auth::domain::access::{Actor as User, Role, ScopedRole};
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xc_shift::application::shift_plan::service::ShiftPlanService;
use xc_shift::domain::shift_plan::shift_template::ShiftTemplateInput;
use xc_shift::infrastructure::shift_plan::pg_store::PgShiftTemplateStore;
use xchange::bootstrap::{self, AppState};
use xchange::presentation::http::v3::open_shift_choices::{self, choices_at};
use xchange::presentation::pages::{Callers, SessionReader};

const AS: &str = "x-test-as";
const HOST: &str = "xc.test";
const SAME_ORIGIN: &str = "http://xc.test";

struct Who {
    id: String,
    spec: String,
}

fn user_of(spec: &str) -> Option<User> {
    let mut parts = spec.splitn(3, '|');
    let (id, role, branch) = (parts.next()?, parts.next()?, parts.next()?);
    let role = Role::parse(role)?;
    let branch = (branch != "*").then(|| BranchId::new(branch));
    let from = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
    Some(User::new(
        id,
        ORG_1,
        vec![ScopedRole::new(role, branch, from, None)],
        None,
    ))
}

struct HeaderUsers;

impl SessionReader for HeaderUsers {
    fn callers<'a>(&'a self, req: &'a HttpRequest) -> Pin<Box<dyn Future<Output = Callers> + 'a>> {
        let spec = req
            .headers()
            .get(AS)
            .map(|v| v.to_str().expect("ASCII").to_owned())
            .unwrap_or_default();
        Box::pin(async move {
            let user = user_of(&spec);
            Callers {
                user: user.clone(),
                staff_user: user,
            }
        })
    }
}

fn hm(h: u32, m: u32) -> NaiveTime {
    NaiveTime::from_hms_opt(h, m, 0).unwrap()
}

fn row(
    name: &str,
    start: NaiveTime,
    end: NaiveTime,
    brk: Option<(NaiveTime, NaiveTime)>,
) -> ShiftTemplateInput {
    ShiftTemplateInput {
        id: None,
        name: name.to_owned(),
        starts_at: start,
        ends_at: end,
        break_window: brk,
    }
}

/// Overlap24 plan: A 06–15, B 14–23 (break 18–19), C 22–07 (cutover 06:00).
fn overlap24() -> Vec<ShiftTemplateInput> {
    vec![
        row("A", hm(6, 0), hm(15, 0), None),
        row("B", hm(14, 0), hm(23, 0), Some((hm(18, 0), hm(19, 0)))),
        row("C", hm(22, 0), hm(7, 0), None),
    ]
}

async fn setup_branch_with_overlap24(db: &TestDb, branch_name: &str) -> BranchId {
    db.branch(branch_name).await;
    let branch = BranchId::new(branch_name);
    let branch_tx = BranchTx::new(Transactor::new(db.pool.clone()));
    let svc = ShiftPlanService::new(Arc::new(PgShiftTemplateStore), branch_tx);
    let mut tx = db.pool.begin().await.unwrap();
    svc.save_in(&mut tx, &branch, ORG_1, overlap24(), Utc::now())
        .await
        .expect("save overlap24 plan");
    tx.commit().await.unwrap();
    branch
}

/// E5: a branch with overlap24 (A 06–15, B 14–23, C 22–07) at 14:30 Bangkok
/// Required: A and B, preselect: null
#[tokio::test]
async fn cs10_e5_overlap24_two_shifts_at_14_30() {
    let db = TestDb::start().await;
    let branch = setup_branch_with_overlap24(&db, "CS10-E5").await;

    let now_14_30 = Bangkok
        .with_ymd_and_hms(2026, 11, 4, 14, 30, 0)
        .unwrap()
        .with_timezone(&Utc);

    let res = choices_at(&db.pool, &branch, now_14_30)
        .await
        .expect("load choices at 14:30");

    assert_eq!(res.choices.len(), 2, "must return choices A and B");
    assert_eq!(res.preselect, None, "preselect must be null when 2 cover");

    let a = &res.choices[0];
    assert_eq!(a.name, "A");
    assert_eq!(a.business_date, "2026-11-04");
    assert_eq!(a.starts_at, "2026-11-03T23:00:00Z"); // 06:00 BKK -> 23:00 UTC previous day
    assert_eq!(a.ends_at, "2026-11-04T08:00:00Z"); // 15:00 BKK -> 08:00 UTC
    assert_eq!(a.break_starts_at, None);
    assert_eq!(a.break_ends_at, None);

    let b = &res.choices[1];
    assert_eq!(b.name, "B");
    assert_eq!(b.business_date, "2026-11-04");
    assert_eq!(b.starts_at, "2026-11-04T07:00:00Z"); // 14:00 BKK -> 07:00 UTC
    assert_eq!(b.ends_at, "2026-11-04T16:00:00Z"); // 23:00 BKK -> 16:00 UTC
    assert_eq!(b.break_starts_at.as_deref(), Some("2026-11-04T11:00:00Z")); // 18:00 BKK -> 11:00 UTC
    assert_eq!(b.break_ends_at.as_deref(), Some("2026-11-04T12:00:00Z")); // 19:00 BKK -> 12:00 UTC

    db.finish().await;
}

/// E5b: the same at 10:00
/// Required: A only, preselect = A's id
#[tokio::test]
async fn cs10_e5b_overlap24_one_shift_at_10_00() {
    let db = TestDb::start().await;
    let branch = setup_branch_with_overlap24(&db, "CS10-E5B").await;

    let now_10_00 = Bangkok
        .with_ymd_and_hms(2026, 11, 4, 10, 0, 0)
        .unwrap()
        .with_timezone(&Utc);

    let res = choices_at(&db.pool, &branch, now_10_00)
        .await
        .expect("load choices at 10:00");

    assert_eq!(res.choices.len(), 1, "must return only shift A");
    let a = &res.choices[0];
    assert_eq!(a.name, "A");
    assert_eq!(a.business_date, "2026-11-04");
    assert_eq!(a.starts_at, "2026-11-03T23:00:00Z");
    assert_eq!(a.ends_at, "2026-11-04T08:00:00Z");
    assert_eq!(a.break_starts_at, None);
    assert_eq!(a.break_ends_at, None);

    assert_eq!(
        res.preselect.as_deref(),
        Some(a.template_id.as_str()),
        "preselect must match A's template id when exactly one covers"
    );

    db.finish().await;
}

/// E5c: a caller without the myShift permission gets 403
#[actix_web::test]
async fn cs10_e5c_caller_without_permission_is_refused() {
    let db = TestDb::start().await;
    let branch = setup_branch_with_overlap24(&db, "CS10-E5C").await;

    // A teller from another branch does not hold myShift permission for this branch -> outOfScope (403)
    let other_teller = Who {
        id: "teller-other".to_string(),
        spec: format!("teller-other|{}|OTHER-BRANCH", Role::Teller.as_str()),
    };
    let teller = Who {
        id: "teller-1".to_string(),
        spec: format!("teller-1|{}|{}", Role::Teller.as_str(), branch.as_str()),
    };

    let state = AppState::new(Arc::new(common::settings(&[])), db.pool.clone());
    let reader: Arc<dyn SessionReader> = Arc::new(HeaderUsers);
    let app = atest::init_service(
        App::new()
            .configure(move |cfg| bootstrap::configure(cfg, state))
            .app_data(web::Data::from(reader)),
    )
    .await;

    let uri = format!("/api/v3/views/open_shift_choices?branch={}", branch.as_str());

    // 1. Caller without myShift permission on this branch -> 403 Forbidden (outOfScope)
    let req_unauth = atest::TestRequest::default()
        .method(Method::GET)
        .uri(&uri)
        .insert_header((header::HOST, HOST))
        .insert_header((AS, other_teller.spec))
        .to_request();
    let resp = atest::call_service(&app, req_unauth).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes);
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "caller without myShift permission must receive 403: {text}"
    );

    // 2. Teller caller with myShift permission on this branch -> 200 OK
    let req_auth = atest::TestRequest::default()
        .method(Method::GET)
        .uri(&uri)
        .insert_header((header::HOST, HOST))
        .insert_header((AS, teller.spec))
        .to_request();
    let resp = atest::call_service(&app, req_auth).await;
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "caller with myShift permission must receive 200"
    );

    let bytes = atest::read_body(resp).await;
    let body: Value = serde_json::from_slice(&bytes).unwrap();
    assert!(body["data"]["choices"].is_array(), "data.choices must be an array: {body}");

    db.finish().await;
}

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("after 1970")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("cs10-scratch-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create scratch dir");
        Self(dir)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn cs10_export_types() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../web/src/shared/api/generated");
    open_shift_choices::export_types(&dir).expect("export open_shift_choices types");
}

#[test]
fn cs10_generated_ts_matches_export_types() {
    let scratch = Scratch::new();
    open_shift_choices::export_types(&scratch.0).expect("export open_shift_choices types");

    let exported = std::fs::read_to_string(scratch.0.join("open_shift_choices.ts"))
        .expect("read exported open_shift_choices.ts");

    let committed_file = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../web/src/shared/api/generated/open_shift_choices.ts");
    if committed_file.exists() {
        let committed = std::fs::read_to_string(&committed_file).expect("read committed open_shift_choices.ts");
        assert_eq!(committed, exported, "committed open_shift_choices.ts must match exported");
    }
}
