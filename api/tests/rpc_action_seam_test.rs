//! Written before the implementation (ticket T11a-0). Do not edit, delete or weaken.
//!
//! The seam behind the RPC dispatcher: `RpcAction`, one file per family of
//! actions. These tests register families of their own — the route table reads
//! them as `web::Data<RpcActions>`, registered after `bootstrap::configure` so
//! that they replace the server's — and ask the two routes **through HTTP**,
//! with real sessions:
//!
//! - a family whose `handles` says yes is run, and its answer goes out the way
//!   the answer of one of the dispatcher's own ten does: the role judged first,
//!   the branch resolved first, the keys in the order given, the staff links on
//!   the staff route;
//! - a family whose `handles` says no is not run and the name is answered as a
//!   name nobody knows (the gap);
//! - of two families that both say yes, the first registered answers;
//! - a name of the dispatcher's own ten is never offered to a family.
//!
//! Needs `TEST_DATABASE_URL` (see `snapshot_common/mod.rs`); nothing here skips.

mod common;
mod compat_common;
mod rpc_common;
mod snapshot_common;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use chrono::SecondsFormat;
use pos_error::AppError;
use rpc_common::{
    Answer, App, BAD_BRANCH, Call, Cookies, Desk, EMPLOYEE_AUTH_REQUIRED, INVALID_REQUEST,
    NO_SUCH_BRANCH, ROLE_NOT_ALLOWED_RPC, ROLE_NOT_ALLOWED_STAFF, RPC, SERVER_ERROR, STAFF_RPC,
    one_of_each_role, sign_in,
};
use serde_json::{Value, json};
use xchange::application::compat::actions::{ActionFuture, RpcAction, RpcActions, RpcCall};
use xchange::application::compat::envelope::{RpcRefusal, RpcRoute};
use xchange::application::compat::rpc::RpcFailure;
use xchange::application::compat::v2_json::V2Json;

const ORIGIN: &str = "http://desk.test";
const BOTH: &[RpcRoute] = &[RpcRoute::Rpc, RpcRoute::StaffRpc];
/// The clock of a fresh `Desk`.
const NOW: &str = "2026-10-06T05:00:00.000Z";

/// What a test family does when it is run.
#[derive(Clone, Copy)]
enum Does {
    /// Answers what it was called with.
    Tells,
    /// Refuses with a sentence, as a use case does.
    RefusesAsDomain(&'static str),
    /// Refuses with a refusal of its own making.
    Refuses(u16, &'static str),
    /// Breaks.
    Breaks,
}

/// A family of the test's own.
struct Family {
    tag: &'static str,
    names: &'static [&'static str],
    routes: &'static [RpcRoute],
    does: Does,
    asked: AtomicUsize,
    ran: AtomicUsize,
}

impl Family {
    fn new(
        tag: &'static str,
        names: &'static [&'static str],
        routes: &'static [RpcRoute],
    ) -> Arc<Family> {
        Family::that(tag, names, routes, Does::Tells)
    }

    fn that(
        tag: &'static str,
        names: &'static [&'static str],
        routes: &'static [RpcRoute],
        does: Does,
    ) -> Arc<Family> {
        Arc::new(Family {
            tag,
            names,
            routes,
            does,
            asked: AtomicUsize::new(0),
            ran: AtomicUsize::new(0),
        })
    }

    fn asked(&self) -> usize {
        self.asked.load(Ordering::Relaxed)
    }

    fn ran(&self) -> usize {
        self.ran.load(Ordering::Relaxed)
    }
}

impl RpcAction for Family {
    fn handles(&self, route: RpcRoute, action: &str) -> bool {
        self.asked.fetch_add(1, Ordering::Relaxed);
        self.routes.contains(&route) && self.names.contains(&action)
    }

