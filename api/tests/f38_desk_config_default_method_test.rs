//! #564 F38, owner ruling R5 (A) (2026-11-06 07:56 ICT, #564 issuecomment-6673699296): "The desk reads the
//! kiosk's owner-set defaultPaymentMethod (one setting, default Cash). Cash needs no account name."
//!
//! Written before the implementation by the ticket author (claude/33691c06). Do not edit, delete or weaken
//! these tests; add your own in a file of your own.
//!
//! - The desk config view (`desk_config` and `staff_desk_config`, `api/src/presentation/http/v3/desk/config.rs`)
//!   answers `defaultPaymentMethod`: the branch's `kioskScreen` setting (`api/src/application/kiosk_screen.rs`,
//!   the one the kiosk reads), read through the same reader, so a missing, blank or unreadable value answers
//!   `"Cash"` (`DEFAULT_PAYMENT_METHOD`) and a set value answers trimmed.
//! - The setting is per branch: a row in `MAIN` does not change `B9`.
//! - Both paths answer the same value (one core, D-11).
//!
//! The assertions read the JSON (`data["defaultPaymentMethod"]`), so the file compiles before the field
//! exists on `DeskConfig`. Needs `TEST_DATABASE_URL`; nothing skips.

mod common;
mod compat_common;
#[macro_use]
mod rpc_common;
mod shift_setup_common;
mod snapshot_common;

use actix_web::http::Method;
use rpc_common::{App, Call, Cookies, Desk, one_of_each_role, sign_in};
use serde_json::{Value, json};

const ORIGIN: &str = "http://desk.test";
const OWNER_VIEW: &str = "desk_config";
const STAFF_VIEW: &str = "staff_desk_config";
const MAIN: &str = "MAIN";
const B9: &str = "B9";
const FIELD: &str = "defaultPaymentMethod";

struct Room {
    desk: Desk,
    app: App,
    cookies: Cookies,
}

impl Room {
    /// `MAIN` (from the migration) and `B9`; the owner and one employee of every staff role in `MAIN`.
    async fn open() -> Room {
        let desk = Desk::open().await;
        desk.db.seed_branch(B9, "Nine").await;
        let app = rpc_app!(desk);
        let people = one_of_each_role();
        desk.employ(&people).await;
        let cookies = sign_in(&app, ORIGIN, &people).await;
        Room { desk, app, cookies }
    }

    /// The view's `data` as `who`.
    async fn data(&self, who: &str, view: &str, query: &str) -> Value {
        let path = if query.is_empty() {
            format!("/api/v3/views/{view}")
        } else {
            format!("/api/v3/views/{view}?{query}")
        };
        let mut call = Call::post(ORIGIN, &path, Vec::<u8>::new())
            .without("x-xchange-request")
            .without("content-type")
            .without("origin");
        call.method = Method::GET;
        let answer = self.app.send(call.cookie(&self.cookies[who])).await;
        assert_eq!(
            answer.status, 200,
            "GET {path} as {who}: {:.400}",
            answer.text
        );
        let body = answer.json();
        assert_eq!(body["changed"], json!(true), "GET {path}: {body}");
        body.get("data")
            .cloned()
            .unwrap_or_else(|| panic!("GET {path}: data in {body}"))
    }

    /// The owner's view of `branch` and the FRONT employee's view of MAIN.
    async fn owner(&self, branch: &str) -> Value {
        self.data("OWNER", OWNER_VIEW, &format!("branch={branch}"))
            .await
    }

    async fn front(&self) -> Value {
        self.data("FRONT", STAFF_VIEW, "").await
    }

    /// Writes `branch`'s `kioskScreen` row, as an owner save would (no save screen exists yet).
    async fn set_kiosk_screen(&self, branch: &str, body: Value) {
        sqlx::query(
            "INSERT INTO branch_settings (branch_id, name, version, body, updated_at, updated_by) \
             VALUES ($1, 'kioskScreen', 1, $2, now(), 'owner') \
             ON CONFLICT (branch_id, name) DO UPDATE \
               SET body = EXCLUDED.body, version = branch_settings.version + 1, updated_at = now()",
        )
        .bind(branch)
        .bind(body)
        .execute(&self.desk.db.pool)
        .await
        .expect("write the kioskScreen row");
    }