    fn run<'a>(&'a self, call: RpcCall<'a>) -> ActionFuture<'a> {
        Box::pin(async move {
            self.ran.fetch_add(1, Ordering::Relaxed);
            match self.does {
                Does::Tells => {}
                Does::RefusesAsDomain(sentence) => {
                    return Err(AppError::Domain(sentence.to_string()).into());
                }
                Does::Refuses(status, sentence) => {
                    return Err(RpcFailure::Refused(RpcRefusal {
                        status,
                        sentence: sentence.to_string(),
                        no_store: true,
                    }));
                }
                Does::Breaks => {
                    return Err(AppError::Internal("the cause, not to be told".to_string()).into());
                }
            }
            let route = match call.route {
                RpcRoute::Rpc => "rpc",
                RpcRoute::StaffRpc => "staff",
            };
            // keys on purpose not in the order of the alphabet: the answer goes
            // out in the order a family gives it
            Ok(V2Json::object(vec![
                ("zeta", V2Json::text(self.tag)),
                ("action", V2Json::text(call.action)),
                ("route", V2Json::text(route)),
                ("branch", V2Json::text(call.branch.as_str())),
                ("who", V2Json::text(call.caller.username.as_str())),
                (
                    "now",
                    V2Json::text(call.now.to_rfc3339_opts(SecondsFormat::Millis, true)),
                ),
                (
                    "payload",
                    V2Json::from_value(&Value::Object(call.payload.clone())),
                ),
                ("passportUrl", V2Json::text("/files/abc")),
                ("folderUrl", V2Json::text("/documents?ref=R1")),
                ("sheetUrl", V2Json::text("/#transactions")),
                ("alpha", V2Json::int(1)),
            ]))
        })
    }
}

/// The server's whole route table with the test's clock, and `$actions` in
/// place of the families the server registers.
macro_rules! seam_app {
    ($desk:expr, $actions:expr) => {{
        let state = $desk.state();
        let clock: Arc<dyn xchange::presentation::http::rpc::RpcClock> = $desk.clock.clone();
        let actions: RpcActions = $actions;
        App::new(
            actix_web::test::init_service(
                actix_web::App::new()
                    .configure(move |cfg| xchange::bootstrap::configure(cfg, state))
                    .app_data(actix_web::web::Data::from(clock))
                    .app_data(actix_web::web::Data::new(actions)),
            )
            .await,
            |t| t.to_request(),
        )
    }};
}

async fn everybody(desk: &Desk, app: &App) -> Cookies {
    let people = one_of_each_role();
    desk.employ(&people).await;
    sign_in(app, ORIGIN, &people).await
}

async fn ask(
    app: &App,
    cookies: &Cookies,
    path: &str,
    action: &str,
    who: &str,
    payload: Value,
) -> Answer {
    app.send(Call::action(ORIGIN, path, action, payload).cookie(&cookies[who]))
        .await
}

/// A family that handles a name answers it on `/api/rpc`: status 200, JSON,
/// never cached, and the body is the family's answer byte for byte — keys in
/// the order given, links as given — called with the route, the name, the
/// payload, the caller, the branch the owner named and the request's clock.
#[actix_web::test]
async fn a_family_that_handles_a_name_answers_it_in_its_own_words() {
    let desk = Desk::open().await;
    let family = Family::new("one", &["saveTransaction", "rateDraft"], BOTH);
    let app = seam_app!(desk, vec![family.clone() as Arc<dyn RpcAction>]);
    let cookies = everybody(&desk, &app).await;

    // OR-4b (#767, design §4.2 C2): ExchangeTransact is TELLER's, so the owner's user no longer asks
    // saveTransaction; the owner's name of part two here is rateDraft (RatesDraft is EXECUTIVE's)
    let answer = ask(
        &app,
        &cookies,
        RPC,
        "rateDraft",
        "OWNER",
        json!({ "n": 1, "branchId": "MAIN" }),
    )
    .await;
    assert_eq!(answer.status, 200, "{:.300}", answer.text);
    assert_eq!(
        answer.text,
        format!(
            "{{\"zeta\":\"one\",\"action\":\"rateDraft\",\"route\":\"rpc\",\
             \"branch\":\"MAIN\",\"who\":\"owner\",\"now\":\"{NOW}\",\
             \"payload\":{{\"branchId\":\"MAIN\",\"n\":1}},\"passportUrl\":\"/files/abc\",\
             \"folderUrl\":\"/documents?ref=R1\",\"sheetUrl\":\"/#transactions\",\"alpha\":1}}"
        )
    );
    assert_eq!(answer.header("cache-control"), Some("no-store"));
    assert_eq!(answer.header("content-type"), Some("application/json"));
    assert_eq!(family.ran(), 1);

    // an employee on the same route: their own branch and name, links as given
    let answer = ask(&app, &cookies, RPC, "saveTransaction", "FRONT", json!({})).await;
    assert_eq!(answer.status, 200, "{:.300}", answer.text);
    assert_eq!(
        answer.text,
        format!(
            "{{\"zeta\":\"one\",\"action\":\"saveTransaction\",\"route\":\"rpc\",\
             \"branch\":\"MAIN\",\"who\":\"role0\",\"now\":\"{NOW}\",\
             \"payload\":{{}},\"passportUrl\":\"/files/abc\",\
             \"folderUrl\":\"/documents?ref=R1\",\"sheetUrl\":\"/#transactions\",\"alpha\":1}}"
        )
    );
    assert_eq!(family.ran(), 2);
    desk.finish().await;
}

/// On `/api/staff/rpc` the same answer goes out as every answer of that route
/// does: the links turned to the staff paths, the keys still in the family's
/// order — and the branch is the employee's own whatever the payload names.
#[actix_web::test]
async fn on_the_staff_route_a_familys_answer_gets_the_staff_links() {
    let desk = Desk::open().await;
    let family = Family::new("one", &["saveTransaction", "documents"], BOTH);
    let app = seam_app!(desk, vec![family.clone() as Arc<dyn RpcAction>]);
    let cookies = everybody(&desk, &app).await;

    let answer = ask(
        &app,
        &cookies,
        STAFF_RPC,
        "saveTransaction",
        "FRONT",
        json!({ "branchId": "ELSEWHERE" }),
    )
    .await;
    assert_eq!(answer.status, 200, "{:.300}", answer.text);
    assert_eq!(
        answer.text,
        format!(
            "{{\"zeta\":\"one\",\"action\":\"saveTransaction\",\"route\":\"staff\",\
             \"branch\":\"MAIN\",\"who\":\"role0\",\"now\":\"{NOW}\",\
             \"payload\":{{\"branchId\":\"ELSEWHERE\"}},\
             \"passportUrl\":\"/api/staff/file/abc\",\
             \"folderUrl\":\"/staff/documents?ref=R1\",\"sheetUrl\":\"/staff\",\"alpha\":1}}"
        )
    );
    assert_eq!(answer.header("cache-control"), Some("no-store"));

    // a name of another family of v2, another role (CA-10 #626 retires `ops*`: `documents` here)
    let answer = ask(&app, &cookies, STAFF_RPC, "documents", "MANAGER", json!({})).await;
    assert_eq!(answer.status, 200, "{:.300}", answer.text);
    assert_eq!(answer.json()["action"], "documents");
    assert_eq!(answer.json()["passportUrl"], "/api/staff/file/abc");
    assert_eq!(family.ran(), 2);
    desk.finish().await;
}