    async fn finish(self) {
        self.desk.finish().await;
    }
}

/// A branch with no `kioskScreen` row answers "Cash" on both paths.
#[actix_web::test]
async fn f38_not_set_answers_cash_on_both_paths() {
    let room = Room::open().await;
    let owner = room.owner(MAIN).await;
    assert_eq!(owner[FIELD], json!("Cash"), "not set: Cash. {owner}");
    let front = room.front().await;
    assert_eq!(front[FIELD], json!("Cash"), "staff path: Cash. {front}");
    room.finish().await;
}

/// The owner-set value is answered, on both paths, and only for its own branch.
#[actix_web::test]
async fn f38_the_branch_setting_is_answered_for_its_branch_only() {
    let room = Room::open().await;
    room.set_kiosk_screen(MAIN, json!({ "defaultPaymentMethod": "Bank Transfer" }))
        .await;
    let owner = room.owner(MAIN).await;
    assert_eq!(owner[FIELD], json!("Bank Transfer"), "{owner}");
    let front = room.front().await;
    assert_eq!(front[FIELD], json!("Bank Transfer"), "{front}");
    let nine = room.owner(B9).await;
    assert_eq!(
        nine[FIELD],
        json!("Cash"),
        "B9 has no row of its own: {nine}"
    );
    room.finish().await;
}

/// The desk reads the kiosk's value through the kiosk's reader: trimmed when set, "Cash" when the field is
/// missing or blank.
#[actix_web::test]
async fn f38_missing_or_blank_answers_cash_and_a_set_value_is_trimmed() {
    let room = Room::open().await;
    for (body, expected) in [
        (json!({ "tiles": 9 }), "Cash"),
        (json!({ "defaultPaymentMethod": "   " }), "Cash"),
        (json!({ "defaultPaymentMethod": 7 }), "Cash"),
        (
            json!({ "defaultPaymentMethod": "  PromptPay  " }),
            "PromptPay",
        ),
    ] {
        room.set_kiosk_screen(MAIN, body.clone()).await;
        let owner = room.owner(MAIN).await;
        assert_eq!(owner[FIELD], json!(expected), "row {body}: {owner}");
    }
    room.finish().await;
}

// ── the generated type ───────────────────────────────────────────────────────

/// A scratch directory per call: the type tests run on parallel threads of one binary.
struct Scratch(std::path::PathBuf);

impl Scratch {
    fn new() -> Self {
        static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("f38-desk-config-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create a scratch directory");
        Self(dir)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn exported() -> String {
    let view = xchange::presentation::http::v3::VIEWS
        .iter()
        .find(|v| v.name == OWNER_VIEW)
        .unwrap_or_else(|| panic!("{OWNER_VIEW} is in the generated view table"));
    let scratch = Scratch::new();
    (view.export_types)(&scratch.0).unwrap_or_else(|e| panic!("export {OWNER_VIEW}: {e}"));
    std::fs::read_to_string(scratch.0.join("desk_config.ts"))
        .unwrap_or_else(|e| panic!("{OWNER_VIEW} writes desk_config.ts: {e}"))
}

/// The web reads the field through the generated `DeskConfig`, a plain string.
#[test]
fn f38_the_generated_desk_config_carries_the_default_method() {
    let ts = exported();
    assert!(
        ts.contains("defaultPaymentMethod: string"),
        "DeskConfig has defaultPaymentMethod: string:\n{ts}"
    );
}

/// The committed file is what the DTO generates.
#[test]
fn f38_the_generated_desk_config_file_is_committed() {
    let file = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../web/src/shared/api/generated/desk_config.ts");
    let committed = std::fs::read_to_string(&file)
        .unwrap_or_else(|e| panic!("web/src/shared/api/generated/desk_config.ts: {e}"));
    assert_eq!(
        committed,
        exported(),
        "desk_config.ts is what the DTO generates"
    );
}