/// The role is judged before any family is asked: a caller `Policy` refuses
/// gets the route's refusal and no family hears of the request — not even its
/// `handles`.
#[actix_web::test]
async fn the_role_is_judged_before_a_family_is_asked() {
    let desk = Desk::open().await;
    let family = Family::new(
        "one",
        &["saveTransaction", "opsClock", "somethingNobodyKnows"],
        BOTH,
    );
    let app = seam_app!(desk, vec![family.clone() as Arc<dyn RpcAction>]);
    let cookies = everybody(&desk, &app).await;

    // FINANCE may not save (RBAC §11.2 #3659 widens RATES with ExchangeTransact)
    ask(&app, &cookies, RPC, "saveTransaction", "FINANCE", json!({}))
        .await
        .assert_refused(400, ROLE_NOT_ALLOWED_RPC, "FINANCE saves on /api/rpc");
    ask(
        &app,
        &cookies,
        STAFF_RPC,
        "saveTransaction",
        "FINANCE",
        json!({}),
    )
    .await
    .assert_refused(
        400,
        ROLE_NOT_ALLOWED_STAFF,
        "FINANCE saves on the staff route",
    );
    // `/api/rpc` knows no `ops*` name, for anybody
    ask(&app, &cookies, RPC, "opsClock", "OWNER", json!({}))
        .await
        .assert_refused(400, INVALID_REQUEST, "the owner asks opsClock on /api/rpc");
    ask(&app, &cookies, RPC, "opsClock", "FRONT", json!({}))
        .await
        .assert_refused(400, ROLE_NOT_ALLOWED_RPC, "FRONT asks opsClock on /api/rpc");
    // the owner has no session on the staff route
    ask(&app, &cookies, STAFF_RPC, "opsClock", "OWNER", json!({}))
        .await
        .assert_refused(401, EMPLOYEE_AUTH_REQUIRED, "the owner on the staff route");
    // a name `Policy` does not know is not a family's to answer
    ask(
        &app,
        &cookies,
        RPC,
        "somethingNobodyKnows",
        "OWNER",
        json!({}),
    )
    .await
    .assert_refused(400, INVALID_REQUEST, "a name nobody knows, the owner");
    ask(
        &app,
        &cookies,
        STAFF_RPC,
        "somethingNobodyKnows",
        "FRONT",
        json!({}),
    )
    .await
    .assert_refused(400, ROLE_NOT_ALLOWED_STAFF, "a name nobody knows, FRONT");

    assert_eq!(family.ran(), 0);
    assert_eq!(family.asked(), 0, "handles was asked for a refused caller");
    desk.finish().await;
}

/// The branch is resolved, and found to exist, before a family runs: the two
/// sentences of the dispatcher, and the family is not run.
#[actix_web::test]
async fn the_branch_is_resolved_before_a_family_runs() {
    let desk = Desk::open().await;
    let family = Family::new("one", &["saveTransaction", "rateDraft"], BOTH);
    // OR-4b (#767, design §4.2 C2): ExchangeTransact is TELLER's, so the owner's user no longer asks
    // saveTransaction; the owner's name of part two here is rateDraft (RatesDraft is EXECUTIVE's)
    let app = seam_app!(desk, vec![family.clone() as Arc<dyn RpcAction>]);
    let cookies = everybody(&desk, &app).await;

    ask(
        &app,
        &cookies,
        RPC,
        "rateDraft",
        "OWNER",
        json!({ "branchId": "NOWHERE-1" }),
    )
    .await
    .assert_refused(400, NO_SUCH_BRANCH, "a branch that does not exist");
    ask(
        &app,
        &cookies,
        RPC,
        "rateDraft",
        "OWNER",
        json!({ "branchId": "not a branch id" }),
    )
    .await
    .assert_refused(400, BAD_BRANCH, "not a branch id");
    assert_eq!(family.ran(), 0);

    // and the owner's choice of a branch that exists is what the family gets
    desk.db.seed_branch("B2", "Second").await;
    let answer = ask(
        &app,
        &cookies,
        RPC,
        "rateDraft",
        "OWNER",
        json!({ "branchId": "B2" }),
    )
    .await;
    assert_eq!(answer.status, 200, "{:.300}", answer.text);
    assert_eq!(answer.json()["branch"], "B2");
    assert_eq!(family.ran(), 1);
    desk.finish().await;
}

/// A family whose `handles` says no is not run, and the name is answered as a
/// name nobody knows — the gap, in the route's and the caller's sentence. The
/// same with no family registered at all.
#[actix_web::test]
async fn a_family_that_does_not_handle_a_name_leaves_it_to_the_gap() {
    let desk = Desk::open().await;
    let other = Family::new("other", &["documents"], BOTH);
    let none = Family::new("none", &[], BOTH);
    let app = seam_app!(
        desk,
        vec![
            none.clone() as Arc<dyn RpcAction>,
            other.clone() as Arc<dyn RpcAction>
        ]
    );
    let cookies = everybody(&desk, &app).await;

    // RBAC §11.2 (#3659): the owner holds ExchangeTransact again (via ADMIN), so the gap answers
    // saveTransaction as INVALID_REQUEST for the owner on /api/rpc
    for (path, who, sentence) in [
        (RPC, "OWNER", INVALID_REQUEST),
        (RPC, "FRONT", ROLE_NOT_ALLOWED_RPC),
        (STAFF_RPC, "FRONT", ROLE_NOT_ALLOWED_STAFF),
    ] {
        ask(&app, &cookies, path, "saveTransaction", who, json!({}))
            .await
            .assert_refused(
                400,
                sentence,
                &format!("saveTransaction on {path} as {who}"),
            );
    }
    // the gap for the owner's user, on a name it may ask
    ask(&app, &cookies, RPC, "rateDraft", "OWNER", json!({}))
        .await
        .assert_refused(400, INVALID_REQUEST, "rateDraft on /api/rpc as OWNER");
    ask(&app, &cookies, STAFF_RPC, "opsNope", "FRONT", json!({}))
        .await
        .assert_refused(400, ROLE_NOT_ALLOWED_STAFF, "opsNope");
    assert_eq!(none.ran() + other.ran(), 0);
    // every registered family was asked, each time
    // (re-expected for CA-10 #626: `opsNope` is refused by `Policy` at the gate like any name the
    // staff route does not know, design §6)
    assert_eq!(none.asked(), 4);
    assert_eq!(other.asked(), 4);

    // the one that does handle a name still answers it, behind one that does not
    let answer = ask(&app, &cookies, STAFF_RPC, "documents", "FRONT", json!({})).await;
    assert_eq!(answer.status, 200, "{:.300}", answer.text);
    assert_eq!(answer.json()["zeta"], "other");
    desk.finish().await;

    let desk = Desk::open().await;
    let app = seam_app!(desk, Vec::new());
    let cookies = everybody(&desk, &app).await;
    ask(&app, &cookies, RPC, "rateDraft", "OWNER", json!({}))
        .await
        .assert_refused(400, INVALID_REQUEST, "no family at all");
    // the gap is answered before the branch is looked at, as it always was
    ask(
        &app,
        &cookies,
        RPC,
        "rateDraft",
        "OWNER",
        json!({ "branchId": "NOWHERE-1" }),
    )
    .await
    .assert_refused(400, INVALID_REQUEST, "the gap comes before the branch");
    desk.finish().await;
}

/// `handles` is asked with the route: a family of the staff route only does
/// not answer the same name on `/api/rpc`.
#[actix_web::test]
async fn a_family_answers_only_on_the_routes_it_handles() {
    let desk = Desk::open().await;
    let family = Family::new(
        "staff only",
        &["saveTransaction", "rateDraft"],
        &[RpcRoute::StaffRpc],
    );
    // OR-4b (#767, design §4.2 C2): ExchangeTransact is TELLER's, so the owner's user no longer asks
    // saveTransaction; the owner's name of part two here is rateDraft (RatesDraft is EXECUTIVE's)
    let app = seam_app!(desk, vec![family.clone() as Arc<dyn RpcAction>]);
    let cookies = everybody(&desk, &app).await;

    ask(&app, &cookies, RPC, "rateDraft", "OWNER", json!({}))
        .await
        .assert_refused(400, INVALID_REQUEST, "on /api/rpc");
    assert_eq!(family.ran(), 0);
    let answer = ask(
        &app,
        &cookies,
        STAFF_RPC,
        "saveTransaction",
        "FRONT",
        json!({}),
    )
    .await;
    assert_eq!(answer.status, 200, "{:.300}", answer.text);
    assert_eq!(answer.json()["route"], "staff");
    assert_eq!(family.ran(), 1);
    desk.finish().await;
}

/// Of two families that both handle a name the first registered answers, and
/// the second is neither run nor asked.
#[actix_web::test]
async fn of_two_families_that_handle_a_name_the_first_registered_answers() {
    for (first, second) in [("a", "b"), ("b", "a")] {
        let desk = Desk::open().await;
        let one = Family::new(first, &["saveTransaction", "rateDraft"], BOTH);
        let two = Family::new(second, &["saveTransaction", "rateDraft"], BOTH);
        let app = seam_app!(
            desk,
            vec![
                one.clone() as Arc<dyn RpcAction>,
                two.clone() as Arc<dyn RpcAction>
            ]
        );
        let cookies = everybody(&desk, &app).await;

        // OR-4b (#767, design §4.2 C2): ExchangeTransact is TELLER's, so the owner's user no longer asks
        // saveTransaction; the owner's name of part two here is rateDraft (RatesDraft is EXECUTIVE's)
        let answer = ask(&app, &cookies, RPC, "rateDraft", "OWNER", json!({})).await;
        assert_eq!(answer.status, 200, "{:.300}", answer.text);
        assert_eq!(answer.json()["zeta"], first);
        assert_eq!((one.ran(), two.ran()), (1, 0));
        assert_eq!(two.asked(), 0, "the second family was asked");
        desk.finish().await;
    }
}

/// A name of the dispatcher's own ten is never offered to a family: one that
/// says it handles such a name changes nothing.
#[actix_web::test]
async fn a_name_of_part_one_is_never_offered_to_a_family() {
    let desk = Desk::open().await;
    let family = Family::new("thief", &rpc_common::PART_ONE, BOTH);
    let app = seam_app!(desk, vec![family.clone() as Arc<dyn RpcAction>]);
    let cookies = everybody(&desk, &app).await;

    let answer = ask(
        &app,
        &cookies,
        RPC,
        "getConnectionStatus",
        "OWNER",
        json!({}),
    )
    .await;
    assert_eq!(answer.status, 200, "{:.300}", answer.text);
    assert_eq!(
        answer.text,
        "{\"ready\":true,\"sheetUrl\":\"/#transactions\",\"folderUrl\":\"/documents\"}"
    );
    for (path, who) in [(RPC, "OWNER"), (STAFF_RPC, "FRONT")] {
        let answer = ask(&app, &cookies, path, "adminSnapshot", who, json!({})).await;
        assert_eq!(answer.status, 200, "{path}: {:.300}", answer.text);
        assert!(answer.json().get("zeta").is_none(), "{path}");
        assert_eq!(answer.json()["syncedAt"], NOW, "{path}");
    }
    assert_eq!(family.ran(), 0);
    assert_eq!(family.asked(), 0, "a part-one name was offered to a family");
    desk.finish().await;
}

/// What a family refuses or breaks goes out as it does for any other action: a
/// use case's sentence is a 400 refusal, a refusal of the family's own making
/// goes out as made, and anything else is the server-error sentence with the
/// cause untold.
#[actix_web::test]
async fn what_a_family_refuses_or_breaks_goes_out_as_for_any_other_action() {
    let desk = Desk::open().await;
    let app = seam_app!(
        desk,
        vec![
            Family::that(
                "domain",
                &["rateDraft"],
                BOTH,
                Does::RefusesAsDomain("ลงเวลาเข้าก่อนทำรายการ")
            ) as Arc<dyn RpcAction>,
            Family::that(
                "own",
                &["saveTransaction"],
                BOTH,
                Does::Refuses(409, "มีรายการค้างอยู่")
            ) as Arc<dyn RpcAction>,
            Family::that("broken", &["documents"], BOTH, Does::Breaks) as Arc<dyn RpcAction>,
        ]
    );
    let cookies = everybody(&desk, &app).await;

    ask(&app, &cookies, STAFF_RPC, "rateDraft", "RATES", json!({}))
        .await
        .assert_refused(400, "ลงเวลาเข้าก่อนทำรายการ", "a use case's sentence");
    ask(
        &app,
        &cookies,
        STAFF_RPC,
        "saveTransaction",
        "FRONT",
        json!({}),
    )
    .await
    .assert_refused(409, "มีรายการค้างอยู่", "a refusal of the family's own");
    let answer = ask(&app, &cookies, STAFF_RPC, "documents", "FRONT", json!({})).await;
    assert_eq!(answer.status, 500, "{:.300}", answer.text);
    assert_eq!(answer.json(), json!({ "error": SERVER_ERROR }));
    assert!(!answer.text.contains("cause"), "the cause is not told");
    desk.finish().await;
}
